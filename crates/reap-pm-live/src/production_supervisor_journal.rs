//! Durable recovery journal owned by the continuous production supervisor.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use reap_durable_writer::DurableLease;
use reap_pm_core::{PmOrderSide, U256};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt as _;

use crate::production_supervisor::{
    MAX_PM_SUPERVISOR_FILLS, MAX_PM_SUPERVISOR_ORDERS, PmSupervisorFill,
    PmSupervisorMutationClassification, PmSupervisorOrderFacts, PmSupervisorScope,
};

const MAX_PM_SUPERVISOR_JOURNAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PM_SUPERVISOR_JOURNAL_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PmSupervisorJournalRecord {
    Header {
        schema_version: u8,
        scope: PmSupervisorScope,
    },
    PositionBaseline {
        token_id: String,
        quantity: U256,
    },
    PlaceIntent {
        facts: PmSupervisorOrderFacts,
    },
    PlaceResult {
        expected_venue_order_id: String,
        classification: PmSupervisorMutationClassification,
    },
    CancelIntent {
        venue_order_id: String,
    },
    CancelResult {
        venue_order_id: String,
        classification: PmSupervisorMutationClassification,
    },
    FillApplied {
        fill: PmSupervisorJournalFill,
    },
    PollReconciled {
        sequence: u64,
    },
    CleanShutdown {
        terminal_poll_sequence: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmSupervisorJournalFill {
    fill_id: String,
    venue_order_id: String,
    token_id: String,
    #[serde(with = "crate::production_supervisor_serde")]
    side: PmOrderSide,
    quantity: U256,
}

impl From<&PmSupervisorFill> for PmSupervisorJournalFill {
    fn from(fill: &PmSupervisorFill) -> Self {
        Self {
            fill_id: fill.fill_id.clone(),
            venue_order_id: fill.venue_order_id.clone(),
            token_id: fill.token_id.clone(),
            side: fill.side,
            quantity: fill.quantity,
        }
    }
}

impl From<PmSupervisorJournalFill> for PmSupervisorFill {
    fn from(fill: PmSupervisorJournalFill) -> Self {
        Self {
            fill_id: fill.fill_id,
            venue_order_id: fill.venue_order_id,
            token_id: fill.token_id,
            side: fill.side,
            quantity: fill.quantity,
        }
    }
}

#[derive(Debug)]
pub struct PmSupervisorJournalRecovery {
    pub(crate) records: Box<[PmSupervisorJournalRecord]>,
}

impl PmSupervisorJournalRecovery {
    #[must_use]
    pub fn records(&self) -> &[PmSupervisorJournalRecord] {
        &self.records
    }
}

#[derive(Debug, Error)]
pub enum PmSupervisorJournalError {
    #[error("supervisor journal path or existing contents are invalid")]
    Invalid,
    #[error("supervisor journal I/O failed")]
    Io,
    #[error("supervisor journal serialization failed")]
    Serialization,
}

pub struct PmSupervisorJournal {
    path: PathBuf,
    file: tokio::fs::File,
    _lease: DurableLease,
}

impl fmt::Debug for PmSupervisorJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmSupervisorJournal")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PmSupervisorJournal {
    pub async fn open(
        path: PathBuf,
        scope: &PmSupervisorScope,
    ) -> Result<(Self, PmSupervisorJournalRecovery), PmSupervisorJournalError> {
        let lease = DurableLease::acquire(&path).map_err(|_| PmSupervisorJournalError::Invalid)?;
        let path = lease.journal_path().to_path_buf();
        let records = read_journal(&path, scope)?;
        let exists = path.exists();
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|_| PmSupervisorJournalError::Io)?;
        #[cfg(unix)]
        std::fs::set_permissions(&path, private_permissions())
            .map_err(|_| PmSupervisorJournalError::Io)?;
        let mut journal = Self {
            path,
            file: tokio::fs::File::from_std(file),
            _lease: lease,
        };
        if !exists {
            journal
                .append_durable(&PmSupervisorJournalRecord::Header {
                    schema_version: 1,
                    scope: scope.clone(),
                })
                .await?;
            sync_parent_directory(&journal.path)?;
        }
        Ok((
            journal,
            PmSupervisorJournalRecovery {
                records: records.into_boxed_slice(),
            },
        ))
    }

    pub(crate) async fn append_durable(
        &mut self,
        record: &PmSupervisorJournalRecord,
    ) -> Result<(), PmSupervisorJournalError> {
        let mut bytes =
            serde_json::to_vec(record).map_err(|_| PmSupervisorJournalError::Serialization)?;
        if bytes.len() > MAX_PM_SUPERVISOR_JOURNAL_LINE_BYTES {
            return Err(PmSupervisorJournalError::Serialization);
        }
        bytes.push(b'\n');
        self.file
            .write_all(&bytes)
            .await
            .map_err(|_| PmSupervisorJournalError::Io)?;
        self.file
            .flush()
            .await
            .map_err(|_| PmSupervisorJournalError::Io)?;
        self.file
            .sync_data()
            .await
            .map_err(|_| PmSupervisorJournalError::Io)
    }
}

fn sync_parent_directory(path: &Path) -> Result<(), PmSupervisorJournalError> {
    let parent = path.parent().ok_or(PmSupervisorJournalError::Invalid)?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| PmSupervisorJournalError::Io)
}

#[cfg(unix)]
fn private_permissions() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::Permissions::from_mode(0o600)
}

fn read_journal(
    path: &Path,
    scope: &PmSupervisorScope,
) -> Result<Vec<PmSupervisorJournalRecord>, PmSupervisorJournalError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let metadata = std::fs::metadata(path).map_err(|_| PmSupervisorJournalError::Io)?;
    if !metadata.is_file() || metadata.len() > MAX_PM_SUPERVISOR_JOURNAL_BYTES {
        return Err(PmSupervisorJournalError::Invalid);
    }
    let bytes = std::fs::read(path).map_err(|_| PmSupervisorJournalError::Io)?;
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Err(PmSupervisorJournalError::Invalid);
    }
    let mut records = Vec::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        if line.len() > MAX_PM_SUPERVISOR_JOURNAL_LINE_BYTES {
            return Err(PmSupervisorJournalError::Invalid);
        }
        let record = serde_json::from_slice::<PmSupervisorJournalRecord>(line)
            .map_err(|_| PmSupervisorJournalError::Invalid)?;
        records.push(record);
        if records.len() > MAX_PM_SUPERVISOR_FILLS + MAX_PM_SUPERVISOR_ORDERS * 4 + 4_096 {
            return Err(PmSupervisorJournalError::Invalid);
        }
    }
    if records
        .iter()
        .skip(1)
        .any(|record| matches!(record, PmSupervisorJournalRecord::Header { .. }))
    {
        return Err(PmSupervisorJournalError::Invalid);
    }
    match records.first() {
        Some(PmSupervisorJournalRecord::Header {
            schema_version: 1,
            scope: observed,
        }) if observed == scope => Ok(records),
        _ => Err(PmSupervisorJournalError::Invalid),
    }
}
