use std::path::PathBuf;
use std::time::Duration;
use std::{fs::OpenOptions, io::Write};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use reap_backtest::{BacktestConfig, BacktestRunner, run_research_manifest_path};
use reap_capture::{CaptureConfig, CaptureRunOptions, analyze_capture_path, run_capture};
use reap_live::{
    EmergencyCancelOptions, LiveConfig, LiveMode, LiveRunOptions, OperatorCommand,
    run_emergency_cancel_path, send_operator_command,
};
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
        #[arg(
            long,
            help = "Exit non-zero on late, invalid, missed, or failed funding accounting"
        )]
        require_complete_accounting: bool,
    },
    #[command(about = "Run deterministic walk-forward selection and execution sensitivity gates")]
    Research {
        #[arg(short, long)]
        manifest: PathBuf,
        #[arg(
            short,
            long,
            help = "Create a JSON evidence artifact; an existing path is refused"
        )]
        output: Option<PathBuf>,
        #[arg(
            long,
            help = "Exit non-zero unless every configured research gate passes"
        )]
        require_pass: bool,
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
    AnalyzeCapture {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(short = 'e', long)]
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
        #[arg(
            long,
            help = "Create this raw output instead of the configured path; existing files are refused"
        )]
        raw_path: Option<PathBuf>,
        #[arg(
            long,
            help = "Create this normalized diagnostic output instead of the configured path"
        )]
        normalized_path: Option<PathBuf>,
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
    #[command(
        about = "Cancel and verify all regular OKX orders for selected accounts",
        long_about = "Arm OKX Cancel All After, cancel every regular pending order account-wide, and verify zero after the trigger horizon. This independent incident path excludes algo and spread orders."
    )]
    EmergencyCancel {
        #[arg(
            short,
            long,
            help = "Live TOML used only for REST/account safety settings"
        )]
        config: PathBuf,
        #[arg(
            long,
            conflicts_with = "all_configured_accounts",
            help = "Configured account id; repeat to select multiple accounts"
        )]
        account: Vec<String>,
        #[arg(
            long,
            conflicts_with = "account",
            help = "Select every account in the config"
        )]
        all_configured_accounts: bool,
        #[arg(long, help = "Acknowledge that cancellation is account-wide")]
        confirm_account_wide_cancel: bool,
        #[arg(
            long,
            help = "Attest that every order producer for the selected accounts is stopped"
        )]
        confirm_order_producers_stopped: bool,
        #[arg(
            long,
            help = "Additional acknowledgement required by production configs"
        )]
        confirm_production: bool,
        #[arg(
            long,
            default_value_t = 30,
            help = "Absolute deadline for each account"
        )]
        account_timeout_secs: u64,
        #[arg(long, default_value_t = 250, help = "Delay between zero checks")]
        poll_interval_ms: u64,
        #[arg(
            long,
            default_value_t = 10,
            help = "OKX Cancel All After trigger delay (10-120 seconds)"
        )]
        deadman_timeout_secs: u64,
        #[arg(long, help = "Pretty-print JSON evidence")]
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
            require_complete_accounting,
        } => {
            let config_text = std::fs::read_to_string(&config)
                .with_context(|| format!("failed to read config {}", config.display()))?;
            let config: BacktestConfig = toml::from_str(&config_text)
                .with_context(|| format!("failed to parse config {}", config.display()))?;
            let runner = BacktestRunner::from_config(config)?;
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
            if require_complete_accounting && !report.accounting_complete {
                anyhow::bail!("backtest accounting is incomplete");
            }
        }
        Command::Research {
            manifest,
            output,
            require_pass,
            pretty,
        } => {
            let report = run_research_manifest_path(&manifest).with_context(|| {
                format!("failed to run research manifest {}", manifest.display())
            })?;
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let Some(output) = output {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&output)
                    .with_context(|| {
                        format!("failed to create research output {}", output.display())
                    })?;
                file.write_all(json.as_bytes())?;
                file.write_all(b"\n")?;
                file.sync_all()?;
            }
            println!("{json}");
            if require_pass && !report.passed {
                anyhow::bail!("research report did not satisfy configured gates");
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
        Command::AnalyzeCapture {
            config,
            events,
            strict,
            pretty,
        } => {
            let config = CaptureConfig::load(&config)
                .with_context(|| format!("failed to load capture config {}", config.display()))?;
            let report = analyze_capture_path(&events, &config)
                .with_context(|| format!("failed to analyze raw capture {}", events.display()))?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", serde_json::to_string(&report)?);
            }
            if strict && !report.integrity_healthy {
                anyhow::bail!("capture analysis failed integrity invariants");
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
            raw_path,
            normalized_path,
            duration_secs,
            require_clean_capture,
            pretty,
        } => {
            reap_telemetry::init_json_tracing("info")
                .map_err(anyhow::Error::msg)
                .context("failed to initialize capture tracing")?;
            let mut capture_config = CaptureConfig::load(&config)
                .with_context(|| format!("failed to load capture config {}", config.display()))?;
            if let Some(path) = raw_path {
                capture_config.output.raw_path = path;
            }
            if let Some(path) = normalized_path {
                capture_config.output.normalized_path = Some(path);
            }
            let report = run_capture(
                capture_config,
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
        Command::EmergencyCancel {
            config,
            account,
            all_configured_accounts,
            confirm_account_wide_cancel,
            confirm_order_producers_stopped,
            confirm_production,
            account_timeout_secs,
            poll_interval_ms,
            deadman_timeout_secs,
            pretty,
        } => {
            reap_telemetry::init_json_tracing("info")
                .map_err(anyhow::Error::msg)
                .context("failed to initialize emergency-cancel tracing")?;
            let report = run_emergency_cancel_path(
                &config,
                EmergencyCancelOptions {
                    account_ids: account,
                    all_configured_accounts,
                    confirm_account_wide_cancel,
                    confirm_order_producers_stopped,
                    confirm_production,
                    account_timeout: Duration::from_secs(account_timeout_secs),
                    poll_interval: Duration::from_millis(poll_interval_ms),
                    deadman_timeout_secs,
                },
            )
            .await
            .with_context(|| {
                format!(
                    "emergency cancel failed before producing evidence for {}",
                    config.display()
                )
            })?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", serde_json::to_string(&report)?);
            }
            if !report.all_clear {
                anyhow::bail!(
                    "emergency cancel did not verify every selected account's regular order book at zero"
                );
            }
        }
    }
    Ok(())
}
