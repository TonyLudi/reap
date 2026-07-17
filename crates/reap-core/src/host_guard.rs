use serde::{Deserialize, Serialize};

pub const MAX_HOST_GUARD_CHECK_INTERVAL_MS: u64 = 60_000;
pub const PRODUCTION_HOST_GUARD_MAX_CHECK_INTERVAL_MS: u64 = 10_000;
pub const PRODUCTION_HOST_GUARD_MIN_DISK_AVAILABLE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
pub const PRODUCTION_HOST_GUARD_MIN_MEMORY_AVAILABLE_BYTES: u64 = 1024 * 1024 * 1024;

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
}
