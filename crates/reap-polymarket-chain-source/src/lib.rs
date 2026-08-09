#![forbid(unsafe_code)]

mod contract;
mod rpc;
mod source;

pub use contract::{
    PM_POLYGON_CHAIN_ID, PM_POLYGON_CONDITIONAL_TOKENS_ADDRESS,
    PM_POLYGON_NEGATIVE_RISK_V2_EXCHANGE_ADDRESS, PM_POLYGON_PUSD_PROXY_ADDRESS,
    PM_POLYGON_RPC_ORIGIN, PM_POLYGON_STANDARD_V2_EXCHANGE_ADDRESS, PmPolygonAuthorizationScope,
    PmPolygonExchangeSpender, PmPolygonFinalizedAuthorizationCommitment,
    PmPolygonFinalizedAuthorizationCut, PmPolygonFinalizedBlock, PmPolygonSystemClockObservation,
};
pub use source::{
    PmPolygonAuthorizationSource, PmPolygonChainSourceError,
    PmProductionPolygonFinalizedAuthorizationCut, PmProductionPolygonFinalizedAuthorizationError,
};
