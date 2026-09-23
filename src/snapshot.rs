//! Versioned frontend contract. Policy stays here; clients only lay out the data.
use crate::cache::CacheState;
use crate::models::{
    CodexResetCredit, CostSnapshot, IdentitySnapshot, Provider, RateWindow, UsageSnapshot,
};
use crate::pace;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize)]
pub struct Snapshot {
    schema_version: u32,
    generated_at: DateTime<Utc>,
    cache_updated_at: DateTime<Utc>,
    providers: Vec<ProviderSnapshot>,
}

#[derive(Serialize)]
struct ProviderSnapshot {
    id: Provider,
    name: &'static str,
    usage_url: Option<&'static str>,
    available: bool,
    stale: bool,
    error: Option<String>,
    updated_at: Option<DateTime<Utc>>,
    identity: Option<IdentitySnapshot>,
    summary: String,
    compact_summary: String,
    severity: &'static str,
    windows: Vec<Window>,
    cost: Option<CostSnapshot>,
    reset_credits: Option<ResetCredits>,
}

#[derive(Serialize)]
struct Window {
    id: String,
    title: String,
    #[serde(flatten)]
    quota: RateWindow,
    expired: bool,
    severity: &'static str,
    reset_text: String,
    pace_text: Option<String>,
    expected_used_percent: Option<f64>,
}

#[derive(Serialize)]
struct ResetCredits {
    available_count: usize,
    updated_at: DateTime<Utc>,
    credits: Vec<Credit>,
}

#[derive(Serialize)]
struct Credit {
    #[serde(flatten)]
    credit: CodexResetCredit,
    available: bool,
}

fn window(
    id: &str,
    title: &str,
    quota: &RateWindow,
    snapshot: &UsageSnapshot,
    now: DateTime<Utc>,
) -> Window {
    let expired = quota.is_expired(snapshot.updated_at, now);
    let pace = if id == "secondary" && !expired {
        pace::compute_pace(snapshot.provider, quota, now)
    } else {
        None
    };
    Window {
        id: id.to_owned(),
        title: title.to_owned(),
        quota: quota.clone(),
        expired,
        severity: if expired {
            "unknown"
        } else {
            quota.status_class()
        },
        reset_text: if expired {
            "Window lapsed · refresh pending".to_owned()
        } else if let Some(reset) = quota.resets_at {
            format!(
                "Resets in {}",
                pace::format_duration((reset - now).num_seconds() as f64)
            )
        } else {
            quota
                .reset_description
                .as_ref()
                .map(|text| format!("Resets {text}"))
                .unwrap_or_default()
        },
        pace_text: pace.as_ref().map(|p| {
            [Some(pace::format_pace_left(p)), pace::format_pace_right(p)]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ")
        }),
        expected_used_percent: pace.as_ref().map(|p| p.expected_used_percent),
    }
}

fn percentage(window: &Window) -> String {
    if window.expired {
        "?".to_owned()
    } else {
        format!("{:.0}%", window.quota.used_percent)
    }
}

pub fn build(state: &CacheState, now: DateTime<Utc>) -> Snapshot {
    let providers = [Provider::Claude, Provider::Codex]
        .into_iter()
        .map(|provider| {
            let snapshot = state.get(provider);
            let error = state.errors.get(&provider).cloned();
            let mut windows = Vec::new();
            if let Some(s) = snapshot {
                for (id, title, quota) in [
                    ("primary", "Current session", &s.primary),
                    ("secondary", "Current week (all models)", &s.secondary),
                    ("tertiary", "Current week (Sonnet only)", &s.tertiary),
                ] {
                    if let Some(quota) = quota {
                        windows.push(window(id, title, quota, s, now));
                    }
                }
                for named in &s.extra_rate_windows {
                    windows.push(window(&named.id, &named.title, &named.window, s, now));
                }
            }
            let primary = windows.iter().find(|w| w.id == "primary");
            // Match the established indicator's preference for Claude's Fable quota.
            let week = windows
                .iter()
                .find(|w| {
                    provider == Provider::Claude
                        && (w
                            .id
                            .split('-')
                            .any(|part| part.eq_ignore_ascii_case("fable"))
                            || matches!(
                                w.title.trim().to_lowercase().as_str(),
                                "fable" | "fable only"
                            ))
                })
                .or_else(|| windows.iter().find(|w| w.id == "secondary"));
            let compact_summary = primary
                .or(week)
                .map(percentage)
                .unwrap_or_else(|| "—".to_owned());
            let mut parts: Vec<String> = [primary, week]
                .into_iter()
                .flatten()
                .map(percentage)
                .collect();
            if let Some(w) = primary.or(week).filter(|w| !w.expired) {
                if let Some(reset) = w.quota.resets_at {
                    let seconds = (reset - now).num_seconds();
                    parts.push(if primary.is_some() {
                        crate::format_session_glance(seconds)
                    } else {
                        crate::format_week_glance(seconds)
                    });
                }
            }
            let summary = if parts.is_empty() {
                "—".to_owned()
            } else {
                parts.join(" ")
            };
            let severity = if windows.iter().any(|w| w.severity == "critical") {
                "critical"
            } else if windows.iter().any(|w| w.severity == "warning") {
                "warning"
            } else {
                "normal"
            };
            let stale = error.is_some()
                || snapshot.is_some_and(|s| {
                    (now - s.updated_at).num_seconds() > crate::MIN_FETCH_INTERVAL_SECS * 2
                })
                || windows.iter().any(|w| w.expired);
            ProviderSnapshot {
                id: provider,
                name: provider.display_name(),
                usage_url: provider.usage_url(),
                available: snapshot.is_some(),
                stale,
                error,
                updated_at: snapshot.map(|s| s.updated_at),
                identity: snapshot.and_then(|s| s.identity.clone()),
                summary,
                compact_summary,
                severity,
                windows,
                cost: snapshot.and_then(|s| s.cost.clone()),
                reset_credits: snapshot.and_then(|s| s.codex_reset_credits.as_ref()).map(
                    |credits| ResetCredits {
                        available_count: credits.available_credits(now).len(),
                        updated_at: credits.updated_at,
                        credits: credits
                            .credits
                            .iter()
                            .map(|credit| Credit {
                                available: credit.status == "available"
                                    && credit.expires_at.is_none_or(|expiry| expiry > now),
                                credit: credit.clone(),
                            })
                            .collect(),
                    },
                ),
            }
        })
        .collect();
    Snapshot {
        schema_version: 1,
        generated_at: now,
        cache_updated_at: state.updated_at,
        providers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::collections::HashMap;

    fn state() -> CacheState {
        CacheState {
            snapshots: crate::mock::mock_snapshots(),
            errors: HashMap::new(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn contract_has_both_providers_full_cost_credits_and_fable_summary() {
        let output = serde_json::to_value(build(&state(), Utc::now())).unwrap();
        assert_eq!(output["schema_version"], 1);
        assert_eq!(output["providers"].as_array().unwrap().len(), 2);
        let claude = &output["providers"][0];
        assert_eq!(claude["id"], "claude");
        assert!(claude["summary"].as_str().unwrap().starts_with("72% 33%"));
        assert_eq!(claude["cost"]["used"], 42.5);
        assert!(claude["windows"][1]["pace_text"].is_string());
        assert_eq!(
            output["providers"][1]["reset_credits"]["available_count"],
            1
        );
    }

    #[test]
    fn failed_fetch_preserves_data_and_marks_stale() {
        let mut state = state();
        state
            .errors
            .insert(Provider::Claude, "Refresh failed".to_owned());
        let output = build(&state, Utc::now());
        assert!(output.providers[0].available);
        assert!(output.providers[0].stale);
        assert_eq!(output.providers[0].error.as_deref(), Some("Refresh failed"));
        assert!(!output.providers[1].stale);
    }

    #[test]
    fn unavailable_provider_does_not_inherit_other_provider_data() {
        let mut state = state();
        state.snapshots.remove(&Provider::Claude);
        let output = build(&state, Utc::now());
        assert!(!output.providers[0].available);
        assert!(output.providers[0].windows.is_empty());
        assert_eq!(output.providers[0].summary, "—");
        assert!(output.providers[1].available);
    }

    #[test]
    fn expired_windows_and_credits_are_recomputed_even_for_cached_data() {
        let mut state = state();
        let now = Utc::now() + Duration::days(40);
        state
            .snapshots
            .get_mut(&Provider::Claude)
            .unwrap()
            .primary
            .as_mut()
            .unwrap()
            .used_percent = 100.0;
        let output = build(&state, now);
        assert_eq!(output.providers[0].compact_summary, "?");
        assert_eq!(output.providers[0].severity, "normal");
        assert!(output.providers[0].windows[1].pace_text.is_none());
        let credits = output.providers[1].reset_credits.as_ref().unwrap();
        assert_eq!(credits.available_count, 0);
        assert!(!credits.credits[0].available);
    }
}
