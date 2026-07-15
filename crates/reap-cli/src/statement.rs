use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use reap_live::{
    BillCollectionOptions, EconomicReconciliationOptions, EconomicReconciliationTolerances,
    FillCollectionOptions, FillStatementReconciliationOptions, FillStatementTolerances,
    collect_okx_bills_paths, collect_recent_okx_fills_paths, reconcile_okx_economics_paths,
    reconcile_okx_fill_collection_paths, reconcile_okx_fill_statement_paths,
    verify_bill_collection_manifest_path,
};

use crate::{persist_reserved_output, reserve_private_output};

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
pub(crate) struct CollectBillsArgs {
    #[arg(
        short,
        long,
        help = "Validated live TOML and account credential mapping"
    )]
    config: PathBuf,
    #[arg(long, help = "Configured Reap account id to query")]
    account: String,
    #[arg(long, help = "Inclusive closed-window start in Unix milliseconds")]
    begin_ms: u64,
    #[arg(long, help = "Inclusive closed-window end in Unix milliseconds")]
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
        default_value_t = 500,
        help = "Minimum delay between authenticated account-bill requests"
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

pub(crate) async fn collect_bills(args: CollectBillsArgs) -> Result<()> {
    let manifest = collect_okx_bills_paths(
        &args.config,
        &args.output_directory,
        BillCollectionOptions {
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
            "failed to collect authenticated account bills into {}",
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
pub(crate) struct VerifyBillCollectionArgs {
    #[arg(
        short,
        long,
        help = "Bill-collection manifest to reconstruct from exact local sources"
    )]
    manifest: PathBuf,
    #[arg(
        short,
        long,
        help = "Optionally create an owner-readable verification summary"
    )]
    output: Option<PathBuf>,
    #[arg(long)]
    pretty: bool,
}

pub(crate) fn verify_bills(args: VerifyBillCollectionArgs) -> Result<()> {
    let mut reserved_output = args
        .output
        .as_ref()
        .map(|path| reserve_private_output(path, "bill verification output"))
        .transpose()?;
    let verified = verify_bill_collection_manifest_path(&args.manifest).with_context(|| {
        format!(
            "failed to verify bill collection {}",
            args.manifest.display()
        )
    })?;
    let summary = verified.summary();
    let json = if args.pretty {
        serde_json::to_string_pretty(&summary)?
    } else {
        serde_json::to_string(&summary)?
    };
    if let (Some(path), Some(file)) = (args.output.as_ref(), reserved_output.as_mut()) {
        persist_reserved_output(file, path, &json, "bill verification output")?;
    }
    println!("{json}");
    Ok(())
}

#[derive(Debug, Args)]
pub(crate) struct ReconcileEconomicsArgs {
    #[arg(long, help = "Stopped runtime's canonical JSONL journal")]
    journal: PathBuf,
    #[arg(
        long,
        help = "Verified manifest from collect-fills; its window must include the trade-delay guard"
    )]
    fill_collection_manifest: PathBuf,
    #[arg(
        long,
        help = "Verified account-wide manifest from collect-bills for the exact reconciliation window"
    )]
    bill_collection_manifest: PathBuf,
    #[arg(
        long,
        help = "Passing account-certification artifact collected before begin-ms"
    )]
    opening_account_certification: PathBuf,
    #[arg(
        long,
        help = "Passing account-certification artifact collected after end-ms"
    )]
    closing_account_certification: PathBuf,
    #[arg(long, help = "Configured Reap account id represented by all sources")]
    account: String,
    #[arg(
        long,
        help = "Inclusive account-bill window start in Unix milliseconds"
    )]
    begin_ms: u64,
    #[arg(long, help = "Inclusive account-bill window end in Unix milliseconds")]
    end_ms: u64,
    #[arg(
        long,
        default_value_t = 1,
        help = "Minimum fully validated trade bills"
    )]
    minimum_trade_bills: u64,
    #[arg(
        long,
        default_value_t = 1,
        help = "Minimum derivative close bills with independently recomputed PnL"
    )]
    minimum_derivative_close_bills: u64,
    #[arg(
        long,
        default_value_t = 1,
        help = "Minimum fully recomputed funding bills"
    )]
    minimum_funding_bills: u64,
    #[arg(
        long,
        default_value_t = 60_000,
        help = "Maximum causal delay from fillTime to trade-bill ts"
    )]
    maximum_trade_bill_delay_ms: u64,
    #[arg(
        long,
        default_value_t = 60_000,
        help = "Maximum causal delay from scheduled settlement to funding-bill ts"
    )]
    maximum_funding_bill_delay_ms: u64,
    #[arg(
        long,
        default_value_t = 1_000,
        help = "Maximum exchange-time distance on each side of the funding assessment mark bracket"
    )]
    maximum_funding_mark_bracket_distance_ms: u64,
    #[arg(
        long,
        default_value_t = 60_000,
        help = "Maximum gap from each account certification to its reconciliation-window boundary"
    )]
    maximum_account_boundary_gap_ms: u64,
    #[arg(long, default_value_t = 0.0, help = "Absolute trade-price tolerance")]
    price_tolerance: f64,
    #[arg(
        long,
        default_value_t = 1e-9,
        help = "Absolute bill-quantity tolerance"
    )]
    quantity_tolerance: f64,
    #[arg(long, default_value_t = 1e-12, help = "Absolute signed-fee tolerance")]
    fee_tolerance: f64,
    #[arg(
        long,
        default_value_t = 1e-10,
        help = "Absolute balance-equation tolerance"
    )]
    balance_tolerance: f64,
    #[arg(
        long,
        default_value_t = 1e-10,
        help = "Absolute derivative trade-PnL formula tolerance"
    )]
    trade_pnl_absolute_tolerance: f64,
    #[arg(
        long,
        default_value_t = 1e-10,
        help = "Relative derivative trade-PnL formula tolerance"
    )]
    trade_pnl_relative_tolerance: f64,
    #[arg(
        long,
        default_value_t = 1e-12,
        help = "Absolute funding-PnL formula tolerance"
    )]
    funding_pnl_absolute_tolerance: f64,
    #[arg(
        long,
        default_value_t = 1e-8,
        help = "Relative funding-PnL formula tolerance"
    )]
    funding_pnl_relative_tolerance: f64,
    #[arg(
        long,
        default_value_t = 1e-8,
        help = "Absolute tolerance around the journaled funding mark bracket"
    )]
    funding_mark_absolute_tolerance: f64,
    #[arg(
        long,
        default_value_t = 1e-5,
        help = "Relative tolerance around the journaled funding mark bracket"
    )]
    funding_mark_relative_tolerance: f64,
    #[arg(
        short,
        long,
        help = "Create this owner-readable reconciliation artifact; existing files are refused"
    )]
    output: PathBuf,
    #[arg(long, help = "Exit non-zero unless economic reconciliation passes")]
    require_pass: bool,
    #[arg(long)]
    pretty: bool,
}

pub(crate) fn reconcile_economics(args: ReconcileEconomicsArgs) -> Result<()> {
    let mut output = reserve_private_output(&args.output, "economic reconciliation output")?;
    let report = reconcile_okx_economics_paths(
        &args.journal,
        &args.fill_collection_manifest,
        &args.bill_collection_manifest,
        &args.opening_account_certification,
        &args.closing_account_certification,
        EconomicReconciliationOptions {
            account_id: args.account,
            begin_ms: args.begin_ms,
            end_ms: args.end_ms,
            minimum_trade_bills: args.minimum_trade_bills,
            minimum_derivative_close_bills: args.minimum_derivative_close_bills,
            minimum_funding_bills: args.minimum_funding_bills,
            maximum_trade_bill_delay_ms: args.maximum_trade_bill_delay_ms,
            maximum_funding_bill_delay_ms: args.maximum_funding_bill_delay_ms,
            maximum_funding_mark_bracket_distance_ms: args.maximum_funding_mark_bracket_distance_ms,
            maximum_account_boundary_gap_ms: args.maximum_account_boundary_gap_ms,
            tolerances: EconomicReconciliationTolerances {
                price_abs: args.price_tolerance,
                quantity_abs: args.quantity_tolerance,
                fee_abs: args.fee_tolerance,
                balance_abs: args.balance_tolerance,
                trade_pnl_abs: args.trade_pnl_absolute_tolerance,
                trade_pnl_relative: args.trade_pnl_relative_tolerance,
                funding_pnl_abs: args.funding_pnl_absolute_tolerance,
                funding_pnl_relative: args.funding_pnl_relative_tolerance,
                funding_mark_abs: args.funding_mark_absolute_tolerance,
                funding_mark_relative: args.funding_mark_relative_tolerance,
            },
        },
    )?;
    let json = if args.pretty {
        serde_json::to_string_pretty(&report)?
    } else {
        serde_json::to_string(&report)?
    };
    persist_reserved_output(
        &mut output,
        &args.output,
        &json,
        "economic reconciliation output",
    )?;
    println!("{json}");
    if args.require_pass && !report.passed {
        anyhow::bail!("trade/funding economic reconciliation did not pass");
    }
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

    let mut output_file = reserve_private_output(&args.output, "fill reconciliation output")?;

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
    persist_reserved_output(
        &mut output_file,
        &args.output,
        &json,
        "fill reconciliation output",
    )?;
    println!("{json}");

    if args.require_pass && !report.passed {
        anyhow::bail!("fill/fee statement reconciliation did not pass");
    }
    Ok(())
}
