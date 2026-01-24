use crate::models::{IdentitySnapshot, Provider, RateWindow, UsageSnapshot};
use crate::providers::ProviderFetcher;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::env;
use std::path::PathBuf;

const DEFAULT_CHATGPT_BASE_URL: &str = "https://chatgpt.com/backend-api";
const CHATGPT_USAGE_PATH: &str = "/wham/usage";
const CODEX_USAGE_PATH: &str = "/api/codex/usage";
const USER_AGENT: &str = "quotabar";

#[derive(Debug, Deserialize)]
struct AuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<AuthTokens>,
}

#[derive(Debug, Deserialize)]
struct AuthTokens {
    access_token: String,
    #[allow(dead_code)]
    refresh_token: Option<String>,
    id_token: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug)]
struct Credentials {
    access_token: String,
    id_token: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    plan_type: Option<String>,
    rate_limit: Option<RateLimitDetails>,
    #[allow(dead_code)]
    credits: Option<CreditDetails>,
}

#[derive(Debug, Deserialize)]
struct RateLimitDetails {
    primary_window: Option<WindowSnapshot>,
    secondary_window: Option<WindowSnapshot>,
}

#[derive(Debug, Deserialize)]
struct WindowSnapshot {
    used_percent: i64,
    reset_at: i64,
    limit_window_seconds: i64,
}

#[derive(Debug, Deserialize)]
struct CreditDetails {
    #[allow(dead_code)]
    has_credits: Option<bool>,
    #[allow(dead_code)]
    unlimited: Option<bool>,
    #[allow(dead_code)]
    #[serde(default, deserialize_with = "deserialize_balance_opt")]
    balance: Option<f64>,
}

pub struct CodexProvider {
    client: reqwest::Client,
}

impl CodexProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    fn credentials_path() -> PathBuf {
        if let Ok(codex_home) = env::var("CODEX_HOME") {
            let trimmed = codex_home.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join("auth.json");
            }
        }

        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codex")
            .join("auth.json")
    }

    fn config_path() -> PathBuf {
        if let Ok(codex_home) = env::var("CODEX_HOME") {
            let trimmed = codex_home.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join("config.toml");
            }
        }

        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".codex")
            .join("config.toml")
    }

    fn load_credentials() -> Result<Credentials> {
        let path = Self::credentials_path();
        if !path.exists() {
            return Err(anyhow!(
                "Codex credentials not found at {}. Run `codex` first.",
                path.display()
            ));
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let auth: AuthFile =
            serde_json::from_str(&content).with_context(|| "Failed to parse auth.json")?;

        if let Some(api_key) = auth
            .openai_api_key
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            return Ok(Credentials {
                access_token: api_key,
                id_token: None,
                account_id: None,
            });
        }

        let tokens = auth
            .tokens
            .ok_or_else(|| anyhow!("Codex auth.json missing tokens. Run `codex` to log in."))?;

        if tokens.access_token.trim().is_empty() {
            return Err(anyhow!(
                "Codex auth.json missing access token. Run `codex` to log in."
            ));
        }

        Ok(Credentials {
            access_token: tokens.access_token,
            id_token: tokens.id_token,
            account_id: tokens.account_id,
        })
    }

    fn resolve_usage_url() -> reqwest::Url {
        let base = Self::resolve_chatgpt_base_url();
        let normalized = Self::normalize_chatgpt_base_url(&base);
        let path = if normalized.contains("/backend-api") {
            CHATGPT_USAGE_PATH
        } else {
            CODEX_USAGE_PATH
        };
        let full = format!("{}{}", normalized, path);
        reqwest::Url::parse(&full).unwrap_or_else(|_| {
            reqwest::Url::parse(&format!(
                "{}{}",
                DEFAULT_CHATGPT_BASE_URL, CHATGPT_USAGE_PATH
            ))
            .expect("default Codex usage URL is valid")
        })
    }

    fn resolve_chatgpt_base_url() -> String {
        if let Ok(contents) = std::fs::read_to_string(Self::config_path()) {
            if let Some(parsed) = Self::parse_chatgpt_base_url(&contents) {
                return parsed;
            }
        }
        DEFAULT_CHATGPT_BASE_URL.to_string()
    }

    fn parse_chatgpt_base_url(contents: &str) -> Option<String> {
        let value: toml::Value = toml::from_str(contents).ok()?;
        value
            .get("chatgpt_base_url")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn normalize_chatgpt_base_url(value: &str) -> String {
        let mut trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            trimmed = DEFAULT_CHATGPT_BASE_URL.to_string();
        }
        while trimmed.ends_with('/') {
            trimmed.pop();
        }
        if (trimmed.starts_with("https://chatgpt.com")
            || trimmed.starts_with("https://chat.openai.com"))
            && !trimmed.contains("/backend-api")
        {
            trimmed.push_str("/backend-api");
        }
        trimmed
    }

    async fn fetch_usage(&self, creds: &Credentials) -> Result<UsageResponse> {
        let url = Self::resolve_usage_url();
        let mut request = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", creds.access_token))
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT);

        if let Some(account_id) = creds
            .account_id
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            request = request.header("ChatGPT-Account-Id", account_id);
        }

        let response = request
            .send()
            .await
            .context("Failed to connect to Codex usage API")?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!(
                "Codex OAuth token expired or invalid. Run `codex` to re-authenticate."
            ));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Codex API error ({}): {}", status, body));
        }

        response
            .json()
            .await
            .context("Failed to parse Codex usage response")
    }

    fn make_window(window: Option<&WindowSnapshot>, now: DateTime<Utc>) -> Option<RateWindow> {
        let window = window?;
        let reset = Utc.timestamp_opt(window.reset_at, 0).single();
        let reset_description = reset.map(|dt| format_reset_time(dt, now));
        let minutes = (window.limit_window_seconds / 60) as i32;
        Some(RateWindow {
            used_percent: window.used_percent as f64,
            window_minutes: Some(minutes),
            resets_at: reset,
            reset_description,
        })
    }

    fn resolve_identity(creds: &Credentials, response: &UsageResponse) -> Option<IdentitySnapshot> {
        let payload = creds.id_token.as_deref().and_then(parse_jwt_payload);

        let email = payload
            .as_ref()
            .and_then(|p| p.get("email"))
            .and_then(Value::as_str)
            .or_else(|| {
                payload
                    .as_ref()
                    .and_then(|p| p.get("https://api.openai.com/profile"))
                    .and_then(Value::as_object)
                    .and_then(|obj| obj.get("email"))
                    .and_then(Value::as_str)
            })
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let plan = response
            .plan_type
            .as_deref()
            .and_then(normalize_plan_name)
            .or_else(|| {
                payload
                    .as_ref()
                    .and_then(|p| p.get("https://api.openai.com/auth"))
                    .and_then(Value::as_object)
                    .and_then(|obj| obj.get("chatgpt_plan_type"))
                    .and_then(Value::as_str)
                    .and_then(normalize_plan_name)
            })
            .or_else(|| {
                payload
                    .as_ref()
                    .and_then(|p| p.get("chatgpt_plan_type"))
                    .and_then(Value::as_str)
                    .and_then(normalize_plan_name)
            });

        if email.is_none() && plan.is_none() {
            return None;
        }

        Some(IdentitySnapshot {
            email,
            plan,
            organization: None,
        })
    }
}

impl Default for CodexProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProviderFetcher for CodexProvider {
    async fn fetch(&self) -> Result<UsageSnapshot> {
        let creds = Self::load_credentials()?;
        let usage = self.fetch_usage(&creds).await?;
        let now = Utc::now();

        let primary = usage
            .rate_limit
            .as_ref()
            .and_then(|r| Self::make_window(r.primary_window.as_ref(), now));
        let secondary = usage
            .rate_limit
            .as_ref()
            .and_then(|r| Self::make_window(r.secondary_window.as_ref(), now));

        Ok(UsageSnapshot {
            provider: Provider::Codex,
            primary,
            secondary,
            tertiary: None,
            cost: None,
            identity: Self::resolve_identity(&creds, &usage),
            updated_at: now,
        })
    }

    fn name(&self) -> &'static str {
        "Codex"
    }
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

fn parse_jwt_payload(token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let decoded = URL_SAFE_NO_PAD.decode(payload.as_bytes()).ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn normalize_plan_name(plan: &str) -> Option<String> {
    let trimmed = plan.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    let label = match lower.as_str() {
        "free" => "Free",
        "plus" => "Plus",
        "pro" => "Pro",
        "team" => "Team",
        "enterprise" => "Enterprise",
        "business" => "Business",
        "education" => "Education",
        "go" => "Go",
        "guest" => "Guest",
        "free_workspace" => "Free Workspace",
        "k12" => "K-12",
        "quorum" => "Quorum",
        "edu" => "Edu",
        _ => trimmed,
    };
    Some(label.to_string())
}

fn deserialize_balance_opt<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Number(num) => num
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("Invalid number value for credit balance")),
        serde_json::Value::String(s) => s
            .parse::<f64>()
            .map_err(|_| serde::de::Error::custom("Invalid string value for credit balance")),
        _ => Err(serde::de::Error::custom(
            "Invalid value type for credit balance",
        )),
    }
    .map(Some)
}
