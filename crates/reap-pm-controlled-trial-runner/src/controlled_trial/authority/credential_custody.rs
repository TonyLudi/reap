use std::{
    fmt,
    fs::{File, Metadata, OpenOptions},
    io::{ErrorKind, Read as _},
    os::unix::{
        fs::{MetadataExt as _, OpenOptionsExt as _},
        io::AsRawFd as _,
    },
    path::{Path, PathBuf},
};

use reap_polymarket_auth::{
    EoaAddress, EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials,
};
use thiserror::Error;
use zeroize::Zeroizing;

const MAX_ENTRY_NAME_BYTES: usize = 128;
const PRIVATE_KEY_BYTES: usize = 66;
const API_KEY_BYTES: usize = 36;
const MAX_L2_SECRET_BYTES: usize = 172;
const MAX_PASSPHRASE_BYTES: usize = 128;

/// Four staged entries for the one mode that may sign a fresh place request.
///
/// Names are resolved only beneath `directory`; they are never interpreted as
/// paths. This type is intentionally distinct from recovery custody.
#[must_use = "a fresh-place credential file specification must be loaded or deliberately dropped"]
pub(super) struct FreshPlaceCredentialFiles {
    directory: PathBuf,
    private_key_entry: String,
    api_key_entry: String,
    l2_secret_entry: String,
    passphrase_entry: String,
}

impl FreshPlaceCredentialFiles {
    pub(super) fn new(
        directory: PathBuf,
        private_key_entry: String,
        api_key_entry: String,
        l2_secret_entry: String,
        passphrase_entry: String,
    ) -> Self {
        Self {
            directory,
            private_key_entry,
            api_key_entry,
            l2_secret_entry,
            passphrase_entry,
        }
    }

    /// Load and bind all four fresh-place secrets under one pinned directory.
    pub(super) fn load(
        self,
        configured_signer: EoaAddress,
    ) -> Result<FreshPlaceCredentialHandoff, CredentialCustodyError> {
        let loaded = load_exact(
            self.directory,
            [
                EntrySpec::new(
                    CredentialRole::PrivateKey,
                    self.private_key_entry,
                    PRIVATE_KEY_BYTES,
                ),
                EntrySpec::new(CredentialRole::ApiKey, self.api_key_entry, API_KEY_BYTES),
                EntrySpec::new(
                    CredentialRole::L2Secret,
                    self.l2_secret_entry,
                    MAX_L2_SECRET_BYTES,
                ),
                EntrySpec::new(
                    CredentialRole::Passphrase,
                    self.passphrase_entry,
                    MAX_PASSPHRASE_BYTES,
                ),
            ],
        )?;
        let LoadedCredentials {
            directory,
            values,
            staged,
        } = loaded;
        let [mut private_key, mut api_key, mut l2_secret, mut passphrase] = values;
        let [
            private_key_file,
            api_key_file,
            l2_secret_file,
            passphrase_file,
        ] = staged;
        let configured_signer_text = configured_signer.to_string();

        let signer = FixedEoaSigner::bind(
            EoaPrivateKeyInput::new(take_secret_string(&mut private_key)),
            &configured_signer_text,
        )
        .map_err(|_| CredentialCustodyError::SignerBindingRejected)?;
        let l2 = L2Credentials::bind(
            &configured_signer_text,
            L2CredentialInput::new(
                take_secret_string(&mut api_key),
                take_secret_string(&mut l2_secret),
                take_secret_string(&mut passphrase),
            ),
        )
        .map_err(|_| CredentialCustodyError::L2BindingRejected)?;
        if signer.address() != configured_signer
            || l2.address() != configured_signer
            || signer.address() != l2.address()
        {
            return Err(CredentialCustodyError::CredentialIdentityMismatch);
        }

        Ok(FreshPlaceCredentialHandoff {
            signer,
            l2,
            teardown: FreshPlaceCredentialTeardown {
                directory,
                private_key: Some(private_key_file),
                l2: StagedL2Files::new(api_key_file, l2_secret_file, passphrase_file),
            },
        })
    }
}

impl fmt::Debug for FreshPlaceCredentialFiles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FreshPlaceCredentialFiles([REDACTED])")
    }
}

/// Three staged L2 entries for recovery. There is deliberately no private-key
/// field, private-key entry argument, signer authority, or place capability.
#[must_use = "a recovery-only credential file specification must be loaded or deliberately dropped"]
pub(super) struct RecoveryOnlyCredentialFiles {
    directory: PathBuf,
    api_key_entry: String,
    l2_secret_entry: String,
    passphrase_entry: String,
}

impl RecoveryOnlyCredentialFiles {
    pub(super) fn new(
        directory: PathBuf,
        api_key_entry: String,
        l2_secret_entry: String,
        passphrase_entry: String,
    ) -> Self {
        Self {
            directory,
            api_key_entry,
            l2_secret_entry,
            passphrase_entry,
        }
    }

    /// Load and structurally bind only the three L2 recovery secrets.
    pub(super) fn load(
        self,
        configured_signer: EoaAddress,
    ) -> Result<RecoveryOnlyCredentialHandoff, CredentialCustodyError> {
        let loaded = load_exact(
            self.directory,
            [
                EntrySpec::new(CredentialRole::ApiKey, self.api_key_entry, API_KEY_BYTES),
                EntrySpec::new(
                    CredentialRole::L2Secret,
                    self.l2_secret_entry,
                    MAX_L2_SECRET_BYTES,
                ),
                EntrySpec::new(
                    CredentialRole::Passphrase,
                    self.passphrase_entry,
                    MAX_PASSPHRASE_BYTES,
                ),
            ],
        )?;
        let LoadedCredentials {
            directory,
            values,
            staged,
        } = loaded;
        let [mut api_key, mut l2_secret, mut passphrase] = values;
        let [api_key_file, l2_secret_file, passphrase_file] = staged;
        let l2 = L2Credentials::bind(
            &configured_signer.to_string(),
            L2CredentialInput::new(
                take_secret_string(&mut api_key),
                take_secret_string(&mut l2_secret),
                take_secret_string(&mut passphrase),
            ),
        )
        .map_err(|_| CredentialCustodyError::L2BindingRejected)?;
        if l2.address() != configured_signer {
            return Err(CredentialCustodyError::CredentialIdentityMismatch);
        }

        Ok(RecoveryOnlyCredentialHandoff {
            l2,
            teardown: RecoveryOnlyCredentialTeardown {
                directory,
                l2: StagedL2Files::new(api_key_file, l2_secret_file, passphrase_file),
            },
        })
    }
}

impl fmt::Debug for RecoveryOnlyCredentialFiles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryOnlyCredentialFiles([REDACTED])")
    }
}

/// Move-only fresh-place authority handoff. Only its immediate authority parent
/// can consume it; no crate sibling can name or decompose this custody, and no
/// operation exposes secret text or bytes.
#[must_use = "credential authorities and their staged-file teardown must remain owned"]
pub(super) struct FreshPlaceCredentialHandoff {
    signer: FixedEoaSigner,
    l2: L2Credentials,
    teardown: FreshPlaceCredentialTeardown,
}

impl FreshPlaceCredentialHandoff {
    pub(super) fn into_authorities_and_teardown(
        self,
    ) -> (FixedEoaSigner, L2Credentials, FreshPlaceCredentialTeardown) {
        let Self {
            signer,
            l2,
            teardown,
        } = self;
        (signer, l2, teardown)
    }
}

impl fmt::Debug for FreshPlaceCredentialHandoff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FreshPlaceCredentialHandoff([REDACTED])")
    }
}

/// Move-only recovery authority handoff. Only its immediate authority parent
/// can consume it; no crate sibling can name or decompose this custody. Its
/// shape cannot carry a signer or a private-key teardown capability.
#[must_use = "the L2 authority and its staged-file teardown must remain owned"]
pub(super) struct RecoveryOnlyCredentialHandoff {
    l2: L2Credentials,
    teardown: RecoveryOnlyCredentialTeardown,
}

impl RecoveryOnlyCredentialHandoff {
    pub(super) fn into_authority_and_teardown(
        self,
    ) -> (L2Credentials, RecoveryOnlyCredentialTeardown) {
        let Self { l2, teardown } = self;
        (l2, teardown)
    }
}

impl fmt::Debug for RecoveryOnlyCredentialHandoff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryOnlyCredentialHandoff([REDACTED])")
    }
}

/// Exact staged-file capabilities retained by fresh mode. The private-key
/// entry is independently removable before terminal L2 teardown.
#[must_use = "dropping this token leaves any remaining staged credentials unchanged"]
pub(super) struct FreshPlaceCredentialTeardown {
    directory: PinnedDirectory,
    private_key: Option<StagedCredential>,
    l2: StagedL2Files,
}

impl FreshPlaceCredentialTeardown {
    pub(super) fn remove_private_key(&mut self) -> Result<(), CredentialCustodyError> {
        unlink_slot(&mut self.directory, &mut self.private_key, true)
    }

    pub(super) fn remove_l2_files(&mut self) -> Result<(), CredentialCustodyError> {
        if self.private_key.is_some() {
            return Err(CredentialCustodyError::PrivateKeyTeardownRequired);
        }
        self.l2.remove_all(&mut self.directory)
    }

    #[cfg(test)]
    fn is_complete(&self) -> bool {
        self.private_key.is_none() && self.l2.is_complete()
    }
}

impl fmt::Debug for FreshPlaceCredentialTeardown {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FreshPlaceCredentialTeardown([REDACTED])")
    }
}

/// Exact staged-file capabilities retained by recovery mode. Its type has no
/// private-key entry and cannot request private-key deletion.
#[must_use = "dropping this token leaves any remaining staged credentials unchanged"]
pub(super) struct RecoveryOnlyCredentialTeardown {
    directory: PinnedDirectory,
    l2: StagedL2Files,
}

impl RecoveryOnlyCredentialTeardown {
    pub(super) fn remove_l2_files(&mut self) -> Result<(), CredentialCustodyError> {
        self.l2.remove_all(&mut self.directory)
    }

    #[cfg(test)]
    fn is_complete(&self) -> bool {
        self.l2.is_complete()
    }
}

impl fmt::Debug for RecoveryOnlyCredentialTeardown {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryOnlyCredentialTeardown([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CredentialRole {
    PrivateKey,
    ApiKey,
    L2Secret,
    Passphrase,
}

impl fmt::Display for CredentialRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PrivateKey => "private-key",
            Self::ApiKey => "API-key",
            Self::L2Secret => "L2-secret",
            Self::Passphrase => "passphrase",
        })
    }
}

/// Content-free credential failures. No variant retains a caller path, file
/// name, secret value, or operating-system error string.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(super) enum CredentialCustodyError {
    #[error("the credential directory must be one real protected directory")]
    InvalidDirectory,
    #[error("the credential directory is not owned by the effective user")]
    WrongDirectoryOwner,
    #[error("the credential directory mode is not exactly 0700")]
    WrongDirectoryMode,
    #[error("the credential directory changed during descriptor-pinned loading")]
    DirectoryChanged,
    #[error("the effective user identity is unavailable")]
    EffectiveUidUnavailable,
    #[error("the {0} entry is not one canonical direct basename")]
    InvalidEntryName(CredentialRole),
    #[error("credential entry basenames must be distinct")]
    DuplicateEntryName,
    #[error("the protected {0} entry could not be opened safely")]
    Open(CredentialRole),
    #[error("the protected {0} entry could not be inspected")]
    Inspect(CredentialRole),
    #[error("the protected {0} entry is not a regular file")]
    NotRegular(CredentialRole),
    #[error("the protected {0} entry is not owned by the effective user")]
    WrongOwner(CredentialRole),
    #[error("the protected {0} entry mode is not exactly 0600")]
    WrongMode(CredentialRole),
    #[error("the protected {0} entry does not have exactly one hard link")]
    WrongLinkCount(CredentialRole),
    #[error("the protected {0} entry exceeds its fixed size bound")]
    TooLarge(CredentialRole),
    #[error("two credential roles resolve to one inode")]
    DuplicateInode,
    #[error("the protected {0} entry could not be read")]
    Read(CredentialRole),
    #[error("the protected {0} entry changed during descriptor-pinned custody")]
    FileChanged(CredentialRole),
    #[error("the protected {0} entry is not canonical UTF-8")]
    NonUtf8(CredentialRole),
    #[error("the protected {0} value does not use its closed canonical grammar")]
    NonCanonical(CredentialRole),
    #[error("the private key does not bind the configured signer")]
    SignerBindingRejected,
    #[error("the L2 bundle grammar or signer binding is invalid")]
    L2BindingRejected,
    #[error("the private-key and L2 signer identities disagree")]
    CredentialIdentityMismatch,
    #[error("the private-key teardown must complete before fresh-mode L2 teardown")]
    PrivateKeyTeardownRequired,
    #[error("the staged {0} entry was already removed")]
    AlreadyRemoved(CredentialRole),
    #[error("the exact staged {0} inode could not be unlinked")]
    Unlink(CredentialRole),
    #[error("the staged {0} basename was populated again during teardown")]
    EntryReappeared(CredentialRole),
    #[error("the protected credential directory could not be synchronized durably")]
    DirectorySync,
    #[error("an internal fixed-size custody invariant failed")]
    InternalInvariant,
}

struct EntrySpec {
    role: CredentialRole,
    name: String,
    maximum_bytes: usize,
}

impl EntrySpec {
    fn new(role: CredentialRole, name: String, maximum_bytes: usize) -> Self {
        Self {
            role,
            name,
            maximum_bytes,
        }
    }
}

struct LoadedCredentials<const N: usize> {
    directory: PinnedDirectory,
    values: [Zeroizing<String>; N],
    staged: [StagedCredential; N],
}

fn load_exact<const N: usize>(
    directory_path: PathBuf,
    entries: [EntrySpec; N],
) -> Result<LoadedCredentials<N>, CredentialCustodyError> {
    for entry in &entries {
        validate_entry_name(entry.role, &entry.name)?;
    }
    for left in 0..entries.len() {
        for right in left + 1..entries.len() {
            if entries[left].name == entries[right].name {
                return Err(CredentialCustodyError::DuplicateEntryName);
            }
        }
    }

    let directory = PinnedDirectory::open(directory_path)?;
    let mut opened = Vec::with_capacity(N);
    for entry in &entries {
        opened.push(directory.open_entry(entry)?);
    }
    for left in 0..opened.len() {
        for right in left + 1..opened.len() {
            if opened[left].snapshot.inode() == opened[right].snapshot.inode() {
                return Err(CredentialCustodyError::DuplicateInode);
            }
        }
    }

    let snapshots = opened
        .iter()
        .map(|entry| entry.snapshot)
        .collect::<Vec<_>>();
    let mut values = Vec::with_capacity(N);
    for entry in &mut opened {
        values.push(entry.read()?);
    }
    for (entry, expected) in entries.iter().zip(&snapshots) {
        directory.revalidate_entry(entry, *expected)?;
    }
    directory.validate_load_stability()?;
    drop(opened);

    let staged = entries
        .into_iter()
        .zip(snapshots)
        .map(|(entry, snapshot)| StagedCredential {
            role: entry.role,
            name: entry.name,
            maximum_bytes: entry.maximum_bytes,
            snapshot,
        })
        .collect::<Vec<_>>();
    let values = match values.try_into() {
        Ok(values) => values,
        Err(_values) => return Err(CredentialCustodyError::InternalInvariant),
    };
    let staged = match staged.try_into() {
        Ok(staged) => staged,
        Err(_staged) => return Err(CredentialCustodyError::InternalInvariant),
    };
    Ok(LoadedCredentials {
        directory,
        values,
        staged,
    })
}

fn validate_entry_name(role: CredentialRole, entry: &str) -> Result<(), CredentialCustodyError> {
    let path = Path::new(entry);
    if entry.is_empty()
        || entry.len() > MAX_ENTRY_NAME_BYTES
        || path.components().count() != 1
        || matches!(entry, "." | "..")
        || !entry
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CredentialCustodyError::InvalidEntryName(role));
    }
    Ok(())
}

fn validate_canonical_value(
    role: CredentialRole,
    value: &str,
) -> Result<(), CredentialCustodyError> {
    let bytes = value.as_bytes();
    let canonical = match role {
        CredentialRole::PrivateKey => {
            bytes.len() == PRIVATE_KEY_BYTES
                && bytes.starts_with(b"0x")
                && bytes[2..]
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        }
        CredentialRole::ApiKey => {
            bytes.len() == API_KEY_BYTES
                && bytes.iter().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        *byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
                    }
                })
        }
        CredentialRole::L2Secret => {
            !bytes.is_empty()
                && bytes.len() <= MAX_L2_SECRET_BYTES
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'='))
                && bytes
                    .iter()
                    .position(|byte| *byte == b'=')
                    .is_none_or(|first_padding| {
                        bytes[first_padding..].len() <= 2
                            && bytes[first_padding..].iter().all(|byte| *byte == b'=')
                    })
        }
        CredentialRole::Passphrase => {
            !bytes.is_empty()
                && bytes.len() <= MAX_PASSPHRASE_BYTES
                && bytes.iter().all(|byte| (0x21..=0x7e).contains(byte))
        }
    };
    if !canonical {
        return Err(CredentialCustodyError::NonCanonical(role));
    }
    Ok(())
}

fn take_secret_string(value: &mut Zeroizing<String>) -> String {
    std::mem::take(&mut **value)
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
    fn from_metadata(metadata: &Metadata) -> Self {
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

    const fn inode(self) -> (u64, u64) {
        (self.dev, self.ino)
    }
}

struct OpenedCredential {
    role: CredentialRole,
    file: File,
    snapshot: Snapshot,
}

impl OpenedCredential {
    fn read(&mut self) -> Result<Zeroizing<String>, CredentialCustodyError> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(
            usize::try_from(self.snapshot.len)
                .unwrap_or(0)
                .min(MAX_L2_SECRET_BYTES),
        ));
        self.file
            .by_ref()
            .take(self.snapshot.len.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| CredentialCustodyError::Read(self.role))?;
        if bytes.len() as u64 != self.snapshot.len {
            return Err(CredentialCustodyError::FileChanged(self.role));
        }
        let after = self
            .file
            .metadata()
            .map_err(|_| CredentialCustodyError::Inspect(self.role))?;
        if !after.is_file() || Snapshot::from_metadata(&after) != self.snapshot {
            return Err(CredentialCustodyError::FileChanged(self.role));
        }
        let text = std::str::from_utf8(bytes.as_slice())
            .map_err(|_| CredentialCustodyError::NonUtf8(self.role))?;
        validate_canonical_value(self.role, text)?;
        let value = match String::from_utf8(std::mem::take(&mut *bytes)) {
            Ok(value) => value,
            Err(error) => {
                let _bytes = Zeroizing::new(error.into_bytes());
                return Err(CredentialCustodyError::NonUtf8(self.role));
            }
        };
        Ok(Zeroizing::new(value))
    }
}

struct StagedCredential {
    role: CredentialRole,
    name: String,
    maximum_bytes: usize,
    snapshot: Snapshot,
}

struct StagedL2Files {
    api_key: Option<StagedCredential>,
    l2_secret: Option<StagedCredential>,
    passphrase: Option<StagedCredential>,
}

impl StagedL2Files {
    fn new(
        api_key: StagedCredential,
        l2_secret: StagedCredential,
        passphrase: StagedCredential,
    ) -> Self {
        Self {
            api_key: Some(api_key),
            l2_secret: Some(l2_secret),
            passphrase: Some(passphrase),
        }
    }

    fn remove_all(
        &mut self,
        directory: &mut PinnedDirectory,
    ) -> Result<(), CredentialCustodyError> {
        unlink_slot(directory, &mut self.api_key, false)?;
        unlink_slot(directory, &mut self.l2_secret, false)?;
        unlink_slot(directory, &mut self.passphrase, false)
    }

    #[cfg(test)]
    fn is_complete(&self) -> bool {
        self.api_key.is_none() && self.l2_secret.is_none() && self.passphrase.is_none()
    }
}

struct PinnedDirectory {
    file: File,
    original_path: PathBuf,
    descriptor_root: PathBuf,
    load_snapshot: Snapshot,
    effective_uid: u32,
}

impl PinnedDirectory {
    fn open(original_path: PathBuf) -> Result<Self, CredentialCustodyError> {
        let path_metadata = std::fs::symlink_metadata(&original_path)
            .map_err(|_| CredentialCustodyError::InvalidDirectory)?;
        let effective_uid = effective_uid()?;
        validate_directory(&path_metadata, effective_uid)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&original_path)
            .map_err(|_| CredentialCustodyError::InvalidDirectory)?;
        let metadata = file
            .metadata()
            .map_err(|_| CredentialCustodyError::InvalidDirectory)?;
        validate_directory(&metadata, effective_uid)?;
        let load_snapshot = Snapshot::from_metadata(&metadata);
        if load_snapshot != Snapshot::from_metadata(&path_metadata) {
            return Err(CredentialCustodyError::DirectoryChanged);
        }
        let descriptor_root = PathBuf::from("/proc/self/fd").join(file.as_raw_fd().to_string());
        Ok(Self {
            file,
            original_path,
            descriptor_root,
            load_snapshot,
            effective_uid,
        })
    }

    fn open_entry(&self, entry: &EntrySpec) -> Result<OpenedCredential, CredentialCustodyError> {
        self.open_named(entry.role, &entry.name, entry.maximum_bytes)
    }

    fn open_named(
        &self,
        role: CredentialRole,
        name: &str,
        maximum_bytes: usize,
    ) -> Result<OpenedCredential, CredentialCustodyError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.descriptor_root.join(name))
            .map_err(|_| CredentialCustodyError::Open(role))?;
        let metadata = file
            .metadata()
            .map_err(|_| CredentialCustodyError::Inspect(role))?;
        validate_file(&metadata, self.effective_uid, role, maximum_bytes)?;
        Ok(OpenedCredential {
            role,
            file,
            snapshot: Snapshot::from_metadata(&metadata),
        })
    }

    fn revalidate_entry(
        &self,
        entry: &EntrySpec,
        expected: Snapshot,
    ) -> Result<(), CredentialCustodyError> {
        let reopened = self.open_entry(entry)?;
        if reopened.snapshot != expected {
            return Err(CredentialCustodyError::FileChanged(entry.role));
        }
        Ok(())
    }

    fn validate_load_stability(&self) -> Result<(), CredentialCustodyError> {
        let descriptor = self
            .file
            .metadata()
            .map_err(|_| CredentialCustodyError::DirectoryChanged)?;
        let by_path = std::fs::symlink_metadata(&self.original_path)
            .map_err(|_| CredentialCustodyError::DirectoryChanged)?;
        validate_directory(&descriptor, self.effective_uid)?;
        validate_directory(&by_path, self.effective_uid)?;
        if Snapshot::from_metadata(&descriptor) != self.load_snapshot
            || Snapshot::from_metadata(&by_path) != self.load_snapshot
            || by_path.file_type().is_symlink()
        {
            return Err(CredentialCustodyError::DirectoryChanged);
        }
        Ok(())
    }

    fn validate_teardown_identity(&self) -> Result<(), CredentialCustodyError> {
        let descriptor = self
            .file
            .metadata()
            .map_err(|_| CredentialCustodyError::DirectoryChanged)?;
        let by_path = std::fs::symlink_metadata(&self.original_path)
            .map_err(|_| CredentialCustodyError::DirectoryChanged)?;
        validate_directory(&descriptor, self.effective_uid)?;
        validate_directory(&by_path, self.effective_uid)?;
        let expected = self.load_snapshot.inode();
        if Snapshot::from_metadata(&descriptor).inode() != expected
            || Snapshot::from_metadata(&by_path).inode() != expected
            || by_path.file_type().is_symlink()
        {
            return Err(CredentialCustodyError::DirectoryChanged);
        }
        Ok(())
    }

    fn unlink_exact(&mut self, staged: &StagedCredential) -> Result<(), ExactUnlinkFailure> {
        self.validate_teardown_identity()
            .map_err(ExactUnlinkFailure::before)?;
        let held = self
            .open_named(staged.role, &staged.name, staged.maximum_bytes)
            .map_err(ExactUnlinkFailure::before)?;
        if held.snapshot != staged.snapshot {
            return Err(ExactUnlinkFailure::before(
                CredentialCustodyError::FileChanged(staged.role),
            ));
        }
        std::fs::remove_file(self.descriptor_root.join(&staged.name))
            .map_err(|_| ExactUnlinkFailure::before(CredentialCustodyError::Unlink(staged.role)))?;

        let after = held.file.metadata().map_err(|_| {
            ExactUnlinkFailure::after_name_unlink(CredentialCustodyError::Inspect(staged.role))
        })?;
        if (after.dev(), after.ino()) != staged.snapshot.inode() || after.nlink() != 0 {
            return Err(ExactUnlinkFailure::after_name_unlink(
                CredentialCustodyError::Unlink(staged.role),
            ));
        }
        match std::fs::symlink_metadata(self.descriptor_root.join(&staged.name)) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            _ => {
                return Err(ExactUnlinkFailure::after_name_unlink(
                    CredentialCustodyError::EntryReappeared(staged.role),
                ));
            }
        }
        self.file.sync_all().map_err(|_| {
            ExactUnlinkFailure::after_name_unlink(CredentialCustodyError::DirectorySync)
        })?;
        self.validate_teardown_identity()
            .map_err(ExactUnlinkFailure::after_name_unlink)
    }
}

impl fmt::Debug for PinnedDirectory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PinnedDirectory([REDACTED])")
    }
}

struct ExactUnlinkFailure {
    error: CredentialCustodyError,
    retain_staged_token: bool,
}

impl ExactUnlinkFailure {
    const fn before(error: CredentialCustodyError) -> Self {
        Self {
            error,
            retain_staged_token: true,
        }
    }

    const fn after_name_unlink(error: CredentialCustodyError) -> Self {
        Self {
            error,
            retain_staged_token: false,
        }
    }
}

fn unlink_slot(
    directory: &mut PinnedDirectory,
    slot: &mut Option<StagedCredential>,
    error_if_absent: bool,
) -> Result<(), CredentialCustodyError> {
    let Some(staged) = slot.take() else {
        return if error_if_absent {
            Err(CredentialCustodyError::AlreadyRemoved(
                CredentialRole::PrivateKey,
            ))
        } else {
            Ok(())
        };
    };
    match directory.unlink_exact(&staged) {
        Ok(()) => Ok(()),
        Err(failure) => {
            if failure.retain_staged_token {
                *slot = Some(staged);
            }
            Err(failure.error)
        }
    }
}

fn validate_directory(
    metadata: &Metadata,
    effective_uid: u32,
) -> Result<(), CredentialCustodyError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CredentialCustodyError::InvalidDirectory);
    }
    if metadata.uid() != effective_uid {
        return Err(CredentialCustodyError::WrongDirectoryOwner);
    }
    if metadata.mode() & 0o7777 != 0o700 {
        return Err(CredentialCustodyError::WrongDirectoryMode);
    }
    Ok(())
}

fn validate_file(
    metadata: &Metadata,
    effective_uid: u32,
    role: CredentialRole,
    maximum_bytes: usize,
) -> Result<(), CredentialCustodyError> {
    if !metadata.is_file() {
        return Err(CredentialCustodyError::NotRegular(role));
    }
    if metadata.uid() != effective_uid {
        return Err(CredentialCustodyError::WrongOwner(role));
    }
    if metadata.mode() & 0o7777 != 0o600 {
        return Err(CredentialCustodyError::WrongMode(role));
    }
    if metadata.nlink() != 1 {
        return Err(CredentialCustodyError::WrongLinkCount(role));
    }
    if metadata.len() > maximum_bytes as u64 {
        return Err(CredentialCustodyError::TooLarge(role));
    }
    Ok(())
}

fn effective_uid() -> Result<u32, CredentialCustodyError> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|_| CredentialCustodyError::EffectiveUidUnavailable)?;
    status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_ascii_whitespace().nth(2))
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(CredentialCustodyError::EffectiveUidUnavailable)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
        path::Path,
        thread,
        time::Duration,
    };

    use tempfile::TempDir;

    use super::*;

    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const OTHER_SIGNER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const L2_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";

    fn write(directory: &Path, name: &str, value: &str) {
        let path = directory.join(name);
        fs::write(&path, value).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn protected_directory() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    fn stage_four() -> TempDir {
        let directory = protected_directory();
        write(directory.path(), "private-key", KEY);
        write(directory.path(), "api-key", API_KEY);
        write(directory.path(), "l2-secret", L2_SECRET);
        write(directory.path(), "passphrase", PASSPHRASE);
        directory
    }

    fn fresh_files(directory: &Path) -> FreshPlaceCredentialFiles {
        FreshPlaceCredentialFiles::new(
            directory.to_owned(),
            "private-key".into(),
            "api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
        )
    }

    fn recovery_files(directory: &Path) -> RecoveryOnlyCredentialFiles {
        RecoveryOnlyCredentialFiles::new(
            directory.to_owned(),
            "api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
        )
    }

    fn signer() -> EoaAddress {
        EoaAddress::parse(SIGNER).unwrap()
    }

    #[test]
    fn fresh_handoff_is_redacted_bound_and_tears_down_in_order() {
        let directory = stage_four();
        let handoff = fresh_files(directory.path()).load(signer()).unwrap();
        assert_eq!(
            format!("{handoff:?}"),
            "FreshPlaceCredentialHandoff([REDACTED])"
        );
        let (signer_authority, l2_authority, mut teardown) =
            handoff.into_authorities_and_teardown();
        assert_eq!(signer_authority.address(), signer());
        assert_eq!(l2_authority.address(), signer());
        assert_eq!(
            teardown.remove_l2_files(),
            Err(CredentialCustodyError::PrivateKeyTeardownRequired)
        );

        teardown.remove_private_key().unwrap();
        assert!(!directory.path().join("private-key").exists());
        assert!(directory.path().join("api-key").exists());
        assert_eq!(
            teardown.remove_private_key(),
            Err(CredentialCustodyError::AlreadyRemoved(
                CredentialRole::PrivateKey
            ))
        );
        teardown.remove_l2_files().unwrap();
        assert!(teardown.is_complete());
        assert!(!directory.path().join("api-key").exists());
        assert!(!directory.path().join("l2-secret").exists());
        assert!(!directory.path().join("passphrase").exists());
    }

    #[test]
    fn recovery_loads_and_removes_only_three_l2_files() {
        let directory = stage_four();
        let handoff = recovery_files(directory.path()).load(signer()).unwrap();
        assert_eq!(
            format!("{handoff:?}"),
            "RecoveryOnlyCredentialHandoff([REDACTED])"
        );
        let (l2, mut teardown) = handoff.into_authority_and_teardown();
        assert_eq!(l2.address(), signer());
        teardown.remove_l2_files().unwrap();
        assert!(teardown.is_complete());
        assert!(directory.path().join("private-key").exists());
        assert!(!directory.path().join("api-key").exists());
        assert!(!directory.path().join("l2-secret").exists());
        assert!(!directory.path().join("passphrase").exists());
    }

    #[test]
    fn fresh_signer_mismatch_rejects_without_removing_any_file() {
        let directory = stage_four();
        let error = fresh_files(directory.path())
            .load(EoaAddress::parse(OTHER_SIGNER).unwrap())
            .unwrap_err();
        assert_eq!(error, CredentialCustodyError::SignerBindingRejected);
        for name in ["private-key", "api-key", "l2-secret", "passphrase"] {
            assert!(directory.path().join(name).exists());
        }
    }

    #[test]
    fn malformed_secret_and_sensitive_path_never_enter_errors() {
        let directory = protected_directory();
        let marker = "SECRET!MARKER-must-not-render";
        write(directory.path(), "api-key", API_KEY);
        write(directory.path(), "l2-secret", marker);
        write(directory.path(), "passphrase", PASSPHRASE);
        let error = recovery_files(directory.path()).load(signer()).unwrap_err();
        assert_eq!(
            error,
            CredentialCustodyError::NonCanonical(CredentialRole::L2Secret)
        );
        for rendered in [format!("{error}"), format!("{error:?}")] {
            assert!(!rendered.contains(marker));
            assert!(!rendered.contains(directory.path().to_string_lossy().as_ref()));
        }
    }

    #[test]
    fn exact_modes_and_no_follow_are_enforced() {
        let directory = stage_four();
        fs::set_permissions(
            directory.path().join("api-key"),
            fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        assert_eq!(
            recovery_files(directory.path()).load(signer()).unwrap_err(),
            CredentialCustodyError::WrongMode(CredentialRole::ApiKey)
        );

        fs::set_permissions(
            directory.path().join("api-key"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::rename(
            directory.path().join("l2-secret"),
            directory.path().join("real-secret"),
        )
        .unwrap();
        symlink("real-secret", directory.path().join("l2-secret")).unwrap();
        assert_eq!(
            recovery_files(directory.path()).load(signer()).unwrap_err(),
            CredentialCustodyError::Open(CredentialRole::L2Secret)
        );
    }

    #[test]
    fn hard_links_and_noncanonical_entry_names_are_rejected() {
        let directory = stage_four();
        fs::hard_link(
            directory.path().join("api-key"),
            directory.path().join("api-key-link"),
        )
        .unwrap();
        assert_eq!(
            recovery_files(directory.path()).load(signer()).unwrap_err(),
            CredentialCustodyError::WrongLinkCount(CredentialRole::ApiKey)
        );

        let invalid = RecoveryOnlyCredentialFiles::new(
            directory.path().to_owned(),
            "../api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
        );
        assert_eq!(
            invalid.load(signer()).unwrap_err(),
            CredentialCustodyError::InvalidEntryName(CredentialRole::ApiKey)
        );
    }

    #[test]
    fn teardown_rejects_same_inode_content_change_using_time_snapshot() {
        let directory = stage_four();
        let handoff = fresh_files(directory.path()).load(signer()).unwrap();
        let (_signer, _l2, mut teardown) = handoff.into_authorities_and_teardown();
        thread::sleep(Duration::from_millis(2));
        write(directory.path(), "private-key", KEY);

        assert_eq!(
            teardown.remove_private_key(),
            Err(CredentialCustodyError::FileChanged(
                CredentialRole::PrivateKey
            ))
        );
        assert!(directory.path().join("private-key").exists());
    }

    #[test]
    fn teardown_refuses_a_replaced_inode_and_does_not_delete_it() {
        let directory = stage_four();
        let handoff = fresh_files(directory.path()).load(signer()).unwrap();
        let (_signer, _l2, mut teardown) = handoff.into_authorities_and_teardown();
        fs::rename(
            directory.path().join("private-key"),
            directory.path().join("original-private-key"),
        )
        .unwrap();
        write(directory.path(), "private-key", KEY);

        assert_eq!(
            teardown.remove_private_key(),
            Err(CredentialCustodyError::FileChanged(
                CredentialRole::PrivateKey
            ))
        );
        assert!(directory.path().join("private-key").exists());
        assert!(directory.path().join("original-private-key").exists());
    }

    #[test]
    fn directory_must_be_exact_owner_only_mode() {
        let directory = stage_four();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(
            recovery_files(directory.path()).load(signer()).unwrap_err(),
            CredentialCustodyError::WrongDirectoryMode
        );
    }
}
