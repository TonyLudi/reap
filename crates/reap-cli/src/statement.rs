use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use reap_live::{
    FillCollectionOptions, FillStatementReconciliationOptions, FillStatementTolerances,
    collect_recent_okx_fills_paths, reconcile_okx_fill_collection_paths,
    reconcile_okx_fill_statement_paths,
};

#[derive(Debug, Args)]
pub(crate) struct CollectFillsArgs {
    #[arg(
        short,
        long,
        help = "Validated live TOML and account credential mapping"
    )]
    config: PathBuf,
    #[arg(long, help = "Configured Reap account id to query")]
    account: String,
    #[arg(long, help = "Inclusive collection-window start in Unix milliseconds")]
    begin_ms: u64,
    #[arg(long, help = "Inclusive collection-window end in Unix milliseconds")]
    end_ms: u64,
    #[arg(
        short,
        long = "output",
        help = "Create this private evidence directory; an existing path is refused"
    )]
    output_directory: PathBuf,
    #[arg(long, default_value_t = 100, help = "Fail-closed pagination bound")]
    max_pages: usize,
    #[arg(
        long,
        default_value_t = 200,
        help = "Minimum delay between authenticated fill-page requests"
    )]
    page_interval_ms: u64,
    #[arg(
        long,
        default_value_t = 60_000,
        help = "Required delay from window end to exchange collection start"
    )]
    minimum_window_close_delay_ms: u64,
    #[arg(long)]
    pretty: bool,
}

pub(crate) async fn collect(args: CollectFillsArgs) -> Result<()> {
    let manifest = collect_recent_okx_fills_paths(
        &args.config,
        &args.output_directory,
        FillCollectionOptions {
            account_id: args.account,
            begin_ms: args.begin_ms,
            end_ms: args.end_ms,
            max_pages: args.max_pages,
            page_interval_ms: args.page_interval_ms,
            minimum_window_close_delay_ms: args.minimum_window_close_delay_ms,
        },
    )
    .await
    .with_context(|| {
        format!(
            "failed to collect authenticated fills into {}",
            args.output_directory.display()
        )
    })?;
    let json = if args.pretty {
        serde_json::to_string_pretty(&manifest)?
    } else {
        serde_json::to_string(&manifest)?
    };
    println!("{json}");
    Ok(())
}

#[derive(Debug, Args)]
pub(crate) struct ReconcileFillsArgs {
    #[arg(long, help = "Stopped runtime's canonical JSONL journal")]
    journal: PathBuf,
    #[arg(
        long = "statement",
        help = "Unmodified OKX fills/fills-history JSON response; repeat for every manual page"
    )]
    statements: Vec<PathBuf>,
    #[arg(
        long,
        help = "Verified manifest produced by collect-fills; replaces manual --statement pages"
    )]
    collection_manifest: Option<PathBuf>,
    #[arg(
        long,
        help = "Configured Reap account id represented by the OKX responses"
    )]
    account: String,
    #[arg(
        long,
        help = "Inclusive reconciliation-window start in Unix milliseconds"
    )]
    begin_ms: u64,
    #[arg(
        long,
        help = "Inclusive reconciliation-window end in Unix milliseconds"
    )]
    end_ms: u64,
    #[arg(
        long,
        default_value_t = 1,
        help = "Minimum compared fills required to pass"
    )]
    minimum_fills: u64,
    #[arg(long, default_value_t = 0.0, help = "Absolute fill-price tolerance")]
    price_tolerance: f64,
    #[arg(long, default_value_t = 0.0, help = "Absolute fill-quantity tolerance")]
    quantity_tolerance: f64,
    #[arg(long, default_value_t = 0.0, help = "Absolute signed-fee tolerance")]
    fee_tolerance: f64,
    #[arg(
        long,
        help = "Attest that the supplied responses are from this account and completely cover the window"
    )]
    confirm_statement_account_and_window_complete: bool,
    #[arg(
        short,
        long,
        help = "Create the JSON evidence artifact; an existing path is refused"
    )]
    output: PathBuf,
    #[arg(long, help = "Exit non-zero unless reconciliation passes")]
    require_pass: bool,
    #[arg(long)]
    pretty: bool,
}

pub(crate) fn run(args: ReconcileFillsArgs) -> Result<()> {
    match (
        args.statements.is_empty(),
        args.collection_manifest.is_some(),
    ) {
        (true, false) => {
            anyhow::bail!("provide either --collection-manifest or at least one --statement page")
        }
        (false, true) => anyhow::bail!(
            "--collection-manifest and manual --statement pages are mutually exclusive"
        ),
        _ => {}
    }
    if args.collection_manifest.is_some() && args.confirm_statement_account_and_window_complete {
        anyhow::bail!(
            "--confirm-statement-account-and-window-complete applies only to manual --statement pages"
        );
    }

    let mut output_options = OpenOptions::new();
    output_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        output_options.mode(0o600);
    }
    let mut output_file = output_options.open(&args.output).with_context(|| {
        format!(
            "failed to reserve fill reconciliation output {}",
            args.output.display()
        )
    })?;

    let options = FillStatementReconciliationOptions {
        account_id: args.account,
        begin_ms: args.begin_ms,
        end_ms: args.end_ms,
        minimum_fills: args.minimum_fills,
        tolerances: FillStatementTolerances {
            price_abs: args.price_tolerance,
            quantity_abs: args.quantity_tolerance,
            fee_abs: args.fee_tolerance,
        },
        statement_account_and_window_completeness_attested: args
            .confirm_statement_account_and_window_complete,
    };
    let report = if let Some(manifest) = args.collection_manifest {
        reconcile_okx_fill_collection_paths(&args.journal, manifest, options)?
    } else {
        reconcile_okx_fill_statement_paths(&args.journal, &args.statements, options)?
    };
    let json = if args.pretty {
        serde_json::to_string_pretty(&report)?
    } else {
        serde_json::to_string(&report)?
    };
    output_file.write_all(json.as_bytes())?;
    output_file.write_all(b"\n")?;
    output_file.sync_all()?;
    println!("{json}");

    if args.require_pass && !report.passed {
        anyhow::bail!("fill/fee statement reconciliation did not pass");
    }
    Ok(())
}
