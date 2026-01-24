use anyhow::Result;
use clap::{Parser, Subcommand};

mod cache;
mod config;
mod mock;
mod models;
mod popup;

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

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Popup { mock } => {
            popup::run(mock)?;
        }
        Commands::Waybar => {
            eprintln!("waybar command not yet implemented");
        }
        Commands::Status => {
            eprintln!("status command not yet implemented");
        }
        Commands::Fetch => {
            eprintln!("fetch command not yet implemented");
        }
    }

    Ok(())
}
