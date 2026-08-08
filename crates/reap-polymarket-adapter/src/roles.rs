//! Backend-neutral names for the existing owner-bound observation roles.
//!
//! The historical `PmFixture*` names remain stable compatibility aliases for
//! Goal F. Live composition can use these names without selecting a runtime
//! backend or acquiring any additional authority.

pub type PmReadOwnerGrant = crate::PmFixtureReadOwnerGrant;
pub type PmPrivateRoleGrant = crate::PmFixturePrivateRoleGrant;
pub type PmReconciliationRoleGrant = crate::PmFixtureReconciliationRoleGrant;
pub type PmAccountRoleGrant = crate::PmFixtureAccountRoleGrant;

pub type PmInstrumentScope = crate::PmFixtureInstrumentScope;
pub type PmDeliveryScope = crate::PmFixtureDeliveryScope;
pub type PmCompletionOccurrence = crate::PmFixtureCompletionOccurrence;
pub type PmAggregateDelivery<P> = crate::PmFixtureAggregateDelivery<P>;
pub type PmServicedAggregate<P> = crate::PmFixtureServicedAggregate<P>;

pub type PmPrivateLifecycle = crate::PmFixturePrivateLifecycle;
pub type PmPrivateBatch = crate::PmFixturePrivateBatch;
pub type PmPrivateDelivery = crate::PmFixturePrivateDelivery;
pub type PmFeeEvidence = crate::PmFixtureFeeEvidence;

pub type PmReconciliation = crate::PmFixtureReconciliation;
pub type PmOpenOrdersRequest = crate::PmFixtureOpenOrdersRequest;
pub type PmOrderDetailRequest = crate::PmFixtureOrderDetailRequest;
pub type PmFillQueryRequest = crate::PmFixtureFillQueryRequest;

pub type PmAccountPositionSnapshot = crate::PmFixtureAccountPositionSnapshot;
pub type PmAccountSnapshotRequest = crate::PmFixtureAccountSnapshotRequest;
