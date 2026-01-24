#![allow(dead_code)]

use anyhow::Result;
use cache::CacheState;
use chrono::Utc;
use clap::{Parser, Subcommand};
use models::Provider;
use providers::claude::ClaudeProvider;
use providers::ProviderFetcher;
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
            eprintln!("waybar command not yet implemented");
        }
        Commands::Status => {
            let snapshot = fetch_claude().await;
            match snapshot {
                Ok(s) => print_status(&s),
                Err(e) => eprintln!("Claude: {}", e),
            }
        }
        Commands::Fetch => {
            let snapshot = fetch_claude().await;
            match snapshot {
                Ok(s) => {
                    let mut snapshots = HashMap::new();
                    snapshots.insert(Provider::Claude, s);
                    let state = CacheState {
                        snapshots,
                        updated_at: Utc::now(),
                    };
                    state.save()?;
                    println!("Cache updated at {}", CacheState::cache_path().display());
                }
                Err(e) => eprintln!("Failed to fetch Claude: {}", e),
            }
        }
    }

    Ok(())
}

async fn fetch_claude() -> Result<models::UsageSnapshot> {
    let provider = ClaudeProvider::new();
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
            "  Session: {:.0}% used {}",
            primary.used_percent,
            primary.reset_description.as_deref().unwrap_or("")
        );
    }
    if let Some(ref secondary) = snapshot.secondary {
        println!(
            "  Weekly:  {:.0}% used {}",
            secondary.used_percent,
            secondary.reset_description.as_deref().unwrap_or("")
        );
    }
    if let Some(ref tertiary) = snapshot.tertiary {
        println!(
            "  Model:   {:.0}% used {}",
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
