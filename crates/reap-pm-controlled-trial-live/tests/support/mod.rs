use std::{fs, os::unix::fs::PermissionsExt as _, path::Path};

use chrono::{DateTime, Utc};
use reap_pm_controlled_trial::{
    AuthorizationApproval, AuthorizationBuildBinding, AuthorizationHostBinding,
    AuthorizationRuntimeBinding, CanonicalAuthorization, CanonicalTrialConfig,
    CanonicalTrialPreflight, OfflineAuthorizationState, PreparedAuthorizationConsumption,
    TrialAccount, TrialAccountPreflight, TrialAuthorization, TrialBookPreflight,
    TrialClosedOnlyEvidence, TrialCompleteCutEvidence, TrialConfiguredPositionState,
    TrialCredentialSlot, TrialDataApiPositionPreflight, TrialDomain, TrialEnvironmentPreflight,
    TrialExactDetailCutEvidence, TrialFinalizedChainPreflight, TrialGeoblockEvidence,
    TrialJournalBinding, TrialJournalLeaseEvidence, TrialMarket, TrialMarketPreflight,
    TrialObservationStamp, TrialOrder, TrialOrderType, TrialPhase, TrialPhaseGateEvidence,
    TrialPreflightBinding, TrialPreflightEvidence, TrialPreflightWindow, TrialPrivateAccountCut,
    TrialReconciliationPreflight, TrialRiskPreflight, TrialServerTimeEvidence, TrialSide,
    TrialTimeLimits, TrialUserStreamPreflight, load_canonical_authorization,
    load_canonical_trial_config, prepare_authorization_consumption,
    validate_canonical_trial_preflight,
};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const CONDITION: &str = "0x3333333333333333333333333333333333333333333333333333333333333333";
const QUESTION: &str = "0x4444444444444444444444444444444444444444444444444444444444444444";

pub struct Fixture {
    pub directory: TempDir,
    pub config: CanonicalTrialConfig,
    pub authorization: CanonicalAuthorization,
}

impl Fixture {
    pub fn new() -> (Self, PreparedAuthorizationConsumption) {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("protect tempdir");
        let mut raw = trial_config();
        raw.journal.artifact_directory = directory.path().to_str().expect("utf8 path").into();
        let config_path = directory.path().join("config.json");
        write_0600(
            &config_path,
            &serde_json::to_vec(&raw).expect("config json"),
        );
        let config = load_canonical_trial_config(&config_path).expect("canonical config");
        let authorization_path = directory.path().join("authorization.json");
        write_0600(
            &authorization_path,
            &serde_json::to_vec(&trial_authorization(&config)).expect("authorization json"),
        );
        let authorization =
            load_canonical_authorization(&authorization_path).expect("canonical authorization");
        let prepared = prepare_authorization_consumption(
            &config,
            &authorization,
            &runtime(&authorization, "2026-08-09T12:05:00Z"),
        )
        .expect("Prepared authorization ledger");
        (
            Self {
                directory,
                config,
                authorization,
            },
            prepared,
        )
    }

    pub fn runtime(&self, observed_at_utc: &str) -> AuthorizationRuntimeBinding {
        runtime(&self.authorization, observed_at_utc)
    }

    pub fn canonical_preflight(
        &self,
        leases: TrialJournalLeaseEvidence,
    ) -> CanonicalTrialPreflight {
        let evidence = preflight_evidence(&self.config, &self.authorization, leases);
        let bytes = serde_json::to_vec(&evidence).expect("preflight json");
        validate_canonical_trial_preflight(
            &self.config,
            &self.authorization,
            &bytes,
            time("2026-08-09T12:05:04Z"),
            4_000_000_100,
        )
        .expect("canonical preflight")
    }

    pub fn path(&self, name: &str) -> std::path::PathBuf {
        self.directory.path().join(name)
    }
}

pub fn time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp")
        .with_timezone(&Utc)
}

pub fn l2_time(value: &str) -> u64 {
    u64::try_from(time(value).timestamp()).expect("positive timestamp")
}

pub fn hex(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

fn write_0600(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("write fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("protect fixture");
}

fn runtime(
    authorization: &CanonicalAuthorization,
    observed_at_utc: &str,
) -> AuthorizationRuntimeBinding {
    AuthorizationRuntimeBinding {
        release_binary_sha256: authorization.value().build.release_binary_sha256.clone(),
        release_binary_length: authorization.value().build.release_binary_length,
        host: authorization.value().host.clone(),
        observed_at_utc: observed_at_utc.into(),
    }
}

fn trial_config() -> reap_pm_controlled_trial::TrialConfig {
    reap_pm_controlled_trial::TrialConfig {
        schema_version: 1,
        profile: "pm_t2_type1_proxy_offline_a0".into(),
        phase: TrialPhase::APlaceCancel,
        source_pin_manifest_sha256: hex(0x21),
        runbook_revision: "pm-t2-runbook-v1".into(),
        runbook_sha256: hex(0x22),
        account: TrialAccount {
            chain_id: 137,
            signature_type: 1,
            wallet_profile: "poly_proxy".into(),
            signer: SIGNER.into(),
            funder: FUNDER.into(),
        },
        market: TrialMarket {
            condition_id: CONDITION.into(),
            question_id: QUESTION.into(),
            token_id: "123456789".into(),
            outcome_label: "YES".into(),
            domain: TrialDomain::Standard,
            exchange: "0xE111180000d2663C0091e4f400237545B87B996B".into(),
            pusd_contract: "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB".into(),
            conditional_tokens_contract: "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045".into(),
        },
        order: TrialOrder {
            salt: 1,
            timestamp_ms: 1_800_000_000_000,
            side: TrialSide::Buy,
            price: "0.5".into(),
            quantity: "5".into(),
            tick: "0.01".into(),
            minimum_order_size: "5".into(),
            maker_amount: "2500000".into(),
            taker_amount: "5000000".into(),
            maximum_loss_pusd_base_units: "2500000".into(),
            reservation_pusd_base_units: "2500000".into(),
            sell_outcome_share_payout_risk_cap_base_units: None,
            order_type: TrialOrderType::Gtc,
            post_only: true,
            defer_exec: false,
            expiration: "0".into(),
            metadata: format!("0x{}", "00".repeat(32)),
            builder: format!("0x{}", "00".repeat(32)),
            no_fee_or_rebate_credit_in_loss_bound: true,
            place_dispatch_allowance: 1,
            replacement_or_reprice_allowed: false,
            primary_cancel_dispatch_budget: 1,
            recovery_cancel_dispatch_budget: 2,
        },
        time_limits: TrialTimeLimits {
            maximum_preflight_observation_age_ms: 5_000,
            maximum_resting_duration_ms: 30_000,
            primary_cancel_deadline_ms: 65_000,
            cleanup_not_after_ms: 120_000,
            maximum_remediation_duration_ms: 90_000,
        },
        credential_slot: TrialCredentialSlot {
            slot_id: "pm-t2-slot-1".into(),
            nonsecret_fingerprint_sha256: hex(0x23),
            signer_to_proxy_evidence_reference: "reviewed-account-record:pm-t2-account-v1".into(),
        },
        journal: TrialJournalBinding {
            artifact_directory: "/tmp/reap-pm-t2-artifacts".into(),
            journal_family: reap_pm_controlled_trial::PM_T2_JOURNAL_FAMILY_V1.into(),
            journal_version: reap_pm_controlled_trial::PM_T2_JOURNAL_VERSION_V1,
            authorization_consumption_ledger_file:
                reap_pm_controlled_trial::PM_T2_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V1.into(),
            authorization_consumption_claim_file:
                reap_pm_controlled_trial::PM_T2_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V1.into(),
        },
    }
}

fn trial_authorization(config: &CanonicalTrialConfig) -> TrialAuthorization {
    TrialAuthorization {
        schema_version: 1,
        authorization_id: "pm-t2-a-authorization-1".into(),
        phase: TrialPhase::APlaceCancel,
        issuing_reviewer: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T11:59:00Z".into(),
        purpose: "one_exact_pm_t2_phase_a_passive_place_cancel_attempt".into(),
        not_before_utc: "2026-08-09T12:00:00Z".into(),
        expires_at_utc: "2026-08-09T12:15:00Z".into(),
        cleanup_not_after_utc: "2026-08-09T12:20:00Z".into(),
        build: AuthorizationBuildBinding {
            repository_commit: "66".repeat(20),
            clean_tree_attested: true,
            cargo_lock_sha256: hex(0x77),
            release_binary_sha256: hex(0x88),
            release_binary_length: 1_000_000,
            canonical_config_sha256: config.canonical_sha256().into(),
            canonical_config_length: config.canonical_length(),
            canonical_config_fingerprint: config.fingerprint().into(),
        },
        host: AuthorizationHostBinding {
            host_identity: "trial-host-1".into(),
            boot_identity: "01234567-89ab-cdef-0123-456789abcdef".into(),
            runtime_user: "reap-trial".into(),
            egress_identity: "203.0.113.7".into(),
        },
        trial: config.value().clone(),
        trial_plan_fingerprint: config.plan_fingerprint().into(),
        approval: AuthorizationApproval {
            only_named_phase: true,
            exactly_one_attempt: true,
            one_possible_fill_is_within_loss_cap: true,
            post_only_does_not_mean_no_fill: true,
            no_concurrent_proxy_trading_attested: true,
            independent_cleanup_method_reviewed: true,
        },
    }
}

fn preflight_evidence(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    leases: TrialJournalLeaseEvidence,
) -> TrialPreflightEvidence {
    let value = config.value();
    let approval = authorization.value();
    let common_stamp = || TrialObservationStamp {
        observed_at_monotonic_ns: 2_000_000_100,
        response_sha256: hex(0xa0),
    };
    TrialPreflightEvidence {
        schema_version: reap_pm_controlled_trial::TRIAL_PREFLIGHT_SCHEMA_VERSION,
        binding: TrialPreflightBinding {
            phase: value.phase,
            canonical_config_sha256: config.canonical_sha256().into(),
            canonical_config_length: config.canonical_length(),
            canonical_config_fingerprint: config.fingerprint().into(),
            trial_plan_fingerprint: config.plan_fingerprint().into(),
            authorization_id: approval.authorization_id.clone(),
            authorization_fingerprint: authorization.fingerprint().into(),
            source_pin_manifest_sha256: value.source_pin_manifest_sha256.clone(),
            runbook_revision: value.runbook_revision.clone(),
            runbook_sha256: value.runbook_sha256.clone(),
            repository_commit: approval.build.repository_commit.clone(),
            cargo_lock_sha256: approval.build.cargo_lock_sha256.clone(),
            clean_tree_attested: true,
            release_binary_sha256: approval.build.release_binary_sha256.clone(),
            release_binary_length: approval.build.release_binary_length,
            host: approval.host.clone(),
            credential_slot_id: value.credential_slot.slot_id.clone(),
            credential_slot_nonsecret_fingerprint_sha256: value
                .credential_slot
                .nonsecret_fingerprint_sha256
                .clone(),
            signer_to_proxy_evidence_reference: value
                .credential_slot
                .signer_to_proxy_evidence_reference
                .clone(),
            journal: value.journal.clone(),
            leases,
        },
        window: TrialPreflightWindow {
            observation_started_at_utc: "2026-08-09T12:05:00Z".into(),
            observation_completed_at_utc: "2026-08-09T12:05:03Z".into(),
            validated_at_utc: "2026-08-09T12:05:04Z".into(),
            maximum_observation_age_ms: 5_000,
            dispatch_deadline_at_utc: "2026-08-09T12:05:05Z".into(),
            observation_started_monotonic_ns: 100,
            observation_completed_monotonic_ns: 3_000_000_100,
            validated_at_monotonic_ns: 4_000_000_100,
            dispatch_deadline_monotonic_ns: 5_000_000_100,
        },
        environment: TrialEnvironmentPreflight {
            geoblock: TrialGeoblockEvidence {
                endpoint: "https://polymarket.com/api/geoblock".into(),
                egress_identity: approval.host.egress_identity.clone(),
                reported_ip: approval.host.egress_identity.clone(),
                country: "US".into(),
                region: "NY".into(),
                blocked: false,
                same_egress_as_clob: true,
                stamp: common_stamp(),
            },
            server_time: TrialServerTimeEvidence {
                endpoint: "https://clob.polymarket.com/time".into(),
                server_unix_ms: 1_786_277_102_000,
                previous_server_unix_ms: 1_786_277_101_000,
                local_wall_receive_unix_ms: 1_786_277_102_100,
                maximum_absolute_skew_ms: 5_000,
                product_clock_epoch_fingerprint: hex(0x05),
                previous_product_clock_epoch_fingerprint: hex(0x05),
                timestamp_regression_absent: true,
                epoch_unchanged: true,
                stamp: common_stamp(),
            },
            clob_health_response_sha256: hex(0x06),
            clob_health_green: true,
            matching_engine_restart_reported: false,
            restricted_mode_observed: false,
            health_stamp: TrialObservationStamp {
                observed_at_monotonic_ns: 2_000_000_100,
                response_sha256: hex(0x06),
            },
        },
        market: TrialMarketPreflight {
            condition_id: value.market.condition_id.clone(),
            question_id: value.market.question_id.clone(),
            token_id: value.market.token_id.clone(),
            outcome_label: value.market.outcome_label.clone(),
            token_count: 2,
            exact_token_membership: true,
            domain: value.market.domain,
            exchange: value.market.exchange.clone(),
            pusd_contract: value.market.pusd_contract.clone(),
            conditional_tokens_contract: value.market.conditional_tokens_contract.clone(),
            active: true,
            closed: false,
            archived: false,
            accepting_orders: true,
            order_book_enabled: true,
            tick: value.order.tick.clone(),
            minimum_order_size: value.order.minimum_order_size.clone(),
            maker_base_fee_bps: 0,
            taker_base_fee_bps: 0,
            fee_rate: Some("0.020".into()),
            fee_exponent: Some("2.0".into()),
            fee_taker_only: Some(true),
            fee_policy_reviewed_and_supported: true,
            take_only_delay_enabled: false,
            seconds_delay: 0,
            cancel_book_on_start: false,
            minimum_order_age_seconds: 0,
            sports_market: false,
            accepting_order_timestamp_utc: Some("2026-08-09T12:00:00Z".into()),
            game_start_time_utc: None,
            end_time_utc: Some("2026-08-10T12:00:00Z".into()),
            resolution_not_imminent: true,
            long_market_response_sha256: hex(0x07),
            clob_market_response_sha256: hex(0x08),
            stamp: common_stamp(),
        },
        book: TrialBookPreflight {
            condition_id: value.market.condition_id.clone(),
            token_id: value.market.token_id.clone(),
            tick: value.order.tick.clone(),
            exact_order_price: value.order.price.clone(),
            best_bid: "0.49".into(),
            best_ask: "0.51".into(),
            snapshot_sequence: 1,
            stream_epoch_fingerprint: hex(0x09),
            public_stream_established: true,
            two_sided: true,
            not_crossed: true,
            tick_unchanged: true,
            passive_at_dispatch: true,
            stamp: common_stamp(),
        },
        account: TrialAccountPreflight {
            closed_only: TrialClosedOnlyEvidence {
                endpoint: "https://clob.polymarket.com/auth/ban-status/closed-only".into(),
                signer: value.account.signer.clone(),
                closed_only: false,
                stamp: common_stamp(),
            },
            private_cut: TrialPrivateAccountCut {
                signature_type: 1,
                signer: value.account.signer.clone(),
                funder: value.account.funder.clone(),
                private_key_derived_signer_matches: true,
                l2_signer_matches: true,
                remote_api_key_owner_matches_signer: true,
                signer_to_proxy_evidence_reviewed: true,
                collateral_response_sha256: hex(0x0a),
                configured_token_response_sha256: hex(0x0b),
                balance_response_count: 2,
                allowance_entry_count: 2,
                collateral_balance_sufficient_for_order: true,
                collateral_allowance_sufficient_for_order: true,
                configured_token_balance_sufficient_for_order: true,
                configured_token_allowance_sufficient_for_order: true,
                all_remote_reservations_accounted: true,
                stamp: common_stamp(),
            },
        },
        finalized_chain: TrialFinalizedChainPreflight {
            rpc_origin: "https://polygon.drpc.org".into(),
            chain_id: 137,
            proxy_funder: value.account.funder.clone(),
            exchange_spender: value.market.exchange.clone(),
            pusd_contract: value.market.pusd_contract.clone(),
            conditional_tokens_contract: value.market.conditional_tokens_contract.clone(),
            finalized_block_number: 77_000_000,
            finalized_block_hash: hex(0x0c),
            finalized_block_unix_seconds: 1_786_277_100,
            rpc_request_count: 5,
            chain_id_response_sha256: hex(0x0d),
            finalized_block_response_sha256: hex(0x0e),
            allowance_response_sha256: hex(0x0f),
            operator_approval_response_sha256: hex(0x10),
            block_reread_response_sha256: hex(0x11),
            one_finalized_block_for_all_calls: true,
            finalized_block_reread_matched: true,
            finalized_block_fresh: true,
            pusd_allowance_sufficient: true,
            conditional_tokens_operator_approved: true,
            stamp: common_stamp(),
        },
        data_api_position: TrialDataApiPositionPreflight {
            proxy_funder: value.account.funder.clone(),
            condition_id: value.market.condition_id.clone(),
            token_id: value.market.token_id.clone(),
            pages_observed: 1,
            rows_observed: 0,
            configured_token_row_count: 0,
            page_walk_response_sha256: hex(0x12),
            complete_bounded_page_walk: true,
            configured_position: TrialConfiguredPositionState::Absent,
            position_consistent_with_account_cut: true,
            stamp: common_stamp(),
        },
        reconciliation: TrialReconciliationPreflight {
            user_stream: TrialUserStreamPreflight {
                signer: value.account.signer.clone(),
                condition_id: value.market.condition_id.clone(),
                epoch_fingerprint: hex(0x13),
                business_event_set_sha256: hex(0x14),
                authenticated: true,
                bounded_subscription_acknowledged: true,
                application_heartbeat_observed: true,
                correlated_pong_observed: true,
                socket_not_retired: true,
                business_event_count: 0,
                business_event_scope_exact: true,
                same_credential_rest_owner_evidence_joined: true,
                epoch_unchanged: true,
                reconnect_count: 0,
                latest_reconnect_monotonic_ns: None,
                complete_cuts_refreshed_after_latest_reconnect: true,
                stamp: common_stamp(),
            },
            account_wide_open_orders: TrialCompleteCutEvidence {
                pages_observed: 1,
                rows_observed: 0,
                terminal_cursor_observed: true,
                complete: true,
                response_set_sha256: hex(0x15),
                stamp: common_stamp(),
            },
            account_wide_trades: TrialCompleteCutEvidence {
                pages_observed: 1,
                rows_observed: 0,
                terminal_cursor_observed: true,
                complete: true,
                response_set_sha256: hex(0x16),
                stamp: common_stamp(),
            },
            exact_details: TrialExactDetailCutEvidence {
                implicated_order_id_count: 0,
                exact_detail_count: 0,
                complete: true,
                response_set_sha256: hex(0x17),
                stamp: common_stamp(),
            },
            no_existing_trial_order: true,
            no_unmanaged_order: true,
            no_unresolved_fill: true,
            no_ambiguous_state: true,
            no_concurrent_proxy_trading_attested: true,
            credential_visible_scope_not_funder_wide: true,
            reconciliation_commitment_sha256: hex(0x18),
        },
        risk: TrialRiskPreflight {
            side: value.order.side,
            price: value.order.price.clone(),
            quantity: value.order.quantity.clone(),
            maker_amount: value.order.maker_amount.clone(),
            taker_amount: value.order.taker_amount.clone(),
            maximum_loss_pusd_base_units: value.order.maximum_loss_pusd_base_units.clone(),
            reservation_pusd_base_units: value.order.reservation_pusd_base_units.clone(),
            sell_outcome_share_payout_risk_cap_base_units: value
                .order
                .sell_outcome_share_payout_risk_cap_base_units
                .clone(),
            unsigned_order_semantic_commitment_sha256: hex(0x19),
            risk_decision_commitment_sha256: hex(0x1a),
            reservation_commitment_sha256: hex(0x1b),
            loss_commitment_sha256: hex(0x1c),
            remote_reservation_set_sha256: hex(0x1d),
            remote_reservation_count: 0,
            exact_amounts_recomputed: true,
            risk_within_exact_cap: true,
            reservation_is_exact_and_durable: true,
            all_remote_reservations_deducted: true,
            available_after_reservations_sufficient: true,
            one_possible_fill_within_loss_cap: true,
            no_fee_or_rebate_credit_in_loss_bound: true,
            one_exact_order_commitment: true,
            replacement_or_reprice_disabled: true,
        },
        phase_gate: TrialPhaseGateEvidence {
            phase_a_implementation_gates_green: true,
            accepted_phase_a_evidence_sha256: None,
            separate_phase_b_authorization: false,
        },
        authorization: OfflineAuthorizationState::DENIED,
    }
}
