use reap_polymarket_live_adapter::{
    PmCancelMutationTimeFinalizer, PmCancelMutationTimeProof, PmCancelMutationTimeProvider,
    PmFinalCancelMutationTime, PmFinalPlaceMutationTime, PmMutationTimeProviderError,
    PmPlaceMutationTimeFinalizer, PmPlaceMutationTimeProof, PmPlaceMutationTimeProvider,
};

struct PlaceProvider;

impl PmPlaceMutationTimeProvider for PlaceProvider {
    fn consume_final_place_time(
        &mut self,
        _time: PmFinalPlaceMutationTime<'_>,
    ) -> Result<(), PmMutationTimeProviderError> {
        Ok(())
    }
}

struct CancelProvider;

impl PmCancelMutationTimeProvider for CancelProvider {
    fn consume_final_cancel_time(
        &mut self,
        _time: PmFinalCancelMutationTime<'_>,
    ) -> Result<(), PmMutationTimeProviderError> {
        Ok(())
    }
}

fn place_cannot_consume_cancel(
    mut finalizer: PmPlaceMutationTimeFinalizer,
    proof: PmCancelMutationTimeProof,
) {
    let _ = finalizer.consume_with(proof, &mut PlaceProvider);
}

fn cancel_cannot_consume_place(
    mut finalizer: PmCancelMutationTimeFinalizer,
    proof: PmPlaceMutationTimeProof,
) {
    let _ = finalizer.consume_with(proof, &mut CancelProvider);
}

fn main() {}
