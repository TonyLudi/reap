use std::collections::BTreeMap;

use reap_polymarket_auth::CredentialOwnedUserFrame;
use reap_polymarket_wire::PmLiveUserEvent;

use crate::live_diagnostics::ForeignDiagnosticsBuilder;
use crate::live_normalization::{
    LiveNormalizationScope, PmLiveNormalizationError, fill_event_from_leg, normalize_trade,
    normalize_user_order,
};
use crate::{
    MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS, PmFixturePrivateBatch, PmFixturePrivateDelivery,
    PmFixturePrivateLifecycle, PmForeignRowDiagnostics, PmPrivateLifecycleObservation,
};

const PRIVATE_FOREIGN_DOMAIN: &[u8] = b"reap.pm.live.private-foreign.v1\0";

/// Owner-bound live private completion plus explicit foreign-row evidence.
pub struct PmLivePrivateCompletion {
    delivery: PmFixturePrivateDelivery,
    foreign_diagnostics: PmForeignRowDiagnostics,
}

impl PmLivePrivateCompletion {
    pub(crate) const fn new(
        delivery: PmFixturePrivateDelivery,
        foreign_diagnostics: PmForeignRowDiagnostics,
    ) -> Self {
        Self {
            delivery,
            foreign_diagnostics,
        }
    }

    #[must_use]
    pub const fn foreign_diagnostics(&self) -> PmForeignRowDiagnostics {
        self.foreign_diagnostics
    }

    #[must_use]
    pub fn into_delivery(self) -> PmFixturePrivateDelivery {
        self.delivery
    }

    #[must_use]
    pub fn into_parts(self) -> (PmFixturePrivateDelivery, PmForeignRowDiagnostics) {
        (self.delivery, self.foreign_diagnostics)
    }
}

pub(crate) fn normalize_live_user_frame(
    role: &PmFixturePrivateLifecycle,
    frame: &CredentialOwnedUserFrame,
) -> Result<PmFixturePrivateBatch, PmLiveNormalizationError> {
    let scope = LiveNormalizationScope {
        account: role.account_scope(),
        instrument: role.instrument_scope(),
        source: role.source(),
    };
    let mut observations = Vec::with_capacity(frame.events().len());
    let mut diagnostics = ForeignDiagnosticsBuilder::new(
        PRIVATE_FOREIGN_DOMAIN,
        MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS,
    );
    let mut seen_trade_legs = BTreeMap::new();

    for event in frame.events() {
        match event {
            PmLiveUserEvent::Order(order) => {
                let row = normalize_user_order(scope, order)?;
                match row.configured {
                    Some(order) => push_observation(
                        &mut observations,
                        PmPrivateLifecycleObservation::Order(order),
                    )?,
                    None => diagnostics.push(row.key_digest, row.facts_digest)?,
                }
            }
            PmLiveUserEvent::Trade(trade) => {
                let normalized = normalize_trade(scope, trade)?;
                for candidate in normalized.candidates {
                    match seen_trade_legs.get(&candidate.key_digest) {
                        Some(facts) if *facts != candidate.facts_digest => {
                            return Err(PmLiveNormalizationError::ForeignDiagnostic(
                                crate::PmForeignDiagnosticError::ConflictingRow,
                            ));
                        }
                        Some(_) => continue,
                        None => {
                            if seen_trade_legs.len() == MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS {
                                return Err(PmLiveNormalizationError::TooManyRows);
                            }
                            seen_trade_legs.insert(candidate.key_digest, candidate.facts_digest);
                        }
                    }
                    match candidate.leg {
                        Some(leg) if scope.is_configured(leg.condition(), leg.token()) => {
                            push_observation(
                                &mut observations,
                                PmPrivateLifecycleObservation::Fill(fill_event_from_leg(
                                    scope, leg,
                                )?),
                            )?;
                        }
                        Some(_) | None => {
                            diagnostics.push(candidate.key_digest, candidate.facts_digest)?;
                        }
                    }
                }
                if normalized.relevant_to_configured
                    && let Some(reason) = normalized.unresolved
                {
                    push_observation(
                        &mut observations,
                        PmPrivateLifecycleObservation::UnresolvedTrade(role.live_unresolved_trade(
                            trade.id(),
                            reason,
                            normalized.settlement,
                        )),
                    )?;
                } else if normalized.unresolved.is_some()
                    && let Some((key, facts)) = normalized.unresolved_diagnostic
                {
                    diagnostics.push(key, facts)?;
                }
            }
        }
    }

    Ok(PmFixturePrivateBatch::from_live(
        role.account_scope(),
        role.source(),
        role.instrument_scope(),
        observations.into_boxed_slice(),
        diagnostics.finish()?,
    ))
}

fn push_observation(
    observations: &mut Vec<PmPrivateLifecycleObservation>,
    observation: PmPrivateLifecycleObservation,
) -> Result<(), PmLiveNormalizationError> {
    if observations.len() == MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS {
        return Err(PmLiveNormalizationError::ForeignDiagnostic(
            crate::PmForeignDiagnosticError::TooManyRows,
        ));
    }
    observations.push(observation);
    Ok(())
}
