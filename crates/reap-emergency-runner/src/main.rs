use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use reap_emergency_core::{EmergencyAccountStopFactory, EmergencyCancelOptions};
use reap_emergency_runner::run_emergency_cancel_path_with_factory;
use reap_okx_emergency_adapter::OkxEmergencyAccountStopFactory;

#[derive(Debug, Parser)]
#[command(name = "reap-emergency")]
#[command(
    about = "Cancel and verify regular, algo, and spread OKX orders",
    long_about = "Arm regular and spread OKX Cancel All After, exhaustively cancel regular, algo, and spread pending orders account-wide, and verify every domain at zero after the trigger horizon."
)]
struct Cli {
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
        default_value_t = 40,
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
    #[arg(
        short,
        long,
        help = "Create a JSON evidence artifact; an existing path is refused"
    )]
    output: Option<PathBuf>,
    #[arg(long, help = "Pretty-print JSON evidence")]
    pretty: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    reap_telemetry::init_json_tracing("info")
        .map_err(anyhow::Error::msg)
        .context("failed to initialize emergency-cancel tracing")?;
    execute_with_factory(cli, &OkxEmergencyAccountStopFactory).await
}

async fn execute_with_factory(cli: Cli, factory: &dyn EmergencyAccountStopFactory) -> Result<()> {
    let Cli {
        config,
        account,
        all_configured_accounts,
        confirm_account_wide_cancel,
        confirm_order_producers_stopped,
        confirm_production,
        account_timeout_secs,
        poll_interval_ms,
        deadman_timeout_secs,
        output,
        pretty,
    } = cli;
    let mut output_file = output
        .as_ref()
        .map(|path| reserve_private_output(path, "emergency-cancel report"))
        .transpose()?;
    let report = run_emergency_cancel_path_with_factory(
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
        factory,
    )
    .await
    .with_context(|| {
        format!(
            "emergency cancel failed before producing evidence for {}",
            config.display()
        )
    })?;
    let json = if pretty {
        serde_json::to_string_pretty(&report)?
    } else {
        serde_json::to_string(&report)?
    };
    if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
        persist_reserved_output(file, path, &json, "emergency-cancel report")?;
    }
    println!("{json}");
    if !report.account_wide_orders_all_clear {
        anyhow::bail!(
            "emergency cancel did not verify every selected account's regular, algo, and spread orders at zero"
        );
    }
    if !report.evidence_complete {
        anyhow::bail!(
            "emergency cancel reached account-wide zero but its provenance evidence is incomplete"
        );
    }
    if !report.all_clear {
        anyhow::bail!("emergency cancel report violated its all-clear invariant");
    }
    Ok(())
}

fn reserve_private_output(path: &Path, label: &str) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("failed to reserve {label} {}", path.display()))
}

fn persist_reserved_output(file: &mut File, path: &Path, json: &str, label: &str) -> Result<()> {
    file.write_all(json.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .with_context(|| format!("failed to persist {label} {}", path.display()))?;
    sync_parent_directory(path)
        .with_context(|| format!("failed to persist {label} directory for {}", path.display()))
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use reap_emergency_core::{
        EmergencyAccountConfig, EmergencyAccountStopRole, EmergencyRoleSetupError,
        EmergencyRuntimeConfig, EmergencyVenueConfig,
    };

    use super::*;

    struct PanicFactory;

    impl EmergencyAccountStopFactory for PanicFactory {
        fn create(
            &self,
            _venue: &EmergencyVenueConfig,
            _runtime: &EmergencyRuntimeConfig,
            _account: &EmergencyAccountConfig,
        ) -> std::result::Result<Box<dyn EmergencyAccountStopRole>, EmergencyRoleSetupError>
        {
            panic!("factory must not run before config validation")
        }
    }

    #[test]
    fn cli_preserves_defaults_and_rejects_conflicting_account_selection() {
        let cli = Cli::try_parse_from([
            "reap-emergency",
            "--config",
            "live.toml",
            "--account",
            "main",
            "--confirm-account-wide-cancel",
            "--confirm-order-producers-stopped",
        ])
        .unwrap();
        assert_eq!(cli.account, ["main"]);
        assert_eq!(cli.account_timeout_secs, 40);
        assert_eq!(cli.poll_interval_ms, 250);
        assert_eq!(cli.deadman_timeout_secs, 10);

        assert!(
            Cli::try_parse_from([
                "reap-emergency",
                "--config",
                "live.toml",
                "--account",
                "main",
                "--all-configured-accounts",
            ])
            .is_err()
        );
    }

    #[tokio::test]
    async fn arbitrary_remote_origin_fails_before_factory_or_network() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("live.toml");
        std::fs::write(
            &config,
            r#"[venue]
environment = "demo"
rest_url = "https://attacker.example"

[[accounts]]
id = "main"
api_key_env = "API_KEY"
secret_key_env = "SECRET_KEY"
passphrase_env = "PASSPHRASE"
"#,
        )
        .unwrap();
        let cli = Cli::try_parse_from([
            "reap-emergency".into(),
            "--config".into(),
            config.into_os_string(),
            "--account".into(),
            "main".into(),
            "--confirm-account-wide-cancel".into(),
            "--confirm-order-producers-stopped".into(),
        ])
        .unwrap();

        let error = execute_with_factory(cli, &PanicFactory).await.unwrap_err();

        assert!(
            format!("{error:#}").contains("host is not a documented OKX REST origin"),
            "{error:#}"
        );
    }

    #[test]
    fn private_output_is_owner_readable_create_new_and_durable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("emergency.json");
        let mut file = reserve_private_output(&path, "test report").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        persist_reserved_output(&mut file, &path, "{\"all_clear\":false}", "test report").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"all_clear\":false}\n"
        );
        assert!(reserve_private_output(&path, "test report").is_err());
    }
}
