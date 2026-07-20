use reap_pm_core::{OkxReferenceHandle, PmInstrumentHandle};

/// Public/time input categories available to a pure PM quote model.
///
/// Private lifecycle, reconciliation, account state, and execution are
/// deliberately absent. The product plan adds those mandatory capabilities
/// independently of model code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmModelInputRequirement {
    OkxReference(OkxReferenceHandle),
    MarketMetadata(PmInstrumentHandle),
    MarketBook(PmInstrumentHandle),
    QuoteEvaluationTimer,
}

/// The closed Goal F input set for one statically supplied model.
///
/// Goal F proves one configured OKX index reference and one configured PM
/// outcome. Later goals may add a separately reviewed bounded multi-market
/// representation without widening this enum into private or mutation
/// authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmModelInputRequirements {
    reference: OkxReferenceHandle,
    instrument: PmInstrumentHandle,
}

impl PmModelInputRequirements {
    #[must_use]
    pub const fn new(reference: OkxReferenceHandle, instrument: PmInstrumentHandle) -> Self {
        Self {
            reference,
            instrument,
        }
    }

    #[must_use]
    pub const fn reference(self) -> OkxReferenceHandle {
        self.reference
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn requirements(self) -> [PmModelInputRequirement; 4] {
        [
            PmModelInputRequirement::OkxReference(self.reference),
            PmModelInputRequirement::MarketMetadata(self.instrument),
            PmModelInputRequirement::MarketBook(self.instrument),
            PmModelInputRequirement::QuoteEvaluationTimer,
        ]
    }
}

/// Static model requirement seam.
///
/// This trait declares input reach only. Quote economics and candidate
/// production remain outside Phase 2.
pub trait PmQuoteModelRequirements {
    fn input_requirements(&self) -> PmModelInputRequirements;
}
