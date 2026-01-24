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
mod mock;
mod models;
mod popup;
mod providers;

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
    Waybar,
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
        Commands::Waybar => {
            let output = waybar_output().await;
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
            let mut snapshots = HashMap::new();

            match fetch_claude().await {
                Ok(s) => {
                    snapshots.insert(Provider::Claude, s);
                }
                Err(e) => eprintln!("Failed to fetch Claude: {}", e),
            }

            match fetch_codex().await {
                Ok(s) => {
                    snapshots.insert(Provider::Codex, s);
                }
                Err(e) => eprintln!("Failed to fetch Codex: {}", e),
            }

            if !snapshots.is_empty() {
                let state = CacheState {
                    snapshots,
                    updated_at: Utc::now(),
                };
                state.save()?;
                println!("Cache updated at {}", CacheState::cache_path().display());
            }
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

async fn waybar_output() -> WaybarOutput {
    // Fetch from all providers (currently Claude + Codex)
    let mut snapshots = HashMap::new();
    let config = Config::load().unwrap_or_default();

    if let Ok(snapshot) = fetch_claude().await {
        snapshots.insert(Provider::Claude, snapshot);
    }
    if let Ok(snapshot) = fetch_codex().await {
        snapshots.insert(Provider::Codex, snapshot);
    }

    // Save to cache
    if !snapshots.is_empty() {
        let state = CacheState {
            snapshots: snapshots.clone(),
            updated_at: Utc::now(),
        };
        let _ = state.save();
    }

    // Build output from snapshots
    build_waybar_output(&snapshots, config.general.selected_provider)
}

fn build_waybar_output(
    snapshots: &HashMap<Provider, UsageSnapshot>,
    selected_provider: Option<Provider>,
) -> WaybarOutput {
    let snapshot = selected_provider
        .and_then(|provider| snapshots.get(&provider))
        .or_else(|| snapshots.get(&Provider::Claude))
        .or_else(|| snapshots.get(&Provider::Codex))
        .or_else(|| snapshots.get(&Provider::OpenCode));
    let Some(snapshot) = snapshot else {
        return WaybarOutput {
            text: "󰧑 --".to_string(),
            tooltip: "No data available".to_string(),
            class: vec!["error".to_string()],
        };
    };

    let session = snapshot.primary.as_ref().map(|r| r.used_percent);
    let week = snapshot.secondary.as_ref().map(|r| r.used_percent);

    // Build text: "󰧑 31% / 51%" (session / week)
    let text = match (session, week) {
        (Some(s), Some(w)) => format!("{} {:.0}% / {:.0}%", snapshot.provider.icon(), s, w),
        (Some(s), None) => format!("{} {:.0}%", snapshot.provider.icon(), s),
        (None, Some(w)) => format!("{} {:.0}%", snapshot.provider.icon(), w),
        (None, None) => format!("{} --", snapshot.provider.icon()),
    };

    // Build tooltip with more detail
    let mut tooltip_parts = vec![snapshot.provider.display_name().to_string()];
    if let Some(ref primary) = snapshot.primary {
        tooltip_parts.push(format!(
            "Session: {:.0}% (resets {})",
            primary.used_percent,
            primary.reset_description.as_deref().unwrap_or("--")
        ));
    }
    if let Some(ref secondary) = snapshot.secondary {
        tooltip_parts.push(format!(
            "Week: {:.0}% (resets {})",
            secondary.used_percent,
            secondary.reset_description.as_deref().unwrap_or("--")
        ));
    }

    // Class based on highest usage
    let max_used = [session, week]
        .into_iter()
        .flatten()
        .fold(0.0_f64, f64::max);
    let class = if max_used >= 90.0 {
        vec!["critical".to_string()]
    } else if max_used >= 75.0 {
        vec!["warning".to_string()]
    } else {
        vec![]
    };

    WaybarOutput {
        text,
        tooltip: tooltip_parts.join("\n"),
        class,
    }
}
