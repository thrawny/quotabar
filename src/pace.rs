use crate::models::{Provider, RateWindow};
use chrono::{DateTime, Utc};

const DEFAULT_WINDOW_MINUTES: i32 = 10080; // 7 days
const MINIMUM_EXPECTED_PERCENT: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaceStage {
    OnTrack,
    SlightlyAhead,
    Ahead,
    FarAhead,
    SlightlyBehind,
    Behind,
    FarBehind,
}

#[derive(Debug, Clone)]
pub struct UsagePace {
    pub stage: PaceStage,
    pub delta_percent: f64,
    pub expected_used_percent: f64,
    pub actual_used_percent: f64,
    pub eta_seconds: Option<f64>,
    pub will_last_to_reset: bool,
}

impl UsagePace {
    pub fn weekly(window: &RateWindow, now: DateTime<Utc>) -> Option<Self> {
        let resets_at = window.resets_at?;
        let minutes = window.window_minutes.unwrap_or(DEFAULT_WINDOW_MINUTES);
        if minutes <= 0 {
            return None;
        }

        let duration = f64::from(minutes) * 60.0;
        let time_until_reset = (resets_at - now).num_milliseconds() as f64 / 1000.0;
        if time_until_reset <= 0.0 || time_until_reset > duration {
            return None;
        }

        let elapsed = (duration - time_until_reset).clamp(0.0, duration);
        let expected = (elapsed / duration * 100.0).clamp(0.0, 100.0);
        let actual = window.used_percent.clamp(0.0, 100.0);

        if elapsed == 0.0 && actual > 0.0 {
            return None;
        }

        let delta = actual - expected;
        let stage = Self::stage_for(delta);

        let mut eta_seconds = None;
        let mut will_last_to_reset = false;

        if elapsed > 0.0 && actual > 0.0 {
            let rate = actual / elapsed;
            if rate > 0.0 {
                let remaining = (100.0 - actual).max(0.0);
                let candidate = remaining / rate;
                if candidate >= time_until_reset {
                    will_last_to_reset = true;
                } else {
                    eta_seconds = Some(candidate);
                }
            }
        } else if elapsed > 0.0 {
            will_last_to_reset = true;
        }

        Some(UsagePace {
            stage,
            delta_percent: delta,
            expected_used_percent: expected,
            actual_used_percent: actual,
            eta_seconds,
            will_last_to_reset,
        })
    }

    fn stage_for(delta: f64) -> PaceStage {
        let abs_delta = delta.abs();
        if abs_delta <= 2.0 {
            PaceStage::OnTrack
        } else if abs_delta <= 6.0 {
            if delta >= 0.0 {
                PaceStage::SlightlyAhead
            } else {
                PaceStage::SlightlyBehind
            }
        } else if abs_delta <= 12.0 {
            if delta >= 0.0 {
                PaceStage::Ahead
            } else {
                PaceStage::Behind
            }
        } else if delta >= 0.0 {
            PaceStage::FarAhead
        } else {
            PaceStage::FarBehind
        }
    }
}

pub fn compute_pace(
    provider: Provider,
    window: &RateWindow,
    now: DateTime<Utc>,
) -> Option<UsagePace> {
    if !matches!(provider, Provider::Claude | Provider::Codex) {
        return None;
    }
    if window.remaining_percent() <= 0.0 {
        return None;
    }
    let pace = UsagePace::weekly(window, now)?;
    if pace.expected_used_percent < MINIMUM_EXPECTED_PERCENT {
        return None;
    }
    Some(pace)
}

pub fn format_pace_left(pace: &UsagePace) -> String {
    match pace.stage {
        PaceStage::OnTrack => "On pace".to_string(),
        PaceStage::SlightlyAhead | PaceStage::Ahead | PaceStage::FarAhead => {
            format!("{}% in deficit", pace.delta_percent.abs().round() as i32)
        }
        PaceStage::SlightlyBehind | PaceStage::Behind | PaceStage::FarBehind => {
            format!("{}% in reserve", pace.delta_percent.abs().round() as i32)
        }
    }
}

pub fn format_pace_right(pace: &UsagePace) -> Option<String> {
    if pace.will_last_to_reset {
        return Some("Lasts until reset".to_string());
    }
    let eta = pace.eta_seconds?;
    let text = format_duration(eta);
    if text == "now" {
        Some("Runs out now".to_string())
    } else {
        Some(format!("Runs out in {}", text))
    }
}

pub fn format_duration(seconds: f64) -> String {
    if seconds < 1.0 {
        return "now".to_string();
    }
    let total_minutes = (seconds / 60.0).ceil().max(1.0) as i64;
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes / 60) % 24;
    let minutes = total_minutes % 60;

    if days > 0 && hours > 0 {
        format!("{}d {}h", days, hours)
    } else if days > 0 {
        format!("{}d", days)
    } else if hours > 0 && minutes > 0 {
        format!("{}h {}m", hours, minutes)
    } else if hours > 0 {
        format!("{}h", hours)
    } else {
        format!("{}m", minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_window(used_percent: f64, window_minutes: i32, resets_in: Duration) -> RateWindow {
        RateWindow {
            used_percent,
            window_minutes: Some(window_minutes),
            resets_at: Some(Utc::now() + resets_in),
            reset_description: None,
        }
    }

    #[test]
    fn test_on_track() {
        // 50% through window, 50% used -> on track
        let window = make_window(50.0, 10080, Duration::days(3) + Duration::hours(12));
        let pace = UsagePace::weekly(&window, Utc::now()).unwrap();
        assert_eq!(pace.stage, PaceStage::OnTrack);
    }

    #[test]
    fn test_ahead_deficit() {
        // 50% through window, 70% used -> 20% in deficit
        let window = make_window(70.0, 10080, Duration::days(3) + Duration::hours(12));
        let pace = UsagePace::weekly(&window, Utc::now()).unwrap();
        assert!(matches!(pace.stage, PaceStage::Ahead | PaceStage::FarAhead));
        assert!(pace.delta_percent > 0.0);
    }

    #[test]
    fn test_behind_reserve() {
        // 50% through window, 30% used -> 20% in reserve
        let window = make_window(30.0, 10080, Duration::days(3) + Duration::hours(12));
        let pace = UsagePace::weekly(&window, Utc::now()).unwrap();
        assert!(matches!(
            pace.stage,
            PaceStage::Behind | PaceStage::FarBehind
        ));
        assert!(pace.delta_percent < 0.0);
    }

    #[test]
    fn test_no_reset_returns_none() {
        let window = RateWindow {
            used_percent: 50.0,
            window_minutes: Some(10080),
            resets_at: None,
            reset_description: None,
        };
        assert!(UsagePace::weekly(&window, Utc::now()).is_none());
    }

    #[test]
    fn test_expired_returns_none() {
        let window = make_window(50.0, 10080, Duration::seconds(-1));
        assert!(UsagePace::weekly(&window, Utc::now()).is_none());
    }

    #[test]
    fn test_will_last_to_reset() {
        // 80% through window, only 10% used -> very slow burn, will last
        let window = make_window(10.0, 10080, Duration::days(1) + Duration::hours(9));
        let pace = UsagePace::weekly(&window, Utc::now()).unwrap();
        assert!(pace.will_last_to_reset);
    }

    #[test]
    fn test_format_duration_days_hours() {
        assert_eq!(format_duration(3.0 * 86400.0 + 5.0 * 3600.0), "3d 5h");
    }

    #[test]
    fn test_format_duration_hours_minutes() {
        assert_eq!(format_duration(2.0 * 3600.0 + 30.0 * 60.0), "2h 30m");
    }

    #[test]
    fn test_format_duration_minutes_only() {
        assert_eq!(format_duration(45.0 * 60.0), "45m");
    }

    #[test]
    fn test_format_duration_now() {
        assert_eq!(format_duration(0.5), "now");
    }

    #[test]
    fn test_gating_opencode_excluded() {
        let window = make_window(50.0, 10080, Duration::days(3));
        assert!(compute_pace(Provider::OpenCode, &window, Utc::now()).is_none());
    }

    #[test]
    fn test_gating_claude_included() {
        let window = make_window(50.0, 10080, Duration::days(3));
        assert!(compute_pace(Provider::Claude, &window, Utc::now()).is_some());
    }

    #[test]
    fn test_gating_fully_used() {
        let window = make_window(100.0, 10080, Duration::days(3));
        assert!(compute_pace(Provider::Claude, &window, Utc::now()).is_none());
    }
}
