use super::*;

pub(super) fn reservation_for(
    candidate: PmValidatedQuoteCandidate,
) -> Result<PmExactReservation, PmCoordinatorError> {
    match candidate.side() {
        PmOrderSide::Buy => Ok(PmExactReservation::policy_approved(
            candidate.maker_amount(),
            U256::ZERO,
        )?),
        PmOrderSide::Sell => Ok(PmExactReservation::policy_approved(
            U256::ZERO,
            candidate.maker_amount(),
        )?),
    }
}

pub(super) fn salt_for(
    sequence: u64,
    side: PmOrderSide,
) -> Result<PmOrderSalt, PmCoordinatorError> {
    let side_rank = match side {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    };
    let value = sequence
        .checked_mul(2)
        .and_then(|value| value.checked_add(side_rank))
        .ok_or(PmCoordinatorError::SaltOverflow)?;
    Ok(PmOrderSalt::from_u64(value)?)
}

pub(super) const fn side_index(side: PmOrderSide) -> usize {
    match side {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }
}

pub(super) const fn cancel_reason(reason: PmJournalCancelReasonV1) -> PmCancelIntentReason {
    match reason {
        PmJournalCancelReasonV1::Replacement => PmCancelIntentReason::Replacement,
        PmJournalCancelReasonV1::StaleReference => PmCancelIntentReason::StaleReference,
        PmJournalCancelReasonV1::StaleBook => PmCancelIntentReason::StaleBook,
        PmJournalCancelReasonV1::SafetyHalt => PmCancelIntentReason::SafetyHalt,
    }
}

pub(super) fn push_metric(
    effects: &mut PmProductEffectBatch,
    kind: PmHealthMetricKind,
    value: u64,
) -> Result<(), PmCoordinatorError> {
    effects
        .push(PmProductEffect::HealthMetricAudit(
            PmHealthMetricEffect::new(kind, value),
        ))
        .map_err(PmCoordinatorError::from)
}
