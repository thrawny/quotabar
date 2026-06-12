use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Supported providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
    OpenCode,
}

impl Provider {
    pub fn display_name(&self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
            Provider::OpenCode => "OpenCode",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Provider::Claude => "󰧑",
            Provider::Codex => "",
            Provider::OpenCode => "󰘦",
        }
    }

    pub fn usage_url(&self) -> Option<&'static str> {
        match self {
            Provider::Claude => Some("https://claude.ai/settings/usage"),
            Provider::Codex => Some("https://chatgpt.com/codex/settings/usage"),
            Provider::OpenCode => Some("https://opencode.ai"),
        }
    }
}

/// A single rate window representing quota usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateWindow {
    /// Usage percentage (0-100)
    pub used_percent: f64,
    /// Minutes until reset (if known)
    pub window_minutes: Option<i32>,
    /// Exact reset timestamp (if known)
    pub resets_at: Option<DateTime<Utc>>,
    /// Human-readable reset description (e.g., "in 2 hours")
    pub reset_description: Option<String>,
}

impl RateWindow {
    pub fn remaining_percent(&self) -> f64 {
        100.0 - self.used_percent
    }

    /// Whether the window this data describes has already ended. Once true,
    /// the cached percentage says nothing about the current window (usage may
    /// have continued on another machine), so it should render as unknown.
    pub fn is_expired(&self, snapshot_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        if let Some(resets_at) = self.resets_at {
            now >= resets_at
        } else if let Some(minutes) = self.window_minutes {
            // No exact reset time (e.g. Claude CLI fallback): the window that
            // produced this number can't outlive its own length.
            now.signed_duration_since(snapshot_at) >= chrono::Duration::minutes(minutes as i64)
        } else {
            false
        }
    }

    pub fn status_class(&self) -> &'static str {
        if self.used_percent >= 90.0 {
            "critical"
        } else if self.used_percent >= 75.0 {
            "warning"
        } else {
            "normal"
        }
    }
}

/// Spend/budget snapshot for providers with cost limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSnapshot {
    /// Amount spent
    pub used: f64,
    /// Spending limit
    pub limit: f64,
    /// Currency code (e.g., "USD")
    pub currency_code: String,
    /// Period description (e.g., "Monthly")
    pub period: Option<String>,
    /// When period resets
    pub resets_at: Option<DateTime<Utc>>,
}

impl CostSnapshot {
    pub fn used_percent(&self) -> f64 {
        if self.limit > 0.0 {
            (self.used / self.limit) * 100.0
        } else {
            0.0
        }
    }
}

/// Identity information for a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentitySnapshot {
    /// Email address
    pub email: Option<String>,
    /// Plan type (e.g., "Pro", "Max")
    pub plan: Option<String>,
    /// Organization name
    pub organization: Option<String>,
}

/// Complete usage snapshot for a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub provider: Provider,
    /// Primary/session quota
    pub primary: Option<RateWindow>,
    /// Secondary quota (e.g., weekly)
    pub secondary: Option<RateWindow>,
    /// Tertiary quota (e.g., Opus limit)
    pub tertiary: Option<RateWindow>,
    /// Cost/budget information
    pub cost: Option<CostSnapshot>,
    /// Identity information
    pub identity: Option<IdentitySnapshot>,
    /// When this snapshot was captured
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn window(resets_at: Option<DateTime<Utc>>, window_minutes: Option<i32>) -> RateWindow {
        RateWindow {
            used_percent: 100.0,
            window_minutes,
            resets_at,
            reset_description: None,
        }
    }

    #[test]
    fn test_is_expired_with_reset_timestamp() {
        let now = Utc::now();
        let snapshot_at = now - Duration::hours(8);

        let past_reset = window(Some(now - Duration::hours(3)), Some(300));
        assert!(past_reset.is_expired(snapshot_at, now));

        let future_reset = window(Some(now + Duration::hours(2)), Some(300));
        assert!(!future_reset.is_expired(snapshot_at, now));
    }

    #[test]
    fn test_is_expired_without_reset_falls_back_to_window_length() {
        let now = Utc::now();
        let w = window(None, Some(300));

        // Snapshot older than the 5h window length: definitely lapsed
        assert!(w.is_expired(now - Duration::hours(8), now));
        // Recent snapshot: window may still be running
        assert!(!w.is_expired(now - Duration::hours(1), now));
    }

    #[test]
    fn test_is_expired_unknown_window_never_expires() {
        let now = Utc::now();
        let w = window(None, None);
        assert!(!w.is_expired(now - Duration::days(30), now));
    }
}

impl UsageSnapshot {
    /// Get the most constrained (highest used) rate window
    pub fn primary_rate(&self) -> Option<&RateWindow> {
        self.primary.as_ref()
    }

    /// Get the lowest remaining percentage across all windows
    pub fn min_remaining(&self) -> Option<f64> {
        [&self.primary, &self.secondary, &self.tertiary]
            .iter()
            .filter_map(|w| w.as_ref().map(|r| r.remaining_percent()))
            .min_by(|a, b| a.partial_cmp(b).unwrap())
    }
}
