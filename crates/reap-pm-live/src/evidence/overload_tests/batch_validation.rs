use std::process::Command;

use reap_benchmark_allocator::AllocationSnapshot;
use reap_pm_core::{
    PmFillEvent, PmFillExecution, PmFillFee, PmFillId, PmFillKey, PmFillRole,
    PmFillSettlementStatus, PmOrderIdentity, PmOrderSide, PmPrice, PmQuantity, PmVenueOrderId,
    PmVenueOrderKey,
};
use reap_polymarket_adapter::PmPrivateLifecycleObservation;

use crate::private_monitor::PmPrivateBatchValidationProbe;

const ALLOCATION_CHILD_ENV: &str = "REAP_PRIVATE_BATCH_VALIDATION_ALLOCATION_CHILD";
const ALLOCATION_TEST: &str = "evidence::overload_tests::batch_validation::repeated_private_batch_validation_is_allocation_free";

fn observation() -> PmPrivateLifecycleObservation {
    let config = super::super::connectivity_config();
    let account = config.account();
    let venue = PmVenueOrderKey::new(
        account.account_scope().handle(),
        PmVenueOrderId::new("allocation-order").expect("valid venue order"),
    );
    PmPrivateLifecycleObservation::Fill(
        PmFillEvent::new(
            account.account_route().source(),
            account.instrument(),
            PmFillKey::new(
                venue,
                PmFillId::new("allocation-fill").expect("valid fill id"),
            ),
            PmOrderIdentity::new(None, Some(venue)).expect("valid fill order"),
            PmFillExecution::new(
                PmOrderSide::Buy,
                PmFillRole::Maker,
                PmFillSettlementStatus::Matched,
                PmPrice::parse_decimal("0.40").expect("valid price"),
                PmQuantity::parse_decimal("5").expect("valid quantity"),
                PmFillFee::Unknown,
            ),
        )
        .expect("valid fill event"),
    )
}

#[test]
fn repeated_private_batch_validation_is_allocation_free() {
    if std::env::var_os(ALLOCATION_CHILD_ENV).is_none() {
        let status = Command::new(std::env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg(ALLOCATION_TEST)
            .arg("--test-threads=1")
            .env(ALLOCATION_CHILD_ENV, "1")
            .status()
            .expect("isolated allocation child starts");
        assert!(status.success(), "isolated allocation test failed");
        return;
    }

    let mut probe = PmPrivateBatchValidationProbe::new(observation());
    assert!(probe.exercise(), "warm batch validation");
    let window = reap_benchmark_allocator::start_measurement().expect("exclusive measurement");
    for _ in 0..1_000 {
        assert!(probe.exercise(), "reused batch validation");
    }
    assert_eq!(
        window.stop().expect("stop allocation measurement"),
        AllocationSnapshot::default()
    );
}
