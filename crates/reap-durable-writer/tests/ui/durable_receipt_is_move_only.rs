use reap_durable_writer::DurableReceipt;

fn replay(receipt: DurableReceipt) {
    let _first = receipt.clone();
    let _second = receipt;
}

fn main() {}
