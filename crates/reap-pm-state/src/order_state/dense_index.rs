use super::*;

impl PmOrderState {
    pub(super) fn canonical_entries(&self) -> impl Iterator<Item = &OrderEntry> {
        self.canonical_index
            .iter()
            .map(|slot| &self.entries[usize::from(*slot)])
    }

    pub(super) fn canonical_insertion_position(
        &self,
        entry: &OrderEntry,
    ) -> Result<usize, PmOrderStateError> {
        match self
            .canonical_index
            .binary_search_by(|slot| compare_entries(&self.entries[usize::from(*slot)], entry))
        {
            Ok(_) => Err(PmOrderStateError::IdentityConflict),
            Err(position) => Ok(position),
        }
    }

    pub(super) fn client_search(&self, client: PmClientOrderKey) -> Result<(usize, usize), usize> {
        self.client_index
            .binary_search_by_key(&client, |slot| {
                self.entries[usize::from(*slot)]
                    .identity
                    .client_order_key()
                    .expect("client index references only client-bearing orders")
            })
            .map(|position| (position, usize::from(self.client_index[position])))
    }

    pub(super) fn client_insertion_position(
        &self,
        entry: &OrderEntry,
    ) -> Result<Option<usize>, PmOrderStateError> {
        let Some(client) = entry.identity.client_order_key() else {
            return Ok(None);
        };
        self.client_search(client).map_or_else(
            |position| Ok(Some(position)),
            |_| Err(PmOrderStateError::IdentityConflict),
        )
    }

    pub(super) fn insert_ordered(&mut self, entry: OrderEntry) -> Result<(), PmOrderStateError> {
        let canonical_position = self.canonical_insertion_position(&entry)?;
        let client_position = self.client_insertion_position(&entry)?;
        self.insert_at_index_positions(canonical_position, client_position, entry);
        Ok(())
    }

    pub(super) fn insert_at_index_positions(
        &mut self,
        canonical_position: usize,
        client_position: Option<usize>,
        entry: OrderEntry,
    ) {
        debug_assert!(self.entries.len() < MAX_PM_PRIVATE_ORDERS);
        debug_assert_eq!(self.entries.len(), self.canonical_index.len());
        debug_assert!(canonical_position <= self.canonical_index.len());
        debug_assert_eq!(
            client_position.is_some(),
            entry.identity.client_order_key().is_some()
        );
        debug_assert!(client_position.is_none_or(|position| position <= self.client_index.len()));
        let dense_slot =
            u16::try_from(self.entries.len()).expect("configured order capacity fits u16");
        self.entries.push(entry);
        if entry.is_live() {
            self.live_count = self
                .live_count
                .checked_add(1)
                .expect("configured order capacity fits u16");
        }
        self.canonical_index.insert(canonical_position, dense_slot);
        if let Some(position) = client_position {
            self.client_index.insert(position, dense_slot);
        }
    }

    pub(super) fn replace_ordered(
        &mut self,
        dense_slot: usize,
        replacement: OrderEntry,
    ) -> Result<(), PmOrderStateError> {
        let current = self.entries[dense_slot];
        let old_canonical_position = self
            .canonical_index
            .iter()
            .position(|slot| usize::from(*slot) == dense_slot)
            .expect("every dense order has one canonical index reference");
        let canonical_key_changed = compare_entries(&current, &replacement) != Ordering::Equal;
        let mut new_canonical_position = old_canonical_position;
        if canonical_key_changed {
            new_canonical_position = self.canonical_insertion_position(&replacement)?;
            if new_canonical_position > old_canonical_position {
                new_canonical_position -= 1;
            }
        }

        let current_client = current.identity.client_order_key();
        let replacement_client = replacement.identity.client_order_key();
        let client_key_changed = current_client != replacement_client;
        let old_client_position = current_client.map(|client| {
            let (position, indexed_slot) = self
                .client_search(client)
                .expect("client-bearing dense order remains indexed");
            debug_assert_eq!(indexed_slot, dense_slot);
            position
        });
        let mut new_client_position = old_client_position;
        if client_key_changed {
            new_client_position = match replacement_client {
                None => None,
                Some(client) => match self.client_search(client) {
                    Ok(_) => return Err(PmOrderStateError::IdentityConflict),
                    Err(mut position) => {
                        if old_client_position.is_some_and(|old| position > old) {
                            position -= 1;
                        }
                        Some(position)
                    }
                },
            };
        }

        self.entries[dense_slot] = replacement;
        match (current.is_live(), replacement.is_live()) {
            (false, true) => {
                self.live_count = self
                    .live_count
                    .checked_add(1)
                    .expect("configured order capacity fits u16");
            }
            (true, false) => {
                self.live_count = self
                    .live_count
                    .checked_sub(1)
                    .expect("a replaced live row contributes to the exact live count");
            }
            (false, false) | (true, true) => {}
        }
        if canonical_key_changed && new_canonical_position != old_canonical_position {
            let encoded_slot = self.canonical_index.remove(old_canonical_position);
            self.canonical_index
                .insert(new_canonical_position, encoded_slot);
        }
        if client_key_changed {
            let encoded_slot =
                old_client_position.map(|position| self.client_index.remove(position));
            match (encoded_slot, new_client_position) {
                (Some(slot), Some(position)) => self.client_index.insert(position, slot),
                (None, Some(position)) => self.client_index.insert(
                    position,
                    u16::try_from(dense_slot).expect("configured order capacity fits u16"),
                ),
                (Some(_), None) | (None, None) => {}
            }
        }
        Ok(())
    }

    pub(super) fn remove_dense(&mut self, dense_slot: usize) -> OrderEntry {
        let removed_entry = self.entries[dense_slot];
        let canonical_position = self
            .canonical_index
            .iter()
            .position(|slot| usize::from(*slot) == dense_slot)
            .expect("every dense order has one canonical index reference");
        let client_position = removed_entry.identity.client_order_key().map(|client| {
            let (position, indexed_slot) = self
                .client_search(client)
                .expect("client-bearing dense order remains indexed");
            debug_assert_eq!(indexed_slot, dense_slot);
            position
        });
        self.canonical_index.remove(canonical_position);
        if let Some(position) = client_position {
            self.client_index.remove(position);
        }

        let old_last = self
            .entries
            .len()
            .checked_sub(1)
            .expect("dense removal requires one retained order");
        if removed_entry.is_live() {
            self.live_count = self
                .live_count
                .checked_sub(1)
                .expect("a removed live row contributes to the exact live count");
        }
        let removed = self.entries.swap_remove(dense_slot);
        if dense_slot != old_last {
            let moved_reference = self
                .canonical_index
                .iter_mut()
                .find(|slot| usize::from(**slot) == old_last)
                .expect("swap-removed order retains one canonical index reference");
            *moved_reference =
                u16::try_from(dense_slot).expect("configured order capacity fits u16");
            if self.entries[dense_slot]
                .identity
                .client_order_key()
                .is_some()
            {
                let moved_client_reference = self
                    .client_index
                    .iter_mut()
                    .find(|slot| usize::from(**slot) == old_last)
                    .expect("swap-removed client-bearing order remains indexed");
                *moved_client_reference =
                    u16::try_from(dense_slot).expect("configured order capacity fits u16");
            }
        }
        debug_assert_eq!(removed, removed_entry);
        removed
    }

    pub(super) fn find(&self, identity: PmOrderIdentity) -> Option<usize> {
        if let Some(client) = identity.client_order_key()
            && let Ok((_, dense_slot)) = self.client_search(client)
        {
            return Some(dense_slot);
        }
        identity.venue_order_key().and_then(|venue| {
            self.canonical_index.iter().find_map(|slot| {
                let dense_slot = usize::from(*slot);
                (self.entries[dense_slot].identity.venue_order_key() == Some(venue))
                    .then_some(dense_slot)
            })
        })
    }

    pub(super) fn overlaps(
        &self,
        identity: PmOrderIdentity,
    ) -> Result<OrderOverlap, PmOrderStateError> {
        let mut first = None;
        let mut second = None;
        for dense_slot in self.canonical_index.iter().copied().map(usize::from) {
            if !identities_overlap(self.entries[dense_slot].identity, identity) {
                continue;
            }
            match (first, second) {
                (None, _) => first = Some(dense_slot),
                (Some(_), None) => second = Some(dense_slot),
                (Some(_), Some(_)) => return Err(PmOrderStateError::IdentityConflict),
            }
        }
        Ok(match (first, second) {
            (None, None) => OrderOverlap::None,
            (Some(index), None) => OrderOverlap::One(index),
            (Some(first), Some(second)) => OrderOverlap::Bridge(first, second),
            (None, Some(_)) => unreachable!("second overlap requires a first"),
        })
    }
}
