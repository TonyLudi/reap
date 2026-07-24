use std::future::Future;

#[global_allocator]
static PHASE6_OVERLOAD_ALLOCATOR: reap_benchmark_allocator::TrackingAllocator =
    reap_benchmark_allocator::TrackingAllocator;

mod batch_validation;
mod reached_capture;
mod reached_mutation_support;
mod reached_persistence;
mod reached_private_age;
mod reached_reconciliation;
mod reached_refresh;
mod reached_scheduler;
mod reached_storage;

fn run_product_test<F, Fut>(test: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("phase6 overload runtime")
        .block_on(test());
}
