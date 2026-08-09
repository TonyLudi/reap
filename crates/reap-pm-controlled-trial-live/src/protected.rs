use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read as _, Seek as _, Write as _},
    os::unix::{
        fs::{MetadataExt as _, OpenOptionsExt as _},
        io::AsRawFd as _,
    },
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{PmTrialLiveJournalError, hash::hash_domain};

const ARTIFACT_DIRECTORY_LEASE_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial-live.artifact-directory-lease.v1\0";

#[derive(Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
    nlink: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}

impl Snapshot {
    fn from(metadata: &Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            uid: metadata.uid(),
            mode: metadata.mode() & 0o7777,
            nlink: metadata.nlink(),
            len: metadata.len(),
            mtime: metadata.mtime(),
            mtime_nsec: metadata.mtime_nsec(),
            ctime: metadata.ctime(),
            ctime_nsec: metadata.ctime_nsec(),
        }
    }
}

pub(crate) struct ProtectedJournal {
    file: File,
    directory: PinnedDirectory,
    parent: PathBuf,
    name: std::ffi::OsString,
    identity: (u64, u64),
    effective_uid: u32,
    maximum_bytes: usize,
}

pub(crate) struct ProtectedArtifactLease {
    directory: PinnedDirectory,
    parent: PathBuf,
    fingerprint: String,
}

#[derive(Serialize)]
struct ArtifactDirectoryLeaseFingerprint<'a> {
    artifact_directory: &'a str,
    device: u64,
    inode: u64,
    owner_uid: u32,
    mode: u32,
}

impl ProtectedArtifactLease {
    pub(crate) fn acquire(path: &Path) -> Result<Self, PmTrialLiveJournalError> {
        if !path.is_absolute() {
            return Err(PmTrialLiveJournalError::Protection);
        }
        let directory = PinnedDirectory::open(path)?;
        acquire_lease(&directory.file)?;
        let path_text = path.to_str().ok_or(PmTrialLiveJournalError::Protection)?;
        let fingerprint = hash_domain(
            ARTIFACT_DIRECTORY_LEASE_FINGERPRINT_DOMAIN,
            &ArtifactDirectoryLeaseFingerprint {
                artifact_directory: path_text,
                device: directory.snapshot.dev,
                inode: directory.snapshot.ino,
                owner_uid: directory.snapshot.uid,
                mode: directory.snapshot.mode,
            },
        )?;
        Ok(Self {
            directory,
            parent: path.to_owned(),
            fingerprint,
        })
    }

    pub(crate) fn refresh_after_bound_create(&mut self) -> Result<(), PmTrialLiveJournalError> {
        self.directory.refresh_after_create(&self.parent)
    }

    pub(crate) fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        self.directory.finish(&self.parent)
    }

    pub(crate) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl ProtectedJournal {
    pub(crate) fn create_new(
        path: &Path,
        maximum_bytes: usize,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let parent = path.parent().ok_or(PmTrialLiveJournalError::Protection)?;
        let name = direct_name(path)?.to_owned();
        let mut directory = PinnedDirectory::open(parent)?;
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(directory.descriptor_root.join(&name))
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    PmTrialLiveJournalError::AlreadyExists
                } else {
                    PmTrialLiveJournalError::Protection
                }
            })?;
        acquire_lease(&file)?;
        file.sync_all()
            .and_then(|()| directory.file.sync_all())
            .map_err(|_| PmTrialLiveJournalError::Durability)?;
        directory.refresh_after_create(parent)?;
        let metadata = file
            .metadata()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        validate_file(&metadata, directory.effective_uid, maximum_bytes)?;
        Ok(Self {
            file,
            identity: (metadata.dev(), metadata.ino()),
            effective_uid: directory.effective_uid,
            directory,
            parent: parent.to_owned(),
            name,
            maximum_bytes,
        })
    }

    pub(crate) fn open_existing(
        path: &Path,
        maximum_bytes: usize,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let parent = path.parent().ok_or(PmTrialLiveJournalError::Protection)?;
        let name = direct_name(path)?.to_owned();
        let directory = PinnedDirectory::open(parent)?;
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(directory.descriptor_root.join(&name))
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    PmTrialLiveJournalError::Absent
                } else {
                    PmTrialLiveJournalError::Protection
                }
            })?;
        acquire_lease(&file)?;
        let metadata = file
            .metadata()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        validate_file(&metadata, directory.effective_uid, maximum_bytes)?;
        directory.finish(parent)?;
        Ok(Self {
            file,
            identity: (metadata.dev(), metadata.ino()),
            effective_uid: directory.effective_uid,
            directory,
            parent: parent.to_owned(),
            name,
            maximum_bytes,
        })
    }

    pub(crate) fn refresh_parent_after_bound_create(
        &mut self,
    ) -> Result<(), PmTrialLiveJournalError> {
        self.directory.refresh_after_create(&self.parent)
    }

    pub(crate) fn append_durable(
        &mut self,
        expected_prefix: &[u8],
        suffix: &[u8],
    ) -> Result<(), PmTrialLiveJournalError> {
        let final_length = expected_prefix
            .len()
            .checked_add(suffix.len())
            .ok_or(PmTrialLiveJournalError::BoundExceeded)?;
        if final_length > self.maximum_bytes {
            return Err(PmTrialLiveJournalError::BoundExceeded);
        }
        self.validate_identity(expected_prefix.len() as u64)?;
        self.file
            .rewind()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let mut actual = Vec::with_capacity(expected_prefix.len());
        std::io::Read::by_ref(&mut self.file)
            .take((self.maximum_bytes + 1) as u64)
            .read_to_end(&mut actual)
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        if actual != expected_prefix {
            return Err(PmTrialLiveJournalError::AmbiguousTail);
        }
        self.file
            .write_all(suffix)
            .and_then(|()| self.file.sync_all())
            .and_then(|()| self.directory.file.sync_all())
            .map_err(|_| PmTrialLiveJournalError::Durability)?;
        self.validate_identity(final_length as u64)?;
        self.directory.finish(&self.parent)
    }

    pub(crate) fn validate_exact_bytes(
        &mut self,
        expected: &[u8],
    ) -> Result<(), PmTrialLiveJournalError> {
        if expected.len() > self.maximum_bytes {
            return Err(PmTrialLiveJournalError::BoundExceeded);
        }
        self.validate_identity(expected.len() as u64)?;
        self.file
            .rewind()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let mut actual = Vec::with_capacity(expected.len());
        std::io::Read::by_ref(&mut self.file)
            .take((self.maximum_bytes + 1) as u64)
            .read_to_end(&mut actual)
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        if actual != expected {
            return Err(PmTrialLiveJournalError::Protection);
        }
        self.validate_identity(expected.len() as u64)
    }

    fn validate_identity(&self, expected_length: u64) -> Result<(), PmTrialLiveJournalError> {
        let held = self
            .file
            .metadata()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        validate_file(&held, self.effective_uid, self.maximum_bytes)?;
        if (held.dev(), held.ino()) != self.identity || held.len() != expected_length {
            return Err(PmTrialLiveJournalError::Protection);
        }
        let reopened = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.directory.descriptor_root.join(&self.name))
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let reopened = reopened
            .metadata()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        validate_file(&reopened, self.effective_uid, self.maximum_bytes)?;
        if (reopened.dev(), reopened.ino()) != self.identity || reopened.len() != expected_length {
            return Err(PmTrialLiveJournalError::Protection);
        }
        self.directory.finish(&self.parent)
    }
}

pub(crate) fn read_protected(
    path: &Path,
    maximum_bytes: usize,
) -> Result<Vec<u8>, PmTrialLiveJournalError> {
    let parent = path.parent().ok_or(PmTrialLiveJournalError::Protection)?;
    let name = direct_name(path)?;
    let directory = PinnedDirectory::open(parent)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(directory.descriptor_root.join(name))
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                PmTrialLiveJournalError::Absent
            } else {
                PmTrialLiveJournalError::Protection
            }
        })?;
    let before = file
        .metadata()
        .map_err(|_| PmTrialLiveJournalError::Protection)?;
    validate_file(&before, directory.effective_uid, maximum_bytes)?;
    let expected = Snapshot::from(&before);
    let mut bytes = Vec::with_capacity((before.len() as usize).min(maximum_bytes));
    let mut reader = &file;
    std::io::Read::by_ref(&mut reader)
        .take((maximum_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| PmTrialLiveJournalError::Protection)?;
    if bytes.len() > maximum_bytes {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    let after = file
        .metadata()
        .map_err(|_| PmTrialLiveJournalError::Protection)?;
    if Snapshot::from(&after) != expected || after.len() != bytes.len() as u64 {
        return Err(PmTrialLiveJournalError::Protection);
    }
    let reopened = directory.open_metadata(name, maximum_bytes)?;
    if Snapshot::from(&reopened) != expected {
        return Err(PmTrialLiveJournalError::Protection);
    }
    directory.finish(parent)?;
    Ok(bytes)
}

fn acquire_lease(file: &File) -> Result<(), PmTrialLiveJournalError> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(std::fs::TryLockError::WouldBlock) => Err(PmTrialLiveJournalError::AlreadyLeased),
        Err(std::fs::TryLockError::Error(_)) => Err(PmTrialLiveJournalError::Protection),
    }
}

fn direct_name(path: &Path) -> Result<&std::ffi::OsStr, PmTrialLiveJournalError> {
    let name = path
        .file_name()
        .ok_or(PmTrialLiveJournalError::Protection)?;
    if name.is_empty() || name == std::ffi::OsStr::new(".") || name == std::ffi::OsStr::new("..") {
        return Err(PmTrialLiveJournalError::Protection);
    }
    Ok(name)
}

struct PinnedDirectory {
    file: File,
    snapshot: Snapshot,
    descriptor_root: PathBuf,
    effective_uid: u32,
}

impl PinnedDirectory {
    fn open(path: &Path) -> Result<Self, PmTrialLiveJournalError> {
        let by_path =
            std::fs::symlink_metadata(path).map_err(|_| PmTrialLiveJournalError::Protection)?;
        let effective_uid = effective_uid()?;
        validate_directory(&by_path, effective_uid)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let held = file
            .metadata()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        validate_directory(&held, effective_uid)?;
        if Snapshot::from(&held) != Snapshot::from(&by_path) {
            return Err(PmTrialLiveJournalError::Protection);
        }
        let descriptor_root = PathBuf::from("/proc/self/fd").join(file.as_raw_fd().to_string());
        Ok(Self {
            file,
            snapshot: Snapshot::from(&held),
            descriptor_root,
            effective_uid,
        })
    }

    fn open_metadata(
        &self,
        name: &std::ffi::OsStr,
        maximum_bytes: usize,
    ) -> Result<Metadata, PmTrialLiveJournalError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.descriptor_root.join(name))
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let metadata = file
            .metadata()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        validate_file(&metadata, self.effective_uid, maximum_bytes)?;
        Ok(metadata)
    }

    fn finish(&self, path: &Path) -> Result<(), PmTrialLiveJournalError> {
        let held = self
            .file
            .metadata()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let by_path =
            std::fs::symlink_metadata(path).map_err(|_| PmTrialLiveJournalError::Protection)?;
        if Snapshot::from(&held) != self.snapshot
            || Snapshot::from(&by_path) != self.snapshot
            || by_path.file_type().is_symlink()
        {
            return Err(PmTrialLiveJournalError::Protection);
        }
        Ok(())
    }

    fn refresh_after_create(&mut self, path: &Path) -> Result<(), PmTrialLiveJournalError> {
        let held = self
            .file
            .metadata()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let by_path =
            std::fs::symlink_metadata(path).map_err(|_| PmTrialLiveJournalError::Protection)?;
        validate_directory(&held, self.effective_uid)?;
        validate_directory(&by_path, self.effective_uid)?;
        if held.dev() != by_path.dev() || held.ino() != by_path.ino() {
            return Err(PmTrialLiveJournalError::Protection);
        }
        self.snapshot = Snapshot::from(&held);
        if Snapshot::from(&by_path) != self.snapshot {
            return Err(PmTrialLiveJournalError::Protection);
        }
        Ok(())
    }
}

fn validate_file(
    metadata: &Metadata,
    effective_uid: u32,
    maximum_bytes: usize,
) -> Result<(), PmTrialLiveJournalError> {
    if !metadata.is_file()
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(PmTrialLiveJournalError::Protection);
    }
    if metadata.len() > maximum_bytes as u64 {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    Ok(())
}

fn validate_directory(
    metadata: &Metadata,
    effective_uid: u32,
) -> Result<(), PmTrialLiveJournalError> {
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(PmTrialLiveJournalError::Protection);
    }
    Ok(())
}

fn effective_uid() -> Result<u32, PmTrialLiveJournalError> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|_| PmTrialLiveJournalError::Protection)?;
    status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_ascii_whitespace().nth(2))
        .and_then(|value| value.parse().ok())
        .ok_or(PmTrialLiveJournalError::Protection)
}
