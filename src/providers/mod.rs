pub mod claude;
pub mod codex;

use crate::models::UsageSnapshot;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

#[async_trait]
pub trait ProviderFetcher: Send + Sync {
    async fn fetch(&self) -> Result<UsageSnapshot>;
    fn name(&self) -> &'static str;
}

pub(crate) fn format_reset_time(reset: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let duration = reset.signed_duration_since(now);
    let total_minutes = duration.num_minutes();

    if total_minutes <= 0 {
        return "now".to_string();
    }

    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;

    if hours < 1 {
        format!("in {} min", total_minutes)
    } else if hours < 24 {
        if minutes > 0 {
            format!("in {}h {}m", hours, minutes)
        } else {
            format!("in {}h", hours)
        }
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
    use super::format_reset_time;
    use chrono::{Duration, Utc};

    #[test]
    fn formats_hour_minute_resets() {
        let now = Utc::now();
        let reset = now + Duration::hours(1) + Duration::minutes(29);
        assert_eq!(format_reset_time(reset, now), "in 1h 29m");
    }

    #[test]
    fn formats_minute_only_resets() {
        let now = Utc::now();
        let reset = now + Duration::minutes(30);
        assert_eq!(format_reset_time(reset, now), "in 30 min");
    }

    #[test]
    fn formats_day_resets() {
        let now = Utc::now();
        let reset = now + Duration::days(3);
        assert_eq!(format_reset_time(reset, now), "in 3 days");
    }
}
