use reap_durable_writer::DurableAcknowledgement;

fn replay(acknowledgement: DurableAcknowledgement) {
    let _first = acknowledgement.clone();
    let _second = acknowledgement;
}

fn main() {}
