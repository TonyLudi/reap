use reap_durable_writer::DurableAcknowledgement;

fn forge() -> DurableAcknowledgement {
    DurableAcknowledgement { _private: () }
}

fn main() {}
