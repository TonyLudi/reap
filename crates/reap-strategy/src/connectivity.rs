use std::cmp::Ordering;
use std::collections::BTreeSet;

use reap_core::{Symbol, TimeMs};

use crate::chaos::{ChaosConfig, ReferenceDataKind};

/// Stable boundary identifiers for inputs consumed by the Chaos decision layer.
///
/// These identifiers describe venue-neutral strategy needs. Exchange channels,
/// connections, and authenticated roles are resolved at the live/venue edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChaosDecisionRequirementId {
    MarketBook,
    MarketTrade,
    ReferenceIndex,
    ReferenceFunding,
    ReferenceMark,
    ReferencePriceLimits,
    Timer,
}

impl ChaosDecisionRequirementId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MarketBook => "CHAOS-MD-BOOK",
            Self::MarketTrade => "CHAOS-MD-TRADE",
            Self::ReferenceIndex => "CHAOS-REF-INDEX",
            Self::ReferenceFunding => "CHAOS-REF-FUNDING",
            Self::ReferenceMark => "CHAOS-REF-MARK",
            Self::ReferencePriceLimits => "CHAOS-REF-LIMITS",
            Self::Timer => "CHAOS-TIMER",
        }
    }
}

impl Ord for ChaosDecisionRequirementId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for ChaosDecisionRequirementId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The named Chaos behavior that consumes a decision input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChaosDecisionConsumer {
    QuoteAndHedgeCalculations,
    ImpliedDepthAndRepricing,
    IndexDeviationValuationAndPricing,
    FundingAwarePricingAndRisk,
    DerivativeValuationAndSafety,
    QuoteAndHedgePriceBounds,
    TimeBasedStrategyLogic,
}

impl ChaosDecisionConsumer {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QuoteAndHedgeCalculations => "quote_and_hedge_calculations",
            Self::ImpliedDepthAndRepricing => "implied_depth_and_repricing",
            Self::IndexDeviationValuationAndPricing => "index_deviation_valuation_and_pricing",
            Self::FundingAwarePricingAndRisk => "funding_aware_pricing_and_risk",
            Self::DerivativeValuationAndSafety => "derivative_valuation_and_safety",
            Self::QuoteAndHedgePriceBounds => "quote_and_hedge_price_bounds",
            Self::TimeBasedStrategyLogic => "time_based_strategy_logic",
        }
    }
}

/// One normalized input consumed by the venue-neutral Chaos decision layer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChaosDecisionInput {
    Book {
        symbol: Symbol,
        max_age_ms: TimeMs,
    },
    Trade {
        symbol: Symbol,
    },
    Reference {
        kind: ReferenceDataKind,
        symbol: Symbol,
        max_age_ms: Option<TimeMs>,
    },
    Timer,
}

impl ChaosDecisionInput {
    pub fn symbol(&self) -> Option<&str> {
        match self {
            Self::Book { symbol, .. } | Self::Trade { symbol } | Self::Reference { symbol, .. } => {
                Some(symbol)
            }
            Self::Timer => None,
        }
    }
}

/// A requirement with its stable boundary identifier and named consumer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChaosDecisionInputRequirement {
    requirement_id: ChaosDecisionRequirementId,
    consumer: ChaosDecisionConsumer,
    input: ChaosDecisionInput,
}

impl ChaosDecisionInputRequirement {
    pub const fn requirement_id(&self) -> ChaosDecisionRequirementId {
        self.requirement_id
    }

    pub const fn consumer(&self) -> ChaosDecisionConsumer {
        self.consumer
    }

    pub const fn input(&self) -> &ChaosDecisionInput {
        &self.input
    }

    fn new(input: ChaosDecisionInput) -> Self {
        let (requirement_id, consumer) = match &input {
            ChaosDecisionInput::Book { .. } => (
                ChaosDecisionRequirementId::MarketBook,
                ChaosDecisionConsumer::QuoteAndHedgeCalculations,
            ),
            ChaosDecisionInput::Trade { .. } => (
                ChaosDecisionRequirementId::MarketTrade,
                ChaosDecisionConsumer::ImpliedDepthAndRepricing,
            ),
            ChaosDecisionInput::Reference {
                kind: ReferenceDataKind::IndexPrice,
                ..
            } => (
                ChaosDecisionRequirementId::ReferenceIndex,
                ChaosDecisionConsumer::IndexDeviationValuationAndPricing,
            ),
            ChaosDecisionInput::Reference {
                kind: ReferenceDataKind::FundingRate,
                ..
            } => (
                ChaosDecisionRequirementId::ReferenceFunding,
                ChaosDecisionConsumer::FundingAwarePricingAndRisk,
            ),
            ChaosDecisionInput::Reference {
                kind: ReferenceDataKind::MarkPrice,
                ..
            } => (
                ChaosDecisionRequirementId::ReferenceMark,
                ChaosDecisionConsumer::DerivativeValuationAndSafety,
            ),
            ChaosDecisionInput::Reference {
                kind: ReferenceDataKind::PriceLimits,
                ..
            } => (
                ChaosDecisionRequirementId::ReferencePriceLimits,
                ChaosDecisionConsumer::QuoteAndHedgePriceBounds,
            ),
            ChaosDecisionInput::Timer => (
                ChaosDecisionRequirementId::Timer,
                ChaosDecisionConsumer::TimeBasedStrategyLogic,
            ),
        };
        Self {
            requirement_id,
            consumer,
            input,
        }
    }
}

/// The exact venue-neutral inputs needed by an effective Chaos configuration.
///
/// Entries are deduplicated and sorted by stable boundary ID and input payload.
/// This type intentionally excludes account, risk, mode, transport, and venue
/// concerns; the live composition boundary adds those to its connectivity plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChaosDecisionRequirements {
    inputs: Vec<ChaosDecisionInputRequirement>,
}

impl ChaosDecisionRequirements {
    pub fn from_config(config: &ChaosConfig) -> Self {
        let effective = config.effective();
        let mut inputs = BTreeSet::new();
        let reference_max_age_ms = effective.reference_data_stale_threshold_ms;

        for instrument in &effective.instruments {
            inputs.insert(ChaosDecisionInputRequirement::new(
                ChaosDecisionInput::Book {
                    symbol: instrument.symbol.clone(),
                    max_age_ms: instrument.depth_stale_threshold_ms,
                },
            ));
            inputs.insert(ChaosDecisionInputRequirement::new(
                ChaosDecisionInput::Trade {
                    symbol: instrument.symbol.clone(),
                },
            ));

            // Every supported Chaos instrument applies venue price limits when
            // supplied. Freshness is mandatory only when strict reference-data
            // freshness is configured, but the decision input remains required.
            inputs.insert(ChaosDecisionInputRequirement::new(
                ChaosDecisionInput::Reference {
                    kind: ReferenceDataKind::PriceLimits,
                    symbol: instrument.symbol.clone(),
                    max_age_ms: reference_max_age_ms,
                },
            ));

            if instrument.kind.is_derivative() {
                inputs.insert(ChaosDecisionInputRequirement::new(
                    ChaosDecisionInput::Reference {
                        kind: ReferenceDataKind::MarkPrice,
                        symbol: instrument.symbol.clone(),
                        max_age_ms: reference_max_age_ms,
                    },
                ));
            }
            if instrument.kind.is_swap() {
                inputs.insert(ChaosDecisionInputRequirement::new(
                    ChaosDecisionInput::Reference {
                        kind: ReferenceDataKind::FundingRate,
                        symbol: instrument.symbol.clone(),
                        max_age_ms: reference_max_age_ms,
                    },
                ));
            }
            if let Some(index_symbol) = &instrument.index_symbol {
                inputs.insert(ChaosDecisionInputRequirement::new(
                    ChaosDecisionInput::Reference {
                        kind: ReferenceDataKind::IndexPrice,
                        symbol: index_symbol.clone(),
                        max_age_ms: reference_max_age_ms,
                    },
                ));
            }
        }

        inputs.insert(ChaosDecisionInputRequirement::new(
            ChaosDecisionInput::Timer,
        ));

        Self {
            inputs: inputs.into_iter().collect(),
        }
    }

    pub fn inputs(&self) -> &[ChaosDecisionInputRequirement] {
        &self.inputs
    }
}

impl ChaosConfig {
    pub fn decision_requirements(&self) -> ChaosDecisionRequirements {
        ChaosDecisionRequirements::from_config(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chaos::{InstrumentConfig, InstrumentKindConfig};

    fn instrument(
        symbol: &str,
        kind: InstrumentKindConfig,
        index_symbol: Option<&str>,
    ) -> InstrumentConfig {
        InstrumentConfig {
            symbol: symbol.to_string(),
            kind,
            index_symbol: index_symbol.map(str::to_string),
            depth_stale_threshold_ms: 7_500,
            ..InstrumentConfig::default()
        }
    }

    fn requirement<'a>(
        requirements: &'a ChaosDecisionRequirements,
        id: ChaosDecisionRequirementId,
        symbol: Option<&str>,
    ) -> &'a ChaosDecisionInputRequirement {
        requirements
            .inputs()
            .iter()
            .find(|item| item.requirement_id() == id && item.input().symbol() == symbol)
            .unwrap_or_else(|| panic!("missing {} requirement for {symbol:?}", id.as_str()))
    }

    #[test]
    fn spot_requires_book_trade_price_limits_and_timer_only() {
        let config = ChaosConfig {
            instruments: vec![instrument("BTC-USDT", InstrumentKindConfig::Spot, None)],
            ..ChaosConfig::default()
        };

        let requirements = config.decision_requirements();

        assert_eq!(requirements.inputs().len(), 4);
        assert!(matches!(
            requirement(
                &requirements,
                ChaosDecisionRequirementId::MarketBook,
                Some("BTC-USDT")
            )
            .input(),
            ChaosDecisionInput::Book {
                max_age_ms: 7_500,
                ..
            }
        ));
        requirement(
            &requirements,
            ChaosDecisionRequirementId::MarketTrade,
            Some("BTC-USDT"),
        );
        requirement(
            &requirements,
            ChaosDecisionRequirementId::ReferencePriceLimits,
            Some("BTC-USDT"),
        );
        requirement(&requirements, ChaosDecisionRequirementId::Timer, None);
        assert!(!requirements.inputs().iter().any(|item| matches!(
            item.requirement_id(),
            ChaosDecisionRequirementId::ReferenceFunding
                | ChaosDecisionRequirementId::ReferenceMark
                | ChaosDecisionRequirementId::ReferenceIndex
        )));
    }

    #[test]
    fn linear_swap_requires_all_configured_references() {
        let config = ChaosConfig {
            reference_data_stale_threshold_ms: Some(2_000),
            instruments: vec![instrument(
                "BTC-USDT-SWAP",
                InstrumentKindConfig::LinearSwap,
                Some("BTC-USDT-INDEX"),
            )],
            ..ChaosConfig::default()
        };

        let requirements = config.decision_requirements();

        for (id, symbol) in [
            (
                ChaosDecisionRequirementId::ReferenceFunding,
                "BTC-USDT-SWAP",
            ),
            (ChaosDecisionRequirementId::ReferenceMark, "BTC-USDT-SWAP"),
            (
                ChaosDecisionRequirementId::ReferencePriceLimits,
                "BTC-USDT-SWAP",
            ),
            (ChaosDecisionRequirementId::ReferenceIndex, "BTC-USDT-INDEX"),
        ] {
            assert!(matches!(
                requirement(&requirements, id, Some(symbol)).input(),
                ChaosDecisionInput::Reference {
                    max_age_ms: Some(2_000),
                    ..
                }
            ));
        }
    }

    #[test]
    fn inverse_future_needs_mark_and_limits_but_not_funding() {
        let config = ChaosConfig {
            instruments: vec![instrument(
                "BTC-USD-261225",
                InstrumentKindConfig::InverseFuture,
                None,
            )],
            ..ChaosConfig::default()
        };

        let requirements = config.decision_requirements();

        requirement(
            &requirements,
            ChaosDecisionRequirementId::ReferenceMark,
            Some("BTC-USD-261225"),
        );
        requirement(
            &requirements,
            ChaosDecisionRequirementId::ReferencePriceLimits,
            Some("BTC-USD-261225"),
        );
        assert!(
            !requirements.inputs().iter().any(|item| {
                item.requirement_id() == ChaosDecisionRequirementId::ReferenceFunding
            })
        );
    }

    #[test]
    fn inverse_swap_requires_funding() {
        let config = ChaosConfig {
            instruments: vec![instrument(
                "BTC-USD-SWAP",
                InstrumentKindConfig::InverseSwap,
                None,
            )],
            ..ChaosConfig::default()
        };

        let requirements = config.decision_requirements();

        requirement(
            &requirements,
            ChaosDecisionRequirementId::ReferenceFunding,
            Some("BTC-USD-SWAP"),
        );
    }

    #[test]
    fn requirements_are_sorted_deduplicated_and_have_typed_consumers() {
        let instruments = vec![
            instrument(
                "ETH-USDT-SWAP",
                InstrumentKindConfig::LinearSwap,
                Some("SHARED-INDEX"),
            ),
            instrument(
                "BTC-USDT-SWAP",
                InstrumentKindConfig::LinearSwap,
                Some("SHARED-INDEX"),
            ),
        ];
        let config = ChaosConfig {
            instruments: instruments.clone(),
            ..ChaosConfig::default()
        };
        let reversed = ChaosConfig {
            instruments: instruments.into_iter().rev().collect(),
            ..ChaosConfig::default()
        };

        let requirements = config.decision_requirements();
        assert_eq!(requirements, reversed.decision_requirements());
        assert!(requirements.inputs().windows(2).all(|items| {
            (items[0].requirement_id(), items[0].input())
                <= (items[1].requirement_id(), items[1].input())
        }));
        assert_eq!(
            requirements
                .inputs()
                .iter()
                .filter(|item| {
                    item.requirement_id() == ChaosDecisionRequirementId::ReferenceIndex
                        && item.input().symbol() == Some("SHARED-INDEX")
                })
                .count(),
            1
        );
        for item in requirements.inputs() {
            let expected_consumer = match item.requirement_id() {
                ChaosDecisionRequirementId::MarketBook => {
                    ChaosDecisionConsumer::QuoteAndHedgeCalculations
                }
                ChaosDecisionRequirementId::MarketTrade => {
                    ChaosDecisionConsumer::ImpliedDepthAndRepricing
                }
                ChaosDecisionRequirementId::ReferenceIndex => {
                    ChaosDecisionConsumer::IndexDeviationValuationAndPricing
                }
                ChaosDecisionRequirementId::ReferenceFunding => {
                    ChaosDecisionConsumer::FundingAwarePricingAndRisk
                }
                ChaosDecisionRequirementId::ReferenceMark => {
                    ChaosDecisionConsumer::DerivativeValuationAndSafety
                }
                ChaosDecisionRequirementId::ReferencePriceLimits => {
                    ChaosDecisionConsumer::QuoteAndHedgePriceBounds
                }
                ChaosDecisionRequirementId::Timer => ChaosDecisionConsumer::TimeBasedStrategyLogic,
            };
            assert_eq!(item.consumer(), expected_consumer);
            assert!(!item.requirement_id().as_str().is_empty());
            assert!(!item.consumer().as_str().is_empty());
        }
    }
}
