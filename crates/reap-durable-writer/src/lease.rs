use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LeaseError {
    #[error("durable writer IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("durable path {path} is invalid: {message}")]
    InvalidPath { path: PathBuf, message: String },
    #[error("durable journal {path} is already owned by another process via {lock_path}")]
    AlreadyLocked { path: PathBuf, lock_path: PathBuf },
}

#[derive(Debug)]
pub struct DurableLease {
    journal_path: PathBuf,
    lock_path: PathBuf,
    lock_file: std::fs::File,
}

impl DurableLease {
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, LeaseError> {
        let journal_path = normalize_journal_path(path.as_ref())?;
        let lock_path = lock_path_for(&journal_path)?;
        validate_regular_file_if_present(&lock_path, "lock path")?;

        let mut options = std::fs::OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut lock_file = options.open(&lock_path)?;
        match lock_file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(LeaseError::AlreadyLocked {
                    path: journal_path,
                    lock_path,
                });
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        #[cfg(unix)]
        std::fs::set_permissions(&lock_path, private_permissions())?;
        lock_file.set_len(0)?;
        lock_file.seek(SeekFrom::Start(0))?;
        writeln!(
            lock_file,
            "pid={} acquired_at_ms={}",
            std::process::id(),
            unix_time_ms()
        )?;
        lock_file.sync_data()?;

        Ok(Self {
            journal_path,
            lock_path,
            lock_file,
        })
    }

    #[must_use]
    pub fn journal_path(&self) -> &Path {
        &self.journal_path
    }

    #[must_use]
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

impl Drop for DurableLease {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
    }
}

pub(crate) fn normalize_journal_path(path: &Path) -> Result<PathBuf, LeaseError> {
    let file_name = path.file_name().ok_or_else(|| LeaseError::InvalidPath {
        path: path.to_path_buf(),
        message: "journal path must name a file".to_string(),
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    let journal_path = canonical_parent.join(file_name);
    validate_regular_file_if_present(&journal_path, "journal path")?;
    Ok(journal_path)
}

fn lock_path_for(journal_path: &Path) -> Result<PathBuf, LeaseError> {
    let file_name = journal_path
        .file_name()
        .ok_or_else(|| LeaseError::InvalidPath {
            path: journal_path.to_path_buf(),
            message: "journal path must name a file".to_string(),
        })?;
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".lock");
    Ok(journal_path.with_file_name(lock_name))
}

fn validate_regular_file_if_present(path: &Path, label: &str) -> Result<(), LeaseError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(LeaseError::InvalidPath {
            path: path.to_path_buf(),
            message: format!("{label} must not be a symbolic link"),
        }),
        Ok(metadata) if !metadata.is_file() => Err(LeaseError::InvalidPath {
            path: path.to_path_buf(),
            message: format!("{label} must be a regular file"),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn private_permissions() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(0o600)
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
