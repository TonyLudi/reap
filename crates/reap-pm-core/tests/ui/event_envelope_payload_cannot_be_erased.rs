use reap_pm_core::EventEnvelope;

struct MarketPayload;
struct OrderPayload;

fn payload_types_do_not_interchange(
    envelope: EventEnvelope<MarketPayload>,
) -> EventEnvelope<OrderPayload> {
    envelope
}

fn no_erase_escape(envelope: EventEnvelope<MarketPayload>) {
    let _ = envelope.erase();
}

fn no_payload_mapping_escape(envelope: EventEnvelope<MarketPayload>) {
    let _ = envelope.map_payload(|_| OrderPayload);
}

fn main() {}
