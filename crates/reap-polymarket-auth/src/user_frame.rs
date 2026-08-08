//! Credential-owner binding for parsed authenticated user-stream frames.
//!
//! Protocol authority (reviewed 2026-08-08): the current official user-stream
//! wire examples identify order `owner`/`order_owner` and trade-level
//! `owner`/`trade_owner` as the subscribed CLOB API key, while a nested maker
//! row carries the maker's CLOB API key. See
//! <https://docs.polymarket.com/market-data/websocket/user-channel>.
//!
//! Official `clob-client-v2` commit
//! `f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69` corroborates the authenticated
//! subscription and owner-bearing maker/trade DTOs but does not assign user-WS
//! ownership semantics. Pinned Predarb object
//! `8222273a9c72033b760e1d2fec813bc77144556d` is transport corroboration only:
//! its parsed user messages deliberately omit these owner fields.

use std::fmt;

use reap_polymarket_wire::{PmLiveUserEvent, PmLiveUserFrame};

use crate::{L2Credentials, PmAuthError};

/// One parsed user-stream frame whose account-owner fields have all been
/// matched to one exact L2 credential bundle.
///
/// The wrapper is move-only, has no raw-frame escape, duplicates no
/// credential material, and can only be constructed by
/// [`L2Credentials::bind_user_stream_frame`]. Nested maker rows are preserved
/// as received: their `owner` identifies that maker's CLOB API key and is not
/// an account-scope assertion about the subscribed credential.
pub struct CredentialOwnedUserFrame(PmLiveUserFrame);

impl CredentialOwnedUserFrame {
    /// Read the already-bound event views without discarding the ownership
    /// proof carried by this wrapper.
    #[must_use]
    pub fn events(&self) -> &[PmLiveUserEvent] {
        self.0.events()
    }
}

impl fmt::Debug for CredentialOwnedUserFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialOwnedUserFrame([REDACTED])")
    }
}

impl L2Credentials {
    /// Consume one parsed authenticated user-stream frame and bind every
    /// account-scope owner field to this exact L2 credential bundle.
    ///
    /// This is intentionally specific to the current Polymarket user stream;
    /// it is not a generic owner, response, or frame-validation capability.
    pub fn bind_user_stream_frame(
        &self,
        frame: PmLiveUserFrame,
    ) -> Result<CredentialOwnedUserFrame, PmAuthError> {
        for event in frame.events() {
            match event {
                PmLiveUserEvent::Order(order) => {
                    if !self.matches_credential_owner(order.owner()) {
                        return Err(PmAuthError::UserOrderOwnerMismatch);
                    }
                    if order
                        .order_owner()
                        .is_some_and(|owner| !self.matches_credential_owner(owner))
                    {
                        return Err(PmAuthError::UserOrderOrderOwnerMismatch);
                    }
                }
                PmLiveUserEvent::Trade(trade) => {
                    if !self.matches_credential_owner(trade.owner()) {
                        return Err(PmAuthError::UserTradeOwnerMismatch);
                    }
                    if trade
                        .trade_owner()
                        .is_some_and(|owner| !self.matches_credential_owner(owner))
                    {
                        return Err(PmAuthError::UserTradeTradeOwnerMismatch);
                    }
                }
            }
        }

        Ok(CredentialOwnedUserFrame(frame))
    }
}
