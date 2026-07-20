use crate::cache;
use crate::config::NotificationConfig;
use crate::models::{Provider, UsageSnapshot};
use chrono::{DateTime, Duration, Utc};
use fs2::FileExt;
use notify_rust::{Notification, Timeout, Urgency};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const RESET_EXPIRY_WARNING_HOURS: i64 = 6;
const NORMAL_REPEAT_MINUTES: i64 = 60;
const FINAL_HOUR_REPEAT_MINUTES: i64 = 15;

#[derive(Debug)]
struct ExpiringResetCredit {
    key: String,
    expires_at: DateTime<Utc>,
    available_count: usize,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct NotificationState {
    #[serde(default)]
    codex_reset_last_notified: HashMap<String, DateTime<Utc>>,
}

pub fn maybe_notify_expiring_codex_reset(
    snapshots: &HashMap<Provider, UsageSnapshot>,
    config: &NotificationConfig,
    now: DateTime<Utc>,
) {
    if !config.enabled {
        return;
    }

    let Some(snapshot) = snapshots.get(&Provider::Codex) else {
        return;
    };
    let Some(expiring) = expiring_reset_credit(snapshot, now) else {
        return;
    };

    let state_path = notification_state_path();
    let Some(parent) = state_path.parent() else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(parent) {
        cache::append_log(&format!(
            "Failed to create notification state directory: {error}"
        ));
        return;
    }

    let mut file = match OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&state_path)
    {
        Ok(file) => file,
        Err(error) => {
            cache::append_log(&format!("Failed to open notification state: {error}"));
            return;
        }
    };
    if let Err(error) = file.lock_exclusive() {
        cache::append_log(&format!("Failed to lock notification state: {error}"));
        return;
    }

    let mut content = String::new();
    let mut state = if file.read_to_string(&mut content).is_ok() {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        NotificationState::default()
    };
    let expires_in = expiring.expires_at - now;
    if !reset_notification_is_due(
        state.codex_reset_last_notified.get(&expiring.key).copied(),
        now,
        expires_in,
    ) {
        return;
    }

    // Persist before sending so concurrent Claude and Codex Waybar modules do
    // not both emit the same notification.
    state
        .codex_reset_last_notified
        .insert(expiring.key.clone(), now);
    let serialized = match serde_json::to_vec_pretty(&state) {
        Ok(serialized) => serialized,
        Err(error) => {
            cache::append_log(&format!("Failed to serialize notification state: {error}"));
            return;
        }
    };
    if file.seek(SeekFrom::Start(0)).is_err()
        || file.set_len(0).is_err()
        || file.write_all(&serialized).is_err()
        || file.sync_all().is_err()
    {
        cache::append_log("Failed to save notification state");
        return;
    }
    drop(file);

    let title = format!(
        "Codex reset expires in {}",
        format_expiry_countdown(expires_in)
    );
    let mut body = format!(
        "Use it or lose it. {} reset credit{} available.",
        expiring.available_count,
        if expiring.available_count == 1 {
            ""
        } else {
            "s"
        }
    );
    if let Some(weekly) = snapshot.secondary.as_ref() {
        body.push_str(&format!(" Weekly quota: {:.0}% used.", weekly.used_percent));
    }

    if let Err(error) = Notification::new()
        .appname("quotabar")
        .summary(&title)
        .body(&body)
        .icon("dialog-warning")
        .urgency(Urgency::Critical)
        .timeout(Timeout::Milliseconds(20_000))
        .show()
    {
        cache::append_log(&format!(
            "Failed to show reset expiry notification: {error}"
        ));
    }
}

fn expiring_reset_credit(
    snapshot: &UsageSnapshot,
    now: DateTime<Utc>,
) -> Option<ExpiringResetCredit> {
    let credits = snapshot.codex_reset_credits.as_ref()?;
    let available = credits.available_credits(now);
    let credit = available
        .iter()
        .filter(|credit| credit.expires_at.is_some())
        .min_by_key(|credit| credit.expires_at)?;
    let expires_at = credit.expires_at?;
    let expires_in = expires_at - now;
    if expires_in > Duration::hours(RESET_EXPIRY_WARNING_HOURS) {
        return None;
    }

    Some(ExpiringResetCredit {
        key: format!(
            "{}:{}:{}",
            credit.reset_type,
            credit.granted_at.timestamp(),
            expires_at.timestamp()
        ),
        expires_at,
        available_count: available.len(),
    })
}

fn reset_notification_repeat_interval(expires_in: Duration) -> Duration {
    if expires_in <= Duration::hours(1) {
        Duration::minutes(FINAL_HOUR_REPEAT_MINUTES)
    } else {
        Duration::minutes(NORMAL_REPEAT_MINUTES)
    }
}

fn reset_notification_is_due(
    last_notified: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    expires_in: Duration,
) -> bool {
    last_notified.is_none_or(|last| {
        now.signed_duration_since(last) >= reset_notification_repeat_interval(expires_in)
    })
}

fn format_expiry_countdown(duration: Duration) -> String {
    let minutes = duration.num_minutes().max(1);
    if minutes >= 60 {
        let hours = minutes / 60;
        let remainder = minutes % 60;
        if remainder == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {remainder}m")
        }
    } else {
        format!("{minutes}m")
    }
}

fn notification_state_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("quotabar")
        .join("notifications.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CodexResetCredit, CodexResetCreditsSnapshot, IdentitySnapshot, RateWindow,
    };

    fn snapshot(now: DateTime<Utc>, expires_in: Duration) -> UsageSnapshot {
        UsageSnapshot {
            provider: Provider::Codex,
            primary: None,
            secondary: Some(RateWindow {
                used_percent: 42.0,
                window_minutes: Some(7 * 24 * 60),
                resets_at: None,
                reset_description: None,
            }),
            tertiary: None,
            extra_rate_windows: vec![],
            cost: None,
            codex_reset_credits: Some(CodexResetCreditsSnapshot {
                credits: vec![CodexResetCredit {
                    reset_type: "codex_rate_limits".to_string(),
                    status: "available".to_string(),
                    granted_at: now - Duration::days(1),
                    expires_at: Some(now + expires_in),
                    redeem_started_at: None,
                    redeemed_at: None,
                    title: None,
                    description: None,
                }],
                available_count: 1,
                updated_at: now,
            }),
            identity: Some(IdentitySnapshot {
                email: None,
                plan: None,
                organization: None,
            }),
            updated_at: now,
        }
    }

    #[test]
    fn only_selects_credits_expiring_within_six_hours() {
        let now = Utc::now();
        assert!(expiring_reset_credit(&snapshot(now, Duration::hours(6)), now).is_some());
        assert!(expiring_reset_credit(&snapshot(now, Duration::hours(7)), now).is_none());
    }

    #[test]
    fn repeats_more_often_during_final_hour() {
        assert_eq!(
            reset_notification_repeat_interval(Duration::hours(5)),
            Duration::hours(1)
        );
        assert_eq!(
            reset_notification_repeat_interval(Duration::minutes(59)),
            Duration::minutes(15)
        );
    }

    #[test]
    fn repeats_on_the_hour_then_every_fifteen_minutes() {
        let now = Utc::now();
        assert!(!reset_notification_is_due(
            Some(now - Duration::minutes(59)),
            now,
            Duration::hours(5)
        ));
        assert!(reset_notification_is_due(
            Some(now - Duration::minutes(60)),
            now,
            Duration::hours(5)
        ));
        assert!(!reset_notification_is_due(
            Some(now - Duration::minutes(14)),
            now,
            Duration::minutes(45)
        ));
        assert!(reset_notification_is_due(
            Some(now - Duration::minutes(15)),
            now,
            Duration::minutes(45)
        ));
    }

    #[test]
    fn formats_expiry_countdown() {
        assert_eq!(format_expiry_countdown(Duration::minutes(341)), "5h 41m");
        assert_eq!(format_expiry_countdown(Duration::minutes(45)), "45m");
    }
}
