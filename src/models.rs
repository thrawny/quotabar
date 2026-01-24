use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Supported providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
