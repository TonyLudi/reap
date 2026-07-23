use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::identity::{
    OkxReferenceHandle, OkxReferenceInstrument, OkxReferenceKind, PmInstrumentHandle,
    PmInstrumentId, PmMarketHandle, PmTokenHandle,
};

pub const MAX_OKX_REFERENCES_PER_MAPPING: usize = 16;
const PUBLIC_IDENTITY_FINGERPRINT_PREFIX: &[u8] = b"reap.pm.public-identity-tables.v1\0";

/// SHA-256 over the canonical ordered public-identity tables.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmConfigurationFingerprint([u8; 32]);

impl PmConfigurationFingerprint {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Checked Goal-F assignment of raw public identities to compact handles.
///
/// Goal F reaches exactly one OKX reference and one PM market/outcome. Each
/// identity class is therefore a one-row canonically ordered table whose
/// compact handle is the zero-based `u16` row ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmPublicObservationGrant {
    okx_reference: OkxReferenceHandle,
    okx_instrument: OkxReferenceInstrument,
    instrument: PmInstrumentHandle,
    polymarket_instrument: PmInstrumentId,
    configuration_fingerprint: PmConfigurationFingerprint,
}

impl PmPublicObservationGrant {
    #[must_use]
    pub fn derive_goal_f(
        okx_instrument: OkxReferenceInstrument,
        polymarket_instrument: PmInstrumentId,
    ) -> Self {
        let okx_reference = OkxReferenceHandle::from_ordinal(0);
        let instrument = PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(0),
            PmTokenHandle::from_ordinal(0),
        );
        let configuration_fingerprint = fingerprint_public_identity_tables(
            okx_reference,
            okx_instrument,
            instrument,
            polymarket_instrument,
        );
        Self {
            okx_reference,
            okx_instrument,
            instrument,
            polymarket_instrument,
            configuration_fingerprint,
        }
    }

    #[must_use]
    pub const fn okx_reference(self) -> OkxReferenceHandle {
        self.okx_reference
    }

    #[must_use]
    pub const fn okx_instrument(self) -> OkxReferenceInstrument {
        self.okx_instrument
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn polymarket_instrument(self) -> PmInstrumentId {
        self.polymarket_instrument
    }

    #[must_use]
    pub const fn configuration_fingerprint(self) -> PmConfigurationFingerprint {
        self.configuration_fingerprint
    }
}

fn fingerprint_public_identity_tables(
    okx_reference: OkxReferenceHandle,
    okx_instrument: OkxReferenceInstrument,
    instrument: PmInstrumentHandle,
    polymarket_instrument: PmInstrumentId,
) -> PmConfigurationFingerprint {
    let mut digest = Sha256::new();
    digest.update(PUBLIC_IDENTITY_FINGERPRINT_PREFIX);

    digest.update(b"okx-reference\0");
    digest.update(1_u16.to_be_bytes());
    digest.update(okx_reference.ordinal().to_be_bytes());
    digest.update([match okx_instrument.kind() {
        OkxReferenceKind::Index => 1,
    }]);
    let okx_raw = okx_instrument.instrument_id();
    let okx_bytes = okx_raw.as_str().as_bytes();
    digest.update((okx_bytes.len() as u16).to_be_bytes());
    digest.update(okx_bytes);

    digest.update(b"pm-market\0");
    digest.update(1_u16.to_be_bytes());
    digest.update(instrument.market().ordinal().to_be_bytes());
    digest.update(polymarket_instrument.market().bytes());

    digest.update(b"pm-token\0");
    digest.update(1_u16.to_be_bytes());
    digest.update(instrument.token().ordinal().to_be_bytes());
    digest.update(polymarket_instrument.token().units().to_be_bytes());

    PmConfigurationFingerprint(digest.finalize().into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmMappingError {
    #[error("reference mapping must name at least one OKX reference")]
    Empty,
    #[error("reference mapping exceeds its fixed bound")]
    TooMany,
    #[error("reference mapping array is not canonical for its count")]
    NonCanonicalArray,
    #[error("reference mapping contains a duplicate OKX reference")]
    DuplicateReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct PmReferenceMapping {
    target: PmInstrumentHandle,
    references: [Option<OkxReferenceHandle>; MAX_OKX_REFERENCES_PER_MAPPING],
    reference_count: u8,
}

impl PmReferenceMapping {
    pub fn new(
        target: PmInstrumentHandle,
        mut references: [Option<OkxReferenceHandle>; MAX_OKX_REFERENCES_PER_MAPPING],
        reference_count: u8,
    ) -> Result<Self, PmMappingError> {
        let count = usize::from(reference_count);
        if count == 0 {
            return Err(PmMappingError::Empty);
        }
        if count > MAX_OKX_REFERENCES_PER_MAPPING {
            return Err(PmMappingError::TooMany);
        }
        if references[..count].iter().any(Option::is_none)
            || references[count..].iter().any(Option::is_some)
        {
            return Err(PmMappingError::NonCanonicalArray);
        }

        references[..count].sort_unstable();
        if references[..count]
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(PmMappingError::DuplicateReference);
        }

        Ok(Self {
            target,
            references,
            reference_count,
        })
    }

    #[must_use]
    pub const fn target(self) -> PmInstrumentHandle {
        self.target
    }

    pub fn references(&self) -> impl Iterator<Item = OkxReferenceHandle> + '_ {
        self.references[..usize::from(self.reference_count)]
            .iter()
            .flatten()
            .copied()
    }

    #[must_use]
    pub const fn reference_count(self) -> u8 {
        self.reference_count
    }
}

impl<'de> Deserialize<'de> for PmReferenceMapping {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            target: PmInstrumentHandle,
            references: [Option<OkxReferenceHandle>; MAX_OKX_REFERENCES_PER_MAPPING],
            reference_count: u8,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.target, wire.references, wire.reference_count)
            .map_err(serde::de::Error::custom)
    }
}
