use std::{error::Error, path::PathBuf};

use clap::{Parser, Subcommand};
use reap_pm_readiness::{
    collect_pm_read_only_account_path, collect_pm_read_only_smoke_path,
    require_pm_read_only_account_pass, require_pm_read_only_smoke_pass,
    resolve_pm_read_only_credentials_directory, verify_pm_read_only_account_path_with_anchors,
    verify_pm_read_only_smoke_path_with_anchors,
};

#[derive(Debug, Parser)]
#[command(
    name = "reap-pm-readiness",
    about = "Credentialed, read-only Polymarket readiness evidence"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Collect one bounded production read-only certification artifact.
    Certify {
        #[arg(long)]
        config: PathBuf,
        /// Protected runtime directory containing the three configured entries.
        /// If omitted, the non-secret CREDENTIALS_DIRECTORY path is used.
        #[arg(long)]
        credentials_dir: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
        /// Pretty-print only the secret-free summary to stdout.
        #[arg(long)]
        pretty: bool,
    },
    /// Collect exactly two public-time reads and two authenticated account GETs.
    CertifyAccount {
        #[arg(long)]
        config: PathBuf,
        /// Protected runtime directory containing the three configured entries.
        /// If omitted, the non-secret CREDENTIALS_DIRECTORY path is used.
        #[arg(long)]
        credentials_dir: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
        /// Pretty-print only the secret-free summary to stdout.
        #[arg(long)]
        pretty: bool,
    },
    /// Verify one artifact against a reviewed config and this exact executable.
    Verify {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        require_pass: bool,
        /// Pretty-print only the secret-free summary to stdout.
        #[arg(long)]
        pretty: bool,
    },
    /// Verify one account-only artifact against reviewed config and executable.
    VerifyAccount {
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        require_pass: bool,
        /// Pretty-print only the secret-free summary to stdout.
        #[arg(long)]
        pretty: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Certify {
            config,
            credentials_dir,
            output,
            pretty,
        } => {
            let credentials_dir =
                resolve_pm_read_only_credentials_directory(credentials_dir.as_deref())?;
            let artifact =
                collect_pm_read_only_smoke_path(&config, &credentials_dir, &output).await?;
            print_json(if pretty {
                serde_json::to_string_pretty(&artifact.summary)?
            } else {
                serde_json::to_string(&artifact.summary)?
            });
            require_pm_read_only_smoke_pass(&artifact)?;
        }
        Command::Verify {
            artifact,
            config,
            require_pass,
            pretty,
        } => {
            let verified = verify_pm_read_only_smoke_path_with_anchors(&artifact, &config)?;
            print_json(if pretty {
                serde_json::to_string_pretty(&verified.summary)?
            } else {
                serde_json::to_string(&verified.summary)?
            });
            if require_pass {
                require_pm_read_only_smoke_pass(&verified)?;
            }
        }
        Command::CertifyAccount {
            config,
            credentials_dir,
            output,
            pretty,
        } => {
            let credentials_dir =
                resolve_pm_read_only_credentials_directory(credentials_dir.as_deref())?;
            let artifact =
                collect_pm_read_only_account_path(&config, &credentials_dir, &output).await?;
            print_json(if pretty {
                serde_json::to_string_pretty(&artifact.summary)?
            } else {
                serde_json::to_string(&artifact.summary)?
            });
            require_pm_read_only_account_pass(&artifact)?;
        }
        Command::VerifyAccount {
            artifact,
            config,
            require_pass,
            pretty,
        } => {
            let verified = verify_pm_read_only_account_path_with_anchors(&artifact, &config)?;
            print_json(if pretty {
                serde_json::to_string_pretty(&verified.summary)?
            } else {
                serde_json::to_string(&verified.summary)?
            });
            if require_pass {
                require_pm_read_only_account_pass(&verified)?;
            }
        }
    }
    Ok(())
}

fn print_json(value: String) {
    println!("{value}");
}
