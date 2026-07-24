use reap_pm_core::{
    OkxReferenceHandle, OkxReferencePrice, PmInstrumentHandle, PmOrderSide, PmQuantity,
};

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

/// Immutable normalized inputs reached by the pure Goal-F quote-model seam.
///
/// The exact OKX price is intentionally supplied even when a deterministic
/// architecture fixture ignores its value. No account, order, transport or
/// mutation capability is reachable here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmQuoteModelInput {
    reference: OkxReferencePrice,
    reference_revision: u64,
    instrument: PmInstrumentHandle,
    monotonic_observed_ns: u64,
}

impl PmQuoteModelInput {
    pub fn new(
        reference: OkxReferencePrice,
        reference_revision: u64,
        instrument: PmInstrumentHandle,
        monotonic_observed_ns: u64,
    ) -> Result<Self, PmQuoteModelError> {
        if reference_revision == 0 || monotonic_observed_ns == 0 {
            return Err(PmQuoteModelError::InvalidRevisionOrTime);
        }
        Ok(Self {
            reference,
            reference_revision,
            instrument,
            monotonic_observed_ns,
        })
    }

    #[must_use]
    pub const fn reference(self) -> OkxReferencePrice {
        self.reference
    }

    #[must_use]
    pub const fn reference_revision(self) -> u64 {
        self.reference_revision
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn monotonic_observed_ns(self) -> u64 {
        self.monotonic_observed_ns
    }
}

/// Fixed two-slot side set returned by a pure model without allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmQuoteSides {
    Buy,
    Sell,
    Both,
}

impl PmQuoteSides {
    #[must_use]
    pub const fn ordered(self) -> [Option<PmOrderSide>; 2] {
        match self {
            Self::Buy => [Some(PmOrderSide::Buy), None],
            Self::Sell => [Some(PmOrderSide::Sell), None],
            Self::Both => [Some(PmOrderSide::Buy), Some(PmOrderSide::Sell)],
        }
    }
}

/// Pure model output. Floating point stops here; quote policy converts it once
/// into side-aware exact protocol units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmQuoteModelOutput {
    fair_probability: f64,
    quantity: PmQuantity,
    sides: PmQuoteSides,
}

impl PmQuoteModelOutput {
    pub fn new(
        fair_probability: f64,
        quantity: PmQuantity,
        sides: PmQuoteSides,
    ) -> Result<Self, PmQuoteModelError> {
        if !fair_probability.is_finite() || !(0.0..1.0).contains(&fair_probability) {
            return Err(PmQuoteModelError::InvalidFixtureProbability);
        }
        Ok(Self {
            fair_probability,
            quantity,
            sides,
        })
    }

    #[must_use]
    pub const fn fair_probability(self) -> f64 {
        self.fair_probability
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn sides(self) -> PmQuoteSides {
        self.sides
    }
}

/// Static pure quote model used by a product coordinator.
pub trait PmQuoteModel: PmQuoteModelRequirements {
    fn evaluate(&self, input: PmQuoteModelInput) -> PmQuoteModelOutput;
}

/// Deterministic architecture fixture, deliberately not production economics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmFixtureQuoteModel {
    requirements: PmModelInputRequirements,
    output: PmQuoteModelOutput,
}

impl PmFixtureQuoteModel {
    pub fn new(
        requirements: PmModelInputRequirements,
        fair_probability: f64,
        quantity: PmQuantity,
        sides: PmQuoteSides,
    ) -> Result<Self, PmQuoteModelError> {
        Ok(Self {
            requirements,
            output: PmQuoteModelOutput::new(fair_probability, quantity, sides)?,
        })
    }
}

impl PmQuoteModelRequirements for PmFixtureQuoteModel {
    fn input_requirements(&self) -> PmModelInputRequirements {
        self.requirements
    }
}

impl PmQuoteModel for PmFixtureQuoteModel {
    fn evaluate(&self, input: PmQuoteModelInput) -> PmQuoteModelOutput {
        debug_assert_eq!(input.instrument(), self.requirements.instrument());
        self.output
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmQuoteModelError {
    InvalidRevisionOrTime,
    InvalidFixtureProbability,
}
