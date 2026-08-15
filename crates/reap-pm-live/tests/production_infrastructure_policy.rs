const LIB: &str = include_str!("../src/lib.rs");
const READS: &str = include_str!("../src/production_read_infrastructure.rs");
const SUPERVISOR: &str = include_str!("../src/production_supervisor.rs");
const ROLES: &str = include_str!("../src/production_supervisor_roles.rs");

#[test]
fn concrete_production_reads_remain_complete_scoped_and_fail_closed() {
    for required in [
        "PmProductionExecutionReadInfrastructure",
        "begin_condition_open_orders",
        "continue_open_orders",
        "begin_condition_trades",
        "continue_trades",
        "production_observe_configured_token",
        "if !server_time.is_production()",
        "!authenticated_http.is_production()",
        "!role.is_production()",
        "!source.is_production()",
        "authenticated_http.configured_l2_signer() != scope.expected_signer",
        "authenticated_http.configured_expected_maker() != scope.expected_maker",
        "role.configured_l2_signer() != scope.expected_signer",
        "role.configured_expected_maker() != scope.expected_maker",
        "PmUserWsEvent::SubscriptionSent(_)",
        "PmSupervisorWsEvent::Connected",
        "PmSupervisorWsEvent::Disconnected",
        "PmSupervisorWsEvent::ReconciliationRequired",
        ".strip_prefix(\"TRADE_STATUS_\")",
        "TradeDisposition::Confirmed",
        "(fill.fill_id.clone(), fill.venue_order_id.clone())",
        "into_production_supervisor_roles",
    ] {
        assert!(
            READS.contains(required),
            "production reads lost `{required}`"
        );
    }

    let ws_constructor = READS
        .split_once("fn from_scope(\n        scope: ProductionReadScope,\n        role: PmAuthenticatedUserWsRole,")
        .and_then(|(_, tail)| tail.split_once("fn start(&mut self)").map(|(head, _)| head))
        .expect("private WebSocket constructor/start boundary");
    assert!(
        !ws_constructor.contains(".spawn("),
        "private WebSocket must not start before journal recovery"
    );
}

#[test]
fn supervisor_requires_private_connectivity_and_routes_exact_cancel_from_durable_facts() {
    for required in [
        "private_ws_connected: bool",
        "state.ready = false;",
        "state.ready = state.private_ws_connected",
        "async fn cancel_exact(\n        &mut self,\n        venue_order_id: &str,\n        token_id: &str,",
        "let token_id = order.facts.token_id.clone();",
        "role.shutdown().await.is_ok()",
    ] {
        assert!(
            SUPERVISOR.contains(required),
            "production supervisor lost `{required}`"
        );
    }
    for required in [
        "PmSupervisorProductionMutationRole",
        "matches_supervisor_scope",
        "self.l2_signer == l2_signer",
        "self.expected_maker == expected_maker",
        "async fn cancel_exact(\n        &mut self,\n        venue_order_id: &str,\n        token_id: &str,",
    ] {
        assert!(
            ROLES.contains(required),
            "production roles lost `{required}`"
        );
    }
    assert!(LIB.contains("mod production_read_infrastructure;"));
    assert!(LIB.contains("PmSupervisorProductionMutationRole"));
}
