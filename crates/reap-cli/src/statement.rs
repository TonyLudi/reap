use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use reap_live::{
    FillStatementReconciliationOptions, FillStatementTolerances, reconcile_okx_fill_statement_paths,
};

#[derive(Debug, Args)]
pub(crate) struct ReconcileFillsArgs {
    #[arg(long, help = "Stopped runtime's canonical JSONL journal")]
    journal: PathBuf,
    #[arg(
        long = "statement",
        required = true,
        help = "Unmodified OKX fills/fills-history JSON response; repeat for every page"
    )]
    statements: Vec<PathBuf>,
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

    let report = reconcile_okx_fill_statement_paths(
        &args.journal,
        &args.statements,
        FillStatementReconciliationOptions {
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
        },
    )?;
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
