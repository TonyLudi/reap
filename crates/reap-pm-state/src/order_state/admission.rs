use super::*;

impl PmOrderState {
    pub(crate) fn prepare_owned_registration(
        &self,
        registration: PmOwnedOrderRegistration,
        config: &PmPrivateStateConfig,
    ) -> Result<PmOwnedRegistrationPlan, PmOrderStateError> {
        validate_registration(registration, config)?;
        let identity = PmOrderIdentity::new(Some(registration.client_order), None)
            .expect("registration always carries a client key");
        let planned_entry = OrderEntry {
            identity,
            instrument: registration.instrument,
            event: None,
            ownership: OwnershipState::ProvenOwned,
            registered_terms: Some((registration.side, registration.price, registration.quantity)),
            reservation: Some(PmReservationKnowledge::Known(registration.reservation)),
            missing_from_complete_open_snapshot: false,
            terminal_by_detail_absence: false,
            last_occurrence: None,
        };
        if let Some(index) = self.find(identity) {
            let entry = self.entries[index];
            if entry.ownership != OwnershipState::ProvenOwned || entry.event.is_some() {
                return Err(PmOrderStateError::OwnershipRegisteredTooLate);
            }
            if entry.reservation != planned_entry.reservation
                || entry.registered_terms != planned_entry.registered_terms
            {
                return Err(PmOrderStateError::IdentityConflict);
            }
            return Ok(PmOwnedRegistrationPlan {
                entry: planned_entry,
                action: PmOwnedRegistrationAction::Existing,
            });
        }
        if self.entries.len() == MAX_PM_PRIVATE_ORDERS {
            return Err(PmOrderStateError::Capacity);
        }
        let canonical_position = self.canonical_insertion_position(&planned_entry)?;
        let client_position = self
            .client_insertion_position(&planned_entry)?
            .expect("owned registration always carries a client key");
        Ok(PmOwnedRegistrationPlan {
            entry: planned_entry,
            action: PmOwnedRegistrationAction::Insert {
                canonical_position,
                client_position,
            },
        })
    }

    pub(crate) fn commit_preflighted_owned_registration(&mut self, plan: PmOwnedRegistrationPlan) {
        match plan.action {
            PmOwnedRegistrationAction::Existing => {}
            PmOwnedRegistrationAction::Insert {
                canonical_position,
                client_position,
            } => {
                debug_assert!(self.entries.len() < MAX_PM_PRIVATE_ORDERS);
                self.insert_at_index_positions(
                    canonical_position,
                    Some(client_position),
                    plan.entry,
                );
                self.counters.registrations = self.counters.registrations.saturating_add(1);
            }
        }
    }
}
