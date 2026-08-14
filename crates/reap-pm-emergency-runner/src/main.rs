//! Explicit Polymarket account-wide emergency cancel executable.

#![forbid(unsafe_code)]

use std::{error::Error, net::IpAddr, path::PathBuf, time::Duration};

use clap::Parser;
use reap_polymarket_credential_file::load_predarb_credential_file;
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_emergency_adapter::{PmEmergencyAccountStopRole, PmEmergencyProductionConfig};
use serde::Serialize;

const AUTHORIZATION_PHRASE: &str = "I_AUTHORIZE_POLYMARKET_ACCOUNT_WIDE_CANCEL_ALL";

#[derive(Debug, Parser)]
#[command(
    name = "reap-pm-emergency-runner",
    about = "Isolated Polymarket cancel-all plus complete zero-order verification"
)]
struct Cli {
    /// Existing protected Predarb .env; secret values are never printed.
    #[arg(long, default_value = "../predarb/.env")]
    credential_env: PathBuf,
    /// One currently reviewed clob.polymarket.com IP; no runtime DNS fallback.
    #[arg(long)]
    fixed_peer_ip: String,
    /// Exact Linux interface used for the connection.
    #[arg(long)]
    interface_name: String,
    /// Exact local interface address, including a private address behind NAT.
    #[arg(long)]
    local_source_ip: IpAddr,
    /// Must equal I_AUTHORIZE_POLYMARKET_ACCOUNT_WIDE_CANCEL_ALL.
    #[arg(long)]
    authorization_phrase: String,
    /// Bounded broad-cancel attempts, each followed by a complete order cut.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=10))]
    max_attempts: u8,
    /// Delay between non-zero verification cuts.
    #[arg(long, default_value_t = 500, value_parser = clap::value_parser!(u64).range(50..=5000))]
    poll_interval_ms: u64,
}

#[derive(Debug, Serialize)]
struct EmergencyReport {
    schema_version: u8,
    attempts: u8,
    canceled_orders_reported: usize,
    not_canceled_orders_reported: usize,
    final_cut_pages: usize,
    final_open_orders: usize,
    all_clear: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    if cli.authorization_phrase != AUTHORIZATION_PHRASE {
        return Err("exact account-wide cancellation authorization phrase required".into());
    }
    let authorities = load_predarb_credential_file(&cli.credential_env)?;
    let (signer, l2, _funder_identity) = authorities.into_parts();
    // The emergency plane has no order-signing operation. Destroy the signer
    // before constructing its L2-only account-stop role.
    drop(signer);

    let peer = PmFixedTlsPeerSelection::production("clob.polymarket.com", &cli.fixed_peer_ip)?;
    let egress = PmLocalEgressSelection::production(&cli.interface_name, cli.local_source_ip)?;
    let config =
        PmEmergencyProductionConfig::production_on_fixed_tls_peer_and_selected_local_egress(
            Duration::from_secs(5),
            Duration::from_secs(15),
            peer,
            egress,
        )?;
    let role = PmEmergencyAccountStopRole::new(config, l2)?;

    let mut report = EmergencyReport {
        schema_version: 1,
        attempts: 0,
        canceled_orders_reported: 0,
        not_canceled_orders_reported: 0,
        final_cut_pages: 0,
        final_open_orders: usize::MAX,
        all_clear: false,
    };
    for attempt in 1..=cli.max_attempts {
        let cancellation = role.cancel_all().await?;
        report.attempts = attempt;
        report.canceled_orders_reported = report
            .canceled_orders_reported
            .saturating_add(cancellation.canceled_orders());
        report.not_canceled_orders_reported = report
            .not_canceled_orders_reported
            .saturating_add(cancellation.not_canceled_orders());
        let cut = role.complete_open_orders().await?;
        report.final_cut_pages = cut.pages();
        report.final_open_orders = cut.open_orders();
        if cut.is_zero() {
            report.all_clear = true;
            break;
        }
        if attempt < cli.max_attempts {
            tokio::time::sleep(Duration::from_millis(cli.poll_interval_ms)).await;
        }
    }

    println!("{}", serde_json::to_string(&report)?);
    if !report.all_clear {
        return Err("Polymarket emergency cleanup did not verify zero open orders".into());
    }
    Ok(())
}
