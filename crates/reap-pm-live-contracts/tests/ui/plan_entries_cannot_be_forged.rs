use reap_pm_live_contracts::{
    PmCapabilityLane, PmConnectionRoute, PmPlanEntry, PmPlanOwner, PmReadinessDependency,
    PmRequirementConsumer, PmRequirementKey, PmRequirementOrigin,
};

fn forge(
    key: PmRequirementKey,
    origin: PmRequirementOrigin,
    consumer: PmRequirementConsumer,
    owner: PmPlanOwner,
    lane: PmCapabilityLane,
    readiness: PmReadinessDependency,
    route: Option<PmConnectionRoute>,
) -> PmPlanEntry {
    PmPlanEntry {
        key,
        origin,
        consumer,
        owner,
        lane,
        readiness,
        route,
    }
}

fn main() {}
