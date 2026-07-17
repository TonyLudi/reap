#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LiveCleanSoakInputs {
    pub(crate) duration_elapsed: bool,
    pub(crate) reached_ready: bool,
    pub(crate) readiness_at_stop_ready: bool,
    pub(crate) reconciliation_drift_free: bool,
    pub(crate) operator_mutation_free: bool,
    pub(crate) storage_records_complete: bool,
    pub(crate) no_active_orders_after_shutdown: bool,
    pub(crate) alert_delivery_failure_free: bool,
}

impl LiveCleanSoakInputs {
    pub(crate) const fn qualifies_as_clean_soak(self) -> bool {
        self.duration_elapsed
            && self.reached_ready
            && self.readiness_at_stop_ready
            && self.reconciliation_drift_free
            && self.operator_mutation_free
            && self.storage_records_complete
            && self.no_active_orders_after_shutdown
            && self.alert_delivery_failure_free
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveFaultFailureCode {
    DeadmanHeartbeat,
    ExchangeClockSkew,
    ExchangeClockCheck,
    ExchangeStatus,
    ExchangeStatusCheck,
    ExchangeFeeDrift,
    ExchangeFeeCheck,
    ExchangeInstrumentDrift,
    ExchangeInstrumentCheck,
    AccountConfigDrift,
    AccountConfigCheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveFaultFailureClass {
    DeadmanHeartbeat,
    ExchangeClock,
    ExchangeStatus,
    ExchangeFee,
    ExchangeInstrument,
    AccountConfig,
}

impl LiveFaultFailureCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::DeadmanHeartbeat => "deadman_heartbeat",
            Self::ExchangeClockSkew => "exchange_clock_skew",
            Self::ExchangeClockCheck => "exchange_clock_check",
            Self::ExchangeStatus => "exchange_status",
            Self::ExchangeStatusCheck => "exchange_status_check",
            Self::ExchangeFeeDrift => "exchange_fee_drift",
            Self::ExchangeFeeCheck => "exchange_fee_check",
            Self::ExchangeInstrumentDrift => "exchange_instrument_drift",
            Self::ExchangeInstrumentCheck => "exchange_instrument_check",
            Self::AccountConfigDrift => "account_config_drift",
            Self::AccountConfigCheck => "account_config_check",
        }
    }

    pub(crate) fn parse(code: &str) -> Option<Self> {
        match code {
            "deadman_heartbeat" => Some(Self::DeadmanHeartbeat),
            "exchange_clock_skew" => Some(Self::ExchangeClockSkew),
            "exchange_clock_check" => Some(Self::ExchangeClockCheck),
            "exchange_status" => Some(Self::ExchangeStatus),
            "exchange_status_check" => Some(Self::ExchangeStatusCheck),
            "exchange_fee_drift" => Some(Self::ExchangeFeeDrift),
            "exchange_fee_check" => Some(Self::ExchangeFeeCheck),
            "exchange_instrument_drift" => Some(Self::ExchangeInstrumentDrift),
            "exchange_instrument_check" => Some(Self::ExchangeInstrumentCheck),
            "account_config_drift" => Some(Self::AccountConfigDrift),
            "account_config_check" => Some(Self::AccountConfigCheck),
            _ => None,
        }
    }

    pub(crate) const fn class(self) -> LiveFaultFailureClass {
        match self {
            Self::DeadmanHeartbeat => LiveFaultFailureClass::DeadmanHeartbeat,
            Self::ExchangeClockSkew | Self::ExchangeClockCheck => {
                LiveFaultFailureClass::ExchangeClock
            }
            Self::ExchangeStatus | Self::ExchangeStatusCheck => {
                LiveFaultFailureClass::ExchangeStatus
            }
            Self::ExchangeFeeDrift | Self::ExchangeFeeCheck => LiveFaultFailureClass::ExchangeFee,
            Self::ExchangeInstrumentDrift | Self::ExchangeInstrumentCheck => {
                LiveFaultFailureClass::ExchangeInstrument
            }
            Self::AccountConfigDrift | Self::AccountConfigCheck => {
                LiveFaultFailureClass::AccountConfig
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAULT_FAILURE_CASES: [(LiveFaultFailureCode, &str, LiveFaultFailureClass); 11] = [
        (
            LiveFaultFailureCode::DeadmanHeartbeat,
            "deadman_heartbeat",
            LiveFaultFailureClass::DeadmanHeartbeat,
        ),
        (
            LiveFaultFailureCode::ExchangeClockSkew,
            "exchange_clock_skew",
            LiveFaultFailureClass::ExchangeClock,
        ),
        (
            LiveFaultFailureCode::ExchangeClockCheck,
            "exchange_clock_check",
            LiveFaultFailureClass::ExchangeClock,
        ),
        (
            LiveFaultFailureCode::ExchangeStatus,
            "exchange_status",
            LiveFaultFailureClass::ExchangeStatus,
        ),
        (
            LiveFaultFailureCode::ExchangeStatusCheck,
            "exchange_status_check",
            LiveFaultFailureClass::ExchangeStatus,
        ),
        (
            LiveFaultFailureCode::ExchangeFeeDrift,
            "exchange_fee_drift",
            LiveFaultFailureClass::ExchangeFee,
        ),
        (
            LiveFaultFailureCode::ExchangeFeeCheck,
            "exchange_fee_check",
            LiveFaultFailureClass::ExchangeFee,
        ),
        (
            LiveFaultFailureCode::ExchangeInstrumentDrift,
            "exchange_instrument_drift",
            LiveFaultFailureClass::ExchangeInstrument,
        ),
        (
            LiveFaultFailureCode::ExchangeInstrumentCheck,
            "exchange_instrument_check",
            LiveFaultFailureClass::ExchangeInstrument,
        ),
        (
            LiveFaultFailureCode::AccountConfigDrift,
            "account_config_drift",
            LiveFaultFailureClass::AccountConfig,
        ),
        (
            LiveFaultFailureCode::AccountConfigCheck,
            "account_config_check",
            LiveFaultFailureClass::AccountConfig,
        ),
    ];

    #[test]
    fn clean_soak_truth_table_requires_all_eight_conditions() {
        for mask in 0_u16..=u8::MAX.into() {
            let inputs = LiveCleanSoakInputs {
                duration_elapsed: mask & (1 << 0) != 0,
                reached_ready: mask & (1 << 1) != 0,
                readiness_at_stop_ready: mask & (1 << 2) != 0,
                reconciliation_drift_free: mask & (1 << 3) != 0,
                operator_mutation_free: mask & (1 << 4) != 0,
                storage_records_complete: mask & (1 << 5) != 0,
                no_active_orders_after_shutdown: mask & (1 << 6) != 0,
                alert_delivery_failure_free: mask & (1 << 7) != 0,
            };

            assert_eq!(
                inputs.qualifies_as_clean_soak(),
                mask == u16::from(u8::MAX),
                "unexpected clean-soak classification for condition mask {mask:#010b}"
            );
        }
    }

    #[test]
    fn fault_failure_codes_round_trip_with_their_exact_class() {
        for (code, serialized, class) in FAULT_FAILURE_CASES {
            assert_eq!(code.as_str(), serialized);
            assert_eq!(LiveFaultFailureCode::parse(serialized), Some(code));
            assert_eq!(code.class(), class);
        }
        assert_eq!(LiveFaultFailureCode::parse("runtime_failure"), None);
    }
}
