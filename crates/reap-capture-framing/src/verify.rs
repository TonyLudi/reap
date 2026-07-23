use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{digest_hex, trim_jsonl_newline};

#[derive(Debug, Error)]
pub enum JsonlVerifyError {
    #[error("invalid input path {path}: {message}")]
    InvalidInputPath { path: PathBuf, message: String },
    #[error("input {path} is too large: {actual} bytes exceeds {limit}")]
    InputTooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("JSONL frame limit for input {path} must be positive")]
    InvalidFrameLimit { path: PathBuf },
    #[error(
        "input {path} record {record} exceeds the {limit} byte frame limit (observed at least {observed_at_least} bytes)"
    )]
    FrameTooLarge {
        path: PathBuf,
        record: u64,
        observed_at_least: usize,
        limit: usize,
    },
    #[error("failed to {operation} input {path}: {source}")]
    ReadInput {
        path: PathBuf,
        operation: VerifyIoOperation,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyIoOperation {
    Open,
    InspectMetadata,
    Read,
}

impl std::fmt::Display for VerifyIoOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::InspectMetadata => "inspect",
            Self::Read => "read",
        })
    }
}

impl JsonlVerifyError {
    pub fn actual_bytes(&self) -> Option<u64> {
        match self {
            Self::InputTooLarge { actual, .. } => Some(*actual),
            Self::FrameTooLarge {
                observed_at_least, ..
            } => Some(*observed_at_least as u64),
            Self::InvalidInputPath { .. }
            | Self::InvalidFrameLimit { .. }
            | Self::ReadInput { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonlFileScan {
    pub source_path: PathBuf,
    pub records: u64,
    pub bytes: u64,
    pub sha256: String,
    pub invalid_records: u64,
    pub first_invalid_record: Option<u64>,
    pub has_trailing_partial_record: bool,
    pub stable_while_reading: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSnapshot {
    bytes: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    change_seconds: i64,
    #[cfg(unix)]
    change_nanoseconds: i64,
}

impl FileSnapshot {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            bytes: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            change_seconds: metadata.ctime(),
            #[cfg(unix)]
            change_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

/// Scans a regular non-symlink JSONL file with a strict per-frame memory bound.
///
/// `max_frame_bytes` includes the trailing newline when present. The validator
/// receives the exact record bytes. The scanner fails before its frame buffer
/// can grow beyond the configured limit.
pub fn scan_jsonl_file_bounded(
    path: &Path,
    max_frame_bytes: usize,
    mut validate_frame: impl FnMut(&[u8]) -> bool,
) -> Result<JsonlFileScan, JsonlVerifyError> {
    scan_jsonl_file_bounded_total(path, max_frame_bytes, u64::MAX, |frame| {
        validate_frame(frame)
    })
}

/// Scans a regular non-symlink JSONL file with strict per-frame and whole-file
/// byte bounds enforced against the same open file.
pub fn scan_jsonl_file_bounded_total(
    path: &Path,
    max_frame_bytes: usize,
    max_total_bytes: u64,
    mut validate_frame: impl FnMut(&[u8]) -> bool,
) -> Result<JsonlFileScan, JsonlVerifyError> {
    if max_frame_bytes == 0 {
        return Err(JsonlVerifyError::InvalidFrameLimit {
            path: path.to_path_buf(),
        });
    }
    let (source_path, file, initial) = open_jsonl_file(path)?;
    if initial.bytes > max_total_bytes {
        return Err(JsonlVerifyError::InputTooLarge {
            path: source_path,
            actual: initial.bytes,
            limit: max_total_bytes,
        });
    }
    let mut reader = BufReader::new(file);
    let mut frame = Vec::with_capacity(max_frame_bytes.min(64 * 1024));
    let mut hasher = Sha256::new();
    let mut records = 0_u64;
    let mut bytes = 0_u64;
    let mut invalid_records = 0_u64;
    let mut first_invalid_record = None;
    let mut has_trailing_partial_record = false;

    loop {
        frame.clear();
        let read = read_frame_bounded(
            &mut reader,
            &mut frame,
            max_frame_bytes,
            &source_path,
            records.saturating_add(1),
        )?;
        if read == 0 {
            break;
        }
        records = records.saturating_add(1);
        let next_bytes = bytes.saturating_add(read as u64);
        if next_bytes > max_total_bytes {
            return Err(JsonlVerifyError::InputTooLarge {
                path: source_path,
                actual: next_bytes,
                limit: max_total_bytes,
            });
        }
        bytes = next_bytes;
        hasher.update(&frame);
        has_trailing_partial_record = !frame.ends_with(b"\n");
        if !validate_frame(&frame) {
            invalid_records = invalid_records.saturating_add(1);
            first_invalid_record.get_or_insert(records);
        }
    }
    let final_snapshot = inspect_file(reader.get_ref(), &source_path)?;

    Ok(JsonlFileScan {
        source_path,
        records,
        bytes,
        sha256: digest_hex(hasher.finalize()),
        invalid_records,
        first_invalid_record,
        has_trailing_partial_record,
        stable_while_reading: bytes == initial.bytes && final_snapshot == initial,
    })
}

/// Entry-count-compatible JSONL scan with no per-frame memory ceiling.
///
/// This exists solely for an existing verification wrapper whose historical
/// inputs had no record-size limit. New paths must call
/// [`scan_jsonl_file_bounded`].
#[cfg(feature = "legacy-reap-capture")]
pub fn scan_jsonl_file_legacy_unbounded(
    path: &Path,
    mut validate_frame: impl FnMut(&[u8]) -> bool,
) -> Result<JsonlFileScan, JsonlVerifyError> {
    let (source_path, file, initial) = open_jsonl_file(path)?;
    let mut reader = BufReader::new(file);
    let mut frame = Vec::new();
    let mut hasher = Sha256::new();
    let mut records = 0_u64;
    let mut bytes = 0_u64;
    let mut invalid_records = 0_u64;
    let mut first_invalid_record = None;
    let mut has_trailing_partial_record = false;

    loop {
        frame.clear();
        let read =
            reader
                .read_until(b'\n', &mut frame)
                .map_err(|source| JsonlVerifyError::ReadInput {
                    path: source_path.clone(),
                    operation: VerifyIoOperation::Read,
                    source,
                })?;
        if read == 0 {
            break;
        }
        records = records.saturating_add(1);
        bytes = bytes.saturating_add(read as u64);
        hasher.update(&frame);
        has_trailing_partial_record = !frame.ends_with(b"\n");
        if !validate_frame(&frame) {
            invalid_records = invalid_records.saturating_add(1);
            first_invalid_record.get_or_insert(records);
        }
    }
    let final_snapshot = inspect_file(reader.get_ref(), &source_path)?;

    Ok(JsonlFileScan {
        source_path,
        records,
        bytes,
        sha256: digest_hex(hasher.finalize()),
        invalid_records,
        first_invalid_record,
        has_trailing_partial_record,
        stable_while_reading: bytes == initial.bytes && final_snapshot == initial,
    })
}

fn read_frame_bounded(
    reader: &mut impl BufRead,
    frame: &mut Vec<u8>,
    max_frame_bytes: usize,
    source_path: &Path,
    record: u64,
) -> Result<usize, JsonlVerifyError> {
    let mut total = 0_usize;
    loop {
        let available = reader
            .fill_buf()
            .map_err(|source| JsonlVerifyError::ReadInput {
                path: source_path.to_path_buf(),
                operation: VerifyIoOperation::Read,
                source,
            })?;
        if available.is_empty() {
            return Ok(total);
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        let next_total = total.saturating_add(take);
        if next_total > max_frame_bytes {
            return Err(JsonlVerifyError::FrameTooLarge {
                path: source_path.to_path_buf(),
                record,
                observed_at_least: max_frame_bytes.saturating_add(1),
                limit: max_frame_bytes,
            });
        }
        frame.extend_from_slice(&available[..take]);
        let ended = available[take - 1] == b'\n';
        reader.consume(take);
        total = next_total;
        if ended {
            return Ok(total);
        }
    }
}

fn open_jsonl_file(path: &Path) -> Result<(PathBuf, File, FileSnapshot), JsonlVerifyError> {
    open_verified_regular_file(path)
}

fn inspect_file(file: &File, source_path: &Path) -> Result<FileSnapshot, JsonlVerifyError> {
    file.metadata()
        .map(|metadata| FileSnapshot::from_metadata(&metadata))
        .map_err(|source| JsonlVerifyError::ReadInput {
            path: source_path.to_path_buf(),
            operation: VerifyIoOperation::InspectMetadata,
            source,
        })
}

pub fn read_bounded_regular_file(
    path: &Path,
    limit: u64,
) -> Result<(PathBuf, Vec<u8>), JsonlVerifyError> {
    let (canonical, file, initial) = open_verified_regular_file(path)?;
    let initial_bytes = initial.bytes;
    if initial_bytes > limit {
        return Err(JsonlVerifyError::InputTooLarge {
            path: canonical,
            actual: initial_bytes,
            limit,
        });
    }
    let read_limit = limit.saturating_add(1);
    let initial_capacity = usize::try_from(read_limit.min(64 * 1024)).unwrap_or(64 * 1024);
    let mut bytes = Vec::with_capacity(initial_capacity);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|source| JsonlVerifyError::ReadInput {
            path: canonical.clone(),
            operation: VerifyIoOperation::Read,
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(JsonlVerifyError::InputTooLarge {
            path: canonical,
            actual: bytes.len() as u64,
            limit,
        });
    }
    Ok((canonical, bytes))
}

pub fn canonical_regular_file(path: &Path) -> Result<PathBuf, JsonlVerifyError> {
    regular_path_metadata(path)?;
    canonicalize_input_path(path)
}

fn open_verified_regular_file(
    path: &Path,
) -> Result<(PathBuf, File, FileSnapshot), JsonlVerifyError> {
    let target_before_open = regular_path_metadata(path)?;
    let source_path = canonicalize_input_path(path)?;
    let file = open_read_only_no_follow(path).map_err(|source| JsonlVerifyError::ReadInput {
        path: source_path.clone(),
        operation: VerifyIoOperation::Open,
        source,
    })?;
    let opened_metadata = file
        .metadata()
        .map_err(|source| JsonlVerifyError::ReadInput {
            path: source_path.clone(),
            operation: VerifyIoOperation::InspectMetadata,
            source,
        })?;
    let target_after_open = regular_path_metadata(path)?;
    if !opened_metadata.is_file()
        || !same_file_identity(&target_before_open, &opened_metadata)
        || !same_file_identity(&target_after_open, &opened_metadata)
    {
        return Err(JsonlVerifyError::InvalidInputPath {
            path: path.to_path_buf(),
            message: "changed while opening or did not resolve to the expected regular file"
                .to_string(),
        });
    }
    let initial = FileSnapshot::from_metadata(&opened_metadata);
    Ok((source_path, file, initial))
}

fn regular_path_metadata(path: &Path) -> Result<std::fs::Metadata, JsonlVerifyError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|error| JsonlVerifyError::InvalidInputPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(JsonlVerifyError::InvalidInputPath {
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    Ok(metadata)
}

fn canonicalize_input_path(path: &Path) -> Result<PathBuf, JsonlVerifyError> {
    std::fs::canonicalize(path).map_err(|error| JsonlVerifyError::InvalidInputPath {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

#[cfg(unix)]
fn open_read_only_no_follow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    options.open(path)
}

#[cfg(not(unix))]
fn open_read_only_no_follow(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(unix)]
fn same_file_identity(expected: &std::fs::Metadata, actual: &std::fs::Metadata) -> bool {
    expected.dev() == actual.dev() && expected.ino() == actual.ino()
}

#[cfg(not(unix))]
fn same_file_identity(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

/// Returns the JSON payload bytes for a validator that does not care whether
/// the source record ended in a newline.
pub fn jsonl_payload(frame: &[u8]) -> &[u8] {
    trim_jsonl_newline(frame)
}
