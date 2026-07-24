use super::*;

impl PmOwnedOrderLifecycle {
    #[must_use]
    pub fn new(account_scope: PmAccountScope, instrument: PmInstrumentHandle) -> Self {
        Self {
            account_scope,
            instrument,
            slots: [None; 2],
            entries: Vec::with_capacity(MAX_PM_OWNED_ORDER_HISTORY),
            client_order_index: Vec::with_capacity(MAX_PM_OWNED_ORDER_HISTORY),
            intent_index: Vec::with_capacity(MAX_PM_OWNED_ORDER_HISTORY),
            fills: Vec::with_capacity(MAX_PM_OWNED_FILL_KEYS),
            compacted_intent_high_watermark: None,
            current_epoch: None,
            counters: PmOwnedLifecycleCounters::default(),
        }
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        std::mem::size_of_val(&self.slots)
            .saturating_add(
                self.entries
                    .capacity()
                    .saturating_mul(std::mem::size_of::<OwnedOrderEntry>()),
            )
            .saturating_add(
                self.client_order_index
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u16>()),
            )
            .saturating_add(
                self.intent_index
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u16>()),
            )
            .saturating_add(
                self.fills
                    .capacity()
                    .saturating_mul(std::mem::size_of::<OwnedFillEntry>()),
            )
    }

    pub(crate) fn order_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn fill_key_count(&self) -> usize {
        self.fills.len()
    }

    pub(crate) fn occupied_slot_count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn validate_epoch(&self, epoch: ConnectionEpoch) -> Result<(), PmOwnedOrderLifecycleError> {
        if epoch.value() == 0 || self.current_epoch.is_some_and(|current| epoch <= current) {
            Err(PmOwnedOrderLifecycleError::EpochDidNotAdvance)
        } else {
            Ok(())
        }
    }

    pub fn begin_epoch(
        &mut self,
        epoch: ConnectionEpoch,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        self.validate_epoch(epoch)?;
        self.current_epoch = Some(epoch);
        for entry in &mut self.entries {
            if !entry.is_live() {
                continue;
            }
            entry.reconciliation_required = true;
            if entry.submit == PmOwnedSubmitState::Pending {
                entry.submit = PmOwnedSubmitState::Ambiguous;
            }
        }
        self.counters.reconnects = self.counters.reconnects.saturating_add(1);
        Ok(())
    }

    /// Restores journal-proven occurrence ordering without claiming a live
    /// reconnect or making an order ready.
    pub(crate) fn restore_epoch_for_recovery(
        &mut self,
        epoch: ConnectionEpoch,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        if epoch.value() == 0 || self.current_epoch.is_some_and(|current| epoch <= current) {
            return Err(PmOwnedOrderLifecycleError::EpochDidNotAdvance);
        }
        self.current_epoch = Some(epoch);
        Ok(())
    }

    pub(crate) fn preflight_admit_quote(
        &self,
        intent: PmOwnedQuoteIntent,
    ) -> Result<PmOwnedQuoteAdmission, PmOwnedOrderLifecycleError> {
        self.prepare_admit_quote(intent).map(|plan| plan.outcome())
    }

    pub(crate) fn prepare_admit_quote(
        &self,
        intent: PmOwnedQuoteIntent,
    ) -> Result<PmOwnedQuoteAdmissionPlan, PmOwnedOrderLifecycleError> {
        self.validate_intent_scope(intent)?;
        let client_position = match self.search_client(intent.client_order()) {
            Ok((_, dense_index)) => {
                let entry = self.entries[dense_index];
                return if entry.intent == intent {
                    Ok(PmOwnedQuoteAdmissionPlan::new(
                        PmOwnedQuoteAdmission::DuplicateIntent(intent.client_order()),
                        PmOwnedQuoteAdmissionAction::None,
                    ))
                } else {
                    Err(PmOwnedOrderLifecycleError::ClientIdentityConflict)
                };
            }
            Err(client_position) => client_position,
        };
        let intent_position = match self.search_intent(intent.intent()) {
            Ok(_) => return Err(PmOwnedOrderLifecycleError::IntentIdentityConflict),
            Err(intent_position) => intent_position,
        };
        if self
            .compacted_intent_high_watermark
            .is_some_and(|high_watermark| intent.intent() <= high_watermark)
        {
            return Err(PmOwnedOrderLifecycleError::CompactedIntentIdentity);
        }
        let slot_index = slot_index(intent.slot().side());
        if let Some(current_client) = self.slots[slot_index] {
            let current_index = self
                .find_client(current_client)
                .expect("slot points to retained owned history");
            let current = self.entries[current_index];
            if current.is_live() {
                if same_quote(current.intent, intent) {
                    return Ok(PmOwnedQuoteAdmissionPlan::new(
                        PmOwnedQuoteAdmission::DuplicateQuote(current_client),
                        PmOwnedQuoteAdmissionAction::CountDuplicateQuote,
                    ));
                }
                let outcome = self.preflight_cancel_before_replace(current_index)?;
                let action = if matches!(outcome, PmOwnedQuoteAdmission::CancelBeforeReplace(_)) {
                    PmOwnedQuoteAdmissionAction::MarkCancelBeforeReplace {
                        index: current_index,
                    }
                } else {
                    PmOwnedQuoteAdmissionAction::None
                };
                return Ok(PmOwnedQuoteAdmissionPlan::new(outcome, action));
            }
        }
        if self.entries.len() == MAX_PM_OWNED_ORDER_HISTORY {
            return Err(PmOwnedOrderLifecycleError::OrderCapacity);
        }
        let client = intent.client_order();
        Ok(PmOwnedQuoteAdmissionPlan::new(
            PmOwnedQuoteAdmission::Admitted(client),
            PmOwnedQuoteAdmissionAction::Insert {
                client_position,
                intent_position,
                slot_index,
                entry: OwnedOrderEntry {
                    intent,
                    venue_order: None,
                    submit: PmOwnedSubmitState::Pending,
                    status: None,
                    cumulative_filled: U256::ZERO,
                    known_fill_total: U256::ZERO,
                    cancel: PmOwnedCancelState::None,
                    reconciliation_required: false,
                    compaction_generation: None,
                    last_occurrence: None,
                    last_progress: None,
                },
            },
        ))
    }

    pub(crate) fn commit_preflighted_admit_quote(
        &mut self,
        plan: PmOwnedQuoteAdmissionPlan,
    ) -> PmOwnedQuoteAdmission {
        match plan.action {
            PmOwnedQuoteAdmissionAction::None => {}
            PmOwnedQuoteAdmissionAction::CountDuplicateQuote => {
                self.counters.duplicate_quotes = self.counters.duplicate_quotes.saturating_add(1);
            }
            PmOwnedQuoteAdmissionAction::MarkCancelBeforeReplace { index } => {
                debug_assert!(index < self.entries.len());
                self.entries[index].cancel = PmOwnedCancelState::Pending;
                self.counters.cancel_before_replace =
                    self.counters.cancel_before_replace.saturating_add(1);
            }
            PmOwnedQuoteAdmissionAction::Insert {
                client_position,
                intent_position,
                slot_index,
                entry,
            } => {
                debug_assert!(self.entries.len() < MAX_PM_OWNED_ORDER_HISTORY);
                debug_assert!(slot_index < self.slots.len());
                let client = entry.intent.client_order();
                self.insert_dense_order(client_position, intent_position, entry);
                self.slots[slot_index] = Some(client);
                self.counters.admissions = self.counters.admissions.saturating_add(1);
            }
        }
        plan.outcome
    }

    pub fn admit_quote(
        &mut self,
        intent: PmOwnedQuoteIntent,
    ) -> Result<PmOwnedQuoteAdmission, PmOwnedOrderLifecycleError> {
        self.validate_intent_scope(intent)?;
        let client_position = match self.search_client(intent.client_order()) {
            Ok((_, dense_index)) => {
                let entry = self.entries[dense_index];
                if entry.intent == intent {
                    return Ok(PmOwnedQuoteAdmission::DuplicateIntent(
                        intent.client_order(),
                    ));
                }
                return self.fail(PmOwnedOrderLifecycleError::ClientIdentityConflict);
            }
            Err(client_position) => client_position,
        };
        let intent_position = match self.search_intent(intent.intent()) {
            Ok(_) => return self.fail(PmOwnedOrderLifecycleError::IntentIdentityConflict),
            Err(intent_position) => intent_position,
        };
        if self
            .compacted_intent_high_watermark
            .is_some_and(|high_watermark| intent.intent() <= high_watermark)
        {
            return self.fail(PmOwnedOrderLifecycleError::CompactedIntentIdentity);
        }
        let slot_index = slot_index(intent.slot().side());
        if let Some(current_client) = self.slots[slot_index] {
            let current_index = self
                .find_client(current_client)
                .expect("slot points to retained owned history");
            let current = self.entries[current_index];
            if current.is_live() {
                if same_quote(current.intent, intent) {
                    self.counters.duplicate_quotes =
                        self.counters.duplicate_quotes.saturating_add(1);
                    return Ok(PmOwnedQuoteAdmission::DuplicateQuote(current_client));
                }
                return self.cancel_before_replace(current_index);
            }
        }
        if self.entries.len() == MAX_PM_OWNED_ORDER_HISTORY {
            self.counters.order_capacity_failures =
                self.counters.order_capacity_failures.saturating_add(1);
            return Err(PmOwnedOrderLifecycleError::OrderCapacity);
        }
        let client = intent.client_order();
        self.insert_dense_order(
            client_position,
            intent_position,
            OwnedOrderEntry {
                intent,
                venue_order: None,
                submit: PmOwnedSubmitState::Pending,
                status: None,
                cumulative_filled: U256::ZERO,
                known_fill_total: U256::ZERO,
                cancel: PmOwnedCancelState::None,
                reconciliation_required: false,
                compaction_generation: None,
                last_occurrence: None,
                last_progress: None,
            },
        );
        self.slots[slot_index] = Some(client);
        self.counters.admissions = self.counters.admissions.saturating_add(1);
        Ok(PmOwnedQuoteAdmission::Admitted(client))
    }

    pub fn apply_submit_result(
        &mut self,
        client_order: PmClientOrderKey,
        result: PmOwnedSubmitResult,
    ) -> Result<PmOwnedSubmitApply, PmOwnedOrderLifecycleError> {
        let index = self
            .find_client(client_order)
            .ok_or(PmOwnedOrderLifecycleError::UnknownClientOrder)?;
        let prior = self.entries[index];
        let outcome = match (prior.submit, result) {
            (PmOwnedSubmitState::Pending, PmOwnedSubmitResult::Accepted(venue)) => {
                self.validate_venue_binding(client_order, venue)?;
                let entry = &mut self.entries[index];
                entry.venue_order = Some(venue);
                entry.submit = PmOwnedSubmitState::Accepted;
                entry.status = Some(PmOrderStatus::Open);
                self.counters.submit_accepts = self.counters.submit_accepts.saturating_add(1);
                PmOwnedSubmitApply::Accepted
            }
            (PmOwnedSubmitState::Ambiguous, PmOwnedSubmitResult::Accepted(venue)) => {
                self.validate_venue_binding(client_order, venue)?;
                let entry = &mut self.entries[index];
                entry.venue_order = Some(venue);
                entry.submit = PmOwnedSubmitState::Accepted;
                entry.status = Some(PmOrderStatus::Open);
                entry.reconciliation_required = false;
                self.counters.submit_accepts = self.counters.submit_accepts.saturating_add(1);
                PmOwnedSubmitApply::LateAccepted
            }
            (
                PmOwnedSubmitState::Pending | PmOwnedSubmitState::Ambiguous,
                PmOwnedSubmitResult::Rejected,
            ) => {
                let entry = &mut self.entries[index];
                entry.submit = PmOwnedSubmitState::Rejected;
                entry.status = Some(PmOrderStatus::Rejected);
                entry.reconciliation_required = false;
                self.counters.submit_rejections = self.counters.submit_rejections.saturating_add(1);
                PmOwnedSubmitApply::Rejected
            }
            (PmOwnedSubmitState::Pending, PmOwnedSubmitResult::Ambiguous) => {
                let entry = &mut self.entries[index];
                entry.submit = PmOwnedSubmitState::Ambiguous;
                entry.reconciliation_required = true;
                self.counters.submit_ambiguous = self.counters.submit_ambiguous.saturating_add(1);
                PmOwnedSubmitApply::MarkedAmbiguous
            }
            (PmOwnedSubmitState::Ambiguous, PmOwnedSubmitResult::Ambiguous)
            | (PmOwnedSubmitState::Rejected, PmOwnedSubmitResult::Rejected) => {
                PmOwnedSubmitApply::Duplicate
            }
            (PmOwnedSubmitState::Accepted, PmOwnedSubmitResult::Accepted(venue)) => {
                if prior.venue_order == Some(venue) {
                    PmOwnedSubmitApply::Duplicate
                } else {
                    return self.fail(PmOwnedOrderLifecycleError::VenueBindingConflict);
                }
            }
            (PmOwnedSubmitState::Accepted, _) => {
                return self.fail(PmOwnedOrderLifecycleError::InvalidSubmitTransition);
            }
            (PmOwnedSubmitState::Rejected, _) => {
                return self.fail(PmOwnedOrderLifecycleError::TerminalNonResurrection);
            }
        };
        Ok(outcome)
    }

    pub(crate) fn preflight_submit_result(
        &self,
        client_order: PmClientOrderKey,
        result: PmOwnedSubmitResult,
    ) -> Result<PmOwnedSubmitApply, PmOwnedOrderLifecycleError> {
        let index = self
            .find_client(client_order)
            .ok_or(PmOwnedOrderLifecycleError::UnknownClientOrder)?;
        let prior = self.entries[index];
        match (prior.submit, result) {
            (PmOwnedSubmitState::Pending, PmOwnedSubmitResult::Accepted(venue)) => {
                self.preflight_venue_binding(client_order, venue)?;
                Ok(PmOwnedSubmitApply::Accepted)
            }
            (PmOwnedSubmitState::Ambiguous, PmOwnedSubmitResult::Accepted(venue)) => {
                self.preflight_venue_binding(client_order, venue)?;
                Ok(PmOwnedSubmitApply::LateAccepted)
            }
            (
                PmOwnedSubmitState::Pending | PmOwnedSubmitState::Ambiguous,
                PmOwnedSubmitResult::Rejected,
            ) => Ok(PmOwnedSubmitApply::Rejected),
            (PmOwnedSubmitState::Pending, PmOwnedSubmitResult::Ambiguous) => {
                Ok(PmOwnedSubmitApply::MarkedAmbiguous)
            }
            (PmOwnedSubmitState::Ambiguous, PmOwnedSubmitResult::Ambiguous)
            | (PmOwnedSubmitState::Rejected, PmOwnedSubmitResult::Rejected) => {
                Ok(PmOwnedSubmitApply::Duplicate)
            }
            (PmOwnedSubmitState::Accepted, PmOwnedSubmitResult::Accepted(venue))
                if prior.venue_order == Some(venue) =>
            {
                Ok(PmOwnedSubmitApply::Duplicate)
            }
            (PmOwnedSubmitState::Accepted, PmOwnedSubmitResult::Accepted(_)) => {
                Err(PmOwnedOrderLifecycleError::VenueBindingConflict)
            }
            (PmOwnedSubmitState::Accepted, _) => {
                Err(PmOwnedOrderLifecycleError::InvalidSubmitTransition)
            }
            (PmOwnedSubmitState::Rejected, _) => {
                Err(PmOwnedOrderLifecycleError::TerminalNonResurrection)
            }
        }
    }

    pub fn observe_fill(
        &mut self,
        observation: PmOwnedFillObservation,
    ) -> Result<PmOwnedFillApply, PmOwnedOrderLifecycleError> {
        match self.validate_occurrence(observation.occurrence) {
            Ok(()) => {}
            Err(PmOwnedOrderLifecycleError::OldEpoch) => {
                return Ok(PmOwnedFillApply::IgnoredOldEpoch);
            }
            Err(error) => return Err(error),
        }
        let venue = observation.key.venue_order();
        let order_index = self
            .find_venue(venue)
            .ok_or(PmOwnedOrderLifecycleError::UnboundVenueOrder)?;
        let prior_order = self.entries[order_index];
        if prior_order.submit == PmOwnedSubmitState::Rejected {
            return self.fail(PmOwnedOrderLifecycleError::TerminalNonResurrection);
        }
        let original = prior_order.intent.quantity().protocol_units();
        if observation
            .reported_cumulative
            .is_some_and(|cumulative| cumulative > original)
        {
            return self.fail(PmOwnedOrderLifecycleError::Overfill);
        }
        if observation.reported_cumulative.is_some_and(|cumulative| {
            cumulative < prior_order.cumulative_filled
                && prior_order
                    .last_occurrence
                    .is_none_or(|last| observation.occurrence.causal_cmp(last) != Ordering::Less)
        }) {
            return self.fail(PmOwnedOrderLifecycleError::BackwardsCumulative);
        }
        match self.search_fill(observation.key) {
            Ok(fill_index) => {
                let prior_fill = self.fills[fill_index];
                if prior_fill.client_order != prior_order.intent.client_order()
                    || prior_fill.quantity != observation.quantity
                {
                    return self.fail(PmOwnedOrderLifecycleError::FillConflict);
                }
                let next_cumulative = max_units(
                    prior_order.cumulative_filled,
                    observation
                        .reported_cumulative
                        .unwrap_or(prior_order.cumulative_filled),
                );
                let source_added = prior_fill.sources & observation.source.bit() == 0;
                let cumulative_advanced = next_cumulative > prior_order.cumulative_filled;
                let fill = &mut self.fills[fill_index];
                fill.sources |= observation.source.bit();
                fill.last_occurrence = fill.last_occurrence.causal_max(observation.occurrence);
                self.advance_from_fill(order_index, next_cumulative, observation.occurrence)?;
                self.counters.fill_duplicates = self.counters.fill_duplicates.saturating_add(1);
                let order = self.entries[order_index];
                Ok(PmOwnedFillApply::Duplicate {
                    client_order: order.intent.client_order(),
                    cumulative_filled: order.cumulative_filled,
                    remaining: remaining(order),
                    source_added,
                    cumulative_advanced,
                })
            }
            Err(fill_index) => {
                if self.fills.len() == MAX_PM_OWNED_FILL_KEYS {
                    self.counters.fill_capacity_failures =
                        self.counters.fill_capacity_failures.saturating_add(1);
                    return Err(PmOwnedOrderLifecycleError::FillCapacity);
                }
                let Ok(known_fill_total) = prior_order
                    .known_fill_total
                    .checked_add(observation.quantity.protocol_units())
                else {
                    return self.fail(PmOwnedOrderLifecycleError::ArithmeticOverflow);
                };
                if known_fill_total > original {
                    return self.fail(PmOwnedOrderLifecycleError::Overfill);
                }
                let next_cumulative = max_units(
                    max_units(prior_order.cumulative_filled, known_fill_total),
                    observation.reported_cumulative.unwrap_or(U256::ZERO),
                );
                self.fills.insert(
                    fill_index,
                    OwnedFillEntry {
                        key: observation.key,
                        client_order: prior_order.intent.client_order(),
                        quantity: observation.quantity,
                        sources: observation.source.bit(),
                        first_occurrence: observation.occurrence,
                        last_occurrence: observation.occurrence,
                    },
                );
                self.entries[order_index].known_fill_total = known_fill_total;
                self.advance_from_fill(order_index, next_cumulative, observation.occurrence)?;
                self.counters.fills = self.counters.fills.saturating_add(1);
                let order = self.entries[order_index];
                Ok(PmOwnedFillApply::Applied {
                    client_order: order.intent.client_order(),
                    cumulative_filled: order.cumulative_filled,
                    remaining: remaining(order),
                })
            }
        }
    }

    pub(crate) fn preflight_fill(
        &self,
        observation: PmOwnedFillObservation,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        match self.validate_occurrence(observation.occurrence) {
            Ok(()) | Err(PmOwnedOrderLifecycleError::OldEpoch) => {}
            Err(error) => return Err(error),
        }
        if observation
            .occurrence
            .private_occurrence()
            .is_some_and(|private| {
                self.current_epoch
                    .is_some_and(|epoch| private.epoch() < epoch)
            })
        {
            return Ok(());
        }
        let venue = observation.key.venue_order();
        let order_index = self
            .find_venue(venue)
            .ok_or(PmOwnedOrderLifecycleError::UnboundVenueOrder)?;
        let prior_order = self.entries[order_index];
        if prior_order.submit == PmOwnedSubmitState::Rejected {
            return Err(PmOwnedOrderLifecycleError::TerminalNonResurrection);
        }
        let original = prior_order.intent.quantity().protocol_units();
        if observation
            .reported_cumulative
            .is_some_and(|cumulative| cumulative > original)
        {
            return Err(PmOwnedOrderLifecycleError::Overfill);
        }
        if observation.reported_cumulative.is_some_and(|cumulative| {
            cumulative < prior_order.cumulative_filled
                && prior_order
                    .last_occurrence
                    .is_none_or(|last| observation.occurrence.causal_cmp(last) != Ordering::Less)
        }) {
            return Err(PmOwnedOrderLifecycleError::BackwardsCumulative);
        }
        match self.search_fill(observation.key) {
            Ok(fill_index) => {
                let prior_fill = self.fills[fill_index];
                if prior_fill.client_order != prior_order.intent.client_order()
                    || prior_fill.quantity != observation.quantity
                {
                    return Err(PmOwnedOrderLifecycleError::FillConflict);
                }
            }
            Err(_) => {
                if self.fills.len() == MAX_PM_OWNED_FILL_KEYS {
                    return Err(PmOwnedOrderLifecycleError::FillCapacity);
                }
                let known_fill_total = prior_order
                    .known_fill_total
                    .checked_add(observation.quantity.protocol_units())
                    .map_err(|_| PmOwnedOrderLifecycleError::ArithmeticOverflow)?;
                if known_fill_total > original {
                    return Err(PmOwnedOrderLifecycleError::Overfill);
                }
            }
        }
        Ok(())
    }

    pub fn observe_progress(
        &mut self,
        observation: PmOwnedOrderProgressObservation,
    ) -> Result<PmOwnedProgressApply, PmOwnedOrderLifecycleError> {
        match self.validate_occurrence(observation.occurrence) {
            Ok(()) => {}
            Err(PmOwnedOrderLifecycleError::OldEpoch) => {
                return Ok(PmOwnedProgressApply::IgnoredOutOfOrder);
            }
            Err(error) => return Err(error),
        }
        let index = self
            .find_client(observation.client_order)
            .ok_or(PmOwnedOrderLifecycleError::UnknownClientOrder)?;
        let prior = self.entries[index];
        if prior.venue_order != Some(observation.venue_order) {
            return self.fail(PmOwnedOrderLifecycleError::VenueBindingConflict);
        }
        if observation.progress.original_quantity() != prior.intent.quantity() {
            return self.fail(PmOwnedOrderLifecycleError::ProgressOriginalMismatch);
        }
        if prior.last_occurrence.is_some_and(|occurrence| {
            observation.occurrence.causal_cmp(occurrence) == Ordering::Less
        }) {
            return Ok(PmOwnedProgressApply::IgnoredOutOfOrder);
        }
        if let Some((occurrence, progress)) = prior.last_progress
            && occurrence == observation.occurrence
        {
            if progress == observation.progress {
                return Ok(PmOwnedProgressApply::Duplicate);
            }
            return self.fail(PmOwnedOrderLifecycleError::SameOccurrenceConflict);
        }
        let cumulative = observation.progress.cumulative_filled();
        if cumulative < prior.cumulative_filled || cumulative < prior.known_fill_total {
            return self.fail(PmOwnedOrderLifecycleError::BackwardsCumulative);
        }
        validate_terminal_progress(prior, observation.progress)?;
        let status = observation.progress.status();
        let entry = &mut self.entries[index];
        entry.cumulative_filled = cumulative;
        entry.status = Some(status);
        entry.last_occurrence = Some(observation.occurrence);
        entry.last_progress = Some((observation.occurrence, observation.progress));
        entry.reconciliation_required = cumulative != entry.known_fill_total;
        converge_cancel_with_status(entry);
        let _source = observation.source;
        Ok(PmOwnedProgressApply::Applied {
            status,
            cumulative_filled: cumulative,
            remaining: remaining(*entry),
        })
    }

    pub(crate) fn preflight_progress(
        &self,
        observation: PmOwnedOrderProgressObservation,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        match self.validate_occurrence(observation.occurrence) {
            Ok(()) | Err(PmOwnedOrderLifecycleError::OldEpoch) => {}
            Err(error) => return Err(error),
        }
        if observation
            .occurrence
            .private_occurrence()
            .is_some_and(|private| {
                self.current_epoch
                    .is_some_and(|epoch| private.epoch() < epoch)
            })
        {
            return Ok(());
        }
        let index = self
            .find_client(observation.client_order)
            .ok_or(PmOwnedOrderLifecycleError::UnknownClientOrder)?;
        let prior = self.entries[index];
        if prior.venue_order != Some(observation.venue_order) {
            return Err(PmOwnedOrderLifecycleError::VenueBindingConflict);
        }
        if observation.progress.original_quantity() != prior.intent.quantity() {
            return Err(PmOwnedOrderLifecycleError::ProgressOriginalMismatch);
        }
        if prior.last_occurrence.is_some_and(|occurrence| {
            observation.occurrence.causal_cmp(occurrence) == Ordering::Less
        }) {
            return Ok(());
        }
        if let Some((occurrence, progress)) = prior.last_progress
            && occurrence == observation.occurrence
        {
            return if progress == observation.progress {
                Ok(())
            } else {
                Err(PmOwnedOrderLifecycleError::SameOccurrenceConflict)
            };
        }
        let cumulative = observation.progress.cumulative_filled();
        if cumulative < prior.cumulative_filled || cumulative < prior.known_fill_total {
            return Err(PmOwnedOrderLifecycleError::BackwardsCumulative);
        }
        validate_terminal_progress(prior, observation.progress)
    }

    pub fn observe_remote_order(
        &mut self,
        identity: PmOrderIdentity,
    ) -> Result<PmOwnedRemoteOrderApply, PmOwnedOrderLifecycleError> {
        let matched = self.match_remote_order(identity);
        if matched == PmOwnedRemoteOrderApply::AmbiguousRemote
            && let Some(client) = identity.client_order_key()
            && let Some(index) = self.find_client(client)
        {
            self.entries[index].reconciliation_required = true;
        }
        Ok(matched)
    }

    pub(crate) fn match_remote_order(&self, identity: PmOrderIdentity) -> PmOwnedRemoteOrderApply {
        match (identity.client_order_key(), identity.venue_order_key()) {
            (Some(client), Some(venue)) => {
                let Some(index) = self.find_client(client) else {
                    return PmOwnedRemoteOrderApply::AmbiguousRemote;
                };
                if self.entries[index].venue_order == Some(venue) {
                    PmOwnedRemoteOrderApply::Matched(client)
                } else {
                    PmOwnedRemoteOrderApply::AmbiguousRemote
                }
            }
            (None, Some(venue)) => self
                .find_venue(venue)
                .map_or(PmOwnedRemoteOrderApply::AmbiguousRemote, |index| {
                    PmOwnedRemoteOrderApply::Matched(self.entries[index].intent.client_order())
                }),
            (Some(client), None) => self
                .find_client(client)
                .map_or(PmOwnedRemoteOrderApply::AmbiguousRemote, |_| {
                    PmOwnedRemoteOrderApply::Matched(client)
                }),
            (None, None) => unreachable!("core order identity is nonempty"),
        }
    }

    pub fn observe_detail_absence(
        &mut self,
        venue_order: PmVenueOrderKey,
        occurrence: PmOwnedObservationOccurrence,
    ) -> Result<PmOwnedDetailAbsenceApply, PmOwnedOrderLifecycleError> {
        match self.validate_occurrence(occurrence) {
            Ok(()) => {}
            Err(PmOwnedOrderLifecycleError::OldEpoch) => {
                return Ok(PmOwnedDetailAbsenceApply::IgnoredOutOfOrder);
            }
            Err(error) => return Err(error),
        }
        let Some(index) = self.find_venue(venue_order) else {
            return Ok(PmOwnedDetailAbsenceApply::Unmatched);
        };
        let prior = self.entries[index];
        if prior
            .last_occurrence
            .is_some_and(|last| occurrence.causal_cmp(last) == Ordering::Less)
        {
            return Ok(PmOwnedDetailAbsenceApply::IgnoredOutOfOrder);
        }
        if prior.submit != PmOwnedSubmitState::Accepted
            || prior.cancel != PmOwnedCancelState::Accepted
            || prior.status != Some(PmOrderStatus::Cancelled)
            || !prior.cumulative_filled.is_zero()
            || !prior.known_fill_total.is_zero()
        {
            return Ok(PmOwnedDetailAbsenceApply::Unsafe);
        }
        let entry = &mut self.entries[index];
        entry.reconciliation_required = false;
        entry.last_occurrence = Some(occurrence);
        Ok(PmOwnedDetailAbsenceApply::SettledAcceptedCancel(
            prior.intent.client_order(),
        ))
    }

    pub(crate) fn preflight_detail_absence(
        &self,
        venue_order: PmVenueOrderKey,
        occurrence: PmOwnedObservationOccurrence,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        match self.validate_occurrence(occurrence) {
            Ok(()) | Err(PmOwnedOrderLifecycleError::OldEpoch) => {}
            Err(error) => return Err(error),
        }
        let Some(index) = self.find_venue(venue_order) else {
            return Ok(());
        };
        let prior = self.entries[index];
        if prior
            .last_occurrence
            .is_some_and(|last| occurrence.causal_cmp(last) == Ordering::Less)
        {
            return Ok(());
        }
        Ok(())
    }

    pub fn request_cancel(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<PmOwnedCancelRequestApply, PmOwnedOrderLifecycleError> {
        let index = self
            .find_client(client_order)
            .ok_or(PmOwnedOrderLifecycleError::UnknownClientOrder)?;
        let entry = self.entries[index];
        if entry.is_terminal() {
            return Ok(PmOwnedCancelRequestApply::AlreadyTerminal);
        }
        if entry.submit != PmOwnedSubmitState::Accepted {
            return self.fail(PmOwnedOrderLifecycleError::CancelUnavailable);
        }
        let venue_order = entry
            .venue_order
            .ok_or(PmOwnedOrderLifecycleError::CancelUnavailable)?;
        let intent = PmOwnedCancelIntent {
            client_order,
            venue_order,
        };
        match entry.cancel {
            PmOwnedCancelState::None | PmOwnedCancelState::Rejected => {
                self.entries[index].cancel = PmOwnedCancelState::Pending;
                self.counters.cancel_requests = self.counters.cancel_requests.saturating_add(1);
                Ok(PmOwnedCancelRequestApply::Issued(intent))
            }
            PmOwnedCancelState::Pending => Ok(PmOwnedCancelRequestApply::Duplicate(intent)),
            PmOwnedCancelState::Ambiguous => {
                self.fail(PmOwnedOrderLifecycleError::CancelUnavailable)
            }
            PmOwnedCancelState::Accepted | PmOwnedCancelState::FilledRace => {
                Ok(PmOwnedCancelRequestApply::AlreadyTerminal)
            }
        }
    }

    pub fn apply_cancel_result(
        &mut self,
        intent: PmOwnedCancelIntent,
        outcome: PmOwnedCancelOutcome,
    ) -> Result<PmOwnedCancelApply, PmOwnedOrderLifecycleError> {
        let index = self
            .find_client(intent.client_order)
            .ok_or(PmOwnedOrderLifecycleError::UnknownClientOrder)?;
        let prior = self.entries[index];
        if prior.venue_order != Some(intent.venue_order) {
            return self.fail(PmOwnedOrderLifecycleError::VenueBindingConflict);
        }
        if prior.status == Some(PmOrderStatus::Filled)
            && prior.cancel == PmOwnedCancelState::FilledRace
        {
            self.counters.cancel_results = self.counters.cancel_results.saturating_add(1);
            return Ok(PmOwnedCancelApply::ConvergedFilled);
        }
        if !matches!(
            prior.cancel,
            PmOwnedCancelState::Pending | PmOwnedCancelState::Ambiguous
        ) {
            if prior.is_terminal() {
                return Ok(PmOwnedCancelApply::Duplicate);
            }
            return self.fail(PmOwnedOrderLifecycleError::CancelResultWithoutIntent);
        }
        let result = match outcome {
            PmOwnedCancelOutcome::Accepted => {
                if prior.status == Some(PmOrderStatus::Filled) {
                    self.entries[index].cancel = PmOwnedCancelState::FilledRace;
                    PmOwnedCancelApply::ConvergedFilled
                } else {
                    self.entries[index].status = Some(PmOrderStatus::Cancelled);
                    self.entries[index].cancel = PmOwnedCancelState::Accepted;
                    self.entries[index].reconciliation_required = true;
                    PmOwnedCancelApply::Cancelled
                }
            }
            PmOwnedCancelOutcome::Rejected => {
                if prior.status == Some(PmOrderStatus::Filled) {
                    self.entries[index].cancel = PmOwnedCancelState::FilledRace;
                    PmOwnedCancelApply::ConvergedFilled
                } else {
                    self.entries[index].cancel = PmOwnedCancelState::Rejected;
                    PmOwnedCancelApply::Rejected
                }
            }
            PmOwnedCancelOutcome::AlreadyFilled => {
                self.entries[index].cumulative_filled =
                    self.entries[index].intent.quantity().protocol_units();
                self.entries[index].status = Some(PmOrderStatus::Filled);
                self.entries[index].cancel = PmOwnedCancelState::FilledRace;
                self.entries[index].reconciliation_required =
                    self.entries[index].known_fill_total != self.entries[index].cumulative_filled;
                PmOwnedCancelApply::Filled
            }
            PmOwnedCancelOutcome::Ambiguous => {
                if prior.status == Some(PmOrderStatus::Filled) {
                    self.entries[index].cancel = PmOwnedCancelState::FilledRace;
                    PmOwnedCancelApply::ConvergedFilled
                } else {
                    self.entries[index].cancel = PmOwnedCancelState::Ambiguous;
                    self.entries[index].reconciliation_required = true;
                    PmOwnedCancelApply::MarkedAmbiguous
                }
            }
        };
        self.counters.cancel_results = self.counters.cancel_results.saturating_add(1);
        Ok(result)
    }

    pub fn compact_proven_terminal(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<PmOwnedTerminalCompaction, PmOwnedOrderLifecycleError> {
        let (canonical_position, index) = self
            .search_client(client_order)
            .map_err(|_| PmOwnedOrderLifecycleError::UnknownClientOrder)?;
        let (intent_position, intent_index) = self
            .search_intent(self.entries[index].intent.intent())
            .expect("retained owned order remains indexed by intent");
        debug_assert_eq!(intent_index, index);
        let entry = self.entries[index];
        if !entry.is_terminal()
            || entry.reconciliation_required
            || matches!(
                entry.submit,
                PmOwnedSubmitState::Pending | PmOwnedSubmitState::Ambiguous
            )
            || matches!(
                entry.cancel,
                PmOwnedCancelState::Pending | PmOwnedCancelState::Ambiguous
            )
        {
            return self.fail(PmOwnedOrderLifecycleError::TerminalCompactionUnavailable);
        }

        let slot = slot_index(entry.intent.slot().side());
        if self.slots[slot] == Some(client_order) {
            self.slots[slot] = None;
        }
        let removed = self.swap_remove_dense_order(canonical_position, intent_position, index);
        debug_assert_eq!(removed, entry);
        let prior_fill_count = self.fills.len();
        self.fills.retain(|fill| fill.client_order != client_order);
        let fill_keys_removed = prior_fill_count - self.fills.len();
        self.compacted_intent_high_watermark = Some(
            self.compacted_intent_high_watermark
                .map_or(entry.intent.intent(), |prior| {
                    prior.max(entry.intent.intent())
                }),
        );
        self.counters.terminal_compactions = self.counters.terminal_compactions.saturating_add(1);
        Ok(PmOwnedTerminalCompaction {
            client_order,
            intent: entry.intent.intent(),
            fill_keys_removed,
        })
    }

    pub(crate) fn mark_compactable_through(
        &mut self,
        generation: u64,
        through: PmPrivateOccurrence,
    ) -> usize {
        let mut marked = 0;
        for entry in &mut self.entries {
            if entry_compactable_through(*entry, through) {
                entry.compaction_generation = Some(generation);
                marked += 1;
            }
        }
        marked
    }

    pub(crate) fn marked_compaction_clients(
        &self,
        generation: u64,
    ) -> impl Iterator<Item = PmClientOrderKey> + '_ {
        self.client_order_index
            .iter()
            .filter_map(move |dense_index| {
                let entry = self.entries[usize::from(*dense_index)];
                (entry.compaction_generation == Some(generation))
                    .then_some(entry.intent.client_order())
            })
    }

    pub(crate) fn first_marked_compaction_client(
        &self,
        generation: u64,
    ) -> Option<PmClientOrderKey> {
        self.marked_compaction_clients(generation).next()
    }

    pub(crate) fn clear_compaction_marks(&mut self, generation: u64) {
        for entry in &mut self.entries {
            if entry.compaction_generation == Some(generation) {
                entry.compaction_generation = None;
            }
        }
    }

    pub(crate) fn clear_invalid_compaction_marks(
        &mut self,
        generation: u64,
        through: PmPrivateOccurrence,
    ) {
        for entry in &mut self.entries {
            if entry.compaction_generation == Some(generation)
                && !entry_compactable_through(*entry, through)
            {
                entry.compaction_generation = None;
            }
        }
    }

    pub(crate) fn preflight_compact_proven_terminal(
        &self,
        client_order: PmClientOrderKey,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        let index = self
            .find_client(client_order)
            .ok_or(PmOwnedOrderLifecycleError::UnknownClientOrder)?;
        let entry = self.entries[index];
        if !entry.is_terminal()
            || entry.reconciliation_required
            || matches!(
                entry.submit,
                PmOwnedSubmitState::Pending | PmOwnedSubmitState::Ambiguous
            )
            || matches!(
                entry.cancel,
                PmOwnedCancelState::Pending | PmOwnedCancelState::Ambiguous
            )
        {
            Err(PmOwnedOrderLifecycleError::TerminalCompactionUnavailable)
        } else {
            Ok(())
        }
    }

    pub fn orders(&self) -> impl Iterator<Item = PmOwnedOrderProjection> + '_ {
        self.client_order_index
            .iter()
            .map(|dense_index| self.entries[usize::from(*dense_index)].projection())
    }

    #[must_use]
    pub fn order(&self, client_order: PmClientOrderKey) -> Option<PmOwnedOrderProjection> {
        self.find_client(client_order)
            .map(|index| self.entries[index].projection())
    }

    pub fn fills(&self) -> impl Iterator<Item = PmOwnedFillProjection> + '_ {
        self.fills.iter().copied().map(OwnedFillEntry::projection)
    }

    pub fn slots(&self) -> impl Iterator<Item = PmOwnedQuoteSlotProjection> + '_ {
        [PmOrderSide::Buy, PmOrderSide::Sell]
            .into_iter()
            .enumerate()
            .map(|(index, side)| PmOwnedQuoteSlotProjection {
                key: PmOwnedQuoteSlotKey::new(self.account_scope, self.instrument, side),
                current: self.slots[index],
            })
    }

    #[must_use]
    pub const fn counters(&self) -> PmOwnedLifecycleCounters {
        self.counters
    }

    fn validate_intent_scope(
        &self,
        intent: PmOwnedQuoteIntent,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        if intent.slot().account_scope() != self.account_scope
            || intent.slot().instrument() != self.instrument
            || intent.client_order().account() != self.account_scope.handle()
        {
            Err(PmOwnedOrderLifecycleError::ScopeMismatch)
        } else {
            Ok(())
        }
    }

    fn validate_venue_binding(
        &mut self,
        client: PmClientOrderKey,
        venue: PmVenueOrderKey,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        if self.preflight_venue_binding(client, venue).is_err() {
            self.fail(PmOwnedOrderLifecycleError::VenueBindingConflict)
        } else {
            Ok(())
        }
    }

    fn preflight_venue_binding(
        &self,
        client: PmClientOrderKey,
        venue: PmVenueOrderKey,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        if venue.account() != self.account_scope.handle()
            || self.entries.iter().any(|entry| {
                entry.venue_order == Some(venue) && entry.intent.client_order() != client
            })
        {
            Err(PmOwnedOrderLifecycleError::VenueBindingConflict)
        } else {
            Ok(())
        }
    }

    fn validate_occurrence(
        &self,
        occurrence: PmOwnedObservationOccurrence,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        let Some(private) = occurrence.private_occurrence() else {
            return Ok(());
        };
        match self.current_epoch {
            None => Err(PmOwnedOrderLifecycleError::MissingEpoch),
            Some(epoch) if private.epoch() < epoch => Err(PmOwnedOrderLifecycleError::OldEpoch),
            Some(epoch) if private.epoch() > epoch => {
                Err(PmOwnedOrderLifecycleError::EpochDidNotAdvance)
            }
            Some(_) => Ok(()),
        }
    }

    fn cancel_before_replace(
        &mut self,
        index: usize,
    ) -> Result<PmOwnedQuoteAdmission, PmOwnedOrderLifecycleError> {
        let entry = self.entries[index];
        match (entry.submit, entry.cancel, entry.venue_order) {
            (PmOwnedSubmitState::Pending, _, _) => Ok(PmOwnedQuoteAdmission::ReplacementBlocked {
                current: entry.intent.client_order(),
                reason: PmOwnedReplacementBlock::SubmitPending,
            }),
            (PmOwnedSubmitState::Ambiguous, _, _) => {
                Ok(PmOwnedQuoteAdmission::ReplacementBlocked {
                    current: entry.intent.client_order(),
                    reason: PmOwnedReplacementBlock::SubmitAmbiguous,
                })
            }
            (PmOwnedSubmitState::Accepted, PmOwnedCancelState::Ambiguous, _) => {
                Ok(PmOwnedQuoteAdmission::ReplacementBlocked {
                    current: entry.intent.client_order(),
                    reason: PmOwnedReplacementBlock::CancelAmbiguous,
                })
            }
            (
                PmOwnedSubmitState::Accepted,
                PmOwnedCancelState::None
                | PmOwnedCancelState::Rejected
                | PmOwnedCancelState::Pending,
                Some(venue_order),
            ) => {
                self.entries[index].cancel = PmOwnedCancelState::Pending;
                self.counters.cancel_before_replace =
                    self.counters.cancel_before_replace.saturating_add(1);
                Ok(PmOwnedQuoteAdmission::CancelBeforeReplace(
                    PmOwnedCancelIntent {
                        client_order: entry.intent.client_order(),
                        venue_order,
                    },
                ))
            }
            _ => self.fail(PmOwnedOrderLifecycleError::CancelUnavailable),
        }
    }

    fn preflight_cancel_before_replace(
        &self,
        index: usize,
    ) -> Result<PmOwnedQuoteAdmission, PmOwnedOrderLifecycleError> {
        let entry = self.entries[index];
        match (entry.submit, entry.cancel, entry.venue_order) {
            (PmOwnedSubmitState::Pending, _, _) => Ok(PmOwnedQuoteAdmission::ReplacementBlocked {
                current: entry.intent.client_order(),
                reason: PmOwnedReplacementBlock::SubmitPending,
            }),
            (PmOwnedSubmitState::Ambiguous, _, _) => {
                Ok(PmOwnedQuoteAdmission::ReplacementBlocked {
                    current: entry.intent.client_order(),
                    reason: PmOwnedReplacementBlock::SubmitAmbiguous,
                })
            }
            (PmOwnedSubmitState::Accepted, PmOwnedCancelState::Ambiguous, _) => {
                Ok(PmOwnedQuoteAdmission::ReplacementBlocked {
                    current: entry.intent.client_order(),
                    reason: PmOwnedReplacementBlock::CancelAmbiguous,
                })
            }
            (
                PmOwnedSubmitState::Accepted,
                PmOwnedCancelState::None
                | PmOwnedCancelState::Rejected
                | PmOwnedCancelState::Pending,
                Some(venue_order),
            ) => Ok(PmOwnedQuoteAdmission::CancelBeforeReplace(
                PmOwnedCancelIntent {
                    client_order: entry.intent.client_order(),
                    venue_order,
                },
            )),
            _ => Err(PmOwnedOrderLifecycleError::CancelUnavailable),
        }
    }

    fn advance_from_fill(
        &mut self,
        order_index: usize,
        cumulative: U256,
        occurrence: PmOwnedObservationOccurrence,
    ) -> Result<(), PmOwnedOrderLifecycleError> {
        let original = self.entries[order_index].intent.quantity().protocol_units();
        if cumulative > original {
            return self.fail(PmOwnedOrderLifecycleError::Overfill);
        }
        let entry = &mut self.entries[order_index];
        entry.cumulative_filled = cumulative;
        entry.last_occurrence = Some(
            entry
                .last_occurrence
                .map_or(occurrence, |prior| prior.causal_max(occurrence)),
        );
        if cumulative == original {
            if entry.status != Some(PmOrderStatus::Expired) {
                entry.status = Some(PmOrderStatus::Filled);
            }
            if entry.known_fill_total == cumulative {
                entry.reconciliation_required = false;
            }
            if matches!(
                entry.cancel,
                PmOwnedCancelState::Pending
                    | PmOwnedCancelState::Ambiguous
                    | PmOwnedCancelState::Accepted
            ) {
                entry.cancel = PmOwnedCancelState::FilledRace;
            }
        } else if !cumulative.is_zero() && !entry.status.is_some_and(PmOrderStatus::is_terminal) {
            entry.status = Some(PmOrderStatus::PartiallyFilled);
        }
        if entry.known_fill_total == cumulative
            && (cumulative == original || entry.last_progress.is_some())
        {
            entry.reconciliation_required = false;
        }
        Ok(())
    }

    fn find_venue(&self, venue: PmVenueOrderKey) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.venue_order == Some(venue))
    }

    fn search_fill(&self, key: PmFillKey) -> Result<usize, usize> {
        self.fills.binary_search_by_key(&key, |entry| entry.key)
    }

    fn fail<T>(
        &mut self,
        error: PmOwnedOrderLifecycleError,
    ) -> Result<T, PmOwnedOrderLifecycleError> {
        self.counters.contract_violations = self.counters.contract_violations.saturating_add(1);
        Err(error)
    }
}

fn entry_compactable_through(entry: OwnedOrderEntry, through: PmPrivateOccurrence) -> bool {
    let observed_after_cut = entry
        .last_occurrence
        .and_then(PmOwnedObservationOccurrence::private_occurrence)
        .is_some_and(|occurrence| occurrence > through);
    entry.is_terminal()
        && !entry.reconciliation_required
        && !observed_after_cut
        && !matches!(
            entry.submit,
            PmOwnedSubmitState::Pending | PmOwnedSubmitState::Ambiguous
        )
        && !matches!(
            entry.cancel,
            PmOwnedCancelState::Pending | PmOwnedCancelState::Ambiguous
        )
}

fn same_quote(left: PmOwnedQuoteIntent, right: PmOwnedQuoteIntent) -> bool {
    left.slot() == right.slot()
        && left.price() == right.price()
        && left.quantity() == right.quantity()
        && left.reservation() == right.reservation()
}

const fn slot_index(side: PmOrderSide) -> usize {
    match side {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }
}

fn remaining(entry: OwnedOrderEntry) -> U256 {
    entry
        .intent
        .quantity()
        .protocol_units()
        .checked_sub(entry.cumulative_filled)
        .expect("owned cumulative is checked")
}

fn max_units(left: U256, right: U256) -> U256 {
    match left.cmp(&right) {
        Ordering::Less => right,
        Ordering::Equal | Ordering::Greater => left,
    }
}

fn validate_terminal_progress(
    prior: OwnedOrderEntry,
    incoming: PmOrderProgress,
) -> Result<(), PmOwnedOrderLifecycleError> {
    let Some(status) = prior.status else {
        return Ok(());
    };
    if !status.is_terminal() {
        if matches!(
            incoming.status(),
            PmOrderStatus::Pending | PmOrderStatus::Rejected
        ) {
            return Err(PmOwnedOrderLifecycleError::TerminalNonResurrection);
        }
        return Ok(());
    }
    match (status, incoming.status()) {
        (PmOrderStatus::Filled, PmOrderStatus::Filled)
        | (PmOrderStatus::Rejected, PmOrderStatus::Rejected)
        | (PmOrderStatus::Expired, PmOrderStatus::Expired) => Ok(()),
        (PmOrderStatus::Cancelled, PmOrderStatus::Cancelled | PmOrderStatus::Filled) => Ok(()),
        _ => Err(PmOwnedOrderLifecycleError::TerminalNonResurrection),
    }
}

fn converge_cancel_with_status(entry: &mut OwnedOrderEntry) {
    match entry.status {
        Some(PmOrderStatus::Filled)
            if matches!(
                entry.cancel,
                PmOwnedCancelState::Pending
                    | PmOwnedCancelState::Ambiguous
                    | PmOwnedCancelState::Accepted
            ) =>
        {
            entry.cancel = PmOwnedCancelState::FilledRace;
        }
        Some(PmOrderStatus::Cancelled)
            if matches!(
                entry.cancel,
                PmOwnedCancelState::Pending | PmOwnedCancelState::Ambiguous
            ) =>
        {
            entry.cancel = PmOwnedCancelState::Accepted;
        }
        _ => {}
    }
}
