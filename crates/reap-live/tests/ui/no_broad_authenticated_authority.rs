use reap_okx_emergency_adapter as emergency_authority;
use reap_okx_evidence_adapter as evidence_authority;
use reap_okx_wire as raw_wire_authority;

fn main() {
    let _ = std::any::type_name::<emergency_authority::EmergencyAccountStop>();
    let _ = std::any::type_name::<evidence_authority::EvidenceCollector>();
    let _ = std::any::type_name::<raw_wire_authority::Client<()>>();
}
