use serde::{Deserialize, Serialize};

pub const MAX_HOST_GUARD_CHECK_INTERVAL_MS: u64 = 60_000;
pub const PRODUCTION_HOST_GUARD_MAX_CHECK_INTERVAL_MS: u64 = 10_000;
pub const PRODUCTION_HOST_GUARD_MIN_DISK_AVAILABLE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
pub const PRODUCTION_HOST_GUARD_MIN_MEMORY_AVAILABLE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostHealthThresholdAssessment {
    pub disk_low: bool,
    pub memory_low: bool,
    pub clock_unsynchronized: bool,
}

impl HostHealthThresholdAssessment {
    pub fn is_healthy(self) -> bool {
        !self.disk_low && !self.memory_low && !self.clock_unsynchronized
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HostGuardConfig {
    pub enabled: bool,
    pub check_interval_ms: u64,
    pub min_disk_available_bytes: u64,
    pub min_memory_available_bytes: u64,
    pub require_clock_synchronized: bool,
}

impl Default for HostGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            check_interval_ms: 10_000,
            min_disk_available_bytes: 5 * 1024 * 1024 * 1024,
            min_memory_available_bytes: 1024 * 1024 * 1024,
            require_clock_synchronized: true,
        }
    }
}

impl HostGuardConfig {
    pub fn assess_host_health(
        &self,
        disk_available_bytes: u64,
        memory_available_bytes: u64,
        clock_synchronized: bool,
    ) -> HostHealthThresholdAssessment {
        HostHealthThresholdAssessment {
            disk_low: disk_available_bytes < self.min_disk_available_bytes,
            memory_low: memory_available_bytes < self.min_memory_available_bytes,
            clock_unsynchronized: self.require_clock_synchronized && !clock_synchronized,
        }
    }

    pub fn validation_errors(&self, prefix: &str) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut errors = Vec::new();
        for (name, value) in [
            ("check_interval_ms", self.check_interval_ms),
            ("min_disk_available_bytes", self.min_disk_available_bytes),
            (
                "min_memory_available_bytes",
                self.min_memory_available_bytes,
            ),
        ] {
            if value == 0 {
                errors.push(format!("{prefix}.{name} must be positive"));
            }
        }
        if self.check_interval_ms > MAX_HOST_GUARD_CHECK_INTERVAL_MS {
            errors.push(format!(
                "{prefix}.check_interval_ms must not exceed {MAX_HOST_GUARD_CHECK_INTERVAL_MS}"
            ));
        }
        errors
    }

    pub fn production_policy_errors(&self, prefix: &str) -> Vec<String> {
        if !self.enabled {
            return vec![format!("{prefix} must be enabled for production evidence")];
        }
        let mut errors = self.validation_errors(prefix);
        if self.check_interval_ms > PRODUCTION_HOST_GUARD_MAX_CHECK_INTERVAL_MS {
            errors.push(format!(
                "{prefix}.check_interval_ms must not exceed {PRODUCTION_HOST_GUARD_MAX_CHECK_INTERVAL_MS} for production evidence"
            ));
        }
        if self.min_disk_available_bytes < PRODUCTION_HOST_GUARD_MIN_DISK_AVAILABLE_BYTES {
            errors.push(format!(
                "{prefix}.min_disk_available_bytes must be at least {PRODUCTION_HOST_GUARD_MIN_DISK_AVAILABLE_BYTES} for production evidence"
            ));
        }
        if self.min_memory_available_bytes < PRODUCTION_HOST_GUARD_MIN_MEMORY_AVAILABLE_BYTES {
            errors.push(format!(
                "{prefix}.min_memory_available_bytes must be at least {PRODUCTION_HOST_GUARD_MIN_MEMORY_AVAILABLE_BYTES} for production evidence"
            ));
        }
        if !self.require_clock_synchronized {
            errors.push(format!(
                "{prefix}.require_clock_synchronized must be true for production evidence"
            ));
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> HostGuardConfig {
        HostGuardConfig {
            enabled: true,
            check_interval_ms: 1,
            min_disk_available_bytes: 1_000,
            min_memory_available_bytes: 2_000,
            require_clock_synchronized: true,
        }
    }

    #[test]
    fn validates_enabled_thresholds() {
        let mut invalid = config();
        invalid.check_interval_ms = 0;
        assert_eq!(
            invalid.validation_errors("host_guard"),
            vec!["host_guard.check_interval_ms must be positive"]
        );
        invalid.enabled = false;
        assert!(invalid.validation_errors("host_guard").is_empty());

        let mut too_slow = config();
        too_slow.check_interval_ms = MAX_HOST_GUARD_CHECK_INTERVAL_MS + 1;
        assert_eq!(
            too_slow.validation_errors("host_guard"),
            vec!["host_guard.check_interval_ms must not exceed 60000"]
        );
    }

    #[test]
    fn production_policy_rejects_weak_or_disabled_guards() {
        assert_eq!(
            HostGuardConfig::default().production_policy_errors("host_guard"),
            vec!["host_guard must be enabled for production evidence"]
        );

        let mut production = HostGuardConfig {
            enabled: true,
            ..HostGuardConfig::default()
        };
        assert!(production.production_policy_errors("host_guard").is_empty());

        production.check_interval_ms = PRODUCTION_HOST_GUARD_MAX_CHECK_INTERVAL_MS + 1;
        production.min_disk_available_bytes = PRODUCTION_HOST_GUARD_MIN_DISK_AVAILABLE_BYTES - 1;
        production.min_memory_available_bytes =
            PRODUCTION_HOST_GUARD_MIN_MEMORY_AVAILABLE_BYTES - 1;
        production.require_clock_synchronized = false;
        let errors = production.production_policy_errors("capture.host_guard");
        assert_eq!(errors.len(), 4);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("must not exceed 10000"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("at least 5368709120"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("at least 1073741824"))
        );
        assert!(errors.iter().any(|error| error.contains("must be true")));
    }

    #[test]
    fn host_health_assessment_truth_table_and_boundaries_are_exact() {
        let config = config();
        for disk_low in [false, true] {
            for memory_low in [false, true] {
                for clock_unsynchronized in [false, true] {
                    let assessment = config.assess_host_health(
                        if disk_low {
                            config.min_disk_available_bytes - 1
                        } else {
                            config.min_disk_available_bytes
                        },
                        if memory_low {
                            config.min_memory_available_bytes - 1
                        } else {
                            config.min_memory_available_bytes
                        },
                        !clock_unsynchronized,
                    );
                    assert_eq!(
                        assessment,
                        HostHealthThresholdAssessment {
                            disk_low,
                            memory_low,
                            clock_unsynchronized,
                        }
                    );
                    assert_eq!(
                        assessment.is_healthy(),
                        !disk_low && !memory_low && !clock_unsynchronized
                    );
                }
            }
        }

        let mut clock_optional = config;
        clock_optional.require_clock_synchronized = false;
        assert!(
            clock_optional
                .assess_host_health(
                    clock_optional.min_disk_available_bytes,
                    clock_optional.min_memory_available_bytes,
                    false,
                )
                .is_healthy()
        );
    }
}
