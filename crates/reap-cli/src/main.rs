use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{fs::File, fs::OpenOptions, io::Write};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use reap_backtest::{
    BacktestConfig, BacktestRunner, run_research_manifest_path, verify_research_paths,
};
use reap_capture::{
    CaptureConfig, CaptureRunOptions, analyze_capture_path, run_capture, verify_capture_paths,
};
use reap_fault::{
    FaultProxyCommand, FaultProxyConfig, FaultProxyRunOptions, run_fault_proxy,
    send_fault_proxy_command, verify_fault_proxy_run_paths,
};
use reap_live::{
    DeadmanExpiryCertificationOptions, EmergencyCancelOptions, EmergencyCancelVerificationOptions,
    LiveConfig, LiveMode, LiveRunOptions, LiveRuntimeError, OperatorCommand,
    collect_account_certification_path, collect_deadman_expiry_certification_path,
    run_emergency_cancel_path, send_operator_command, verify_account_certification_path,
    verify_deadman_expiry_certification_path, verify_emergency_cancel_paths,
    verify_production_transition_paths,
};
use reap_strategy::ChaosConfig;

mod deployment;
mod latency;
mod production_approval;
mod production_evidence;
mod statement;

use deployment::verify_research_deployment_paths;

use latency::{
    LatencyCalibrationOptions, build_latency_calibration, profile_toml, verify_latency_calibration,
};
use production_approval::{
    generate_production_approval_key_pair, prepare_production_approval_request,
    sign_production_approval_request, verify_production_approval_paths,
    verify_production_approval_policy_path,
};
use production_evidence::verify_production_evidence_manifest_path;

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
    #[command(
        about = "Re-run and independently verify an archived research report",
        long_about = "Strictly bind an archived research report to the exact manifest and this executable, re-run all candidate/dataset/scenario/fold work, and compare the complete path-normalized result. This can be expensive and does not authorize production trading."
    )]
    VerifyResearch {
        #[arg(short, long, help = "Exact research TOML used to produce the report")]
        manifest: PathBuf,
        #[arg(short, long, help = "Archived schema-8 research JSON report")]
        report: PathBuf,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable verification artifact"
        )]
        output: Option<PathBuf>,
        #[arg(long, help = "Exit non-zero unless exact reconstruction passes")]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Bind reconstructed production research to the proposed live strategy",
        long_about = "Re-run independent research reconstruction, require its schema-8 deployment candidate to have won every training fold, load an exact production live config, and compare the two effective strategy hashes. This does not authorize production trading."
    )]
    VerifyResearchDeployment {
        #[arg(short, long, help = "Exact proposed production live TOML")]
        config: PathBuf,
        #[arg(long, help = "Exact schema-8 production research TOML")]
        manifest: PathBuf,
        #[arg(long, help = "Archived schema-8 production research JSON report")]
        report: PathBuf,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable binding artifact"
        )]
        output: Option<PathBuf>,
        #[arg(long, help = "Exit non-zero unless reconstruction and binding pass")]
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
    #[command(about = "Verify a capture report against config and replayed artifact bytes")]
    VerifyCapture {
        #[arg(short, long, help = "Original capture TOML configuration")]
        config: PathBuf,
        #[arg(short, long, help = "Durable JSON report emitted by reap capture")]
        report: PathBuf,
        #[arg(short = 'e', long, help = "Raw capture JSONL; it may have been moved")]
        events: PathBuf,
        #[arg(
            long,
            help = "Normalized JSONL required when the run report declares normalized output"
        )]
        normalized_events: Option<PathBuf>,
        #[arg(
            long,
            help = "Exit non-zero unless all evidence and integrity gates pass"
        )]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    ConfigCheck {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Verify that a production config changes only deployment bindings",
        long_about = "Hash and structurally compare exact validated demo and production live configs. Strategy, risk, runtime, account policy, execution behavior, and safety controls must remain identical; only documented endpoints and deployment/credential bindings may change. This check does not authorize production order entry."
    )]
    VerifyProductionTransition {
        #[arg(long, help = "Exact validated demo live TOML")]
        demo_config: PathBuf,
        #[arg(long, help = "Exact validated production-candidate live TOML")]
        production_config: PathBuf,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable transition artifact"
        )]
        output: Option<PathBuf>,
        #[arg(long, help = "Exit non-zero unless the transition policy passes")]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Reconstruct and cross-bind the complete production evidence bundle",
        long_about = "Read a strict production-evidence manifest, rerun every underlying source verifier, enforce bounded operational-evidence freshness, and bind the exact demo/production configs, deployment candidate, release binary, target host, demo/production account identities, and predeclared approval-policy SHA-256. This gate does not authorize or enable production order entry."
    )]
    VerifyProductionEvidence {
        #[arg(
            short,
            long,
            help = "Strict schema-8 production evidence TOML manifest"
        )]
        manifest: PathBuf,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable bundle verification artifact"
        )]
        output: Option<PathBuf>,
        #[arg(
            long,
            help = "Exit non-zero unless every source, freshness, and identity gate passes"
        )]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Generate one offline Ed25519 production-approval key",
        long_about = "Generate an Ed25519 key on an independent approval workstation. The private PKCS#8 artifact and public-key artifact are created owner-only and never overwrite existing files. The private key is never printed."
    )]
    GenerateProductionApprovalKey {
        #[arg(long, help = "Create this owner-only private PKCS#8 JSON artifact")]
        private_key: PathBuf,
        #[arg(long, help = "Create this public-key JSON artifact")]
        public_key: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Validate and fingerprint a production approval policy",
        long_about = "Strictly parse the two-role Ed25519 approval policy, reject unknown fields, duplicate identities or keys, missing role coverage, and unsafe lifetimes, then report the exact file SHA-256 to predeclare in the schema-8 production evidence manifest."
    )]
    VerifyProductionApprovalPolicy {
        #[arg(short, long, help = "Strict production approval policy TOML")]
        policy: PathBuf,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable policy verification"
        )]
        output: Option<PathBuf>,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Prepare a short-lived production approval request",
        long_about = "Rerun the complete schema-8 production evidence bundle, require it to pass, bind its stable review subject to the predeclared exact two-role approval policy, and create a short-lived request. This does not authorize or enable production order entry."
    )]
    PrepareProductionApproval {
        #[arg(short, long, help = "Strict schema-8 production evidence manifest")]
        manifest: PathBuf,
        #[arg(long, help = "Strict production approval policy TOML")]
        policy: PathBuf,
        #[arg(long, help = "Unique reviewed change or release identifier")]
        request_id: String,
        #[arg(
            long,
            default_value_t = 600,
            help = "Request lifetime in seconds; policy and code cap it at 15 minutes"
        )]
        ttl_secs: u64,
        #[arg(short, long, help = "Create this owner-readable approval request")]
        output: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Sign a reviewed production approval request offline",
        long_about = "Validate the strict request and exact policy, require the private Ed25519 key to match the named policy approver, and create one role-bound signature. Run this on the approver's independent workstation."
    )]
    SignProductionApproval {
        #[arg(
            short,
            long,
            help = "Strict short-lived production approval request JSON"
        )]
        request: PathBuf,
        #[arg(long, help = "Exact production approval policy TOML")]
        policy: PathBuf,
        #[arg(long, help = "Owner-only Ed25519 private-key JSON artifact")]
        private_key: PathBuf,
        #[arg(long, help = "Policy approver identifier represented by this key")]
        approver: String,
        #[arg(short, long, help = "Create this owner-readable signature artifact")]
        output: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Verify fresh production evidence and independent approvals",
        long_about = "Rerun every schema-8 production evidence source, require the stable subject to equal the short-lived request, verify each Ed25519 signature against the predeclared exact policy, and require coverage of every distinct policy role. A pass remains evidence only and never authorizes order entry."
    )]
    VerifyProductionApproval {
        #[arg(short, long, help = "Strict schema-8 production evidence manifest")]
        manifest: PathBuf,
        #[arg(long, help = "Exact production approval policy TOML")]
        policy: PathBuf,
        #[arg(long, help = "Strict short-lived production approval request JSON")]
        request: PathBuf,
        #[arg(
            long = "approval",
            required = true,
            num_args = 1..,
            help = "Role-bound Ed25519 signature artifact; repeat for each required role"
        )]
        approvals: Vec<PathBuf>,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable approval verification"
        )]
        output: Option<PathBuf>,
        #[arg(long, help = "Exit non-zero unless evidence and role quorum pass")]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    Capture {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(
            short,
            long,
            help = "Create this owner-readable JSON run report; existing files are refused"
        )]
        output: Option<PathBuf>,
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
    #[command(
        about = "Run the loopback-only OKX demo REST/WebSocket fault proxy",
        long_about = "Forward loopback HTTP and public/private/order WebSockets only to validated official OKX demo endpoints. Faults are armed over an owner-only Unix socket and produce distinct create-new evidence artifacts. This command is test tooling and cannot target production endpoints."
    )]
    FaultProxy {
        #[arg(short, long, help = "Strict schema-1 fault-proxy TOML")]
        config: PathBuf,
        #[arg(
            short,
            long,
            help = "Create this owner-readable run report; existing files are refused"
        )]
        output: Option<PathBuf>,
        #[arg(long, help = "Stop the proxy after this many seconds")]
        duration_secs: Option<u64>,
        #[arg(
            long,
            help = "Exit non-zero if proxy errors or unconsumed armed faults remain"
        )]
        require_clean_shutdown: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Independently verify one fault-proxy process run report",
        long_about = "Reopen a strict schema-2 fault-proxy run report and its exact proxy config, rederive clean-shutdown invariants, and expose build, host, session, and timing provenance. This does not bind the proxy process to a live fault scenario."
    )]
    VerifyFaultProxyRun {
        #[arg(short, long, help = "Exact fault-proxy TOML used by the run")]
        config: PathBuf,
        #[arg(short, long, help = "Strict schema-2 fault-proxy run report JSON")]
        report: PathBuf,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable verification artifact"
        )]
        output: Option<PathBuf>,
        #[arg(long, help = "Exit non-zero unless independent verification passes")]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Create an exact demo live config routed through a fault proxy",
        long_about = "Require an official OKX demo live config whose endpoint tuple exactly matches the fault proxy upstream, then create a validated loopback config with separate public, private, and order-command WebSocket routes. No credentials are read."
    )]
    RenderFaultLiveConfig {
        #[arg(long, help = "Official-endpoint demo live TOML")]
        live_config: PathBuf,
        #[arg(long, help = "Strict schema-1 fault-proxy TOML")]
        proxy_config: PathBuf,
        #[arg(short, long, help = "Create this owner-readable routed live TOML")]
        output: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    #[command(about = "Submit one strict JSON command to a running fault proxy")]
    FaultProxyControl {
        #[arg(short, long, help = "Fault-proxy Unix control socket")]
        socket: PathBuf,
        #[arg(short, long, help = "Strict schema-1 fault command JSON")]
        command: PathBuf,
        #[arg(long, help = "Exit non-zero unless the proxy accepts the command")]
        require_accepted: bool,
        #[arg(long)]
        pretty: bool,
    },
    Live {
        #[arg(short, long)]
        config: PathBuf,
        #[arg(
            short,
            long,
            help = "Create a JSON evidence artifact; an existing path is refused"
        )]
        output: Option<PathBuf>,
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
    #[command(about = "Verify a live report against exact config bytes and derived invariants")]
    VerifyLiveRun {
        #[arg(short, long, help = "Original live TOML configuration")]
        config: PathBuf,
        #[arg(short, long, help = "Schema-8 live JSON report")]
        report: PathBuf,
        #[arg(long, value_enum, help = "Require this recorded live mode")]
        expected_mode: Option<LiveCliMode>,
        #[arg(long, help = "Exit non-zero unless report evidence is valid")]
        require_valid: bool,
        #[arg(
            long,
            help = "Exit non-zero unless evidence is valid and the clean-soak flag re-derives"
        )]
        require_clean_soak: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Verify the complete schema-8 observe/demo live fault matrix",
        long_about = "Independently verify one exact-config run per required live fault role, bind every run to one build/host/account identity, and hash each external injector record. Process-death, statement, and deployment certification remain separate gates."
    )]
    VerifyLiveFaultMatrix {
        #[arg(short, long, help = "Exact live TOML used by every campaign run")]
        config: PathBuf,
        #[arg(short, long, help = "Schema-1 live fault matrix TOML manifest")]
        manifest: PathBuf,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable verification artifact"
        )]
        output: Option<PathBuf>,
        #[arg(long, help = "Exit non-zero unless the complete live matrix passes")]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(about = "Build a Java-mapped backtest latency profile from bounded live reports")]
    CalibrateLatency {
        #[arg(short, long, help = "Live configuration used by every source report")]
        config: PathBuf,
        #[arg(
            long = "report",
            required = true,
            help = "Create-new live report; repeat for multiple bounded runs"
        )]
        reports: Vec<PathBuf>,
        #[arg(
            short,
            long,
            help = "Create the JSON calibration artifact; an existing path is refused"
        )]
        output: PathBuf,
        #[arg(
            long,
            help = "Optionally create a mergeable [backtest.latency_profile] TOML fragment"
        )]
        profile_output: Option<PathBuf>,
        #[arg(long, default_value_t = 20260713)]
        seed: u64,
        #[arg(long, default_value_t = 1000)]
        minimum_samples_per_series: u64,
        #[arg(
            long,
            help = "Accept dispatch-to-REST-ack samples as conservative matching-delay upper bounds"
        )]
        accept_matching_upper_bounds: bool,
        #[arg(long, help = "Exit non-zero unless every required series passes")]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Rebuild and verify a latency calibration from exact source reports",
        long_about = "Strictly parse a calibration artifact, bind it to the exact live config and supplied source-report hashes, independently verify every live report, rebuild every Java-mapped latency series with the recorded options, and compare path-normalized results. This does not authorize production trading."
    )]
    VerifyLatencyCalibration {
        #[arg(short, long, help = "Exact live TOML used by every source report")]
        config: PathBuf,
        #[arg(short, long, help = "Schema-4 latency calibration JSON artifact")]
        artifact: PathBuf,
        #[arg(
            long = "report",
            required = true,
            help = "Exact source live report; repeat for every artifact source"
        )]
        reports: Vec<PathBuf>,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable verification artifact"
        )]
        output: Option<PathBuf>,
        #[arg(long, help = "Exit non-zero unless independent reconstruction passes")]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(about = "Collect authenticated, read-only recent OKX fill evidence")]
    CollectFills(statement::CollectFillsArgs),
    #[command(about = "Collect authenticated, read-only account-wide OKX bill evidence")]
    CollectBills(statement::CollectBillsArgs),
    #[command(about = "Independently reconstruct and verify an exact OKX bill collection offline")]
    VerifyBillCollection(statement::VerifyBillCollectionArgs),
    #[command(about = "Reconcile normal trade and funding economics from verified OKX collections")]
    ReconcileEconomics(statement::ReconcileEconomicsArgs),
    #[command(about = "Reconcile canonical journal fills and exact fees against raw OKX responses")]
    ReconcileFills(statement::ReconcileFillsArgs),
    #[command(about = "Certify current OKX cash-only and zero-liability account state")]
    CertifyAccount {
        #[arg(short, long, help = "Validated live TOML configuration")]
        config: PathBuf,
        #[arg(long, help = "Configured account id")]
        account: String,
        #[arg(
            short,
            long,
            help = "Create this owner-readable evidence artifact; existing files are refused"
        )]
        output: PathBuf,
        #[arg(long)]
        pretty: bool,
    },
    #[command(about = "Re-derive an account certification from its embedded raw OKX evidence")]
    VerifyAccountCertification {
        #[arg(short, long, help = "Account-certification evidence artifact")]
        artifact: PathBuf,
        #[arg(long, help = "Exit non-zero unless the account policy passed")]
        require_pass: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        about = "Certify that OKX Cancel All After expired after a stopped Reap runtime",
        long_about = "Lease the stopped runtime's journal, recover its live regular orders, and use authenticated read-only OKX requests to require cancellation source 20 plus account-wide zero pending regular orders. No cancel request is sent."
    )]
    CertifyDeadmanExpiry {
        #[arg(short, long, help = "Validated live TOML configuration")]
        config: PathBuf,
        #[arg(long, help = "Configured account id")]
        account: String,
        #[arg(
            short,
            long,
            help = "Create this owner-readable evidence artifact; existing files are refused"
        )]
        output: PathBuf,
        #[arg(
            long,
            help = "Attest that every order producer for this exchange account is stopped"
        )]
        confirm_order_producers_stopped: bool,
        #[arg(long)]
        pretty: bool,
    },
    #[command(
        name = "verify-deadman-certification",
        about = "Re-derive deadman-expiry proof from raw OKX evidence and the exact journal"
    )]
    VerifyDeadmanExpiryCertification {
        #[arg(short, long, help = "Deadman-expiry evidence artifact")]
        artifact: PathBuf,
        #[arg(
            long,
            help = "Exact stopped-runtime journal fingerprinted by the collector"
        )]
        journal: PathBuf,
        #[arg(
            long,
            help = "Exit non-zero unless deadman-expiry certification passed"
        )]
        require_pass: bool,
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
    },
    #[command(
        about = "Independently verify an emergency regular-order cancellation report",
        long_about = "Re-hash the exact emergency config and report, re-derive account coverage, deadman-horizon, provenance, and regular-order zero invariants, and optionally require every configured account. Algo and spread orders remain excluded."
    )]
    VerifyEmergencyCancel {
        #[arg(short, long, help = "Exact live TOML used by emergency-cancel")]
        config: PathBuf,
        #[arg(short, long, help = "Schema-1 emergency-cancel JSON report")]
        report: PathBuf,
        #[arg(
            short,
            long,
            help = "Optionally create this owner-readable verification artifact"
        )]
        output: Option<PathBuf>,
        #[arg(
            long,
            help = "Require the report to cover every account in the supplied config"
        )]
        require_all_configured_accounts: bool,
        #[arg(long, help = "Exit non-zero unless every verification gate passes")]
        require_pass: bool,
        #[arg(long)]
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
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "research output"))
                .transpose()?;
            let report = run_research_manifest_path(&manifest).with_context(|| {
                format!("failed to run research manifest {}", manifest.display())
            })?;
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "research output")?;
            }
            println!("{json}");
            if require_pass && !report.passed {
                anyhow::bail!("research report did not satisfy configured gates");
            }
        }
        Command::VerifyResearch {
            manifest,
            report,
            output,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "research verification"))
                .transpose()?;
            let verification = verify_research_paths(&manifest, &report).with_context(|| {
                format!(
                    "failed to verify research report {} from manifest {}",
                    report.display(),
                    manifest.display()
                )
            })?;
            let json = if pretty {
                serde_json::to_string_pretty(&verification)?
            } else {
                serde_json::to_string(&verification)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "research verification")?;
            }
            println!("{json}");
            if require_pass && !verification.acceptance_passed {
                anyhow::bail!("research reconstruction did not pass");
            }
        }
        Command::VerifyResearchDeployment {
            config,
            manifest,
            report,
            output,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "research deployment verification"))
                .transpose()?;
            let verification = verify_research_deployment_paths(&config, &manifest, &report)?;
            let json = if pretty {
                serde_json::to_string_pretty(&verification)?
            } else {
                serde_json::to_string(&verification)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "research deployment verification")?;
            }
            println!("{json}");
            if require_pass && !verification.acceptance_passed {
                anyhow::bail!("production research does not bind to the proposed live strategy");
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
        Command::VerifyCapture {
            config,
            report,
            events,
            normalized_events,
            require_pass,
            pretty,
        } => {
            let verification =
                verify_capture_paths(&config, &report, &events, normalized_events.as_deref())
                    .with_context(|| {
                        format!(
                            "failed to verify capture report {} against {}",
                            report.display(),
                            events.display()
                        )
                    })?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&verification)?);
            } else {
                println!("{}", serde_json::to_string(&verification)?);
            }
            if require_pass && !verification.passed {
                anyhow::bail!("capture evidence did not satisfy verification gates");
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
        Command::VerifyProductionTransition {
            demo_config,
            production_config,
            output,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "production-transition report"))
                .transpose()?;
            let report = verify_production_transition_paths(&demo_config, &production_config)
                .with_context(|| {
                    format!(
                        "failed to verify production transition from {} to {}",
                        demo_config.display(),
                        production_config.display()
                    )
                })?;
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "production-transition report")?;
            }
            println!("{json}");
            if require_pass && !report.acceptance_passed {
                anyhow::bail!("production configuration transition did not pass");
            }
        }
        Command::VerifyProductionEvidence {
            manifest,
            output,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "production evidence verification"))
                .transpose()?;
            let report =
                verify_production_evidence_manifest_path(&manifest).with_context(|| {
                    format!(
                        "failed to verify production evidence manifest {}",
                        manifest.display()
                    )
                })?;
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "production evidence verification")?;
            }
            println!("{json}");
            if require_pass && !report.evidence_bundle_passed {
                anyhow::bail!(
                    "production evidence bundle did not pass every source, freshness, and binding gate"
                );
            }
        }
        Command::GenerateProductionApprovalKey {
            private_key,
            public_key,
            pretty,
        } => {
            if private_key == public_key {
                anyhow::bail!("approval private and public key paths must differ");
            }
            let mut private_file =
                reserve_private_output(&private_key, "production approval private key")?;
            let mut public_file =
                reserve_private_output(&public_key, "production approval public key")?;
            let (private, public) = generate_production_approval_key_pair()?;
            let private_json = zeroize::Zeroizing::new(if pretty {
                serde_json::to_string_pretty(&private)?
            } else {
                serde_json::to_string(&private)?
            });
            let public_json = if pretty {
                serde_json::to_string_pretty(&public)?
            } else {
                serde_json::to_string(&public)?
            };
            persist_reserved_output(
                &mut private_file,
                &private_key,
                &private_json,
                "production approval private key",
            )?;
            persist_reserved_output(
                &mut public_file,
                &public_key,
                &public_json,
                "production approval public key",
            )?;
            println!("{public_json}");
        }
        Command::VerifyProductionApprovalPolicy {
            policy,
            output,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "production approval policy verification"))
                .transpose()?;
            let verification =
                verify_production_approval_policy_path(&policy).with_context(|| {
                    format!(
                        "failed to verify production approval policy {}",
                        policy.display()
                    )
                })?;
            let json = if pretty {
                serde_json::to_string_pretty(&verification)?
            } else {
                serde_json::to_string(&verification)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(
                    file,
                    path,
                    &json,
                    "production approval policy verification",
                )?;
            }
            println!("{json}");
        }
        Command::PrepareProductionApproval {
            manifest,
            policy,
            request_id,
            ttl_secs,
            output,
            pretty,
        } => {
            let ttl_ms = ttl_secs
                .checked_mul(1_000)
                .context("production approval TTL overflowed milliseconds")?;
            let mut output_file = reserve_private_output(&output, "production approval request")?;
            let request =
                prepare_production_approval_request(&manifest, &policy, &request_id, ttl_ms)
                    .with_context(|| {
                        format!(
                            "failed to prepare production approval request from {}",
                            manifest.display()
                        )
                    })?;
            let json = if pretty {
                serde_json::to_string_pretty(&request)?
            } else {
                serde_json::to_string(&request)?
            };
            persist_reserved_output(
                &mut output_file,
                &output,
                &json,
                "production approval request",
            )?;
            println!("{json}");
        }
        Command::SignProductionApproval {
            request,
            policy,
            private_key,
            approver,
            output,
            pretty,
        } => {
            let mut output_file = reserve_private_output(&output, "production approval signature")?;
            let approval =
                sign_production_approval_request(&request, &policy, &private_key, &approver)
                    .with_context(|| {
                        format!(
                            "failed to sign production approval request {} as {}",
                            request.display(),
                            approver
                        )
                    })?;
            let json = if pretty {
                serde_json::to_string_pretty(&approval)?
            } else {
                serde_json::to_string(&approval)?
            };
            persist_reserved_output(
                &mut output_file,
                &output,
                &json,
                "production approval signature",
            )?;
            println!("{json}");
        }
        Command::VerifyProductionApproval {
            manifest,
            policy,
            request,
            approvals,
            output,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "production approval verification"))
                .transpose()?;
            let report = verify_production_approval_paths(&manifest, &policy, &request, &approvals)
                .with_context(|| {
                    format!(
                        "failed to verify production approval request {}",
                        request.display()
                    )
                })?;
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "production approval verification")?;
            }
            println!("{json}");
            if require_pass && !report.approval_gate_passed {
                anyhow::bail!("production approval did not pass evidence and role-quorum gates");
            }
        }
        Command::Capture {
            config,
            output,
            raw_path,
            normalized_path,
            duration_secs,
            require_clean_capture,
            pretty,
        } => {
            let mut report_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "capture report"))
                .transpose()?;
            reap_telemetry::init_json_tracing("info")
                .map_err(anyhow::Error::msg)
                .context("failed to initialize capture tracing")?;
            let (mut capture_config, config_source) = CaptureConfig::load_with_evidence(&config)
                .with_context(|| format!("failed to load capture config {}", config.display()))?;
            if let Some(path) = raw_path {
                capture_config.output.raw_path = path;
            }
            if let Some(path) = normalized_path {
                capture_config.output.normalized_path = Some(path);
            }
            if output.as_ref().is_some_and(|output| {
                output == &capture_config.output.raw_path
                    || capture_config.output.normalized_path.as_ref() == Some(output)
            }) {
                anyhow::bail!("capture report path must differ from raw and normalized outputs");
            }
            let report = run_capture(
                capture_config,
                CaptureRunOptions {
                    run_duration: duration_secs.map(Duration::from_secs),
                    config_source: Some(config_source),
                },
            )
            .await?;
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let (Some(file), Some(path)) = (&mut report_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "capture report")?;
            }
            println!("{json}");
            if require_clean_capture && !report.clean_capture {
                anyhow::bail!("bounded capture did not satisfy clean integrity invariants");
            }
        }
        Command::FaultProxy {
            config,
            output,
            duration_secs,
            require_clean_shutdown,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "fault-proxy run report"))
                .transpose()?;
            reap_telemetry::init_json_tracing("info")
                .map_err(anyhow::Error::msg)
                .context("failed to initialize fault-proxy tracing")?;
            let (config, config_evidence) = FaultProxyConfig::load(&config).with_context(|| {
                format!("failed to load fault-proxy config {}", config.display())
            })?;
            let report = run_fault_proxy(
                config,
                config_evidence,
                FaultProxyRunOptions {
                    duration: duration_secs.map(Duration::from_secs),
                },
            )
            .await?;
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "fault-proxy run report")?;
            }
            println!("{json}");
            if require_clean_shutdown && !report.clean_shutdown {
                anyhow::bail!("fault proxy did not satisfy clean-shutdown invariants");
            }
        }
        Command::VerifyFaultProxyRun {
            config,
            report,
            output,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "fault-proxy run verification"))
                .transpose()?;
            let verification =
                verify_fault_proxy_run_paths(&config, &report).with_context(|| {
                    format!(
                        "failed to verify fault-proxy run report {} against {}",
                        report.display(),
                        config.display()
                    )
                })?;
            let json = if pretty {
                serde_json::to_string_pretty(&verification)?
            } else {
                serde_json::to_string(&verification)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "fault-proxy run verification")?;
            }
            println!("{json}");
            if require_pass && !verification.acceptance_passed {
                anyhow::bail!("fault-proxy run report did not pass independent verification");
            }
        }
        Command::FaultProxyControl {
            socket,
            command,
            require_accepted,
            pretty,
        } => {
            let metadata = std::fs::metadata(&command).with_context(|| {
                format!("failed to inspect fault command {}", command.display())
            })?;
            if metadata.len() > 1024 * 1024 {
                anyhow::bail!("fault command exceeds the 1 MiB limit");
            }
            let command_bytes = std::fs::read(&command)
                .with_context(|| format!("failed to read fault command {}", command.display()))?;
            if command_bytes.len() > 1024 * 1024 {
                anyhow::bail!("fault command exceeds the 1 MiB limit");
            }
            let command: FaultProxyCommand = serde_json::from_slice(&command_bytes)
                .with_context(|| "failed to parse strict fault command JSON")?;
            let response = send_fault_proxy_command(&socket, &command).await?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!("{}", serde_json::to_string(&response)?);
            }
            if require_accepted && !response.accepted {
                anyhow::bail!("fault proxy rejected the command: {}", response.message);
            }
        }
        Command::RenderFaultLiveConfig {
            live_config,
            proxy_config,
            output,
            pretty,
        } => {
            let live = LiveConfig::load(&live_config).with_context(|| {
                format!("failed to load demo live config {}", live_config.display())
            })?;
            let (proxy, proxy_evidence) =
                FaultProxyConfig::load(&proxy_config).with_context(|| {
                    format!(
                        "failed to load fault-proxy config {}",
                        proxy_config.display()
                    )
                })?;
            let routed = proxy.route_live_config(&live)?;
            let toml = toml::to_string_pretty(&routed)
                .context("failed to serialize routed demo live config")?;
            let mut file = reserve_private_output(&output, "routed demo live config")?;
            persist_reserved_output(&mut file, &output, &toml, "routed demo live config")?;
            let summary = serde_json::json!({
                "output": output,
                "proxy_config_fingerprint": proxy_evidence.effective_fingerprint,
                "live_config_fingerprint": routed.fingerprint()?,
                "rest_url": &routed.venue.rest_url,
                "public_ws_url": &routed.venue.public_ws_url,
                "private_ws_url": &routed.venue.private_ws_url,
                "order_ws_url": routed.venue.order_ws_url(),
            });
            if pretty {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!("{}", serde_json::to_string(&summary)?);
            }
        }
        Command::Live {
            config,
            output,
            mode,
            confirm_demo,
            duration_secs,
            require_clean_soak,
            pretty,
        } => {
            reap_telemetry::init_json_tracing("info")
                .map_err(anyhow::Error::msg)
                .context("failed to initialize live tracing")?;
            let mut output_file = output
                .as_ref()
                .map(|output| reserve_private_output(output, "live report"))
                .transpose()?;
            let run_result = reap_live::run_live_path(
                config,
                LiveRunOptions {
                    mode: mode.into(),
                    demo_confirmed: confirm_demo,
                    run_duration: duration_secs.map(Duration::from_secs),
                },
            )
            .await;
            let (report, runtime_failure) = match run_result {
                Ok(report) => (report, None),
                Err(LiveRuntimeError::ReportedFailure { source, report }) => {
                    (*report, Some(*source))
                }
                Err(error) => return Err(error.into()),
            };
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "live report")?;
            }
            println!("{json}");
            if let Some(error) = runtime_failure {
                return Err(error.into());
            }
            if require_clean_soak && !report.clean_soak {
                anyhow::bail!("bounded live soak did not satisfy clean acceptance invariants");
            }
        }
        Command::VerifyLiveRun {
            config,
            report,
            expected_mode,
            require_valid,
            require_clean_soak,
            pretty,
        } => {
            let verification =
                reap_live::verify_live_run_paths(&config, &report, expected_mode.map(Into::into))?;
            let json = if pretty {
                serde_json::to_string_pretty(&verification)?
            } else {
                serde_json::to_string(&verification)?
            };
            println!("{json}");
            if require_clean_soak && !verification.acceptance_passed {
                anyhow::bail!(
                    "live report evidence is invalid or does not satisfy clean-soak invariants"
                );
            }
            if require_valid && !verification.evidence_valid {
                anyhow::bail!("live report evidence is invalid");
            }
        }
        Command::VerifyLiveFaultMatrix {
            config,
            manifest,
            output,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "live fault matrix report"))
                .transpose()?;
            let verification = reap_live::verify_live_fault_matrix_paths(config, manifest)?;
            let json = if pretty {
                serde_json::to_string_pretty(&verification)?
            } else {
                serde_json::to_string(&verification)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "live fault matrix report")?;
            }
            println!("{json}");
            if require_pass && !verification.live_fault_matrix_passed {
                anyhow::bail!("live fault matrix did not satisfy every evidence gate");
            }
        }
        Command::CalibrateLatency {
            config,
            reports,
            output,
            profile_output,
            seed,
            minimum_samples_per_series,
            accept_matching_upper_bounds,
            require_pass,
            pretty,
        } => {
            if profile_output.as_ref() == Some(&output) {
                anyhow::bail!("--output and --profile-output must be different paths");
            }
            let artifact = build_latency_calibration(
                &config,
                &reports,
                LatencyCalibrationOptions {
                    seed,
                    minimum_samples_per_series,
                    accept_matching_upper_bounds,
                },
            )?;
            let json = if pretty {
                serde_json::to_string_pretty(&artifact)?
            } else {
                serde_json::to_string(&artifact)?
            };
            let mut artifact_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)
                .with_context(|| {
                    format!("failed to create latency artifact {}", output.display())
                })?;
            artifact_file.write_all(json.as_bytes())?;
            artifact_file.write_all(b"\n")?;
            artifact_file.sync_all()?;
            if let Some(profile_output) = profile_output {
                if !artifact.passed {
                    anyhow::bail!(
                        "refusing to emit a latency profile from a failed calibration; inspect {}",
                        output.display()
                    );
                }
                let toml = profile_toml(&artifact.profile)?;
                let mut profile_file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&profile_output)
                    .with_context(|| {
                        format!(
                            "failed to create latency profile {}",
                            profile_output.display()
                        )
                    })?;
                profile_file.write_all(toml.as_bytes())?;
                profile_file.sync_all()?;
            }
            println!("{json}");
            if require_pass && !artifact.passed {
                anyhow::bail!("latency calibration did not satisfy evidence gates");
            }
        }
        Command::VerifyLatencyCalibration {
            config,
            artifact,
            reports,
            output,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "latency calibration verification"))
                .transpose()?;
            let verification = verify_latency_calibration(&config, &artifact, &reports)?;
            let json = if pretty {
                serde_json::to_string_pretty(&verification)?
            } else {
                serde_json::to_string(&verification)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "latency calibration verification")?;
            }
            println!("{json}");
            if require_pass && !verification.acceptance_passed {
                anyhow::bail!("latency calibration reconstruction did not pass");
            }
        }
        Command::CollectFills(args) => statement::collect(args).await?,
        Command::CollectBills(args) => statement::collect_bills(args).await?,
        Command::VerifyBillCollection(args) => statement::verify_bills(args)?,
        Command::ReconcileEconomics(args) => statement::reconcile_economics(args)?,
        Command::ReconcileFills(args) => statement::run(args)?,
        Command::CertifyAccount {
            config,
            account,
            output,
            pretty,
        } => {
            let summary = collect_account_certification_path(&config, &output, &account)
                .await
                .with_context(|| {
                    format!(
                        "failed to collect account certification for {} into {}",
                        account,
                        output.display()
                    )
                })?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!("{}", serde_json::to_string(&summary)?);
            }
            if !summary.passed {
                anyhow::bail!(
                    "account certification evidence was collected but cash/zero-liability policy did not pass"
                );
            }
        }
        Command::VerifyAccountCertification {
            artifact,
            require_pass,
            pretty,
        } => {
            let summary = verify_account_certification_path(&artifact).with_context(|| {
                format!(
                    "failed to verify account certification {}",
                    artifact.display()
                )
            })?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!("{}", serde_json::to_string(&summary)?);
            }
            if require_pass && !summary.passed {
                anyhow::bail!("account certification policy did not pass");
            }
        }
        Command::CertifyDeadmanExpiry {
            config,
            account,
            output,
            confirm_order_producers_stopped,
            pretty,
        } => {
            let summary = collect_deadman_expiry_certification_path(
                &config,
                &output,
                DeadmanExpiryCertificationOptions {
                    account_id: account.clone(),
                    order_producers_stopped_attested: confirm_order_producers_stopped,
                },
            )
            .await
            .with_context(|| {
                format!(
                    "failed to collect deadman-expiry certification for {} into {}",
                    account,
                    output.display()
                )
            })?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!("{}", serde_json::to_string(&summary)?);
            }
            if !summary.passed {
                anyhow::bail!(
                    "deadman-expiry evidence was collected but did not prove exchange cancellation source 20 and regular-order zero"
                );
            }
        }
        Command::VerifyDeadmanExpiryCertification {
            artifact,
            journal,
            require_pass,
            pretty,
        } => {
            let summary = verify_deadman_expiry_certification_path(&artifact, &journal)
                .with_context(|| {
                    format!(
                        "failed to verify deadman-expiry certification {} against {}",
                        artifact.display(),
                        journal.display()
                    )
                })?;
            if pretty {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!("{}", serde_json::to_string(&summary)?);
            }
            if require_pass && !summary.passed {
                anyhow::bail!("deadman-expiry certification did not pass");
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
            output,
            pretty,
        } => {
            reap_telemetry::init_json_tracing("info")
                .map_err(anyhow::Error::msg)
                .context("failed to initialize emergency-cancel tracing")?;
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "emergency-cancel report"))
                .transpose()?;
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
            let json = if pretty {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "emergency-cancel report")?;
            }
            println!("{json}");
            if !report.regular_orders_all_clear {
                anyhow::bail!(
                    "emergency cancel did not verify every selected account's regular order book at zero"
                );
            }
            if !report.evidence_complete {
                anyhow::bail!(
                    "emergency cancel reached regular-order zero but its provenance evidence is incomplete"
                );
            }
            if !report.all_clear {
                anyhow::bail!("emergency cancel report violated its all-clear invariant");
            }
        }
        Command::VerifyEmergencyCancel {
            config,
            report,
            output,
            require_all_configured_accounts,
            require_pass,
            pretty,
        } => {
            let mut output_file = output
                .as_ref()
                .map(|path| reserve_private_output(path, "emergency-cancel verification"))
                .transpose()?;
            let verification = verify_emergency_cancel_paths(
                &config,
                &report,
                EmergencyCancelVerificationOptions {
                    require_all_configured_accounts,
                },
            )
            .with_context(|| {
                format!(
                    "failed to verify emergency-cancel report {} against {}",
                    report.display(),
                    config.display()
                )
            })?;
            let json = if pretty {
                serde_json::to_string_pretty(&verification)?
            } else {
                serde_json::to_string(&verification)?
            };
            if let (Some(file), Some(path)) = (&mut output_file, output.as_deref()) {
                persist_reserved_output(file, path, &json, "emergency-cancel verification")?;
            }
            println!("{json}");
            if require_pass && !verification.acceptance_passed {
                anyhow::bail!("emergency-cancel verification did not pass");
            }
        }
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
    use super::*;

    #[test]
    fn private_output_is_create_new_and_durable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture-report.json");
        let mut file = reserve_private_output(&path, "test report").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        persist_reserved_output(&mut file, &path, "{\"passed\":true}", "test report").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"passed\":true}\n"
        );
        assert!(reserve_private_output(&path, "test report").is_err());
    }
}
