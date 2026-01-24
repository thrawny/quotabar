use crate::models::{CostSnapshot, IdentitySnapshot, Provider, RateWindow, UsageSnapshot};
use chrono::{Duration, Utc};
use std::collections::HashMap;

pub fn mock_snapshots() -> HashMap<Provider, UsageSnapshot> {
    let now = Utc::now();
    let mut snapshots = HashMap::new();

    // Claude: 72% used, with secondary and cost
    snapshots.insert(
        Provider::Claude,
        UsageSnapshot {
            provider: Provider::Claude,
            primary: Some(RateWindow {
                used_percent: 72.0,
                window_minutes: Some(300),
                resets_at: Some(now + Duration::hours(5)),
                reset_description: Some("in 5 hours".to_string()),
            }),
            secondary: Some(RateWindow {
                used_percent: 45.0,
                window_minutes: None,
                resets_at: Some(now + Duration::days(3)),
                reset_description: Some("in 3 days".to_string()),
            }),
            tertiary: None,
            cost: Some(CostSnapshot {
                used: 42.50,
                limit: 100.0,
                currency_code: "USD".to_string(),
                period: Some("Monthly".to_string()),
                resets_at: Some(now + Duration::days(7)),
            }),
            identity: Some(IdentitySnapshot {
                email: Some("user@example.com".to_string()),
                plan: Some("Max".to_string()),
                organization: None,
            }),
            updated_at: now,
        },
    );

    // Codex: 85% used (warning state)
    snapshots.insert(
        Provider::Codex,
        UsageSnapshot {
            provider: Provider::Codex,
            primary: Some(RateWindow {
                used_percent: 85.0,
                window_minutes: Some(60),
                resets_at: Some(now + Duration::hours(1)),
                reset_description: Some("in 1 hour".to_string()),
            }),
            secondary: None,
            tertiary: None,
            cost: None,
            identity: Some(IdentitySnapshot {
                email: Some("user@example.com".to_string()),
                plan: Some("Pro".to_string()),
                organization: Some("Personal".to_string()),
            }),
            updated_at: now,
        },
    );

    // OpenCode: 15% used (healthy)
    snapshots.insert(
        Provider::OpenCode,
        UsageSnapshot {
            provider: Provider::OpenCode,
            primary: Some(RateWindow {
                used_percent: 15.0,
                window_minutes: Some(300),
                resets_at: Some(now + Duration::hours(5)),
                reset_description: Some("in 5 hours".to_string()),
            }),
            secondary: Some(RateWindow {
                used_percent: 8.0,
                window_minutes: None,
                resets_at: Some(now + Duration::days(5)),
                reset_description: Some("in 5 days".to_string()),
            }),
            tertiary: None,
            cost: None,
            identity: Some(IdentitySnapshot {
                email: Some("user@example.com".to_string()),
                plan: Some("Free".to_string()),
                organization: None,
            }),
            updated_at: now,
        },
    );

    snapshots
}
