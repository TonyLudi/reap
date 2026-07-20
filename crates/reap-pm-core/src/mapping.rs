use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::identity::{OkxReferenceHandle, PmInstrumentHandle};

pub const MAX_OKX_REFERENCES_PER_MAPPING: usize = 16;

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
