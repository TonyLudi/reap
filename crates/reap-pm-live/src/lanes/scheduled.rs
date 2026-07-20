use reap_pm_core::{PmAccountHandle, PmTokenHandle};
use thiserror::Error;

use super::policy::SaturationAction;

/// Frozen key for scheduled work, distinct from received-event ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmScheduledKey {
    monotonic_deadline_ns: u64,
    action_variant_rank: u8,
    account: PmAccountHandle,
    token: PmTokenHandle,
    side_rank: u8,
    local_action_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmScheduledKeyError {
    #[error("scheduled deadline must be nonzero")]
    ZeroDeadline,
    #[error("scheduled local action sequence must be nonzero")]
    ZeroActionSequence,
}

impl PmScheduledKey {
    pub(super) fn derived(
        monotonic_deadline_ns: u64,
        action: PmScheduledAction,
        local_action_sequence: u64,
    ) -> Result<Self, PmScheduledKeyError> {
        if monotonic_deadline_ns == 0 {
            return Err(PmScheduledKeyError::ZeroDeadline);
        }
        if local_action_sequence == 0 {
            return Err(PmScheduledKeyError::ZeroActionSequence);
        }
        Ok(Self {
            monotonic_deadline_ns,
            action_variant_rank: action.kind().variant_rank(),
            account: action.account(),
            token: action.token(),
            side_rank: action.side().rank(),
            local_action_sequence,
        })
    }

    #[must_use]
    pub const fn monotonic_deadline_ns(self) -> u64 {
        self.monotonic_deadline_ns
    }

    #[must_use]
    pub const fn action_variant_rank(self) -> u8 {
        self.action_variant_rank
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account
    }

    #[must_use]
    pub const fn token(self) -> PmTokenHandle {
        self.token
    }

    #[must_use]
    pub const fn side_rank(self) -> u8 {
        self.side_rank
    }

    #[must_use]
    pub const fn local_action_sequence(self) -> u64 {
        self.local_action_sequence
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmScheduledActionKind {
    CancelOwned,
    ReconciliationRefresh,
    FreshnessCheck,
    QuoteEvaluation,
}

impl PmScheduledActionKind {
    const fn variant_rank(self) -> u8 {
        match self {
            Self::CancelOwned => 0,
            Self::ReconciliationRefresh => 1,
            Self::FreshnessCheck => 2,
            Self::QuoteEvaluation => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmScheduledSide {
    NotApplicable,
    Bid,
    Ask,
}

impl PmScheduledSide {
    const fn rank(self) -> u8 {
        match self {
            Self::NotApplicable => 0,
            Self::Bid => 1,
            Self::Ask => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmScheduledAction {
    kind: PmScheduledActionKind,
    account: PmAccountHandle,
    token: PmTokenHandle,
    side: PmScheduledSide,
}

impl PmScheduledAction {
    #[must_use]
    pub const fn new(
        kind: PmScheduledActionKind,
        account: PmAccountHandle,
        token: PmTokenHandle,
        side: PmScheduledSide,
    ) -> Self {
        Self {
            kind,
            account,
            token,
            side,
        }
    }

    #[must_use]
    pub const fn kind(self) -> PmScheduledActionKind {
        self.kind
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account
    }

    #[must_use]
    pub const fn token(self) -> PmTokenHandle {
        self.token
    }

    #[must_use]
    pub const fn side(self) -> PmScheduledSide {
        self.side
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PmScheduledEnqueueError {
    Key(PmScheduledKeyError),
    Full {
        action: PmScheduledAction,
        saturation: SaturationAction,
    },
    DuplicateKey {
        action: PmScheduledAction,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ServicedScheduledAction {
    key: PmScheduledKey,
    monotonic_service_ns: u64,
    action: PmScheduledAction,
}

impl ServicedScheduledAction {
    pub(super) const fn new(
        key: PmScheduledKey,
        monotonic_service_ns: u64,
        action: PmScheduledAction,
    ) -> Self {
        Self {
            key,
            monotonic_service_ns,
            action,
        }
    }

    #[must_use]
    pub const fn key(&self) -> PmScheduledKey {
        self.key
    }

    #[must_use]
    pub const fn monotonic_service_ns(&self) -> u64 {
        self.monotonic_service_ns
    }

    #[must_use]
    pub const fn lateness_ns(&self) -> u64 {
        self.monotonic_service_ns - self.key.monotonic_deadline_ns()
    }

    #[must_use]
    pub const fn action(&self) -> PmScheduledAction {
        self.action
    }
}
