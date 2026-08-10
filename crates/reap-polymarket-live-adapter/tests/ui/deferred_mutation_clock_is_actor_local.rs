use reap_polymarket_live_adapter::PmDeferredMutationClockCapsule;

fn move_to_another_thread(value: PmDeferredMutationClockCapsule) {
    let task = std::thread::spawn(move || drop(value));
    let _ = task.join();
}

fn main() {}
