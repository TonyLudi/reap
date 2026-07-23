use reap_pm_core::{PmAccountHandle, PmFillId, PmFillKey, PmVenueOrderId, PmVenueOrderKey};

fn main() {
    let order = PmVenueOrderKey::new(
        PmAccountHandle::from_ordinal(1),
        PmVenueOrderId::new("venue-order").unwrap(),
    );
    let key = PmFillKey::new(order, PmFillId::new("fill").unwrap());

    let _ = serde_json::to_string(&key).unwrap();
}
