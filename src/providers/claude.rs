use crate::models::{CostSnapshot, IdentitySnapshot, Provider, RateWindow, UsageSnapshot};
use crate::providers::format_reset_time;
use crate::providers::ProviderFetcher;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Stdio;

const API_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const USER_AGENT: &str = "quotabar";

/// Claude Code credentials from ~/.claude/.credentials.json
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsFile {
    claude_ai_oauth: Option<OAuthCredentials>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthCredentials {
    access_token: String,
    #[allow(dead_code)]
    refresh_token: Option<String>,
    /// Unix timestamp in milliseconds
    expires_at: Option<i64>,
    #[allow(dead_code)]
    scopes: Option<Vec<String>>,
    rate_limit_tier: Option<String>,
}

impl OAuthCredentials {
    fn is_expired(&self) -> bool {
        if let Some(expires_at_ms) = self.expires_at {
            let expires_at = expires_at_ms / 1000;
            let now = Utc::now().timestamp();
            now >= expires_at
        } else {
            false
        }
    }

    fn plan_name(&self) -> Option<String> {
        self.rate_limit_tier.as_ref().map(|tier| {
            let lower = tier.to_lowercase();
            if lower.contains("enterprise") {
                "Enterprise"
            } else if lower.contains("team") {
                "Team"
            } else if lower.contains("max") {
                "Max"
            } else if lower.contains("pro") {
                "Pro"
            } else if lower.contains("free") {
                "Free"
            } else {
                return tier.clone();
            }
            .to_string()
        })
    }
}

/// API response from /api/oauth/usage
#[derive(Debug, Deserialize)]
struct UsageResponse {
    five_hour: Option<RateWindowResponse>,
    seven_day: Option<RateWindowResponse>,
    #[allow(dead_code)]
    seven_day_oauth_apps: Option<RateWindowResponse>,
    seven_day_opus: Option<RateWindowResponse>,
    seven_day_sonnet: Option<RateWindowResponse>,
    extra_usage: Option<ExtraUsageResponse>,
}

#[derive(Debug, Deserialize)]
struct RateWindowResponse {
    utilization: f64,
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtraUsageResponse {
    is_enabled: bool,
    monthly_limit: Option<f64>,
    used_credits: Option<f64>,
    #[allow(dead_code)]
    utilization: Option<f64>,
    currency: Option<String>,
}

pub struct ClaudeProvider {
    client: reqwest::Client,
}

impl ClaudeProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    fn credentials_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude")
            .join(".credentials.json")
    }

    fn load_credentials() -> Result<OAuthCredentials> {
        let path = Self::credentials_path();
        if !path.exists() {
            return Err(anyhow!(
                "Claude credentials not found at {}. Run `claude login` first.",
                path.display()
            ));
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let creds: CredentialsFile =
            serde_json::from_str(&content).with_context(|| "Failed to parse credentials JSON")?;

        creds
            .claude_ai_oauth
            .ok_or_else(|| anyhow!("No OAuth credentials found. Run `claude login` first."))
    }

    async fn fetch_usage(&self, token: &str) -> Result<UsageResponse> {
        let response = self
            .client
            .get(API_URL)
            .header("Authorization", format!("Bearer {}", token))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .context("Failed to connect to Anthropic API")?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "Claude OAuth token expired or invalid. Run `claude login` to refresh."
            ));
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!(
                "Claude OAuth token missing required scope. Run `claude login` to refresh."
            ));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Anthropic API error ({}): {}", status, body));
        }

        response
            .json()
            .await
            .context("Failed to parse usage response")
    }

    async fn fetch_via_api(&self) -> Result<UsageSnapshot> {
        let creds = Self::load_credentials()?;

        if creds.is_expired() {
            return Err(anyhow!(
                "Claude OAuth token expired. Run `claude login` to refresh."
            ));
        }

        let usage = self.fetch_usage(&creds.access_token).await?;
        let now = Utc::now();

        // Primary: 5-hour session window
        let primary = usage.five_hour.map(|w| RateWindow {
            used_percent: w.utilization,
            window_minutes: Some(300),
            resets_at: w.resets_at.as_ref().and_then(|s| parse_iso8601(s)),
            reset_description: w
                .resets_at
                .as_ref()
                .and_then(|s| parse_iso8601(s))
                .map(|dt| format_reset_time(dt, now)),
        });

        // Secondary: 7-day window
        let secondary = usage.seven_day.map(|w| RateWindow {
            used_percent: w.utilization,
            window_minutes: Some(10080),
            resets_at: w.resets_at.as_ref().and_then(|s| parse_iso8601(s)),
            reset_description: w
                .resets_at
                .as_ref()
                .and_then(|s| parse_iso8601(s))
                .map(|dt| format_reset_time(dt, now)),
        });

        // Tertiary: Model-specific (prefer Sonnet, fallback to Opus)
        let model_window = usage.seven_day_sonnet.or(usage.seven_day_opus);
        let tertiary = model_window.map(|w| RateWindow {
            used_percent: w.utilization,
            window_minutes: Some(10080),
            resets_at: w.resets_at.as_ref().and_then(|s| parse_iso8601(s)),
            reset_description: w
                .resets_at
                .as_ref()
                .and_then(|s| parse_iso8601(s))
                .map(|dt| format_reset_time(dt, now)),
        });

        // Cost: Extra usage (credits in cents)
        let cost = usage.extra_usage.and_then(|e| {
            if !e.is_enabled {
                return None;
            }
            let mut used = e.used_credits.unwrap_or(0.0) / 100.0;
            let mut limit = e.monthly_limit.unwrap_or(0.0) / 100.0;

            // Rescale heuristic for non-enterprise plans
            let is_enterprise = creds
                .rate_limit_tier
                .as_ref()
                .map(|t| t.to_lowercase() == "enterprise")
                .unwrap_or(false);
            if !is_enterprise && limit >= 1000.0 {
                used /= 100.0;
                limit /= 100.0;
            }

            Some(CostSnapshot {
                used,
                limit,
                currency_code: e.currency.unwrap_or_else(|| "USD".to_string()),
                period: Some("Monthly".to_string()),
                resets_at: None,
            })
        });

        Ok(UsageSnapshot {
            provider: Provider::Claude,
            primary,
            secondary,
            tertiary,
            cost,
            identity: Some(IdentitySnapshot {
                email: None,
                plan: creds.plan_name(),
                organization: None,
            }),
            updated_at: now,
        })
    }

    async fn fetch_via_cli(&self) -> Result<UsageSnapshot> {
        let raw = Self::run_cli_usage().await?;
        let usage = parse_cli_usage(&raw)?;
        let plan = Self::load_credentials().ok().and_then(|c| c.plan_name());
        let now = Utc::now();

        Ok(UsageSnapshot {
            provider: Provider::Claude,
            primary: usage.session.map(|w| RateWindow {
                used_percent: w.used_percent,
                window_minutes: Some(300),
                resets_at: None,
                reset_description: w.reset_description,
            }),
            secondary: usage.weekly.map(|w| RateWindow {
                used_percent: w.used_percent,
                window_minutes: Some(10080),
                resets_at: None,
                reset_description: w.reset_description,
            }),
            tertiary: usage.model.map(|w| RateWindow {
                used_percent: w.used_percent,
                window_minutes: Some(10080),
                resets_at: None,
                reset_description: w.reset_description,
            }),
            cost: None,
            identity: Some(IdentitySnapshot {
                email: None,
                plan,
                organization: None,
            }),
            updated_at: now,
        })
    }

    async fn run_cli_usage() -> Result<String> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            tokio::process::Command::new("timeout")
                .args([
                    "15",
                    "script",
                    "-qfc",
                    r#"printf "/usage\n" | claude --allowed-tools """#,
                    "/dev/null",
                ])
                .env_remove("CLAUDECODE")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| anyhow!("CLI usage probe timed out"))?
        .context("Failed to execute claude CLI")?;

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Default for ClaudeProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProviderFetcher for ClaudeProvider {
    async fn fetch(&self) -> Result<UsageSnapshot> {
        match self.fetch_via_api().await {
            Ok(snapshot) => Ok(snapshot),
            Err(api_err) => {
                eprintln!("Claude API failed ({}), trying CLI fallback...", api_err);
                self.fetch_via_cli().await.map_err(|cli_err| {
                    anyhow!("{}\nCLI fallback also failed: {}", api_err, cli_err)
                })
            }
        }
    }

    fn name(&self) -> &'static str {
        "Claude"
    }
}

// --- CLI output parsing ---

struct CliWindow {
    used_percent: f64,
    reset_description: Option<String>,
}

struct CliUsage {
    session: Option<CliWindow>,
    weekly: Option<CliWindow>,
    model: Option<CliWindow>,
}

/// Strip ANSI escape sequences (CSI and OSC) from terminal output.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some(&'[') => {
                    chars.next();
                    // CSI sequence: skip params until final byte (letter, @, ~)
                    let mut final_byte = None;
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() || c == '@' || c == '~' {
                            final_byte = Some(c);
                            break;
                        }
                    }
                    // Cursor movement (C = right, B = down) → emit space
                    if final_byte == Some('C') || final_byte == Some('B') {
                        result.push(' ');
                    }
                }
                Some(&']') => {
                    chars.next();
                    // OSC sequence: skip until BEL or ST (ESC \)
                    let mut prev = '\0';
                    for c in chars.by_ref() {
                        if c == '\x07' {
                            break;
                        }
                        if prev == '\x1b' && c == '\\' {
                            break;
                        }
                        prev = c;
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else if c == '\r' {
            // skip carriage return
        } else {
            result.push(c);
        }
    }
    result
}

/// Normalize text for label matching: lowercase and strip all whitespace.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Isolate the usage panel by finding the last "Settings:" header.
/// This avoids matching percentages from the status bar.
fn trim_to_usage_panel(text: &str) -> &str {
    let lower = text.to_lowercase();
    if let Some(pos) = lower.rfind("settings:") {
        let tail = &text[pos..];
        let lower_tail = &lower[pos..];
        if lower_tail.contains("usage") && tail.contains('%') {
            return tail;
        }
    }
    text
}

/// Parse the stripped CLI /usage output into usage windows.
fn parse_cli_usage(raw: &str) -> Result<CliUsage> {
    let clean = strip_ansi(raw);

    let panel = trim_to_usage_panel(&clean);
    let lines: Vec<&str> = panel.lines().collect();

    let session = extract_cli_window(&lines, &["Current session"]);
    let weekly = extract_cli_window(&lines, &["Current week (all models)"]);
    let model = extract_cli_window(
        &lines,
        &[
            "Current week (Sonnet only)",
            "Current week (Sonnet)",
            "Current week (Opus)",
        ],
    );

    if session.is_none() {
        if let Some(cli_error) = extract_cli_usage_error(&clean) {
            return Err(anyhow!(cli_error));
        }
        return Err(anyhow!(
            "Could not parse CLI usage output: 'Current session' not found"
        ));
    }

    Ok(CliUsage {
        session,
        weekly,
        model,
    })
}

/// Return a user-actionable error when Claude CLI rendered an error instead of the usage panel.
fn extract_cli_usage_error(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();

    if lower.contains("rate_limit_error")
        || lower.contains("rate limited")
        || compact.contains("ratelimited")
    {
        return Some("Claude CLI usage endpoint is rate limited right now. Please try again later.");
    }
    if lower.contains("token_expired") || lower.contains("token has expired") {
        return Some("Claude CLI token expired. Run `claude login` to refresh.");
    }
    if lower.contains("authentication_error") {
        return Some("Claude CLI authentication error. Run `claude login`.");
    }
    if lower.contains("failed to load usage data") || compact.contains("failedtoloadusagedata") {
        return Some("Claude CLI could not load usage data. Open the CLI and retry `/usage`.");
    }

    None
}

/// Find a labeled section and extract its percentage and reset text.
fn extract_cli_window(lines: &[&str], labels: &[&str]) -> Option<CliWindow> {
    // Find the last line matching any label (normalized matching for robustness)
    let label_idx = labels.iter().find_map(|label| {
        let normalized_label = normalize(label);
        lines
            .iter()
            .rposition(|line| normalize(line).contains(&normalized_label))
    })?;

    let mut used_percent = None;
    let mut reset_description = None;

    // Scan up to 10 lines after the label for percentage and reset info
    for line in lines.iter().skip(label_idx).take(10) {
        if used_percent.is_none() {
            used_percent = extract_percent(line);
        }
        if reset_description.is_none() && normalize(line).contains("reset") {
            reset_description = Some(line.trim().to_string());
        }
        // Stop at the next section label
        if used_percent.is_some() {
            let norm = normalize(line);
            if norm.contains("currentweek") || norm.contains("extrausage") {
                break;
            }
        }
    }

    Some(CliWindow {
        used_percent: used_percent?,
        reset_description,
    })
}

/// Extract a percentage (0-100) from a line containing "N%".
fn extract_percent(line: &str) -> Option<f64> {
    let chars: Vec<char> = line.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '%' {
            // Walk backwards to collect the number
            let mut j = i;
            while j > 0 && (chars[j - 1].is_ascii_digit() || chars[j - 1] == '.') {
                j -= 1;
            }
            if j < i {
                let num_str: String = chars[j..i].iter().collect();
                if let Ok(n) = num_str.parse::<f64>() {
                    if (0.0..=100.0).contains(&n) {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

// --- Existing parsing helpers ---

fn parse_iso8601(s: &str) -> Option<DateTime<Utc>> {
    // Try with fractional seconds first, then without
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ")
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso8601() {
        let dt = parse_iso8601("2024-01-15T10:30:00.000Z");
        assert!(dt.is_some());

        let dt = parse_iso8601("2024-01-15T10:30:00Z");
        assert!(dt.is_some());
    }

    #[test]
    fn test_format_reset_time() {
        let now = Utc::now();
        let reset = now + chrono::Duration::hours(5);
        assert_eq!(format_reset_time(reset, now), "in 5h");

        let reset = now + chrono::Duration::hours(1) + chrono::Duration::minutes(29);
        assert_eq!(format_reset_time(reset, now), "in 1h 29m");

        let reset = now + chrono::Duration::minutes(30);
        assert_eq!(format_reset_time(reset, now), "in 30 min");

        let reset = now + chrono::Duration::days(3);
        assert_eq!(format_reset_time(reset, now), "in 3 days");
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[1mhello\x1b[0m"), "hello");
        assert_eq!(strip_ansi("\x1b[38;2;255;0;0mred\x1b[39m"), "red");
        assert_eq!(strip_ansi("no ansi here"), "no ansi here");
        assert_eq!(strip_ansi("line\rone"), "lineone");
        // Cursor right (CSI C) becomes space
        assert_eq!(strip_ansi("hello\x1b[1Cworld"), "hello world");
    }

    #[test]
    fn test_extract_percent() {
        assert_eq!(extract_percent("11% used"), Some(11.0));
        assert_eq!(extract_percent("11%used"), Some(11.0));
        assert_eq!(extract_percent("████░░░░ 85% used"), Some(85.0));
        assert_eq!(extract_percent("no percentage here"), None);
        assert_eq!(extract_percent("200% is too high"), None);
    }

    #[test]
    fn test_parse_cli_usage_clean() {
        let input = "\
Settings: Status Config  Usage  (tab to cycle)

  Current session
  ██████░░░░░░  11% used
  Resets 1pm

  Current week (all models)
  █░░░░░░░░░░░  1% used
  Resets Mar 20, 8am

  Current week (Sonnet only)
  █░░░░░░░░░░░  2% used
  Resets Mar 19, 10am
";
        let result = parse_cli_usage(input).unwrap();
        assert_eq!(result.session.as_ref().unwrap().used_percent, 11.0);
        assert_eq!(result.weekly.as_ref().unwrap().used_percent, 1.0);
        assert_eq!(result.model.as_ref().unwrap().used_percent, 2.0);
    }

    #[test]
    fn test_parse_cli_usage_with_ansi() {
        // Simulates ANSI-laden output where CSI sequences separate words
        let input =
            "\x1b[1mSettings:\x1b[22m Status Config \x1b[48;2;153;204;255m Usage \x1b[49m\n\n\
            \x1b[1mCurrent session\x1b[22m\n\
            \x1b[48;2;69;92;115m████░░░░\x1b[49m11%\x1b[1Cused\n\
            \x1b[38;2;153;153;153mResets 1pm\x1b[39m\n\n\
            \x1b[1mCurrent\x1b[1Cweek\x1b[1C(all\x1b[1Cmodels)\x1b[22m\n\
            \x1b[48;2;69;92;115m█░░░░\x1b[49m1%\x1b[1Cused\n\
            \x1b[38;2;153;153;153mResets Mar 20\x1b[39m\n";
        let result = parse_cli_usage(input).unwrap();
        assert_eq!(result.session.as_ref().unwrap().used_percent, 11.0);
        assert_eq!(result.weekly.as_ref().unwrap().used_percent, 1.0);
    }

    #[test]
    fn test_parse_cli_usage_missing_session() {
        let input = "Some random output without usage data\n";
        assert!(parse_cli_usage(input).is_err());
    }

    #[test]
    fn test_parse_cli_usage_rate_limit_error() {
        let input = r#"Anthropic API error (429 Too Many Requests): {
  "error": {
    "type": "rate_limit_error",
    "message": "Rate limited. Please try again later."
  }
}"#;
        let err = parse_cli_usage(input).err().unwrap().to_string();
        assert_eq!(
            err,
            "Claude CLI usage endpoint is rate limited right now. Please try again later."
        );
    }

    #[test]
    fn test_parse_cli_usage_keeps_partial_quota_when_breakdown_rate_limited() {
        let input = "\
Settings Status Config Usage Stats

Current session
████░░░░░░ 28% used
Resets 3:10pm (Europe/Stockholm)

Current week (all models)
█░░░░░░░░░ 3% used
Resets Jul 11, 8am (Europe/Stockholm)

Per-model breakdown unavailable (rate limited — try again in a moment)
";
        let result = parse_cli_usage(input).unwrap();
        assert_eq!(result.session.as_ref().unwrap().used_percent, 28.0);
        assert_eq!(result.weekly.as_ref().unwrap().used_percent, 3.0);
    }

    #[test]
    fn test_parse_cli_usage_authentication_error() {
        let input = r#"{"type":"authentication_error","message":"invalid token"}"#;
        let err = parse_cli_usage(input).err().unwrap().to_string();
        assert_eq!(err, "Claude CLI authentication error. Run `claude login`.");
    }

    #[test]
    fn test_normalize() {
        assert_eq!(
            normalize("Current week (all models)"),
            "currentweek(allmodels)"
        );
        assert_eq!(normalize("  Hello World  "), "helloworld");
    }
}
