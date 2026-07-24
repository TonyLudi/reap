use reap_pm_core::{
    PM_PROTOCOL_SCALE, PmInstrumentHandle, PmInstrumentId, PmMarketMetadata, PmNumericError,
    PmOrderSide, PmPrice, PmQuantity, U256, exact_order_amounts,
};

/// Model output and exact market facts needed by the pure PM quote boundary.
///
/// `fair_probability` is the only floating-point value admitted by this
/// boundary. The returned candidate contains exact protocol units only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmQuotePolicyInput {
    instrument: PmInstrumentHandle,
    metadata: PmMarketMetadata,
    side: PmOrderSide,
    fair_probability: f64,
    quantity: PmQuantity,
    best_bid: Option<PmPrice>,
    best_ask: Option<PmPrice>,
}

impl PmQuotePolicyInput {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        instrument: PmInstrumentHandle,
        metadata: PmMarketMetadata,
        side: PmOrderSide,
        fair_probability: f64,
        quantity: PmQuantity,
        best_bid: Option<PmPrice>,
        best_ask: Option<PmPrice>,
    ) -> Self {
        Self {
            instrument,
            metadata,
            side,
            fair_probability,
            quantity,
            best_bid,
            best_ask,
        }
    }
}

/// Exact, checked strategy output. This is data, not mutation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmValidatedQuoteCandidate {
    instrument: PmInstrumentHandle,
    instrument_id: PmInstrumentId,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    maker_amount: U256,
    taker_amount: U256,
}

impl PmValidatedQuoteCandidate {
    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn instrument_id(self) -> PmInstrumentId {
        self.instrument_id
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn maker_amount(self) -> U256 {
        self.maker_amount
    }

    #[must_use]
    pub const fn taker_amount(self) -> U256 {
        self.taker_amount
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmQuotePolicyError {
    NonFiniteFairProbability,
    FairProbabilityOutOfRange,
    MarketInactive,
    MarketClosed,
    MarketArchived,
    OrdersNotAccepted,
    OrderBookDisabled,
    LockedOrCrossedBook,
    MissingBestAsk,
    MissingBestBid,
    RoundedPriceOutsideExecutableRange,
    QuoteWouldTakeLiquidity,
    Numeric(PmNumericError),
}

impl From<PmNumericError> for PmQuotePolicyError {
    fn from(error: PmNumericError) -> Self {
        Self::Numeric(error)
    }
}

/// Converts one model probability into an exact side-aware passive candidate.
///
/// Buys round down toward zero and sells round up toward one. The conversion
/// occurs before all exact grid, size, amount, and passivity checks.
pub fn validate_passive_quote_candidate(
    input: PmQuotePolicyInput,
) -> Result<PmValidatedQuoteCandidate, PmQuotePolicyError> {
    validate_market(input.metadata)?;
    validate_book(input.best_bid, input.best_ask)?;

    let price = directional_price(
        input.fair_probability,
        input.metadata.tick().units(),
        input.side,
    )?;
    price.validate_tick(input.metadata.tick())?;
    input
        .quantity
        .validate_order(input.metadata.minimum_order_size())?;
    validate_passivity(input.side, price, input.best_bid, input.best_ask)?;

    let amounts = exact_order_amounts(input.side, price, input.quantity)?;
    Ok(PmValidatedQuoteCandidate {
        instrument: input.instrument,
        instrument_id: PmInstrumentId::new(
            input.metadata.market(),
            input.metadata.outcome().token(),
        ),
        side: input.side,
        price,
        quantity: input.quantity,
        maker_amount: amounts.maker(),
        taker_amount: amounts.taker(),
    })
}

fn validate_market(metadata: PmMarketMetadata) -> Result<(), PmQuotePolicyError> {
    let lifecycle = metadata.lifecycle();
    if !lifecycle.active() {
        return Err(PmQuotePolicyError::MarketInactive);
    }
    if lifecycle.closed() {
        return Err(PmQuotePolicyError::MarketClosed);
    }
    if lifecycle.archived() {
        return Err(PmQuotePolicyError::MarketArchived);
    }
    if !lifecycle.accepting_orders() {
        return Err(PmQuotePolicyError::OrdersNotAccepted);
    }
    if !lifecycle.order_book_enabled() {
        return Err(PmQuotePolicyError::OrderBookDisabled);
    }
    Ok(())
}

fn validate_book(
    best_bid: Option<PmPrice>,
    best_ask: Option<PmPrice>,
) -> Result<(), PmQuotePolicyError> {
    if best_bid.zip(best_ask).is_some_and(|(bid, ask)| bid >= ask) {
        return Err(PmQuotePolicyError::LockedOrCrossedBook);
    }
    Ok(())
}

fn directional_price(
    fair_probability: f64,
    tick_units: u32,
    side: PmOrderSide,
) -> Result<PmPrice, PmQuotePolicyError> {
    if !fair_probability.is_finite() {
        return Err(PmQuotePolicyError::NonFiniteFairProbability);
    }
    if !(0.0..1.0).contains(&fair_probability) || fair_probability == 0.0 {
        return Err(PmQuotePolicyError::FairProbabilityOutOfRange);
    }

    let scaled = fair_probability * f64::from(PM_PROTOCOL_SCALE);
    let tick_index = scaled / f64::from(tick_units);
    let rounded_index = match side {
        PmOrderSide::Buy => tick_index.floor(),
        PmOrderSide::Sell => tick_index.ceil(),
    };
    let rounded_units = rounded_index * f64::from(tick_units);
    if !rounded_units.is_finite()
        || rounded_units < 1.0
        || rounded_units >= f64::from(PM_PROTOCOL_SCALE)
    {
        return Err(PmQuotePolicyError::RoundedPriceOutsideExecutableRange);
    }

    // Bounds above prove this conversion is exact for the supported range:
    // every integral protocol unit is below 2^53.
    PmPrice::from_units(rounded_units as u32)
        .map_err(|_| PmQuotePolicyError::RoundedPriceOutsideExecutableRange)
}

fn validate_passivity(
    side: PmOrderSide,
    price: PmPrice,
    best_bid: Option<PmPrice>,
    best_ask: Option<PmPrice>,
) -> Result<(), PmQuotePolicyError> {
    match side {
        PmOrderSide::Buy => {
            let ask = best_ask.ok_or(PmQuotePolicyError::MissingBestAsk)?;
            if price >= ask {
                return Err(PmQuotePolicyError::QuoteWouldTakeLiquidity);
            }
        }
        PmOrderSide::Sell => {
            let bid = best_bid.ok_or(PmQuotePolicyError::MissingBestBid)?;
            if price <= bid {
                return Err(PmQuotePolicyError::QuoteWouldTakeLiquidity);
            }
        }
    }
    Ok(())
}
