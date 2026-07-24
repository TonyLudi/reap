use super::*;

impl PmOwnedOrderLifecycle {
    pub(super) fn search_client(&self, client: PmClientOrderKey) -> Result<(usize, usize), usize> {
        self.client_order_index
            .binary_search_by_key(&client, |dense_index| {
                self.entries[usize::from(*dense_index)]
                    .intent
                    .client_order()
            })
            .map(|canonical_position| {
                (
                    canonical_position,
                    usize::from(self.client_order_index[canonical_position]),
                )
            })
    }

    pub(super) fn find_client(&self, client: PmClientOrderKey) -> Option<usize> {
        self.search_client(client)
            .ok()
            .map(|(_, dense_index)| dense_index)
    }

    pub(super) fn search_intent(&self, intent: PmOwnedIntentId) -> Result<(usize, usize), usize> {
        self.intent_index
            .binary_search_by_key(&intent, |dense_index| {
                self.entries[usize::from(*dense_index)].intent.intent()
            })
            .map(|intent_position| {
                (
                    intent_position,
                    usize::from(self.intent_index[intent_position]),
                )
            })
    }

    pub(super) fn insert_dense_order(
        &mut self,
        client_position: usize,
        intent_position: usize,
        entry: OwnedOrderEntry,
    ) {
        debug_assert!(self.entries.len() < MAX_PM_OWNED_ORDER_HISTORY);
        debug_assert!(client_position <= self.client_order_index.len());
        debug_assert!(intent_position <= self.intent_index.len());
        let dense_index =
            u16::try_from(self.entries.len()).expect("owned order capacity fits dense u16 index");
        self.entries.push(entry);
        self.client_order_index.insert(client_position, dense_index);
        self.intent_index.insert(intent_position, dense_index);
    }

    pub(super) fn swap_remove_dense_order(
        &mut self,
        client_position: usize,
        intent_position: usize,
        dense_index: usize,
    ) -> OwnedOrderEntry {
        debug_assert_eq!(
            usize::from(self.client_order_index[client_position]),
            dense_index
        );
        debug_assert_eq!(usize::from(self.intent_index[intent_position]), dense_index);
        let prior_last = self
            .entries
            .len()
            .checked_sub(1)
            .expect("preflighted owned order removal");
        let moved_client_position = (dense_index != prior_last).then(|| {
            self.client_order_index
                .iter()
                .position(|candidate| usize::from(*candidate) == prior_last)
                .expect("dense last row remains canonically indexed")
        });
        let moved_intent_position = (dense_index != prior_last).then(|| {
            self.intent_index
                .iter()
                .position(|candidate| usize::from(*candidate) == prior_last)
                .expect("dense last row remains intent-indexed")
        });
        self.client_order_index.remove(client_position);
        self.intent_index.remove(intent_position);
        let removed = self.entries.swap_remove(dense_index);
        if let Some(mut moved_position) = moved_client_position {
            if moved_position > client_position {
                moved_position -= 1;
            }
            self.client_order_index[moved_position] =
                u16::try_from(dense_index).expect("owned order capacity fits dense u16 index");
        }
        if let Some(mut moved_position) = moved_intent_position {
            if moved_position > intent_position {
                moved_position -= 1;
            }
            self.intent_index[moved_position] =
                u16::try_from(dense_index).expect("owned order capacity fits dense u16 index");
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{
        EvmAddress, PmAccountHandle, PmAccountScope, PmChainId, PmClientOrderId, PmEnvironmentId,
        PmFunderId, PmMarketHandle, PmOrderSide, PmPrice, PmQuantity, PmSignerId, PmTokenHandle,
        exact_order_amounts,
    };

    use super::*;

    fn scope() -> PmAccountScope {
        PmAccountScope::new(
            PmEnvironmentId::new("owned-dense-index-test").unwrap(),
            PmChainId::new(137).unwrap(),
            PmSignerId::new(EvmAddress::from_bytes([1; 20]).unwrap()),
            PmFunderId::new(EvmAddress::from_bytes([2; 20]).unwrap()),
            PmAccountHandle::from_ordinal(7),
        )
    }

    fn instrument() -> PmInstrumentHandle {
        PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(11),
            PmTokenHandle::from_ordinal(13),
        )
    }

    fn client(ordinal: usize) -> PmClientOrderKey {
        let mut bytes = [0_u8; 16];
        bytes[8..].copy_from_slice(
            &u64::try_from(ordinal.saturating_add(1))
                .expect("test ordinal fits u64")
                .to_be_bytes(),
        );
        PmClientOrderKey::new(
            scope().handle(),
            PmClientOrderId::from_bytes(bytes).unwrap(),
        )
    }

    fn quote_intent(
        client_ordinal: usize,
        intent_ordinal: usize,
        side: PmOrderSide,
    ) -> PmOwnedQuoteIntent {
        let price = PmPrice::parse_decimal("0.40").unwrap();
        let quantity = PmQuantity::parse_decimal("1").unwrap();
        let maker = exact_order_amounts(side, price, quantity).unwrap().maker();
        let reservation = match side {
            PmOrderSide::Buy => PmExactReservation::policy_approved(maker, U256::ZERO),
            PmOrderSide::Sell => PmExactReservation::policy_approved(U256::ZERO, maker),
        }
        .unwrap();
        PmOwnedQuoteIntent::new(
            PmOwnedIntentId::new(u64::try_from(intent_ordinal).unwrap() + 1).unwrap(),
            PmOwnedQuoteSlotKey::new(scope(), instrument(), side),
            client(client_ordinal),
            price,
            quantity,
            reservation,
        )
        .unwrap()
    }

    fn intent(ordinal: usize, side: PmOrderSide) -> PmOwnedQuoteIntent {
        quote_intent(ordinal, ordinal, side)
    }

    fn entry_with_intent(client_ordinal: usize, intent_ordinal: usize) -> OwnedOrderEntry {
        OwnedOrderEntry {
            intent: quote_intent(client_ordinal, intent_ordinal, PmOrderSide::Buy),
            venue_order: None,
            submit: PmOwnedSubmitState::Rejected,
            status: None,
            cumulative_filled: U256::ZERO,
            known_fill_total: U256::ZERO,
            cancel: PmOwnedCancelState::None,
            reconciliation_required: false,
            compaction_generation: None,
            last_occurrence: None,
            last_progress: None,
        }
    }

    fn shuffled(count: usize) -> Vec<usize> {
        let mut ordinals = (0..count).collect::<Vec<_>>();
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for upper in (1..ordinals.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let index = usize::try_from(seed % u64::try_from(upper + 1).unwrap()).unwrap();
            ordinals.swap(upper, index);
        }
        ordinals
    }

    fn assert_index_invariant(lifecycle: &PmOwnedOrderLifecycle) {
        assert_eq!(lifecycle.client_order_index.len(), lifecycle.entries.len());
        assert_eq!(lifecycle.intent_index.len(), lifecycle.entries.len());
        let mut client_referenced = vec![false; lifecycle.entries.len()];
        let mut prior_client = None;
        for (client_position, dense_index) in
            lifecycle.client_order_index.iter().copied().enumerate()
        {
            let dense_index = usize::from(dense_index);
            assert!(dense_index < lifecycle.entries.len());
            assert!(!client_referenced[dense_index]);
            client_referenced[dense_index] = true;
            let client = lifecycle.entries[dense_index].intent.client_order();
            assert!(prior_client.is_none_or(|prior| prior < client));
            assert_eq!(
                lifecycle.search_client(client),
                Ok((client_position, dense_index))
            );
            assert_eq!(lifecycle.find_client(client), Some(dense_index));
            prior_client = Some(client);
        }
        assert!(client_referenced.into_iter().all(|present| present));

        let mut intent_referenced = vec![false; lifecycle.entries.len()];
        let mut prior_intent = None;
        for (intent_position, dense_index) in lifecycle.intent_index.iter().copied().enumerate() {
            let dense_index = usize::from(dense_index);
            assert!(dense_index < lifecycle.entries.len());
            assert!(!intent_referenced[dense_index]);
            intent_referenced[dense_index] = true;
            let intent = lifecycle.entries[dense_index].intent.intent();
            assert!(prior_intent.is_none_or(|prior| prior < intent));
            assert_eq!(
                lifecycle.search_intent(intent),
                Ok((intent_position, dense_index))
            );
            prior_intent = Some(intent);
        }
        assert!(intent_referenced.into_iter().all(|present| present));
    }

    #[test]
    fn randomized_dense_insertion_matches_canonical_oracle_without_growth() {
        let mut lifecycle = PmOwnedOrderLifecycle::new(scope(), instrument());
        let entry_capacity = lifecycle.entries.capacity();
        let client_index_capacity = lifecycle.client_order_index.capacity();
        let intent_index_capacity = lifecycle.intent_index.capacity();
        let entry_pointer = lifecycle.entries.as_ptr();
        let client_index_pointer = lifecycle.client_order_index.as_ptr();
        let intent_index_pointer = lifecycle.intent_index.as_ptr();
        let reserved_capacity = lifecycle.reserved_capacity_bytes();
        let insertion_order = shuffled(MAX_PM_OWNED_ORDER_HISTORY);

        for ordinal in insertion_order.iter().copied() {
            let intent_ordinal = ordinal.wrapping_mul(613) % MAX_PM_OWNED_ORDER_HISTORY;
            let next = entry_with_intent(ordinal, intent_ordinal);
            let client_position = lifecycle
                .search_client(next.intent.client_order())
                .unwrap_err();
            let intent_position = lifecycle.search_intent(next.intent.intent()).unwrap_err();
            lifecycle.insert_dense_order(client_position, intent_position, next);
            assert_index_invariant(&lifecycle);
        }

        let canonical = lifecycle
            .orders()
            .map(PmOwnedOrderProjection::client_order)
            .collect::<Vec<_>>();
        let expected = (0..MAX_PM_OWNED_ORDER_HISTORY)
            .map(client)
            .collect::<Vec<_>>();
        let dense = lifecycle
            .entries
            .iter()
            .map(|entry| entry.intent.client_order())
            .collect::<Vec<_>>();
        let insertion_clients = insertion_order.into_iter().map(client).collect::<Vec<_>>();
        assert_eq!(canonical, expected);
        assert_eq!(dense, insertion_clients);
        assert_eq!(lifecycle.entries.capacity(), entry_capacity);
        assert_eq!(
            lifecycle.client_order_index.capacity(),
            client_index_capacity
        );
        assert_eq!(lifecycle.intent_index.capacity(), intent_index_capacity);
        assert_eq!(lifecycle.entries.as_ptr(), entry_pointer);
        assert_eq!(lifecycle.client_order_index.as_ptr(), client_index_pointer);
        assert_eq!(lifecycle.intent_index.as_ptr(), intent_index_pointer);
        assert_eq!(lifecycle.reserved_capacity_bytes(), reserved_capacity);

        let error = lifecycle
            .admit_quote(intent(MAX_PM_OWNED_ORDER_HISTORY, PmOrderSide::Sell))
            .unwrap_err();
        assert_eq!(error, PmOwnedOrderLifecycleError::OrderCapacity);
        assert_eq!(lifecycle.counters().order_capacity_failures(), 1);
        assert_index_invariant(&lifecycle);
    }

    #[test]
    fn identity_error_precedence_is_unchanged_across_both_dense_indexes() {
        let mut lifecycle = PmOwnedOrderLifecycle::new(scope(), instrument());
        let retained = quote_intent(1, 1, PmOrderSide::Buy);
        assert!(matches!(
            lifecycle.admit_quote(retained).unwrap(),
            PmOwnedQuoteAdmission::Admitted(_)
        ));
        lifecycle
            .apply_submit_result(client(1), PmOwnedSubmitResult::Rejected)
            .unwrap();

        assert_eq!(
            lifecycle
                .admit_quote(quote_intent(1, 2, PmOrderSide::Buy))
                .unwrap_err(),
            PmOwnedOrderLifecycleError::ClientIdentityConflict
        );
        assert_eq!(
            lifecycle
                .admit_quote(quote_intent(1, 1, PmOrderSide::Sell))
                .unwrap_err(),
            PmOwnedOrderLifecycleError::ClientIdentityConflict
        );
        assert_eq!(
            lifecycle
                .admit_quote(quote_intent(2, 1, PmOrderSide::Buy))
                .unwrap_err(),
            PmOwnedOrderLifecycleError::IntentIdentityConflict
        );
        lifecycle.compact_proven_terminal(client(1)).unwrap();
        assert_eq!(
            lifecycle
                .admit_quote(quote_intent(3, 1, PmOrderSide::Buy))
                .unwrap_err(),
            PmOwnedOrderLifecycleError::CompactedIntentIdentity
        );
        assert_index_invariant(&lifecycle);
    }

    #[test]
    fn arbitrary_terminal_compaction_rewrites_swapped_dense_references_exactly() {
        const RETAINED: usize = 97;
        let mut lifecycle = PmOwnedOrderLifecycle::new(scope(), instrument());
        let insertion_order = shuffled(RETAINED);
        for (intent_ordinal, ordinal) in insertion_order.iter().copied().enumerate() {
            assert!(matches!(
                lifecycle
                    .admit_quote(quote_intent(ordinal, intent_ordinal, PmOrderSide::Buy))
                    .unwrap(),
                PmOwnedQuoteAdmission::Admitted(_)
            ));
            lifecycle
                .apply_submit_result(client(ordinal), PmOwnedSubmitResult::Rejected)
                .unwrap();
        }
        let entry_pointer = lifecycle.entries.as_ptr();
        let client_index_pointer = lifecycle.client_order_index.as_ptr();
        let intent_index_pointer = lifecycle.intent_index.as_ptr();
        let current_slot = *insertion_order.last().unwrap();
        let mut retained = (0..RETAINED).collect::<Vec<_>>();

        for ordinal in shuffled(RETAINED) {
            let compacted = lifecycle.compact_proven_terminal(client(ordinal)).unwrap();
            assert_eq!(compacted.client_order(), client(ordinal));
            retained.retain(|candidate| *candidate != ordinal);
            let canonical = lifecycle
                .orders()
                .map(PmOwnedOrderProjection::client_order)
                .collect::<Vec<_>>();
            let expected = retained.iter().copied().map(client).collect::<Vec<_>>();
            assert_eq!(canonical, expected);
            assert_eq!(lifecycle.order(client(ordinal)), None);
            assert_index_invariant(&lifecycle);
            assert_eq!(lifecycle.entries.as_ptr(), entry_pointer);
            assert_eq!(lifecycle.client_order_index.as_ptr(), client_index_pointer);
            assert_eq!(lifecycle.intent_index.as_ptr(), intent_index_pointer);
            let expected_slot = retained
                .contains(&current_slot)
                .then_some(client(current_slot));
            assert_eq!(lifecycle.slots().next().unwrap().current(), expected_slot);
        }
        assert_eq!(
            lifecycle.counters().terminal_compactions(),
            u64::try_from(RETAINED).unwrap()
        );
    }

    #[test]
    fn marked_compaction_iteration_remains_canonical_over_dense_rows() {
        let mut lifecycle = PmOwnedOrderLifecycle::new(scope(), instrument());
        for ordinal in shuffled(31) {
            let mut next = entry_with_intent(ordinal, ordinal.wrapping_mul(17) % 31);
            if ordinal % 3 == 0 {
                next.compaction_generation = Some(7);
            }
            let client_position = lifecycle
                .search_client(next.intent.client_order())
                .unwrap_err();
            let intent_position = lifecycle.search_intent(next.intent.intent()).unwrap_err();
            lifecycle.insert_dense_order(client_position, intent_position, next);
        }
        let marked = lifecycle.marked_compaction_clients(7).collect::<Vec<_>>();
        let expected = (0..31)
            .filter(|ordinal| ordinal % 3 == 0)
            .map(client)
            .collect::<Vec<_>>();
        assert_eq!(marked, expected);
    }
}
