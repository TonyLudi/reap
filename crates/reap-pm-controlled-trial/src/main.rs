use std::{error::Error, path::PathBuf};

use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use reap_pm_controlled_trial::{
    CustodyPaths, inspect_custody, load_canonical_authorization, load_canonical_trial_config,
    verify_authorization, verify_plan,
};

#[derive(Debug, Parser)]
#[command(
    name = "reap-pm-controlled-trial",
    about = "Offline-only PM-T2 plan, authorization, and secret-custody gate"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Verify one exact canonical non-secret PM-T2 trial plan.
    VerifyPlan {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    /// Structurally verify a separate short-lived exact authorization.
    VerifyAuthorization {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        authorization: PathBuf,
        /// Explicit canonical UTC-seconds observation, for reproducible offline verification.
        #[arg(long)]
        verification_time_utc: String,
        #[arg(long)]
        pretty: bool,
    },
    /// Inspect exactly four staged secret files without signing or transport.
    InspectCustody {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        private_key: PathBuf,
        #[arg(long)]
        api_key: PathBuf,
        #[arg(long)]
        l2_secret: PathBuf,
        #[arg(long)]
        passphrase: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::VerifyPlan { config, pretty } => {
            let config = load_canonical_trial_config(&config)?;
            print_json(&verify_plan(&config), pretty)?;
        }
        Command::VerifyAuthorization {
            config,
            authorization,
            verification_time_utc,
            pretty,
        } => {
            let config = load_canonical_trial_config(&config)?;
            let authorization = load_canonical_authorization(&authorization)?;
            let now = DateTime::parse_from_rfc3339(&verification_time_utc)?.with_timezone(&Utc);
            if now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != verification_time_utc {
                return Err("verification time must be canonical UTC seconds".into());
            }
            print_json(&verify_authorization(&config, &authorization, now)?, pretty)?;
        }
        Command::InspectCustody {
            config,
            private_key,
            api_key,
            l2_secret,
            passphrase,
            pretty,
        } => {
            let config = load_canonical_trial_config(&config)?;
            let inspection = inspect_custody(
                &config,
                CustodyPaths {
                    private_key,
                    api_key,
                    l2_secret,
                    passphrase,
                },
            )?;
            print_json(inspection.summary(), pretty)?;
        }
    }
    Ok(())
}

fn print_json(value: &impl serde::Serialize, pretty: bool) -> Result<(), serde_json::Error> {
    let output = if pretty {
        serde_json::to_string_pretty(value)?
    } else {
        serde_json::to_string(value)?
    };
    println!("{output}");
    Ok(())
}
