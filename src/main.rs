use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reap::{BacktestRunner, ChaosConfig};

#[derive(Debug, Parser)]
#[command(name = "reap")]
#[command(about = "Rust replication of the core imm-strategy chaos strategy/backtest loop")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Backtest {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(short, long)]
        data: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Backtest {
            config,
            data,
            pretty,
        } => {
            let config_text = std::fs::read_to_string(&config)
                .with_context(|| format!("failed to read config {}", config.display()))?;
            let config: ChaosConfig = toml::from_str(&config_text)
                .with_context(|| format!("failed to parse config {}", config.display()))?;
            let report = BacktestRunner::new(config).run_csv_path(&data)?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", serde_json::to_string(&report)?);
            }
        }
    }
    Ok(())
}
