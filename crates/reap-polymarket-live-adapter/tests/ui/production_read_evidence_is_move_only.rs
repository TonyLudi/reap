use reap_polymarket_live_adapter::{
    PmProductionClobLivenessHealthObservation, PmProductionStatusAnnouncementObservation,
};

fn requires_clone<T: Clone>() {}

fn main() {
    requires_clone::<PmProductionClobLivenessHealthObservation>();
    requires_clone::<PmProductionStatusAnnouncementObservation>();
}
