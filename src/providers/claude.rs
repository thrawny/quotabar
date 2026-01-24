use crate::models::{CostSnapshot, IdentitySnapshot, Provider, RateWindow, UsageSnapshot};
use crate::providers::ProviderFetcher;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::PathBuf;

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
    monthly_limit: Option<i64>,
    used_credits: Option<i64>,
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
}

impl Default for ClaudeProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProviderFetcher for ClaudeProvider {
    async fn fetch(&self) -> Result<UsageSnapshot> {
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
            let mut used = e.used_credits.unwrap_or(0) as f64 / 100.0;
            let mut limit = e.monthly_limit.unwrap_or(0) as f64 / 100.0;

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

    fn name(&self) -> &'static str {
        "Claude"
    }
}

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

fn format_reset_time(reset: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let duration = reset.signed_duration_since(now);
    let hours = duration.num_hours();
    let minutes = duration.num_minutes() % 60;

    if hours <= 0 && minutes <= 0 {
        "now".to_string()
    } else if hours < 1 {
        format!("in {} min", minutes.max(1))
    } else if hours < 24 {
        format!("in {}h", hours)
    } else {
        let days = hours / 24;
        if days == 1 {
            "in 1 day".to_string()
        } else {
            format!("in {} days", days)
        }
    }
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

        let reset = now + chrono::Duration::minutes(30);
        assert_eq!(format_reset_time(reset, now), "in 30 min");

        let reset = now + chrono::Duration::days(3);
        assert_eq!(format_reset_time(reset, now), "in 3 days");
    }
}
