use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read as _, Seek as _, Write as _},
    os::unix::{
        fs::{MetadataExt as _, OpenOptionsExt as _},
        io::AsRawFd as _,
    },
    path::{Path, PathBuf},
};

use thiserror::Error;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtectedFileKind {
    Config,
    Authorization,
    OnlinePolicyV2,
    OnlineAuthorizationV2,
    ReviewedProductionDestinationProfileV1,
    ReviewedFreshCredentialSlotLocatorV1,
    OnlineAuthorizationConsumptionV2,
    ConsumptionEvidence,
    PrivateKey,
    ApiKey,
    L2Secret,
    Passphrase,
}

impl std::fmt::Display for ProtectedFileKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Config => "config",
            Self::Authorization => "authorization",
            Self::OnlinePolicyV2 => "online-policy V2",
            Self::OnlineAuthorizationV2 => "online-authorization V2",
            Self::ReviewedProductionDestinationProfileV1 => {
                "reviewed production destination profile V1"
            }
            Self::ReviewedFreshCredentialSlotLocatorV1 => {
                "reviewed fresh credential-slot locator V1"
            }
            Self::OnlineAuthorizationConsumptionV2 => {
                "online-authorization-consumption V2 evidence"
            }
            Self::ConsumptionEvidence => "authorization-consumption evidence",
            Self::PrivateKey => "private-key",
            Self::ApiKey => "API-key",
            Self::L2Secret => "L2-secret",
            Self::Passphrase => "passphrase",
        })
    }
}

#[derive(Debug, Error)]
pub(crate) enum ProtectedFileError {
    #[error("the protected-file parent is not a real directory")]
    InvalidDirectory,
    #[error("the protected-file parent is not owned by the effective user")]
    WrongDirectoryOwner,
    #[error("the protected-file parent mode is not exactly 0700")]
    WrongDirectoryMode,
    #[error("the protected-file parent changed during descriptor-pinned inspection")]
    DirectoryChanged,
    #[error("the effective user identity is unavailable")]
    EffectiveUidUnavailable,
    #[error("the {0} path is not one direct file inside its protected parent")]
    InvalidPath(ProtectedFileKind),
    #[error("the {0} file could not be opened safely")]
    Open(ProtectedFileKind),
    #[error("the {0} file could not be created exclusively")]
    Create(ProtectedFileKind),
    #[error("the {0} entry is not a regular file")]
    NotRegular(ProtectedFileKind),
    #[error("the {0} file is not owned by the effective user")]
    WrongOwner(ProtectedFileKind),
    #[error("the {0} file mode is not exactly 0600")]
    WrongMode(ProtectedFileKind),
    #[error("the {0} file does not have exactly one hard link")]
    WrongLinkCount(ProtectedFileKind),
    #[error("the {0} file exceeds its closed size bound")]
    TooLarge(ProtectedFileKind),
    #[error("the {0} file changed during descriptor-pinned inspection")]
    FileChanged(ProtectedFileKind),
    #[error("the {0} file could not be read")]
    Read(ProtectedFileKind),
    #[error("the {0} file could not be written and synchronized durably")]
    DurableWrite(ProtectedFileKind),
    #[error("two custody roles resolve to one inode")]
    DuplicateInode,
}

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

    fn inode(self) -> (u64, u64) {
        (self.dev, self.ino)
    }
}

struct OpenedFile {
    kind: ProtectedFileKind,
    file: File,
    snapshot: Snapshot,
}

pub(crate) struct DurableCreateNewFile {
    kind: ProtectedFileKind,
    file: File,
    directory: PinnedDirectory,
    original_parent: PathBuf,
    name: std::ffi::OsString,
    identity: (u64, u64),
    effective_uid: u32,
    maximum_bytes: usize,
}

pub(crate) fn read_one(
    path: &Path,
    kind: ProtectedFileKind,
    maximum_bytes: usize,
) -> Result<Zeroizing<Vec<u8>>, ProtectedFileError> {
    let parent = path.parent().ok_or(ProtectedFileError::InvalidPath(kind))?;
    let name = direct_name(path, kind)?;
    let directory = PinnedDirectory::open(parent)?;
    let opened = directory.open_entry(name, kind, maximum_bytes)?;
    let expected = opened.snapshot;
    let bytes = opened.read(maximum_bytes)?;
    directory.revalidate(name, kind, maximum_bytes, expected)?;
    directory.finish(parent)?;
    Ok(bytes)
}

pub(crate) fn read_four(
    paths: [(&Path, ProtectedFileKind, usize); 4],
) -> Result<[Zeroizing<Vec<u8>>; 4], ProtectedFileError> {
    let parent = paths[0]
        .0
        .parent()
        .ok_or(ProtectedFileError::InvalidPath(paths[0].1))?;
    if paths
        .iter()
        .any(|(path, _, _)| path.parent() != Some(parent))
    {
        return Err(ProtectedFileError::InvalidDirectory);
    }
    let names = [
        direct_name(paths[0].0, paths[0].1)?,
        direct_name(paths[1].0, paths[1].1)?,
        direct_name(paths[2].0, paths[2].1)?,
        direct_name(paths[3].0, paths[3].1)?,
    ];
    if names[0] == names[1]
        || names[0] == names[2]
        || names[0] == names[3]
        || names[1] == names[2]
        || names[1] == names[3]
        || names[2] == names[3]
    {
        return Err(ProtectedFileError::DuplicateInode);
    }

    let directory = PinnedDirectory::open(parent)?;
    let opened = [
        directory.open_entry(names[0], paths[0].1, paths[0].2)?,
        directory.open_entry(names[1], paths[1].1, paths[1].2)?,
        directory.open_entry(names[2], paths[2].1, paths[2].2)?,
        directory.open_entry(names[3], paths[3].1, paths[3].2)?,
    ];
    for left in 0..opened.len() {
        for right in left + 1..opened.len() {
            if opened[left].snapshot.inode() == opened[right].snapshot.inode() {
                return Err(ProtectedFileError::DuplicateInode);
            }
        }
    }
    let snapshots = opened.each_ref().map(|entry| entry.snapshot);
    let bytes = [
        opened[0].read_ref(paths[0].2)?,
        opened[1].read_ref(paths[1].2)?,
        opened[2].read_ref(paths[2].2)?,
        opened[3].read_ref(paths[3].2)?,
    ];
    for index in 0..4 {
        directory.revalidate(
            names[index],
            paths[index].1,
            paths[index].2,
            snapshots[index],
        )?;
    }
    directory.finish(parent)?;
    Ok(bytes)
}

pub(crate) fn create_new(
    path: &Path,
    kind: ProtectedFileKind,
    maximum_bytes: usize,
) -> Result<DurableCreateNewFile, ProtectedFileError> {
    let parent = path.parent().ok_or(ProtectedFileError::InvalidPath(kind))?;
    let name = direct_name(path, kind)?.to_owned();
    let mut directory = PinnedDirectory::open(parent)?;
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(directory.descriptor_root.join(&name))
        .map_err(|_| ProtectedFileError::Create(kind))?;
    file.sync_all()
        .map_err(|_| ProtectedFileError::DurableWrite(kind))?;
    directory
        .file
        .sync_all()
        .map_err(|_| ProtectedFileError::DurableWrite(kind))?;
    directory.refresh_after_create(parent)?;
    let metadata = file
        .metadata()
        .map_err(|_| ProtectedFileError::Create(kind))?;
    validate_created_file(&metadata, directory.effective_uid, kind, maximum_bytes)?;
    Ok(DurableCreateNewFile {
        kind,
        file,
        identity: (metadata.dev(), metadata.ino()),
        effective_uid: directory.effective_uid,
        directory,
        original_parent: parent.to_owned(),
        name,
        maximum_bytes,
    })
}

pub(crate) fn open_existing_append(
    path: &Path,
    kind: ProtectedFileKind,
    maximum_bytes: usize,
) -> Result<DurableCreateNewFile, ProtectedFileError> {
    let parent = path.parent().ok_or(ProtectedFileError::InvalidPath(kind))?;
    let name = direct_name(path, kind)?.to_owned();
    let directory = PinnedDirectory::open(parent)?;
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(directory.descriptor_root.join(&name))
        .map_err(|_| ProtectedFileError::Open(kind))?;
    let metadata = file
        .metadata()
        .map_err(|_| ProtectedFileError::Open(kind))?;
    validate_created_file(&metadata, directory.effective_uid, kind, maximum_bytes)?;
    directory.finish(parent)?;
    Ok(DurableCreateNewFile {
        kind,
        file,
        identity: (metadata.dev(), metadata.ino()),
        effective_uid: directory.effective_uid,
        directory,
        original_parent: parent.to_owned(),
        name,
        maximum_bytes,
    })
}

fn direct_name(
    path: &Path,
    kind: ProtectedFileKind,
) -> Result<&std::ffi::OsStr, ProtectedFileError> {
    let name = path
        .file_name()
        .ok_or(ProtectedFileError::InvalidPath(kind))?;
    if name.is_empty() || name == std::ffi::OsStr::new(".") || name == std::ffi::OsStr::new("..") {
        return Err(ProtectedFileError::InvalidPath(kind));
    }
    Ok(name)
}

impl OpenedFile {
    fn read(mut self, maximum_bytes: usize) -> Result<Zeroizing<Vec<u8>>, ProtectedFileError> {
        self.read_inner(maximum_bytes)
    }

    fn read_ref(&self, maximum_bytes: usize) -> Result<Zeroizing<Vec<u8>>, ProtectedFileError> {
        let mut file = self
            .file
            .try_clone()
            .map_err(|_| ProtectedFileError::Read(self.kind))?;
        read_descriptor(&mut file, self.kind, self.snapshot, maximum_bytes)
    }

    fn read_inner(
        &mut self,
        maximum_bytes: usize,
    ) -> Result<Zeroizing<Vec<u8>>, ProtectedFileError> {
        read_descriptor(&mut self.file, self.kind, self.snapshot, maximum_bytes)
    }
}

impl DurableCreateNewFile {
    pub(crate) fn refresh_parent_after_bound_create(&mut self) -> Result<(), ProtectedFileError> {
        self.directory.refresh_after_create(&self.original_parent)
    }

    pub(crate) fn append_durable(
        &mut self,
        expected_prefix: &[u8],
        suffix: &[u8],
    ) -> Result<(), ProtectedFileError> {
        let final_length = expected_prefix
            .len()
            .checked_add(suffix.len())
            .ok_or(ProtectedFileError::TooLarge(self.kind))?;
        if final_length > self.maximum_bytes {
            return Err(ProtectedFileError::TooLarge(self.kind));
        }
        self.validate_path_and_metadata(expected_prefix.len() as u64)?;
        self.file
            .rewind()
            .map_err(|_| ProtectedFileError::Read(self.kind))?;
        let mut actual = Vec::with_capacity(expected_prefix.len());
        std::io::Read::by_ref(&mut self.file)
            .take((self.maximum_bytes + 1) as u64)
            .read_to_end(&mut actual)
            .map_err(|_| ProtectedFileError::Read(self.kind))?;
        if actual != expected_prefix {
            return Err(ProtectedFileError::FileChanged(self.kind));
        }
        self.file
            .write_all(suffix)
            .and_then(|()| self.file.sync_all())
            .map_err(|_| ProtectedFileError::DurableWrite(self.kind))?;
        self.validate_path_and_metadata(final_length as u64)?;
        self.directory.finish(&self.original_parent)?;
        Ok(())
    }

    pub(crate) fn validate_exact_bytes(
        &mut self,
        expected: &[u8],
    ) -> Result<(), ProtectedFileError> {
        if expected.len() > self.maximum_bytes {
            return Err(ProtectedFileError::TooLarge(self.kind));
        }
        self.validate_path_and_metadata(expected.len() as u64)?;
        self.file
            .rewind()
            .map_err(|_| ProtectedFileError::Read(self.kind))?;
        let mut actual = Vec::with_capacity(expected.len());
        std::io::Read::by_ref(&mut self.file)
            .take((self.maximum_bytes + 1) as u64)
            .read_to_end(&mut actual)
            .map_err(|_| ProtectedFileError::Read(self.kind))?;
        if actual != expected {
            return Err(ProtectedFileError::FileChanged(self.kind));
        }
        self.validate_path_and_metadata(expected.len() as u64)
    }

    fn validate_path_and_metadata(&self, expected_length: u64) -> Result<(), ProtectedFileError> {
        let held = self
            .file
            .metadata()
            .map_err(|_| ProtectedFileError::FileChanged(self.kind))?;
        validate_created_file(&held, self.effective_uid, self.kind, self.maximum_bytes)?;
        if (held.dev(), held.ino()) != self.identity || held.len() != expected_length {
            return Err(ProtectedFileError::FileChanged(self.kind));
        }
        let reopened = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.directory.descriptor_root.join(&self.name))
            .map_err(|_| ProtectedFileError::FileChanged(self.kind))?;
        let reopened = reopened
            .metadata()
            .map_err(|_| ProtectedFileError::FileChanged(self.kind))?;
        validate_created_file(&reopened, self.effective_uid, self.kind, self.maximum_bytes)?;
        if (reopened.dev(), reopened.ino()) != self.identity || reopened.len() != expected_length {
            return Err(ProtectedFileError::FileChanged(self.kind));
        }
        self.directory.finish(&self.original_parent)
    }
}

fn read_descriptor(
    file: &mut File,
    kind: ProtectedFileKind,
    expected: Snapshot,
    maximum_bytes: usize,
) -> Result<Zeroizing<Vec<u8>>, ProtectedFileError> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(
        usize::try_from(expected.len)
            .unwrap_or(maximum_bytes)
            .min(maximum_bytes),
    ));
    std::io::Read::by_ref(file)
        .take((maximum_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ProtectedFileError::Read(kind))?;
    if bytes.len() > maximum_bytes {
        return Err(ProtectedFileError::TooLarge(kind));
    }
    let after = file
        .metadata()
        .map_err(|_| ProtectedFileError::FileChanged(kind))?;
    if !after.is_file() || Snapshot::from(&after) != expected || after.len() != bytes.len() as u64 {
        return Err(ProtectedFileError::FileChanged(kind));
    }
    Ok(bytes)
}

struct PinnedDirectory {
    file: File,
    snapshot: Snapshot,
    descriptor_root: PathBuf,
    effective_uid: u32,
}

impl PinnedDirectory {
    fn open(path: &Path) -> Result<Self, ProtectedFileError> {
        let path_metadata =
            std::fs::symlink_metadata(path).map_err(|_| ProtectedFileError::InvalidDirectory)?;
        let effective_uid = effective_uid()?;
        validate_directory(&path_metadata, effective_uid)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| ProtectedFileError::InvalidDirectory)?;
        let metadata = file
            .metadata()
            .map_err(|_| ProtectedFileError::InvalidDirectory)?;
        validate_directory(&metadata, effective_uid)?;
        if Snapshot::from(&metadata) != Snapshot::from(&path_metadata) {
            return Err(ProtectedFileError::DirectoryChanged);
        }
        let descriptor_root = PathBuf::from("/proc/self/fd").join(file.as_raw_fd().to_string());
        Ok(Self {
            file,
            snapshot: Snapshot::from(&metadata),
            descriptor_root,
            effective_uid,
        })
    }

    fn open_entry(
        &self,
        name: &std::ffi::OsStr,
        kind: ProtectedFileKind,
        maximum_bytes: usize,
    ) -> Result<OpenedFile, ProtectedFileError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.descriptor_root.join(name))
            .map_err(|_| ProtectedFileError::Open(kind))?;
        let metadata = file
            .metadata()
            .map_err(|_| ProtectedFileError::Open(kind))?;
        if !metadata.is_file() {
            return Err(ProtectedFileError::NotRegular(kind));
        }
        if metadata.uid() != self.effective_uid {
            return Err(ProtectedFileError::WrongOwner(kind));
        }
        if metadata.mode() & 0o7777 != 0o600 {
            return Err(ProtectedFileError::WrongMode(kind));
        }
        if metadata.nlink() != 1 {
            return Err(ProtectedFileError::WrongLinkCount(kind));
        }
        if metadata.len() > maximum_bytes as u64 {
            return Err(ProtectedFileError::TooLarge(kind));
        }
        Ok(OpenedFile {
            kind,
            file,
            snapshot: Snapshot::from(&metadata),
        })
    }

    fn revalidate(
        &self,
        name: &std::ffi::OsStr,
        kind: ProtectedFileKind,
        maximum_bytes: usize,
        expected: Snapshot,
    ) -> Result<(), ProtectedFileError> {
        let reopened = self.open_entry(name, kind, maximum_bytes)?;
        if reopened.snapshot != expected {
            return Err(ProtectedFileError::FileChanged(kind));
        }
        Ok(())
    }

    fn finish(&self, original_path: &Path) -> Result<(), ProtectedFileError> {
        let descriptor = self
            .file
            .metadata()
            .map_err(|_| ProtectedFileError::DirectoryChanged)?;
        let by_path = std::fs::symlink_metadata(original_path)
            .map_err(|_| ProtectedFileError::DirectoryChanged)?;
        if Snapshot::from(&descriptor) != self.snapshot
            || Snapshot::from(&by_path) != self.snapshot
            || by_path.file_type().is_symlink()
        {
            return Err(ProtectedFileError::DirectoryChanged);
        }
        Ok(())
    }

    fn refresh_after_create(&mut self, original_path: &Path) -> Result<(), ProtectedFileError> {
        let descriptor = self
            .file
            .metadata()
            .map_err(|_| ProtectedFileError::DirectoryChanged)?;
        let by_path = std::fs::symlink_metadata(original_path)
            .map_err(|_| ProtectedFileError::DirectoryChanged)?;
        validate_directory(&descriptor, self.effective_uid)?;
        validate_directory(&by_path, self.effective_uid)?;
        if descriptor.dev() != by_path.dev() || descriptor.ino() != by_path.ino() {
            return Err(ProtectedFileError::DirectoryChanged);
        }
        self.snapshot = Snapshot::from(&descriptor);
        if Snapshot::from(&by_path) != self.snapshot {
            return Err(ProtectedFileError::DirectoryChanged);
        }
        Ok(())
    }
}

fn validate_created_file(
    metadata: &Metadata,
    effective_uid: u32,
    kind: ProtectedFileKind,
    maximum_bytes: usize,
) -> Result<(), ProtectedFileError> {
    if !metadata.is_file() {
        return Err(ProtectedFileError::NotRegular(kind));
    }
    if metadata.uid() != effective_uid {
        return Err(ProtectedFileError::WrongOwner(kind));
    }
    if metadata.mode() & 0o7777 != 0o600 {
        return Err(ProtectedFileError::WrongMode(kind));
    }
    if metadata.nlink() != 1 {
        return Err(ProtectedFileError::WrongLinkCount(kind));
    }
    if metadata.len() > maximum_bytes as u64 {
        return Err(ProtectedFileError::TooLarge(kind));
    }
    Ok(())
}

fn validate_directory(metadata: &Metadata, effective_uid: u32) -> Result<(), ProtectedFileError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ProtectedFileError::InvalidDirectory);
    }
    if metadata.uid() != effective_uid {
        return Err(ProtectedFileError::WrongDirectoryOwner);
    }
    if metadata.mode() & 0o7777 != 0o700 {
        return Err(ProtectedFileError::WrongDirectoryMode);
    }
    Ok(())
}

fn effective_uid() -> Result<u32, ProtectedFileError> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|_| ProtectedFileError::EffectiveUidUnavailable)?;
    status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_ascii_whitespace().nth(2))
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(ProtectedFileError::EffectiveUidUnavailable)
}
