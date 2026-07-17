use reap_live as live_runtime;
use reap_okx_emergency_adapter as emergency_adapter;
use reap_okx_evidence_adapter as evidence_adapter;
use reap_okx_live_adapter as live_adapter;
use reap_okx_wire as wire;
use reap_venue as venue;

fn main() {
    let _ = std::any::type_name::<live_runtime::LiveRuntime>();
    let _ = std::any::type_name::<emergency_adapter::EmergencyAccountStop>();
    let _ = std::any::type_name::<evidence_adapter::EvidenceCollector>();
    let _ = std::any::type_name::<live_adapter::ObserveRoles>();
    let _ = std::any::type_name::<wire::Client<()>>();
    let _ = std::any::type_name::<venue::okx::OkxRestClient<()>>();
}
