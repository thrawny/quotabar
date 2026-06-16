#![allow(dead_code)]

use anyhow::Result;
use cache::CacheState;
use chrono::Utc;
use clap::{Parser, Subcommand};
use config::Config;
use models::{Provider, UsageSnapshot};
use providers::claude::ClaudeProvider;
use providers::codex::CodexProvider;
use providers::ProviderFetcher;
use serde::Serialize;
use std::collections::HashMap;

mod cache;
mod config;
mod icons;
mod mock;
mod models;
mod pace;
mod popup;
mod providers;
mod themes;

const MIN_FETCH_INTERVAL_SECS: i64 = 300; // 5 minutes
const HIGH_USAGE_FETCH_INTERVAL_SECS: i64 = 30; // 30 seconds when usage >= 80%
const HIGH_USAGE_THRESHOLD: f64 = 80.0;

#[derive(Parser)]
#[command(name = "quotabar")]
#[command(about = "Monitor API quota/usage for AI coding tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show layer-shell popup (reads cache, refreshes in background)
    Popup {
        /// Use mock data instead of real providers
        #[arg(long)]
        mock: bool,
    },
    /// Fetch, cache, and print JSON for Waybar
    Waybar {
        /// Only show this provider (icon is expected to come from Waybar CSS)
        #[arg(long)]
        provider: Option<Provider>,
    },
    /// Print all provider status to terminal
    Status,
    /// Force fetch and update cache
    Fetch,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Popup { mock } => {
            popup::run(mock)?;
        }
        Commands::Waybar { provider } => {
            icons::ensure_icons();
            let output = waybar_output(provider).await;
            println!("{}", serde_json::to_string(&output).unwrap());
        }
        Commands::Status => {
            match fetch_claude().await {
                Ok(s) => print_status(&s),
                Err(e) => eprintln!("Claude: {}", e),
            }
            match fetch_codex().await {
                Ok(s) => print_status(&s),
                Err(e) => eprintln!("Codex: {}", e),
            }
        }
        Commands::Fetch => {
            let previous = CacheState::load().ok().flatten();
            let mut snapshots = HashMap::new();
            let mut errors = HashMap::new();

            match fetch_claude().await {
                Ok(s) => {
                    snapshots.insert(Provider::Claude, s);
                }
                Err(e) => {
                    cache::append_log(&format!("Claude fetch error: {}", e));
                    eprintln!("Failed to fetch Claude: {}", e);
                    errors.insert(Provider::Claude, e.to_string());
                    if let Some(prev) = previous
                        .as_ref()
                        .and_then(|c| c.get(Provider::Claude).cloned())
                    {
                        snapshots.insert(Provider::Claude, prev);
                    }
                }
            }

            match fetch_codex().await {
                Ok(s) => {
                    snapshots.insert(Provider::Codex, s);
                }
                Err(e) => {
                    cache::append_log(&format!("Codex fetch error: {}", e));
                    eprintln!("Failed to fetch Codex: {}", e);
                    errors.insert(Provider::Codex, e.to_string());
                    if let Some(prev) = previous
                        .as_ref()
                        .and_then(|c| c.get(Provider::Codex).cloned())
                    {
                        snapshots.insert(Provider::Codex, prev);
                    }
                }
            }

            let state = CacheState {
                snapshots,
                errors,
                updated_at: Utc::now(),
            };
            state.save()?;
            println!("Cache updated at {}", CacheState::cache_path().display());
        }
    }

    Ok(())
}

async fn fetch_claude() -> Result<models::UsageSnapshot> {
    let provider = ClaudeProvider::new();
    provider.fetch().await
}

async fn fetch_codex() -> Result<models::UsageSnapshot> {
    let provider = CodexProvider::new();
    provider.fetch().await
}

fn print_status(snapshot: &models::UsageSnapshot) {
    println!(
        "{} {} {}",
        snapshot.provider.icon(),
        snapshot.provider.display_name(),
        snapshot
            .identity
            .as_ref()
            .and_then(|i| i.plan.as_ref())
            .map(|p| format!("({})", p))
            .unwrap_or_default()
    );

    if let Some(ref primary) = snapshot.primary {
        println!(
            "  Current session:            {:.0}% used {}",
            primary.used_percent,
            primary.reset_description.as_deref().unwrap_or("")
        );
    }
    if let Some(ref secondary) = snapshot.secondary {
        println!(
            "  Current week (all models):  {:.0}% used {}",
            secondary.used_percent,
            secondary.reset_description.as_deref().unwrap_or("")
        );
    }
    if let Some(ref tertiary) = snapshot.tertiary {
        println!(
            "  Current week (Sonnet only): {:.0}% used {}",
            tertiary.used_percent,
            tertiary.reset_description.as_deref().unwrap_or("")
        );
    }
    if let Some(ref cost) = snapshot.cost {
        println!(
            "  Cost:    ${:.2} / ${:.2} {}",
            cost.used,
            cost.limit,
            cost.period.as_deref().unwrap_or("")
        );
    }
}

#[derive(Serialize)]
struct WaybarOutput {
    text: String,
    tooltip: String,
    class: Vec<String>,
}

async fn waybar_output(provider: Option<Provider>) -> WaybarOutput {
    let config = Config::load().unwrap_or_default();
    let selected = provider.or(config.general.selected_provider);
    // With an explicit --provider the icon comes from Waybar CSS, so omit the glyph
    let show_icon = provider.is_none();

    // Serve from cache if fresh enough (poll faster when usage is high)
    if let Some(cached) = CacheState::load().ok().flatten() {
        let age = Utc::now().signed_duration_since(cached.updated_at);
        let interval = if max_usage(&cached.snapshots) >= HIGH_USAGE_THRESHOLD {
            HIGH_USAGE_FETCH_INTERVAL_SECS
        } else {
            MIN_FETCH_INTERVAL_SECS
        };
        if age.num_seconds() < interval {
            return build_waybar_output(&cached.snapshots, selected, show_icon);
        }
    }

    // Cache is stale or missing, fetch fresh data
    let previous = CacheState::load().ok().flatten();
    let mut snapshots = HashMap::new();
    let mut errors = HashMap::new();

    match fetch_claude().await {
        Ok(snapshot) => {
            snapshots.insert(Provider::Claude, snapshot);
        }
        Err(e) => {
            cache::append_log(&format!("Claude fetch error: {}", e));
            errors.insert(Provider::Claude, e.to_string());
            // Carry forward last known snapshot so reset times stay visible
            if let Some(prev) = previous
                .as_ref()
                .and_then(|c| c.get(Provider::Claude).cloned())
            {
                snapshots.insert(Provider::Claude, prev);
            }
        }
    }
    match fetch_codex().await {
        Ok(snapshot) => {
            snapshots.insert(Provider::Codex, snapshot);
        }
        Err(e) => {
            cache::append_log(&format!("Codex fetch error: {}", e));
            errors.insert(Provider::Codex, e.to_string());
            if let Some(prev) = previous
                .as_ref()
                .and_then(|c| c.get(Provider::Codex).cloned())
            {
                snapshots.insert(Provider::Codex, prev);
            }
        }
    }

    // Save to cache
    let state = CacheState {
        snapshots: snapshots.clone(),
        errors,
        updated_at: Utc::now(),
    };
    let _ = state.save();

    // Build output from snapshots
    build_waybar_output(&snapshots, selected, show_icon)
}

/// Return the highest usage percentage across all providers and rate windows.
fn max_usage(snapshots: &HashMap<Provider, UsageSnapshot>) -> f64 {
    snapshots
        .values()
        .flat_map(|s| {
            [&s.primary, &s.secondary, &s.tertiary]
                .into_iter()
                .filter_map(|w| w.as_ref().map(|r| r.used_percent))
        })
        .fold(0.0_f64, f64::max)
}

fn build_waybar_output(
    snapshots: &HashMap<Provider, UsageSnapshot>,
    selected_provider: Option<Provider>,
    show_icon: bool,
) -> WaybarOutput {
    let icon = if show_icon { "󰧑 " } else { "" };
    let snapshot = if show_icon {
        // Legacy single-module mode: fall back to whichever provider has data
        selected_provider
            .and_then(|provider| snapshots.get(&provider))
            .or_else(|| snapshots.get(&Provider::Claude))
            .or_else(|| snapshots.get(&Provider::Codex))
            .or_else(|| snapshots.get(&Provider::OpenCode))
    } else {
        // Explicit --provider: never show another provider's data
        selected_provider.and_then(|provider| snapshots.get(&provider))
    };
    let Some(snapshot) = snapshot else {
        return WaybarOutput {
            text: format!("{}--", icon),
            tooltip: "No data available".to_string(),
            class: vec!["error".to_string()],
        };
    };

    let now = Utc::now();
    // Expired windows render as unknown: the window ended, and usage may have
    // continued on another machine, so the cached percentage is meaningless.
    let session = snapshot
        .primary
        .as_ref()
        .map(|r| (r, r.is_expired(snapshot.updated_at, now)));
    let week = snapshot
        .secondary
        .as_ref()
        .map(|r| (r, r.is_expired(snapshot.updated_at, now)));

    let fmt = |(r, expired): (&models::RateWindow, bool)| {
        if expired {
            "?".to_string()
        } else {
            format!("{:.0}%", r.used_percent)
        }
    };

    // Coarse time left in the session window, e.g. "2h" / "45m". Shown at any
    // usage level, but only when an exact reset timestamp is known and the
    // window is still live; a lapsed or timestamp-less (CLI fallback) window
    // would make this misleading.
    let session_time_left = session.and_then(|(r, expired)| {
        if expired {
            return None;
        }
        let resets_at = r.resets_at?;
        let secs = (resets_at - now).num_seconds();
        (secs > 0).then(|| format_session_glance(secs))
    });

    // Build text: session % at full strength; the week % and the session
    // time-left trail behind dimmed via Pango alpha. Session % leads because
    // that's the at-a-glance number; time-left sits last as quiet context.
    let mut faint = Vec::new();
    if let Some(w) = week {
        faint.push(fmt(w));
    }
    if let Some(t) = &session_time_left {
        faint.push(t.clone());
    }
    let text = if let Some(s) = session {
        if faint.is_empty() {
            format!("{}{}", icon, fmt(s))
        } else {
            format!(
                "{}{} <span alpha='55%'>{}</span>",
                icon,
                fmt(s),
                faint.join(" ")
            )
        }
    } else if let Some(w) = week {
        format!("{}{}", icon, fmt(w))
    } else {
        format!("{}--", icon)
    };

    // Build tooltip with more detail
    let mut tooltip_parts = vec![snapshot.provider.display_name().to_string()];
    if let Some((primary, expired)) = session {
        if expired {
            tooltip_parts.push("Session: unknown (window lapsed, refresh pending)".to_string());
        } else {
            let mut line = format!("Session: {:.0}%", primary.used_percent);
            if let Some(resets_at) = primary.resets_at {
                let secs = (resets_at - now).num_seconds();
                if secs > 0 {
                    line.push_str(&format!(" · {} left", pace::format_duration(secs as f64)));
                }
                let clock = resets_at.with_timezone(&chrono::Local).format("%H:%M");
                line.push_str(&format!(" · resets {}", clock));
            } else if let Some(desc) = primary.reset_description.as_deref() {
                line.push_str(&format!(" (resets {})", desc));
            }
            tooltip_parts.push(line);
        }
    }
    if let Some((secondary, expired)) = week {
        if expired {
            tooltip_parts.push("Week: unknown (window lapsed, refresh pending)".to_string());
        } else {
            let mut week_line = format!(
                "Week: {:.0}% (resets {})",
                secondary.used_percent,
                secondary.reset_description.as_deref().unwrap_or("--")
            );
            if let Some(p) = pace::compute_pace(snapshot.provider, secondary, now) {
                let left = pace::format_pace_left(&p);
                if let Some(right) = pace::format_pace_right(&p) {
                    week_line.push_str(&format!(" · {} · {}", left, right));
                } else {
                    week_line.push_str(&format!(" · {}", left));
                }
            }
            tooltip_parts.push(week_line);
        }
    }

    // Class based on highest non-expired usage
    let max_used = [session, week]
        .into_iter()
        .flatten()
        .filter(|(_, expired)| !expired)
        .map(|(r, _)| r.used_percent)
        .fold(0.0_f64, f64::max);
    let mut class = if max_used >= 90.0 {
        vec!["critical".to_string()]
    } else if max_used >= 75.0 {
        vec!["warning".to_string()]
    } else {
        vec![]
    };
    if session.is_some_and(|(_, e)| e) || week.is_some_and(|(_, e)| e) {
        class.push("stale".to_string());
    }

    WaybarOutput {
        text,
        tooltip: tooltip_parts.join("\n"),
        class,
    }
}

/// Coarse session time-left for the waybar glance. At an hour or more, round to
/// the nearest whole hour (1h10m → "1h", 1h30m → "2h"); under an hour, show the
/// minutes (e.g. "45m"). Rounding minutes first keeps the boundary clean so a
/// near-hour reads "1h" rather than "60m".
fn format_session_glance(secs: i64) -> String {
    let total_minutes = (secs as f64 / 60.0).round() as i64;
    if total_minutes < 60 {
        format!("{}m", total_minutes.max(1))
    } else {
        let hours = (total_minutes as f64 / 60.0).round() as i64;
        format!("{}h", hours)
    }
}

#[cfg(test)]
mod tests {
    use super::format_session_glance;

    #[test]
    fn glance_rounds_to_nearest_hour() {
        assert_eq!(format_session_glance(70 * 60), "1h"); // 1h10m
        assert_eq!(format_session_glance(90 * 60), "2h"); // 1h30m
        assert_eq!(format_session_glance(95 * 60), "2h"); // 1h35m
        assert_eq!(format_session_glance(5 * 3600), "5h");
    }

    #[test]
    fn glance_shows_minutes_under_an_hour() {
        assert_eq!(format_session_glance(45 * 60), "45m");
        assert_eq!(format_session_glance(60), "1m");
    }

    #[test]
    fn glance_avoids_sixty_minute_boundary() {
        // 59m40s rounds up to "1h" instead of an ugly "60m"
        assert_eq!(format_session_glance(59 * 60 + 40), "1h");
        assert_eq!(format_session_glance(59 * 60), "59m");
    }
}
