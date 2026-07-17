#[derive(Debug, Clone, Copy)]
pub(crate) struct CaptureCleanRunInputs {
    pub(crate) duration_elapsed: bool,
    pub(crate) failure_free: bool,
    pub(crate) reached_all_connections_ready: bool,
    pub(crate) connections_ready_at_stop: bool,
    pub(crate) books_ready: bool,
    pub(crate) stream_coverage_complete: bool,
    pub(crate) raw_records_present: bool,
    pub(crate) raw_record_sequence_complete: bool,
    pub(crate) normalized_records_present_or_disabled: bool,
    pub(crate) parse_clean: bool,
    pub(crate) no_stale_book_events: bool,
    pub(crate) no_recovery_requests: bool,
    pub(crate) no_missing_recovery_routes: bool,
    pub(crate) no_gaps: bool,
    pub(crate) no_recovery_failures: bool,
    pub(crate) session_bounds_valid: bool,
    pub(crate) executable_sha256_valid: bool,
    pub(crate) host_evidence_healthy: bool,
}

pub(crate) fn capture_run_is_clean(inputs: &CaptureCleanRunInputs) -> bool {
    inputs.duration_elapsed
        && inputs.failure_free
        && inputs.reached_all_connections_ready
        && inputs.connections_ready_at_stop
        && inputs.books_ready
        && inputs.stream_coverage_complete
        && inputs.raw_records_present
        && inputs.raw_record_sequence_complete
        && inputs.normalized_records_present_or_disabled
        && inputs.parse_clean
        && inputs.no_stale_book_events
        && inputs.no_recovery_requests
        && inputs.no_missing_recovery_routes
        && inputs.no_gaps
        && inputs.no_recovery_failures
        && inputs.session_bounds_valid
        && inputs.executable_sha256_valid
        && inputs.host_evidence_healthy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_inputs() -> CaptureCleanRunInputs {
        CaptureCleanRunInputs {
            duration_elapsed: true,
            failure_free: true,
            reached_all_connections_ready: true,
            connections_ready_at_stop: true,
            books_ready: true,
            stream_coverage_complete: true,
            raw_records_present: true,
            raw_record_sequence_complete: true,
            normalized_records_present_or_disabled: true,
            parse_clean: true,
            no_stale_book_events: true,
            no_recovery_requests: true,
            no_missing_recovery_routes: true,
            no_gaps: true,
            no_recovery_failures: true,
            session_bounds_valid: true,
            executable_sha256_valid: true,
            host_evidence_healthy: true,
        }
    }

    #[test]
    fn clean_run_truth_table_requires_every_input() {
        assert!(capture_run_is_clean(&valid_inputs()));

        let breakers: [fn(&mut CaptureCleanRunInputs); 18] = [
            |inputs| inputs.duration_elapsed = false,
            |inputs| inputs.failure_free = false,
            |inputs| inputs.reached_all_connections_ready = false,
            |inputs| inputs.connections_ready_at_stop = false,
            |inputs| inputs.books_ready = false,
            |inputs| inputs.stream_coverage_complete = false,
            |inputs| inputs.raw_records_present = false,
            |inputs| inputs.raw_record_sequence_complete = false,
            |inputs| inputs.normalized_records_present_or_disabled = false,
            |inputs| inputs.parse_clean = false,
            |inputs| inputs.no_stale_book_events = false,
            |inputs| inputs.no_recovery_requests = false,
            |inputs| inputs.no_missing_recovery_routes = false,
            |inputs| inputs.no_gaps = false,
            |inputs| inputs.no_recovery_failures = false,
            |inputs| inputs.session_bounds_valid = false,
            |inputs| inputs.executable_sha256_valid = false,
            |inputs| inputs.host_evidence_healthy = false,
        ];
        for break_input in breakers {
            let mut inputs = valid_inputs();
            break_input(&mut inputs);
            assert!(!capture_run_is_clean(&inputs));
        }
    }
}
