use serde::Serialize;

/// Current ownership classification used by the checked-in OKX capability
/// inventory. This is descriptive until the role-specific adapters enforce it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum OkxCapabilityClass {
    ChaosExecution,
    ChaosObservation,
    ReadinessSafety,
    EmergencyCleanup,
    EvidenceOnly,
    TestOnly,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxCapabilityAccess {
    Read,
    Write,
    ReadWrite,
    Connect,
    Filter,
    Authority,
}

/// Secret-free registry row for one currently implemented OKX operation.
///
/// The registry is deliberately data-only: possessing a row grants no signer,
/// transport, credential, or endpoint access. Later boundary phases use the
/// same identifiers to construct narrow role allowlists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OkxCapabilityRegistration {
    pub capability_id: &'static str,
    pub endpoint_or_channel: &'static str,
    pub operation: &'static str,
    pub access: OkxCapabilityAccess,
    pub trust_plane: &'static str,
    pub modes: &'static [&'static str],
    pub requirement_ids: &'static [&'static str],
    pub consumer: &'static str,
    pub class: OkxCapabilityClass,
    pub production_reachable: bool,
    pub allowed_in_live_plan: bool,
    pub target_disposition: &'static str,
}

macro_rules! capability {
    ($name:ident, $id:literal, $surface:literal, $operation:literal, $access:ident,
     $plane:literal, [$($mode:literal),* $(,)?], [$($requirement:literal),* $(,)?],
     $consumer:literal, $class:ident, $production:literal, $live:literal,
     $disposition:literal) => {
        pub(crate) const $name: OkxCapabilityRegistration = OkxCapabilityRegistration {
            capability_id: $id,
            endpoint_or_channel: $surface,
            operation: $operation,
            access: OkxCapabilityAccess::$access,
            trust_plane: $plane,
            modes: &[$($mode),*],
            requirement_ids: &[$($requirement),*],
            consumer: $consumer,
            class: OkxCapabilityClass::$class,
            production_reachable: $production,
            allowed_in_live_plan: $live,
            target_disposition: $disposition,
        };
    };
}

capability!(
    REST_PLACE_REGULAR,
    "OKX-REST-PLACE-REGULAR",
    "/api/v5/trade/order",
    "POST regular limit order",
    Write,
    "regular execution",
    ["demo"],
    ["CHAOS-EXEC-QUOTE", "CHAOS-EXEC-HEDGE"],
    "validated quote/hedge execution policy",
    ChaosExecution,
    true,
    true,
    "keep behind RegularExecution"
);
capability!(
    REST_CANCEL_REGULAR,
    "OKX-REST-CANCEL-REGULAR",
    "/api/v5/trade/cancel-order",
    "POST one regular cancel",
    Write,
    "regular execution and live safety",
    ["demo"],
    ["CHAOS-EXEC-CANCEL-OWNED", "SAFE-REGULAR-CANCEL"],
    "owned regular cancellation",
    ReadinessSafety,
    true,
    true,
    "keep behind owned regular cancel roles"
);
capability!(
    REST_CANCEL_BATCH_REGULAR,
    "OKX-REST-CANCEL-BATCH-REGULAR",
    "/api/v5/trade/cancel-batch-orders",
    "POST account-wide regular batch cancel",
    Write,
    "emergency account stop",
    ["emergency"],
    ["OPS-EMERGENCY-REGULAR"],
    "account-wide emergency regular mitigation",
    EmergencyCleanup,
    true,
    false,
    "move to emergency adapter"
);
capability!(
    REST_REGULAR_CAA,
    "OKX-REST-REGULAR-CAA",
    "/api/v5/trade/cancel-all-after",
    "POST regular Cancel All After",
    Write,
    "live safety and emergency account stop",
    ["demo", "emergency"],
    ["SAFE-REGULAR-CAA", "OPS-EMERGENCY-REGULAR"],
    "regular deadman protection",
    ReadinessSafety,
    true,
    true,
    "split live-safety and emergency roles"
);
capability!(
    REST_PUBLIC_TIME,
    "OKX-REST-PUBLIC-TIME",
    "/api/v5/public/time",
    "GET exchange time",
    Read,
    "live safety, emergency, and offline evidence",
    ["observe", "demo", "emergency", "offline_evidence"],
    [
        "SAFE-CLOCK-STATUS",
        "OPS-EMERGENCY-REGULAR",
        "OPS-EMERGENCY-ALGO",
        "OPS-EMERGENCY-SPREAD",
        "EVIDENCE-PUBLIC-READ"
    ],
    "clock skew and signed-request bracketing",
    ReadinessSafety,
    true,
    true,
    "keep in narrow read roles"
);
capability!(
    REST_INDEX_TICKER,
    "OKX-REST-INDEX-TICKER",
    "/api/v5/market/index-tickers",
    "GET one index ticker",
    Read,
    "offline evidence",
    ["offline_evidence"],
    ["SAFE-STABLECOIN", "EVIDENCE-PUBLIC-READ"],
    "account/equity certification",
    EvidenceOnly,
    true,
    false,
    "move to evidence adapter"
);
capability!(
    REST_SYSTEM_STATUS,
    "OKX-REST-SYSTEM-STATUS",
    "/api/v5/system/status",
    "GET exchange maintenance status",
    Read,
    "live safety and offline evidence",
    ["observe", "demo", "offline_evidence"],
    ["SAFE-CLOCK-STATUS", "EVIDENCE-PUBLIC-READ"],
    "maintenance readiness guard",
    ReadinessSafety,
    true,
    true,
    "keep with plan-derived relevance"
);
capability!(
    REST_REGULAR_PENDING,
    "OKX-REST-REGULAR-PENDING",
    "/api/v5/trade/orders-pending",
    "GET pending regular orders",
    Read,
    "live readiness, emergency, and offline evidence",
    ["observe", "demo", "emergency", "offline_evidence"],
    [
        "SAFE-RECONCILE",
        "OPS-EMERGENCY-REGULAR",
        "EVIDENCE-ACCOUNT-READ"
    ],
    "regular reconciliation and account-wide zero proof",
    ReadinessSafety,
    true,
    true,
    "split reconciliation, emergency, and evidence roles"
);
capability!(
    REST_FILLS,
    "OKX-REST-FILLS",
    "/api/v5/trade/fills",
    "GET regular fills",
    Read,
    "live readiness and offline evidence",
    ["observe", "demo", "offline_evidence"],
    ["SAFE-RECONCILE", "EVIDENCE-ACCOUNT-READ"],
    "fill reconciliation and statement evidence",
    ReadinessSafety,
    true,
    true,
    "split reconciliation and evidence roles"
);
capability!(
    REST_ORDER_DETAILS,
    "OKX-REST-ORDER-DETAILS",
    "/api/v5/trade/order",
    "GET one regular order",
    Read,
    "live readiness and offline evidence",
    ["observe", "demo", "offline_evidence"],
    ["SAFE-RECONCILE", "EVIDENCE-ACCOUNT-READ"],
    "restart ambiguity and deadman evidence",
    ReadinessSafety,
    true,
    true,
    "split reconciliation and evidence roles"
);
capability!(
    REST_ACCOUNT_INSTRUMENTS,
    "OKX-REST-ACCOUNT-INSTRUMENTS",
    "/api/v5/account/instruments",
    "GET authenticated instrument metadata",
    Read,
    "live readiness and offline evidence",
    ["observe", "demo", "offline_evidence"],
    ["SAFE-METADATA", "EVIDENCE-ACCOUNT-READ"],
    "instrument rule bootstrap and drift checks",
    ReadinessSafety,
    true,
    true,
    "split metadata and evidence roles"
);
capability!(
    REST_ACCOUNT_TRADE_FEE,
    "OKX-REST-ACCOUNT-TRADE-FEE",
    "/api/v5/account/trade-fee",
    "GET authenticated fee schedule",
    Read,
    "live readiness and offline evidence",
    ["observe", "demo", "offline_evidence"],
    ["SAFE-METADATA", "EVIDENCE-ACCOUNT-READ"],
    "fee bootstrap and drift checks",
    ReadinessSafety,
    true,
    true,
    "split metadata and evidence roles"
);
capability!(
    REST_ACCOUNT_CONFIG,
    "OKX-REST-ACCOUNT-CONFIG",
    "/api/v5/account/config",
    "GET account configuration",
    Read,
    "live readiness, emergency identity, and offline evidence",
    ["observe", "demo", "emergency", "offline_evidence"],
    [
        "SAFE-METADATA",
        "OPS-EMERGENCY-IDENTITY",
        "EVIDENCE-ACCOUNT-READ"
    ],
    "account mode, STP, key policy, and identity binding",
    ReadinessSafety,
    true,
    true,
    "split metadata, emergency identity, and evidence roles"
);
capability!(
    REST_ACCOUNT_BALANCE,
    "OKX-REST-ACCOUNT-BALANCE",
    "/api/v5/account/balance",
    "GET account balances",
    Read,
    "live readiness and offline evidence",
    ["observe", "demo", "offline_evidence"],
    ["SAFE-RECONCILE", "EVIDENCE-ACCOUNT-READ"],
    "account reconciliation and certification",
    ReadinessSafety,
    true,
    true,
    "split reconciliation and evidence roles"
);
capability!(
    REST_ACCOUNT_POSITIONS,
    "OKX-REST-ACCOUNT-POSITIONS",
    "/api/v5/account/positions",
    "GET account positions",
    Read,
    "live readiness and offline evidence",
    ["observe", "demo", "offline_evidence"],
    [
        "SAFE-RECONCILE",
        "SAFE-ACCOUNT-POSITIONS",
        "EVIDENCE-ACCOUNT-READ"
    ],
    "position reconciliation and foreign exposure detection",
    ReadinessSafety,
    true,
    true,
    "split reconciliation and evidence roles"
);
capability!(
    REST_ACCOUNT_BILLS,
    "OKX-REST-ACCOUNT-BILLS",
    "/api/v5/account/bills",
    "GET account bills",
    Read,
    "offline evidence",
    ["offline_evidence"],
    ["EVIDENCE-ACCOUNT-READ"],
    "economic statement collection",
    EvidenceOnly,
    true,
    false,
    "move to evidence adapter"
);
capability!(
    REST_ALGO_PENDING,
    "OKX-REST-ALGO-PENDING",
    "/api/v5/trade/orders-algo-pending",
    "GET one pending algo family",
    Read,
    "forbidden observer, emergency, and offline evidence",
    ["observe", "demo", "emergency", "offline_evidence"],
    [
        "SAFE-FORBIDDEN-ZERO",
        "OPS-EMERGENCY-ALGO",
        "EVIDENCE-ACCOUNT-READ"
    ],
    "forbidden-domain and emergency zero proof",
    ReadinessSafety,
    true,
    true,
    "keep read-only live; split emergency/evidence roles"
);
capability!(
    REST_CANCEL_ALGO,
    "OKX-REST-CANCEL-ALGO",
    "/api/v5/trade/cancel-algos",
    "POST pending algo cancellation",
    Write,
    "emergency account stop",
    ["emergency"],
    ["OPS-EMERGENCY-ALGO"],
    "account-wide emergency algo mitigation",
    EmergencyCleanup,
    true,
    false,
    "move to emergency adapter"
);
capability!(
    REST_SPREAD_PENDING,
    "OKX-REST-SPREAD-PENDING",
    "/api/v5/sprd/orders-pending",
    "GET pending spread orders",
    Read,
    "forbidden observer, emergency, and offline evidence",
    ["observe", "demo", "emergency", "offline_evidence"],
    [
        "SAFE-FORBIDDEN-ZERO",
        "OPS-EMERGENCY-SPREAD",
        "EVIDENCE-ACCOUNT-READ"
    ],
    "forbidden-domain and emergency zero proof",
    ReadinessSafety,
    true,
    true,
    "keep read-only live; split emergency/evidence roles"
);
capability!(
    REST_SPREAD_MASS_CANCEL,
    "OKX-REST-SPREAD-MASS-CANCEL",
    "/api/v5/sprd/mass-cancel",
    "POST spread mass cancel",
    Write,
    "emergency account stop",
    ["emergency"],
    ["OPS-EMERGENCY-SPREAD"],
    "account-wide emergency spread mitigation",
    EmergencyCleanup,
    true,
    false,
    "move to emergency adapter"
);
capability!(
    REST_SPREAD_CAA,
    "OKX-REST-SPREAD-CAA",
    "/api/v5/sprd/cancel-all-after",
    "POST spread Cancel All After",
    Write,
    "emergency account stop",
    ["emergency"],
    ["OPS-EMERGENCY-SPREAD"],
    "emergency spread deadman protection",
    EmergencyCleanup,
    true,
    false,
    "move to emergency adapter"
);

capability!(
    WS_BOOKS,
    "OKX-WS-BOOKS",
    "books",
    "subscribe and parse full book",
    Read,
    "live observation and capture",
    ["observe", "demo", "capture"],
    ["CHAOS-MD-BOOK", "CAPTURE-PUBLIC-MARKET"],
    "quote/hedge market state and capture",
    ChaosObservation,
    true,
    true,
    "keep plan-derived"
);
capability!(
    WS_BOOKS_L2_TBT,
    "OKX-WS-BOOKS-L2-TBT",
    "books-l2-tbt",
    "subscribe and parse capture depth variants",
    Read,
    "capture",
    ["capture"],
    ["CAPTURE-PUBLIC-MARKET"],
    "credential-free market capture",
    EvidenceOnly,
    true,
    false,
    "keep only in the separate capture contract"
);
capability!(
    WS_BOOKS50_L2_TBT,
    "OKX-WS-BOOKS50-L2-TBT",
    "books50-l2-tbt",
    "subscribe and parse capture depth variant",
    Read,
    "capture",
    ["capture"],
    ["CAPTURE-PUBLIC-MARKET"],
    "credential-free market capture",
    EvidenceOnly,
    true,
    false,
    "keep only in the separate capture contract"
);
capability!(
    WS_TRADES,
    "OKX-WS-TRADES",
    "trades",
    "subscribe and parse public trades",
    Read,
    "live observation and capture",
    ["observe", "demo", "capture"],
    ["CHAOS-MD-TRADE", "CAPTURE-PUBLIC-MARKET"],
    "pinned trade input and capture",
    ChaosObservation,
    true,
    true,
    "keep plan-derived"
);
capability!(
    WS_TRADES_ALL,
    "OKX-WS-TRADES-ALL",
    "trades-all",
    "subscribe and parse capture trade variant",
    Read,
    "capture",
    ["capture"],
    ["CAPTURE-PUBLIC-MARKET"],
    "credential-free market capture",
    EvidenceOnly,
    true,
    false,
    "keep only in the separate capture contract"
);
capability!(
    WS_FUNDING_RATE,
    "OKX-WS-FUNDING-RATE",
    "funding-rate",
    "subscribe and parse funding rate",
    Read,
    "live observation and capture",
    ["observe", "demo", "capture"],
    ["CHAOS-REF-FUNDING", "CAPTURE-PUBLIC-MARKET"],
    "funding-aware pricing and capture",
    ChaosObservation,
    true,
    true,
    "keep when configured"
);
capability!(
    WS_INDEX_TICKERS,
    "OKX-WS-INDEX-TICKERS",
    "index-tickers",
    "subscribe and parse index ticker",
    Read,
    "live observation and capture",
    ["observe", "demo", "capture"],
    [
        "CHAOS-REF-INDEX",
        "SAFE-STABLECOIN",
        "CAPTURE-PUBLIC-MARKET"
    ],
    "strategy/stablecoin references and capture",
    ChaosObservation,
    true,
    true,
    "keep when configured"
);
capability!(
    WS_PRICE_LIMIT,
    "OKX-WS-PRICE-LIMIT",
    "price-limit",
    "subscribe and parse price limits",
    Read,
    "live observation and capture",
    ["observe", "demo", "capture"],
    ["CHAOS-REF-LIMITS", "CAPTURE-PUBLIC-MARKET"],
    "price bounds and capture",
    ChaosObservation,
    true,
    true,
    "keep when configured"
);
capability!(
    WS_MARK_PRICE,
    "OKX-WS-MARK-PRICE",
    "mark-price",
    "subscribe and parse mark price",
    Read,
    "live observation and capture",
    ["observe", "demo", "capture"],
    ["CHAOS-REF-MARK", "CAPTURE-PUBLIC-MARKET"],
    "derivative valuation and capture",
    ChaosObservation,
    true,
    true,
    "keep when configured"
);
capability!(
    WS_ORDERS,
    "OKX-WS-ORDERS",
    "orders",
    "subscribe and parse private regular orders",
    Read,
    "live observation",
    ["observe", "demo"],
    ["CHAOS-STATE-ORDERS"],
    "canonical order/fill convergence",
    ChaosObservation,
    true,
    true,
    "keep on planned private state socket"
);
capability!(
    WS_FILLS,
    "OKX-WS-FILLS",
    "fills",
    "subscribe and parse fee-bearing fills",
    Read,
    "live observation",
    ["observe", "demo"],
    ["CHAOS-STATE-ORDERS", "SAFE-RECONCILE"],
    "optional canonical fee-bearing fill consumer",
    ChaosObservation,
    true,
    true,
    "keep only when explicitly configured"
);
capability!(
    WS_ACCOUNT,
    "OKX-WS-ACCOUNT",
    "account",
    "subscribe and parse account state",
    Read,
    "live observation",
    ["observe", "demo"],
    ["CHAOS-STATE-ACCOUNT"],
    "cash, equity, margin, and risk state",
    ChaosObservation,
    true,
    true,
    "keep on planned private state socket"
);
capability!(
    WS_POSITIONS,
    "OKX-WS-POSITIONS",
    "positions",
    "subscribe and parse position state",
    Read,
    "live observation",
    ["observe", "demo"],
    ["CHAOS-STATE-POSITIONS", "SAFE-ACCOUNT-POSITIONS"],
    "position and fill convergence",
    ChaosObservation,
    true,
    true,
    "keep on planned private state socket"
);
capability!(
    WS_SUBSCRIBE,
    "OKX-WS-SUBSCRIBE",
    "subscribe",
    "send websocket subscription operation",
    Write,
    "live observation and capture",
    ["observe", "demo", "capture"],
    [
        "CHAOS-MD-BOOK",
        "CHAOS-MD-TRADE",
        "CHAOS-REF-INDEX",
        "CHAOS-REF-FUNDING",
        "CHAOS-REF-MARK",
        "CHAOS-REF-LIMITS",
        "SAFE-STABLECOIN",
        "CHAOS-STATE-ORDERS",
        "CHAOS-STATE-ACCOUNT",
        "CHAOS-STATE-POSITIONS",
        "SAFE-RECONCILE",
        "SAFE-ACCOUNT-POSITIONS",
        "CAPTURE-PUBLIC-MARKET"
    ],
    "planned channel subscription bootstrap",
    ChaosObservation,
    true,
    true,
    "keep only through planned session factories"
);
capability!(
    WS_LOGIN,
    "OKX-WS-LOGIN",
    "login",
    "sign and send websocket login",
    Write,
    "live observation and regular execution",
    ["observe", "demo"],
    [
        "CHAOS-STATE-ORDERS",
        "CHAOS-STATE-ACCOUNT",
        "CHAOS-STATE-POSITIONS",
        "SAFE-RECONCILE",
        "SAFE-ACCOUNT-POSITIONS",
        "CHAOS-EXEC-QUOTE",
        "CHAOS-EXEC-HEDGE",
        "CHAOS-EXEC-CANCEL-OWNED",
        "SAFE-REGULAR-CANCEL"
    ],
    "private state and order-command session authentication",
    ChaosObservation,
    true,
    true,
    "make private to role-owned session factories"
);
capability!(
    WS_PLACE_REGULAR,
    "OKX-WS-PLACE-REGULAR",
    "order",
    "send regular websocket place operation",
    Write,
    "regular execution",
    ["demo"],
    ["CHAOS-EXEC-QUOTE", "CHAOS-EXEC-HEDGE"],
    "validated quote/hedge execution policy",
    ChaosExecution,
    true,
    true,
    "keep behind RegularOrderSessionFactory"
);
capability!(
    WS_CANCEL_REGULAR,
    "OKX-WS-CANCEL-REGULAR",
    "cancel-order",
    "send regular websocket cancel operation",
    Write,
    "regular execution and live safety",
    ["demo"],
    ["CHAOS-EXEC-CANCEL-OWNED", "SAFE-REGULAR-CANCEL"],
    "owned regular cancellation",
    ReadinessSafety,
    true,
    true,
    "keep behind RegularOrderSessionFactory"
);
capability!(
    WS_LIVENESS,
    "OKX-WS-LIVENESS",
    "ping / pong / close",
    "send and receive websocket liveness control frames",
    ReadWrite,
    "live observation, regular execution, capture, and fault tooling",
    ["observe", "demo", "capture", "test"],
    [
        "CHAOS-MD-BOOK",
        "CHAOS-MD-TRADE",
        "CHAOS-REF-INDEX",
        "CHAOS-REF-FUNDING",
        "CHAOS-REF-MARK",
        "CHAOS-REF-LIMITS",
        "SAFE-STABLECOIN",
        "CHAOS-STATE-ORDERS",
        "CHAOS-STATE-ACCOUNT",
        "CHAOS-STATE-POSITIONS",
        "SAFE-RECONCILE",
        "SAFE-ACCOUNT-POSITIONS",
        "CHAOS-EXEC-QUOTE",
        "CHAOS-EXEC-HEDGE",
        "CHAOS-EXEC-CANCEL-OWNED",
        "SAFE-REGULAR-CANCEL",
        "CAPTURE-PUBLIC-MARKET",
        "TEST-FAULT-TRANSPORT"
    ],
    "session liveness, bounded shutdown, and reconnect",
    ReadinessSafety,
    true,
    true,
    "keep inside admitted session implementations"
);

capability!(
    CONNECTION_PUBLIC,
    "OKX-CONNECTION-PUBLIC",
    "public websocket",
    "construct supervised public session",
    Connect,
    "live observation",
    ["observe", "demo"],
    [
        "CHAOS-MD-BOOK",
        "CHAOS-MD-TRADE",
        "CHAOS-REF-INDEX",
        "CHAOS-REF-FUNDING",
        "CHAOS-REF-MARK",
        "CHAOS-REF-LIMITS",
        "SAFE-STABLECOIN"
    ],
    "deduplicated plan-derived public consumers",
    ChaosObservation,
    true,
    true,
    "derive exact replicas from live plan"
);
capability!(
    CONNECTION_PRIVATE_STATE,
    "OKX-CONNECTION-PRIVATE-STATE",
    "private websocket",
    "construct authenticated private state session",
    Connect,
    "live observation",
    ["observe", "demo"],
    [
        "CHAOS-STATE-ORDERS",
        "CHAOS-STATE-ACCOUNT",
        "CHAOS-STATE-POSITIONS",
        "SAFE-RECONCILE",
        "SAFE-ACCOUNT-POSITIONS"
    ],
    "canonical private state consumer",
    ChaosObservation,
    true,
    true,
    "pack planned compatible channels per account"
);
capability!(
    CONNECTION_ORDER_COMMAND,
    "OKX-CONNECTION-ORDER-COMMAND",
    "business websocket",
    "construct authenticated order-command session",
    Connect,
    "regular execution",
    ["demo"],
    [
        "CHAOS-EXEC-QUOTE",
        "CHAOS-EXEC-HEDGE",
        "CHAOS-EXEC-CANCEL-OWNED",
        "SAFE-REGULAR-CANCEL"
    ],
    "account-scoped regular dispatch families",
    ChaosExecution,
    true,
    true,
    "derive nonempty shards from live plan"
);
capability!(
    CONNECTION_CAPTURE_PUBLIC,
    "OKX-CONNECTION-CAPTURE-PUBLIC",
    "public websocket",
    "construct capture session",
    Connect,
    "capture",
    ["capture"],
    ["CAPTURE-PUBLIC-MARKET"],
    "credential-free market capture",
    EvidenceOnly,
    true,
    false,
    "keep in separate capture contract"
);
capability!(
    CONNECTION_FAULT_PROXY,
    "OKX-CONNECTION-FAULT-PROXY",
    "loopback REST/public/private/order proxy",
    "construct controlled fault transport",
    Connect,
    "fault tooling",
    ["test"],
    ["TEST-FAULT-TRANSPORT"],
    "controlled loopback failure injection",
    TestOnly,
    false,
    false,
    "keep outside live plan"
);
capability!(
    MAINTENANCE_FILTER,
    "OKX-MAINTENANCE-FILTER",
    "system-status service/environment/product fields",
    "filter readiness-relevant maintenance",
    Filter,
    "live safety",
    ["observe", "demo"],
    ["SAFE-CLOCK-STATUS"],
    "fail-closed maintenance guard",
    ReadinessSafety,
    true,
    true,
    "derive service/product relevance from live plan"
);

capability!(
    AUTH_CREDENTIAL_GETTERS,
    "OKX-AUTH-CREDENTIAL-GETTERS",
    "OkxCredentials getters",
    "read raw credential material",
    Authority,
    "shared authenticated client",
    ["all authenticated callers"],
    [],
    "public bypass surface",
    Remove,
    true,
    false,
    "make private to role-owned wire adapters"
);
capability!(
    AUTH_RAW_SIGNATURE,
    "OKX-AUTH-RAW-SIGNATURE",
    "OkxSigner::signature",
    "sign arbitrary prehash",
    Authority,
    "shared authenticated client",
    ["all authenticated callers"],
    [],
    "public bypass surface",
    Remove,
    true,
    false,
    "make private to role-owned wire adapters"
);
capability!(
    AUTH_SIGN_REQUEST,
    "OKX-AUTH-SIGN-REQUEST",
    "OkxSigner::sign_request / SignedRequest",
    "construct arbitrary authenticated request",
    Authority,
    "shared authenticated client",
    ["all authenticated callers"],
    [],
    "public bypass surface",
    Remove,
    true,
    false,
    "make private to role-owned wire adapters"
);
capability!(
    AUTH_WS_LOGIN,
    "OKX-AUTH-WS-LOGIN",
    "OkxSigner::websocket_login",
    "construct authenticated websocket login",
    Authority,
    "shared authenticated client",
    ["observe", "demo"],
    [],
    "public bypass surface",
    Remove,
    true,
    false,
    "make private to role-owned session factories"
);
capability!(
    AUTH_SIGNER_GETTER,
    "OKX-AUTH-SIGNER-GETTER",
    "OkxRestClient::signer",
    "recover broad signer from REST client",
    Authority,
    "shared authenticated client",
    ["all authenticated callers"],
    [],
    "public bypass surface",
    Remove,
    true,
    false,
    "remove"
);
capability!(
    AUTH_RAW_TRANSPORT,
    "OKX-AUTH-RAW-TRANSPORT",
    "HttpTransport::execute",
    "execute arbitrary signed request",
    Authority,
    "shared authenticated client",
    ["all authenticated callers"],
    [],
    "public bypass surface and test injection",
    Remove,
    true,
    false,
    "make private; inject role fakes instead"
);
capability!(
    AUTH_BROAD_REST_CLIENT,
    "OKX-AUTH-BROAD-REST-CLIENT",
    "OkxRestClient",
    "construct cloneable union of live/emergency/evidence methods",
    Authority,
    "shared authenticated client",
    ["all authenticated callers"],
    [],
    "current composition roots",
    Remove,
    true,
    false,
    "replace with non-interchangeable role clients"
);

pub const OKX_CAPABILITY_REGISTRY: &[OkxCapabilityRegistration] = &[
    REST_PLACE_REGULAR,
    REST_CANCEL_REGULAR,
    REST_CANCEL_BATCH_REGULAR,
    REST_REGULAR_CAA,
    REST_PUBLIC_TIME,
    REST_INDEX_TICKER,
    REST_SYSTEM_STATUS,
    REST_REGULAR_PENDING,
    REST_FILLS,
    REST_ORDER_DETAILS,
    REST_ACCOUNT_INSTRUMENTS,
    REST_ACCOUNT_TRADE_FEE,
    REST_ACCOUNT_CONFIG,
    REST_ACCOUNT_BALANCE,
    REST_ACCOUNT_POSITIONS,
    REST_ACCOUNT_BILLS,
    REST_ALGO_PENDING,
    REST_CANCEL_ALGO,
    REST_SPREAD_PENDING,
    REST_SPREAD_MASS_CANCEL,
    REST_SPREAD_CAA,
    WS_BOOKS,
    WS_BOOKS_L2_TBT,
    WS_BOOKS50_L2_TBT,
    WS_TRADES,
    WS_TRADES_ALL,
    WS_FUNDING_RATE,
    WS_INDEX_TICKERS,
    WS_PRICE_LIMIT,
    WS_MARK_PRICE,
    WS_ORDERS,
    WS_FILLS,
    WS_ACCOUNT,
    WS_POSITIONS,
    WS_SUBSCRIBE,
    WS_LOGIN,
    WS_PLACE_REGULAR,
    WS_CANCEL_REGULAR,
    WS_LIVENESS,
    CONNECTION_PUBLIC,
    CONNECTION_PRIVATE_STATE,
    CONNECTION_ORDER_COMMAND,
    CONNECTION_CAPTURE_PUBLIC,
    CONNECTION_FAULT_PROXY,
    MAINTENANCE_FILTER,
    AUTH_CREDENTIAL_GETTERS,
    AUTH_RAW_SIGNATURE,
    AUTH_SIGN_REQUEST,
    AUTH_WS_LOGIN,
    AUTH_SIGNER_GETTER,
    AUTH_RAW_TRANSPORT,
    AUTH_BROAD_REST_CLIENT,
];

/// Looks up secret-free capability metadata by its stable inventory ID.
pub fn okx_capability_registration(
    capability_id: &str,
) -> Option<&'static OkxCapabilityRegistration> {
    OKX_CAPABILITY_REGISTRY
        .iter()
        .find(|capability| capability.capability_id == capability_id)
}

/// Returns a registered credential-free market-data channel, if supported.
pub fn okx_public_channel_registration(
    channel: &str,
) -> Option<&'static OkxCapabilityRegistration> {
    OKX_CAPABILITY_REGISTRY.iter().find(|capability| {
        capability.endpoint_or_channel == channel
            && capability.access == OkxCapabilityAccess::Read
            && matches!(
                capability.class,
                OkxCapabilityClass::ChaosObservation | OkxCapabilityClass::EvidenceOnly
            )
            && capability
                .modes
                .iter()
                .any(|mode| matches!(*mode, "observe" | "demo" | "capture"))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const INVENTORY: &str = include_str!("../../../../docs/chaos-connectivity-inventory.md");
    const BOUNDARY: &str = include_str!("../../../../docs/chaos-connectivity-boundary.md");

    #[test]
    fn registry_ids_are_unique_and_present_in_the_checked_in_inventory() {
        let mut ids = BTreeSet::new();
        for capability in OKX_CAPABILITY_REGISTRY {
            assert!(
                ids.insert(capability.capability_id),
                "duplicate OKX capability ID {}",
                capability.capability_id
            );
            assert!(
                INVENTORY.contains(&format!("| `{}` |", capability.capability_id)),
                "inventory is missing registry capability {}",
                capability.capability_id
            );
            assert!(!capability.endpoint_or_channel.is_empty());
            assert!(!capability.operation.is_empty());
            assert!(!capability.consumer.is_empty());
        }
        let inventory_ids = INVENTORY
            .lines()
            .filter_map(|line| line.strip_prefix("| `OKX-"))
            .filter_map(|line| line.split_once("` |"))
            .map(|(suffix, _)| format!("OKX-{suffix}"))
            .collect::<BTreeSet<_>>();
        let registry_ids = ids.into_iter().map(str::to_string).collect::<BTreeSet<_>>();
        assert_eq!(registry_ids, inventory_ids);
    }

    #[test]
    fn every_registered_requirement_is_defined_by_the_boundary() {
        for capability in OKX_CAPABILITY_REGISTRY {
            if capability.allowed_in_live_plan {
                assert!(
                    !capability.requirement_ids.is_empty(),
                    "allowed live capability {} has no requirement",
                    capability.capability_id
                );
            }
            for requirement in capability.requirement_ids {
                assert!(
                    BOUNDARY.contains(&format!("`{requirement}`")),
                    "capability {} uses unknown boundary requirement {}",
                    capability.capability_id,
                    requirement
                );
            }
        }
    }

    #[test]
    fn dormant_raw_authority_is_registered_for_removal() {
        let raw = OKX_CAPABILITY_REGISTRY
            .iter()
            .filter(|capability| capability.access == OkxCapabilityAccess::Authority)
            .collect::<Vec<_>>();
        assert_eq!(raw.len(), 7);
        assert!(raw.iter().all(|capability| {
            capability.class == OkxCapabilityClass::Remove
                && !capability.allowed_in_live_plan
                && (capability.target_disposition.contains("private")
                    || capability.target_disposition == "remove"
                    || capability.target_disposition.starts_with("replace"))
        }));
    }
}
