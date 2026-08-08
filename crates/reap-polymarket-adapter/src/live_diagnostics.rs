use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_PM_LIVE_FOREIGN_DIAGNOSTIC_ROWS: usize = reap_pm_core::MAX_PM_RECONCILIATION_FILLS;

const DIAGNOSTIC_SET_DOMAIN: &[u8] = b"reap.pm.live.foreign-diagnostics.v1\0";

/// Secret-free, bounded evidence that authenticated account-wide input also
/// contained rows outside the configured reducer scope.
///
/// The digest has equality semantics only. Credential-owner identities, raw
/// bodies, transport cursors, timestamps, transaction hashes, and local
/// client-order associations never enter it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmForeignRowDiagnostics {
    count: usize,
    digest: [u8; 32],
}

impl PmForeignRowDiagnostics {
    pub(crate) const fn fixture_empty() -> Self {
        Self {
            count: 0,
            digest: [0; 32],
        }
    }

    #[must_use]
    pub const fn count(self) -> usize {
        self.count
    }

    #[must_use]
    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmForeignDiagnosticError {
    #[error("live PM foreign-row diagnostics exceed their fixed bound")]
    TooManyRows,
    #[error("one exact live PM foreign-row identity carries conflicting facts")]
    ConflictingRow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct DiagnosticRow {
    key: [u8; 32],
    facts: [u8; 32],
}

pub(crate) struct ForeignDiagnosticsBuilder {
    domain: &'static [u8],
    maximum: usize,
    rows: Vec<DiagnosticRow>,
}

impl ForeignDiagnosticsBuilder {
    pub(crate) fn new(domain: &'static [u8], maximum: usize) -> Self {
        Self {
            domain,
            maximum: maximum.min(MAX_PM_LIVE_FOREIGN_DIAGNOSTIC_ROWS),
            rows: Vec::new(),
        }
    }

    pub(crate) fn push(
        &mut self,
        key: [u8; 32],
        facts: [u8; 32],
    ) -> Result<(), PmForeignDiagnosticError> {
        if self.rows.len() == self.maximum {
            return Err(PmForeignDiagnosticError::TooManyRows);
        }
        self.rows.push(DiagnosticRow { key, facts });
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<PmForeignRowDiagnostics, PmForeignDiagnosticError> {
        self.rows.sort_unstable();
        let mut output = 0;
        for input in 0..self.rows.len() {
            let row = self.rows[input];
            if output != 0 && self.rows[output - 1].key == row.key {
                if self.rows[output - 1].facts != row.facts {
                    return Err(PmForeignDiagnosticError::ConflictingRow);
                }
                continue;
            }
            self.rows[output] = row;
            output += 1;
        }
        self.rows.truncate(output);

        let mut digest = Sha256::new();
        digest.update(DIAGNOSTIC_SET_DOMAIN);
        encode_bytes(&mut digest, self.domain);
        digest.update(
            u32::try_from(self.rows.len())
                .expect("bounded live diagnostic count fits u32")
                .to_be_bytes(),
        );
        for row in self.rows {
            digest.update(row.key);
            digest.update(row.facts);
        }
        Ok(PmForeignRowDiagnostics {
            count: output,
            digest: digest.finalize().into(),
        })
    }
}

pub(crate) fn semantic_hash(domain: &'static [u8], encode: impl FnOnce(&mut Sha256)) -> [u8; 32] {
    let mut digest = Sha256::new();
    encode_bytes(&mut digest, domain);
    encode(&mut digest);
    digest.finalize().into()
}

pub(crate) fn encode_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u32::try_from(value.len())
            .expect("bounded diagnostic field length fits u32")
            .to_be_bytes(),
    );
    digest.update(value);
}

pub(crate) fn encode_ascii(digest: &mut Sha256, value: &str) {
    encode_bytes(digest, value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_order_invariant_duplicate_convergent_and_conflict_closed() {
        let key_a = semantic_hash(b"key\0", |digest| digest.update([1]));
        let key_b = semantic_hash(b"key\0", |digest| digest.update([2]));
        let fact_a = semantic_hash(b"fact\0", |digest| digest.update([3]));
        let fact_b = semantic_hash(b"fact\0", |digest| digest.update([4]));

        let mut first = ForeignDiagnosticsBuilder::new(b"test\0", 4);
        first.push(key_b, fact_b).unwrap();
        first.push(key_a, fact_a).unwrap();
        first.push(key_a, fact_a).unwrap();
        let first = first.finish().unwrap();

        let mut reordered = ForeignDiagnosticsBuilder::new(b"test\0", 4);
        reordered.push(key_a, fact_a).unwrap();
        reordered.push(key_b, fact_b).unwrap();
        assert_eq!(first, reordered.finish().unwrap());
        assert_eq!(first.count(), 2);

        let mut conflict = ForeignDiagnosticsBuilder::new(b"test\0", 4);
        conflict.push(key_a, fact_a).unwrap();
        conflict.push(key_a, fact_b).unwrap();
        assert_eq!(
            conflict.finish(),
            Err(PmForeignDiagnosticError::ConflictingRow)
        );
    }
}
