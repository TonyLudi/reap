use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use reap_backtest::BacktestRunner;
use reap_capture::CaptureRunOptions;
use reap_live::{LiveConfig, LiveMode, LiveRunOptions, OperatorCommand, send_operator_command};
use reap_strategy::ChaosConfig;

#[derive(Debug, Parser)]
#[command(name = "reap")]
#[command(about = "Rust chaos/iarb2 strategy, capture, backtest, replay, and OKX runtime")]
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
        #[arg(long, value_enum, default_value_t = ReplayFormat::Csv)]
        format: ReplayFormat,
        #[arg(long)]
        pretty: bool,
    },
    ReplayCheck {
        #[arg(short, long)]
        events: PathBuf,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        pretty: bool,
    },
    ConfigCheck {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    Capture {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(long, help = "Stop public data capture after this many seconds")]
        duration_secs: Option<u64>,
        #[arg(
            long,
            requires = "duration_secs",
            help = "Exit non-zero unless the bounded capture satisfies integrity invariants"
        )]
        require_clean_capture: bool,
        #[arg(long)]
        pretty: bool,
    },
    Live {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(long, value_enum, default_value_t = LiveCliMode::Validate)]
        mode: LiveCliMode,
        #[arg(long)]
        confirm_demo: bool,
        #[arg(long, help = "Stop the live event loop after this many seconds")]
        duration_secs: Option<u64>,
        #[arg(
            long,
            requires = "duration_secs",
            help = "Exit non-zero unless the bounded run satisfies soak invariants"
        )]
        require_clean_soak: bool,
        #[arg(long)]
        pretty: bool,
    },
    Operator {
        #[arg(short, long)]
        config: PathBuf,
        #[command(subcommand)]
        command: OperatorCliCommand,
        #[arg(long, global = true)]
        pretty: bool,
    },
}

#[derive(Debug, Subcommand)]
enum OperatorCliCommand {
    Status,
    Kill {
        #[arg(long)]
        reason: String,
    },
    KillAccount {
        #[arg(long)]
        account: String,
        #[arg(long)]
        reason: String,
    },
    Halt {
        #[arg(long)]
        symbol: String,
        #[arg(long)]
        reason: String,
    },
    Resume {
        #[arg(long)]
        symbol: String,
        #[arg(long)]
        reason: String,
    },
    Shutdown {
        #[arg(long)]
        reason: String,
    },
}

impl From<OperatorCliCommand> for OperatorCommand {
    fn from(value: OperatorCliCommand) -> Self {
        match value {
            OperatorCliCommand::Status => Self::Status,
            OperatorCliCommand::Kill { reason } => Self::KillSwitch { reason },
            OperatorCliCommand::KillAccount { account, reason } => Self::KillAccount {
                account_id: account,
                reason,
            },
            OperatorCliCommand::Halt { symbol, reason } => Self::HaltSymbol { symbol, reason },
            OperatorCliCommand::Resume { symbol, reason } => Self::ResumeSymbol { symbol, reason },
            OperatorCliCommand::Shutdown { reason } => Self::Shutdown { reason },
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ReplayFormat {
    Csv,
    NormalizedJsonl,
    RawCapture,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LiveCliMode {
    Validate,
    Observe,
    Demo,
}

impl From<LiveCliMode> for LiveMode {
    fn from(value: LiveCliMode) -> Self {
        match value {
            LiveCliMode::Validate => Self::Validate,
            LiveCliMode::Observe => Self::Observe,
            LiveCliMode::Demo => Self::Demo,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Backtest {
            config,
            data,
            format,
            pretty,
        } => {
            let config_text = std::fs::read_to_string(&config)
                .with_context(|| format!("failed to read config {}", config.display()))?;
            let config: ChaosConfig = toml::from_str(&config_text)
                .with_context(|| format!("failed to parse config {}", config.display()))?;
            let runner = BacktestRunner::new(config)?;
            let report = match format {
                ReplayFormat::Csv => runner.run_csv_path(&data)?,
                ReplayFormat::NormalizedJsonl => runner.run_normalized_jsonl_path(&data)?,
                ReplayFormat::RawCapture => runner.run_raw_capture_path(&data)?,
            };
            if pretty {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", serde_json::to_string(&report)?);
            }
        }
        Command::ReplayCheck {
            events,
            strict,
            pretty,
        } => {
            let report = reap_feed::replay_check_path(&events)?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", serde_json::to_string(&report)?);
            }
            if strict && !report.is_healthy() {
                anyhow::bail!("raw replay check failed");
            }
        }
        Command::ConfigCheck { config, pretty } => {
            let config_text = std::fs::read_to_string(&config)
                .with_context(|| format!("failed to read config {}", config.display()))?;
            let config: ChaosConfig = toml::from_str(&config_text)
                .with_context(|| format!("failed to parse config {}", config.display()))?;
            let report = config.effective().validate();
            if pretty {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", serde_json::to_string(&report)?);
            }
            if !report.valid {
                anyhow::bail!("configuration validation failed");
            }
        }
        Command::Capture {
            config,
            duration_secs,
            require_clean_capture,
            pretty,
        } => {
            reap_telemetry::init_json_tracing("info")
                .map_err(anyhow::Error::msg)
                .context("failed to initialize capture tracing")?;
            let report = reap_capture::run_capture_path(
                config,
                CaptureRunOptions {
                    run_duration: duration_secs.map(Duration::from_secs),
                },
            )
            .await?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", serde_json::to_string(&report)?);
            }
            if require_clean_capture && !report.clean_capture {
                anyhow::bail!("bounded capture did not satisfy clean integrity invariants");
            }
        }
        Command::Live {
            config,
            mode,
            confirm_demo,
            duration_secs,
            require_clean_soak,
            pretty,
        } => {
            reap_telemetry::init_json_tracing("info")
                .map_err(anyhow::Error::msg)
                .context("failed to initialize live tracing")?;
            let report = reap_live::run_live_path(
                config,
                LiveRunOptions {
                    mode: mode.into(),
                    demo_confirmed: confirm_demo,
                    run_duration: duration_secs.map(Duration::from_secs),
                },
            )
            .await?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", serde_json::to_string(&report)?);
            }
            if require_clean_soak && !report.clean_soak {
                anyhow::bail!("bounded live soak did not satisfy clean acceptance invariants");
            }
        }
        Command::Operator {
            config,
            command,
            pretty,
        } => {
            let config = LiveConfig::load(&config)
                .with_context(|| format!("failed to load live config {}", config.display()))?;
            let response = send_operator_command(&config.operator, command.into()).await?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!("{}", serde_json::to_string(&response)?);
            }
            if !response.ok {
                anyhow::bail!("operator command was rejected: {}", response.message);
            }
        }
    }
    Ok(())
}
