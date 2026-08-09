use std::{fmt, io, path::Path};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use reap_polymarket_live_adapter::PmReadOnlyCredentialInput;
use reap_polymarket_wire::MAX_PUBLIC_REST_BODY_BYTES;
use thiserror::Error;
use zeroize::{Zeroize as _, Zeroizing};

pub const MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES: usize = 512;

const MAX_CREDENTIAL_ENTRY_BYTES: usize = 128;
const MAX_L2_SECRET_ENCODED_BYTES: usize = 172;
const MAX_L2_SECRET_DECODED_BYTES: usize = 128;
const MAX_L2_PASSPHRASE_BYTES: usize = 128;
const MAX_RETAINED_PUBLIC_BODY_BASE64_BYTES: usize = MAX_PUBLIC_REST_BODY_BYTES.div_ceil(3) * 4;

/// Move-only, zeroizing credentials loaded from one protected runtime directory.
///
/// The bundle exposes no secret getter. Its collector-only consuming operation
/// transfers the three strings into the adapter while retaining an opaque,
/// zeroizing egress-comparison guard until artifact commit.
pub struct PmReadOnlyCredentialBundle {
    api_key: Zeroizing<String>,
    secret: Zeroizing<String>,
    passphrase: Zeroizing<String>,
}

impl PmReadOnlyCredentialBundle {
    fn artifact_secret_guard(&self) -> PmReadOnlyArtifactSecretGuard {
        PmReadOnlyArtifactSecretGuard::from_credentials(
            &self.api_key,
            &self.secret,
            &self.passphrase,
        )
    }

    #[cfg(test)]
    pub(crate) fn ensure_persisted_config_is_secret_free(
        &self,
        canonical_config: &[u8],
    ) -> Result<(), PmReadOnlyCredentialError> {
        self.artifact_secret_guard()
            .ensure_config_is_secret_free(canonical_config)
    }

    pub(crate) fn into_adapter_input_and_artifact_guard(
        mut self,
    ) -> (PmReadOnlyCredentialInput, PmReadOnlyArtifactSecretGuard) {
        let guard = self.artifact_secret_guard();
        let input = PmReadOnlyCredentialInput::new(
            std::mem::take(&mut *self.api_key),
            std::mem::take(&mut *self.secret),
            std::mem::take(&mut *self.passphrase),
        );
        (input, guard)
    }
}

/// Opaque, zeroizing egress guard for bytes that must never reach an artifact.
///
/// The guard is not an authentication or signing capability. It deliberately
/// exposes only fail-closed membership checks, has no getters, and is retained
/// by the collector until the final serialized bytes have been checked.
pub(crate) struct PmReadOnlyArtifactSecretGuard {
    aliases: Vec<Zeroizing<Vec<u8>>>,
}

impl PmReadOnlyArtifactSecretGuard {
    fn from_credentials(api_key: &str, secret: &str, passphrase: &str) -> Self {
        let mut aliases = Vec::new();
        append_api_key_aliases(&mut aliases, api_key.as_bytes());
        append_aliases(&mut aliases, secret.as_bytes());
        append_aliases(&mut aliases, passphrase.as_bytes());
        if let Ok(decoded) = URL_SAFE.decode(secret.as_bytes()) {
            let decoded = Zeroizing::new(decoded);
            append_aliases(&mut aliases, &decoded);
        }
        Self { aliases }
    }

    pub(crate) fn ensure_config_is_secret_free(
        &self,
        canonical_config: &[u8],
    ) -> Result<(), PmReadOnlyCredentialError> {
        if self.contains_alias(canonical_config) {
            return Err(PmReadOnlyCredentialError::PersistedValueCollision);
        }
        Ok(())
    }

    pub(crate) fn ensure_artifact_is_secret_free(
        &self,
        artifact: &[u8],
    ) -> Result<(), PmReadOnlyCredentialError> {
        if self.contains_alias(artifact) {
            return Err(PmReadOnlyCredentialError::ArtifactValueCollision);
        }
        Ok(())
    }

    pub(crate) fn ensure_base64_artifact_value_is_secret_free(
        &self,
        encoded: &str,
    ) -> Result<(), PmReadOnlyCredentialError> {
        if encoded.len() > MAX_RETAINED_PUBLIC_BODY_BASE64_BYTES {
            return Err(PmReadOnlyCredentialError::ArtifactEncodingInvalid);
        }
        let decoded = Zeroizing::new(
            STANDARD
                .decode(encoded.as_bytes())
                .map_err(|_| PmReadOnlyCredentialError::ArtifactEncodingInvalid)?,
        );
        if decoded.len() > MAX_PUBLIC_REST_BODY_BYTES {
            return Err(PmReadOnlyCredentialError::ArtifactEncodingInvalid);
        }
        self.ensure_artifact_is_secret_free(&decoded)?;

        let _: &serde_json::value::RawValue = serde_json::from_slice(decoded.as_slice())
            .map_err(|_| PmReadOnlyCredentialError::ArtifactEncodingInvalid)?;
        if self
            .semantic_json_contains_alias(decoded.as_slice())
            .map_err(|()| PmReadOnlyCredentialError::ArtifactEncodingInvalid)?
        {
            return Err(PmReadOnlyCredentialError::ArtifactValueCollision);
        }
        Ok(())
    }

    fn contains_alias(&self, candidate: &[u8]) -> bool {
        self.aliases
            .iter()
            .any(|alias| contains_bytes(candidate, alias))
    }

    fn semantic_json_contains_alias(&self, json: &[u8]) -> Result<bool, ()> {
        // The RawValue parse above has already validated the complete JSON
        // grammar. Decode every string token separately so duplicate object
        // members cannot hide an earlier secret-bearing value. One bounded,
        // preallocated buffer avoids reallocations that could retain decoded
        // secret bytes; initialized bytes are scrubbed after every token.
        let mut decoded = Zeroizing::new(Vec::with_capacity(json.len()));
        let mut collision = false;
        let mut cursor = 0;
        while cursor < json.len() {
            if json[cursor] != b'"' {
                cursor += 1;
                continue;
            }

            cursor += 1;
            while cursor < json.len() && json[cursor] != b'"' {
                if json[cursor] != b'\\' {
                    decoded.push(json[cursor]);
                    cursor += 1;
                    continue;
                }

                cursor += 1;
                let escaped = *json.get(cursor).ok_or(())?;
                cursor += 1;
                match escaped {
                    b'"' | b'\\' | b'/' => decoded.push(escaped),
                    b'b' => decoded.push(0x08),
                    b'f' => decoded.push(0x0c),
                    b'n' => decoded.push(b'\n'),
                    b'r' => decoded.push(b'\r'),
                    b't' => decoded.push(b'\t'),
                    b'u' => {
                        let first = decode_json_hex_quad(json, &mut cursor).ok_or(())?;
                        let scalar = if (0xd800..=0xdbff).contains(&first) {
                            if json.get(cursor..cursor.saturating_add(2)) != Some(b"\\u".as_slice())
                            {
                                return Err(());
                            }
                            cursor += 2;
                            let second = decode_json_hex_quad(json, &mut cursor).ok_or(())?;
                            if !(0xdc00..=0xdfff).contains(&second) {
                                return Err(());
                            }
                            0x1_0000
                                + (u32::from(first) - 0xd800) * 0x400
                                + (u32::from(second) - 0xdc00)
                        } else {
                            if (0xdc00..=0xdfff).contains(&first) {
                                return Err(());
                            }
                            u32::from(first)
                        };
                        let character = char::from_u32(scalar).ok_or(())?;
                        let mut encoded = [0_u8; 4];
                        decoded.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                        encoded.zeroize();
                    }
                    _ => return Err(()),
                }
            }
            if json.get(cursor) != Some(&b'"') {
                return Err(());
            }
            cursor += 1;
            collision |= self.contains_alias(&decoded);
            decoded.as_mut_slice().zeroize();
            decoded.clear();
        }
        Ok(collision)
    }

    #[cfg(test)]
    pub(crate) fn for_test(api_key: &str, secret: &str, passphrase: &str) -> Self {
        Self::from_credentials(api_key, secret, passphrase)
    }
}

impl fmt::Debug for PmReadOnlyArtifactSecretGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmReadOnlyArtifactSecretGuard([REDACTED])")
    }
}

impl fmt::Debug for PmReadOnlyCredentialBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmReadOnlyCredentialBundle([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmReadOnlyCredentialKind {
    ApiKey,
    Secret,
    Passphrase,
}

impl fmt::Display for PmReadOnlyCredentialKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ApiKey => "API-key",
            Self::Secret => "HMAC-secret",
            Self::Passphrase => "passphrase",
        })
    }
}

/// Fixed, content-free failures for protected credential loading.
///
/// Paths and secret bytes are deliberately absent. I/O sources contain only an
/// operating-system failure and never a caller-provided credential value.
#[derive(Debug, Error)]
pub enum PmReadOnlyCredentialError {
    #[error("credential loading is supported only on Linux")]
    UnsupportedPlatform,
    #[error("failed to inspect the protected credential directory")]
    InspectDirectory(#[source] io::Error),
    #[error("the protected credential directory must be a real directory, not a symbolic link")]
    InvalidDirectory,
    #[error("the protected credential directory must be owned by the current effective user")]
    WrongDirectoryOwner,
    #[error("the protected credential directory mode must be exactly 0500 or 0700")]
    WrongDirectoryMode,
    #[error("the protected credential directory changed while credentials were loaded")]
    DirectoryChanged,
    #[error("could not determine the current effective user ID")]
    EffectiveUidUnavailable,
    #[error("the {kind} credential entry must be one safe basename")]
    InvalidEntryName { kind: PmReadOnlyCredentialKind },
    #[error("credential entry basenames must be distinct")]
    DuplicateEntryName,
    #[error("failed to open the protected {kind} credential file")]
    Open {
        kind: PmReadOnlyCredentialKind,
        #[source]
        source: io::Error,
    },
    #[error("failed to inspect the opened {kind} credential file")]
    InspectFile {
        kind: PmReadOnlyCredentialKind,
        #[source]
        source: io::Error,
    },
    #[error("the {kind} credential entry must be a regular file")]
    NotRegularFile { kind: PmReadOnlyCredentialKind },
    #[error("the {kind} credential file must be owned by the current effective user")]
    WrongOwner { kind: PmReadOnlyCredentialKind },
    #[error("the {kind} credential file mode must be exactly 0400 or 0600")]
    WrongMode { kind: PmReadOnlyCredentialKind },
    #[error("credential entries must resolve to three distinct files")]
    DuplicateInode,
    #[error("the {kind} credential file must have exactly one hard link")]
    WrongLinkCount { kind: PmReadOnlyCredentialKind },
    #[error("the {kind} credential file exceeds the 512-byte bound")]
    TooLarge { kind: PmReadOnlyCredentialKind },
    #[error("failed to read the protected {kind} credential file")]
    Read {
        kind: PmReadOnlyCredentialKind,
        #[source]
        source: io::Error,
    },
    #[error("the {kind} credential value is empty")]
    Empty { kind: PmReadOnlyCredentialKind },
    #[error("the {kind} credential value is not valid UTF-8")]
    NonUtf8 { kind: PmReadOnlyCredentialKind },
    #[error("the {kind} credential value contains a prohibited NUL or line break")]
    ProhibitedByte { kind: PmReadOnlyCredentialKind },
    #[error("the {kind} credential value does not use its canonical fixed grammar")]
    NonCanonical { kind: PmReadOnlyCredentialKind },
    #[error("the opened {kind} credential file changed while it was read")]
    FileChanged { kind: PmReadOnlyCredentialKind },
    #[error("a persisted non-secret configuration value aliases credential material")]
    PersistedValueCollision,
    #[error("a finalized artifact value aliases credential material")]
    ArtifactValueCollision,
    #[error("a finalized artifact contains an invalid retained-body encoding")]
    ArtifactEncodingInvalid,
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn decode_json_hex_quad(json: &[u8], cursor: &mut usize) -> Option<u16> {
    let end = cursor.checked_add(4)?;
    let digits = json.get(*cursor..end)?;
    let mut value = 0_u16;
    for digit in digits {
        let nibble = match *digit {
            b'0'..=b'9' => *digit - b'0',
            b'a'..=b'f' => *digit - b'a' + 10,
            b'A'..=b'F' => *digit - b'A' + 10,
            _ => return None,
        };
        value = value.checked_mul(16)?.checked_add(u16::from(nibble))?;
    }
    *cursor = end;
    Some(value)
}

fn append_api_key_aliases(aliases: &mut Vec<Zeroizing<Vec<u8>>>, value: &[u8]) {
    append_aliases(aliases, value);

    let uppercase = Zeroizing::new(value.iter().map(u8::to_ascii_uppercase).collect::<Vec<_>>());
    append_aliases(aliases, &uppercase);

    let compact = Zeroizing::new(
        value
            .iter()
            .copied()
            .filter(|byte| *byte != b'-')
            .collect::<Vec<_>>(),
    );
    append_aliases(aliases, &compact);

    let uppercase_compact = Zeroizing::new(
        compact
            .iter()
            .map(u8::to_ascii_uppercase)
            .collect::<Vec<_>>(),
    );
    append_aliases(aliases, &uppercase_compact);
}

fn append_aliases(aliases: &mut Vec<Zeroizing<Vec<u8>>>, value: &[u8]) {
    if value.is_empty() {
        return;
    }
    aliases.push(Zeroizing::new(value.to_vec()));
    for encoded in [
        STANDARD.encode(value),
        STANDARD_NO_PAD.encode(value),
        URL_SAFE.encode(value),
        URL_SAFE_NO_PAD.encode(value),
        hex(value, false),
        hex(value, true),
    ] {
        aliases.push(Zeroizing::new(encoded.into_bytes()));
    }
    for prefix in [b"0x".as_slice(), b"0X".as_slice()] {
        for uppercase in [false, true] {
            let mut prefixed = Vec::with_capacity(2 + value.len().saturating_mul(2));
            prefixed.extend_from_slice(prefix);
            let encoded = Zeroizing::new(hex(value, uppercase));
            prefixed.extend_from_slice(encoded.as_bytes());
            aliases.push(Zeroizing::new(prefixed));
        }
    }
    if let Ok(text) = std::str::from_utf8(value)
        && let Ok(encoded) = serde_json::to_vec(text)
    {
        if encoded.len() >= 2 {
            aliases.push(Zeroizing::new(encoded[1..encoded.len() - 1].to_vec()));
        }
        aliases.push(Zeroizing::new(encoded));
    }
}

fn hex(bytes: &[u8], uppercase: bool) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        if uppercase {
            write!(encoded, "{byte:02X}")
        } else {
            write!(encoded, "{byte:02x}")
        }
        .expect("writing to an owned String cannot fail");
    }
    encoded
}

/// Load the three named credential entries from one separately supplied
/// protected directory.
///
/// Entry values must be safe basenames. On Linux, each entry is opened with
/// `O_NOFOLLOW | O_CLOEXEC`, then all security decisions are made from the
/// opened descriptors rather than pre-open path metadata.
pub fn load_pm_read_only_credentials(
    directory: &Path,
    api_key_entry: &str,
    secret_entry: &str,
    passphrase_entry: &str,
) -> Result<PmReadOnlyCredentialBundle, PmReadOnlyCredentialError> {
    validate_entry(PmReadOnlyCredentialKind::ApiKey, api_key_entry)?;
    validate_entry(PmReadOnlyCredentialKind::Secret, secret_entry)?;
    validate_entry(PmReadOnlyCredentialKind::Passphrase, passphrase_entry)?;
    if api_key_entry == secret_entry
        || api_key_entry == passphrase_entry
        || secret_entry == passphrase_entry
    {
        return Err(PmReadOnlyCredentialError::DuplicateEntryName);
    }

    #[cfg(target_os = "linux")]
    {
        linux::load(directory, api_key_entry, secret_entry, passphrase_entry)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = directory;
        Err(PmReadOnlyCredentialError::UnsupportedPlatform)
    }
}

fn validate_entry(
    kind: PmReadOnlyCredentialKind,
    value: &str,
) -> Result<(), PmReadOnlyCredentialError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_ENTRY_BYTES
        || path.components().count() != 1
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(PmReadOnlyCredentialError::InvalidEntryName { kind });
    }
    Ok(())
}

fn validate_value(
    kind: PmReadOnlyCredentialKind,
    value: &str,
) -> Result<(), PmReadOnlyCredentialError> {
    if value.is_empty() {
        return Err(PmReadOnlyCredentialError::Empty { kind });
    }
    if value
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'\0' | b'\r' | b'\n'))
    {
        return Err(PmReadOnlyCredentialError::ProhibitedByte { kind });
    }

    let canonical = match kind {
        PmReadOnlyCredentialKind::ApiKey => valid_api_key(value.as_bytes()),
        PmReadOnlyCredentialKind::Secret => valid_secret(value),
        PmReadOnlyCredentialKind::Passphrase => valid_passphrase(value.as_bytes()),
    };
    if !canonical {
        return Err(PmReadOnlyCredentialError::NonCanonical { kind });
    }
    Ok(())
}

fn valid_api_key(value: &[u8]) -> bool {
    value.len() == 36
        && value.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        })
}

fn valid_secret(value: &str) -> bool {
    if value.len() > MAX_L2_SECRET_ENCODED_BYTES {
        return false;
    }
    let mut decoded = Zeroizing::new([0_u8; MAX_L2_SECRET_DECODED_BYTES + 1]);
    let decoded_len = match URL_SAFE.decode_slice(value.as_bytes(), &mut *decoded) {
        Ok(decoded_len) => decoded_len,
        Err(_) => return false,
    };
    if decoded_len == 0 || decoded_len > MAX_L2_SECRET_DECODED_BYTES {
        return false;
    }
    let encoded = Zeroizing::new(URL_SAFE.encode(&decoded[..decoded_len]));
    encoded.as_bytes() == value.as_bytes()
}

fn valid_passphrase(value: &[u8]) -> bool {
    value.len() <= MAX_L2_PASSPHRASE_BYTES && value.iter().all(|byte| (0x21..=0x7e).contains(byte))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        fs::{File, Metadata, OpenOptions},
        io::Read as _,
        os::unix::{
            fs::{MetadataExt as _, OpenOptionsExt as _},
            io::AsRawFd as _,
        },
        path::{Path, PathBuf},
    };

    use zeroize::Zeroizing;

    use super::{
        MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES, PmReadOnlyCredentialBundle,
        PmReadOnlyCredentialError, PmReadOnlyCredentialKind, validate_value,
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct CredentialFileSnapshot {
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

    impl CredentialFileSnapshot {
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
    }

    struct OpenedCredential {
        kind: PmReadOnlyCredentialKind,
        file: File,
        snapshot: CredentialFileSnapshot,
    }

    impl OpenedCredential {
        fn identity(&self) -> (u64, u64) {
            (self.snapshot.dev, self.snapshot.ino)
        }

        fn validate_link_count(&self) -> Result<(), PmReadOnlyCredentialError> {
            if self.snapshot.nlink != 1 {
                return Err(PmReadOnlyCredentialError::WrongLinkCount { kind: self.kind });
            }
            Ok(())
        }

        fn read(mut self) -> Result<Zeroizing<String>, PmReadOnlyCredentialError> {
            let mut bytes = Zeroizing::new(Vec::with_capacity(
                usize::try_from(self.snapshot.len)
                    .unwrap_or(MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES)
                    .min(MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES),
            ));
            self.file
                .by_ref()
                .take((MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|source| PmReadOnlyCredentialError::Read {
                    kind: self.kind,
                    source,
                })?;
            if bytes.len() > MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES {
                return Err(PmReadOnlyCredentialError::TooLarge { kind: self.kind });
            }
            let post_read =
                self.file
                    .metadata()
                    .map_err(|source| PmReadOnlyCredentialError::InspectFile {
                        kind: self.kind,
                        source,
                    })?;
            if !post_read.is_file()
                || CredentialFileSnapshot::from_metadata(&post_read) != self.snapshot
                || post_read.len() != bytes.len() as u64
            {
                return Err(PmReadOnlyCredentialError::FileChanged { kind: self.kind });
            }
            let text = std::str::from_utf8(bytes.as_slice())
                .map_err(|_| PmReadOnlyCredentialError::NonUtf8 { kind: self.kind })?;
            validate_value(self.kind, text)?;
            let owned = match String::from_utf8(std::mem::take(&mut *bytes)) {
                Ok(owned) => owned,
                Err(error) => {
                    let _bytes = Zeroizing::new(error.into_bytes());
                    return Err(PmReadOnlyCredentialError::NonUtf8 { kind: self.kind });
                }
            };
            Ok(Zeroizing::new(owned))
        }
    }

    pub(super) fn load(
        directory: &Path,
        api_key_entry: &str,
        secret_entry: &str,
        passphrase_entry: &str,
    ) -> Result<PmReadOnlyCredentialBundle, PmReadOnlyCredentialError> {
        let path_metadata = std::fs::symlink_metadata(directory)
            .map_err(PmReadOnlyCredentialError::InspectDirectory)?;
        if path_metadata.file_type().is_symlink() || !path_metadata.is_dir() {
            return Err(PmReadOnlyCredentialError::InvalidDirectory);
        }
        let effective_uid = current_effective_uid()?;
        if path_metadata.uid() != effective_uid {
            return Err(PmReadOnlyCredentialError::WrongDirectoryOwner);
        }
        if !matches!(path_metadata.mode() & 0o7777, 0o500 | 0o700) {
            return Err(PmReadOnlyCredentialError::WrongDirectoryMode);
        }

        // Pin the inspected directory before resolving any entry. Every
        // credential path below is relative to this one stable descriptor,
        // so a concurrent rename or replacement of the caller's pathname
        // cannot redirect a later entry open into another directory.
        let directory_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory)
            .map_err(PmReadOnlyCredentialError::InspectDirectory)?;
        let directory_metadata = directory_file
            .metadata()
            .map_err(PmReadOnlyCredentialError::InspectDirectory)?;
        if !directory_metadata.is_dir() {
            return Err(PmReadOnlyCredentialError::InvalidDirectory);
        }
        if !same_directory(&path_metadata, &directory_metadata) {
            return Err(PmReadOnlyCredentialError::DirectoryChanged);
        }
        if directory_metadata.uid() != effective_uid {
            return Err(PmReadOnlyCredentialError::WrongDirectoryOwner);
        }
        if !matches!(directory_metadata.mode() & 0o7777, 0o500 | 0o700) {
            return Err(PmReadOnlyCredentialError::WrongDirectoryMode);
        }
        let descriptor_root =
            PathBuf::from("/proc/self/fd").join(directory_file.as_raw_fd().to_string());

        let api_key = open(
            &descriptor_root,
            PmReadOnlyCredentialKind::ApiKey,
            api_key_entry,
            effective_uid,
        )?;
        let secret = open(
            &descriptor_root,
            PmReadOnlyCredentialKind::Secret,
            secret_entry,
            effective_uid,
        )?;
        let passphrase = open(
            &descriptor_root,
            PmReadOnlyCredentialKind::Passphrase,
            passphrase_entry,
            effective_uid,
        )?;

        if api_key.identity() == secret.identity()
            || api_key.identity() == passphrase.identity()
            || secret.identity() == passphrase.identity()
        {
            return Err(PmReadOnlyCredentialError::DuplicateInode);
        }
        api_key.validate_link_count()?;
        secret.validate_link_count()?;
        passphrase.validate_link_count()?;

        let api_key_snapshot = api_key.snapshot;
        let secret_snapshot = secret.snapshot;
        let passphrase_snapshot = passphrase.snapshot;

        let api_key = api_key.read()?;
        let secret = secret.read()?;
        let passphrase = passphrase.read()?;

        validate_entry_snapshot(
            &descriptor_root,
            PmReadOnlyCredentialKind::ApiKey,
            api_key_entry,
            effective_uid,
            api_key_snapshot,
        )?;
        validate_entry_snapshot(
            &descriptor_root,
            PmReadOnlyCredentialKind::Secret,
            secret_entry,
            effective_uid,
            secret_snapshot,
        )?;
        validate_entry_snapshot(
            &descriptor_root,
            PmReadOnlyCredentialKind::Passphrase,
            passphrase_entry,
            effective_uid,
            passphrase_snapshot,
        )?;

        let final_descriptor_metadata = directory_file
            .metadata()
            .map_err(PmReadOnlyCredentialError::InspectDirectory)?;
        let final_path_metadata = std::fs::symlink_metadata(directory)
            .map_err(PmReadOnlyCredentialError::InspectDirectory)?;
        if final_path_metadata.file_type().is_symlink()
            || !same_directory(&directory_metadata, &final_descriptor_metadata)
            || !same_directory(&directory_metadata, &final_path_metadata)
        {
            return Err(PmReadOnlyCredentialError::DirectoryChanged);
        }

        Ok(PmReadOnlyCredentialBundle {
            api_key,
            secret,
            passphrase,
        })
    }

    fn same_directory(expected: &Metadata, actual: &Metadata) -> bool {
        actual.is_dir()
            && actual.dev() == expected.dev()
            && actual.ino() == expected.ino()
            && actual.uid() == expected.uid()
            && actual.nlink() == expected.nlink()
            && actual.mode() & 0o7777 == expected.mode() & 0o7777
            && actual.len() == expected.len()
            && actual.mtime() == expected.mtime()
            && actual.mtime_nsec() == expected.mtime_nsec()
            && actual.ctime() == expected.ctime()
            && actual.ctime_nsec() == expected.ctime_nsec()
    }

    fn validate_entry_snapshot(
        directory: &Path,
        kind: PmReadOnlyCredentialKind,
        entry: &str,
        effective_uid: u32,
        expected: CredentialFileSnapshot,
    ) -> Result<(), PmReadOnlyCredentialError> {
        let current = open(directory, kind, entry, effective_uid)?;
        if current.snapshot != expected {
            return Err(PmReadOnlyCredentialError::FileChanged { kind });
        }
        Ok(())
    }

    fn open(
        directory: &Path,
        kind: PmReadOnlyCredentialKind,
        entry: &str,
        effective_uid: u32,
    ) -> Result<OpenedCredential, PmReadOnlyCredentialError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(directory.join(entry))
            .map_err(|source| PmReadOnlyCredentialError::Open { kind, source })?;
        let metadata = file
            .metadata()
            .map_err(|source| PmReadOnlyCredentialError::InspectFile { kind, source })?;
        if !metadata.is_file() {
            return Err(PmReadOnlyCredentialError::NotRegularFile { kind });
        }
        if metadata.uid() != effective_uid {
            return Err(PmReadOnlyCredentialError::WrongOwner { kind });
        }
        if !matches!(metadata.mode() & 0o7777, 0o400 | 0o600) {
            return Err(PmReadOnlyCredentialError::WrongMode { kind });
        }
        if metadata.len() > MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES as u64 {
            return Err(PmReadOnlyCredentialError::TooLarge { kind });
        }
        Ok(OpenedCredential {
            kind,
            file,
            snapshot: CredentialFileSnapshot::from_metadata(&metadata),
        })
    }

    fn current_effective_uid() -> Result<u32, PmReadOnlyCredentialError> {
        let status = std::fs::read_to_string("/proc/self/status")
            .map_err(|_| PmReadOnlyCredentialError::EffectiveUidUnavailable)?;
        let uid_line = status
            .lines()
            .find(|line| line.starts_with("Uid:"))
            .ok_or(PmReadOnlyCredentialError::EffectiveUidUnavailable)?;
        uid_line
            .split_ascii_whitespace()
            .nth(2)
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or(PmReadOnlyCredentialError::EffectiveUidUnavailable)
    }

    #[cfg(test)]
    mod snapshot_tests {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        use tempfile::TempDir;

        use super::*;

        fn protected_directory() -> TempDir {
            let directory = tempfile::tempdir().unwrap();
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
            directory
        }

        fn write(path: &Path, value: &[u8]) {
            fs::write(path, value).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        #[test]
        fn same_length_in_place_rewrite_is_rejected_after_read() {
            let directory = protected_directory();
            let path = directory.path().join("passphrase");
            write(&path, b"AAAA");
            let opened = open(
                directory.path(),
                PmReadOnlyCredentialKind::Passphrase,
                "passphrase",
                current_effective_uid().unwrap(),
            )
            .unwrap();

            write(&path, b"BBBB");
            assert!(matches!(
                opened.read(),
                Err(PmReadOnlyCredentialError::FileChanged {
                    kind: PmReadOnlyCredentialKind::Passphrase
                })
            ));
        }

        #[test]
        fn final_basename_reresolution_rejects_a_rotated_inode() {
            let directory = protected_directory();
            let path = directory.path().join("passphrase");
            let replacement = directory.path().join("replacement");
            write(&path, b"AAAA");
            write(&replacement, b"BBBB");
            let opened = open(
                directory.path(),
                PmReadOnlyCredentialKind::Passphrase,
                "passphrase",
                current_effective_uid().unwrap(),
            )
            .unwrap();
            let snapshot = opened.snapshot;
            fs::rename(&replacement, &path).unwrap();

            assert!(matches!(
                validate_entry_snapshot(
                    directory.path(),
                    PmReadOnlyCredentialKind::Passphrase,
                    "passphrase",
                    current_effective_uid().unwrap(),
                    snapshot,
                ),
                Err(PmReadOnlyCredentialError::FileChanged {
                    kind: PmReadOnlyCredentialKind::Passphrase
                })
            ));
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
        path::Path,
    };

    use tempfile::TempDir;

    use super::*;

    const API_KEY: &[u8] = b"00000000-0000-4000-8000-000000000001";
    const SECRET: &[u8] = b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &[u8] = b"synthetic-passphrase";

    fn write(directory: &Path, entry: &str, value: &[u8]) {
        let path = directory.join(entry);
        fs::write(&path, value).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn valid_directory() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        write(directory.path(), "api-key", API_KEY);
        write(directory.path(), "secret", SECRET);
        write(directory.path(), "passphrase", PASSPHRASE);
        directory
    }

    fn load(directory: &Path) -> Result<PmReadOnlyCredentialBundle, PmReadOnlyCredentialError> {
        load_pm_read_only_credentials(directory, "api-key", "secret", "passphrase")
    }

    #[test]
    fn valid_bundle_is_move_only_redacted_and_converts_into_adapter_input() {
        let directory = valid_directory();
        fs::set_permissions(
            directory.path().join("api-key"),
            fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        let bundle = load(directory.path()).unwrap();
        assert_eq!(
            format!("{bundle:?}"),
            "PmReadOnlyCredentialBundle([REDACTED])"
        );
        let (adapter, guard) = bundle.into_adapter_input_and_artifact_guard();
        assert_eq!(
            format!("{adapter:?}"),
            "PmReadOnlyCredentialInput([REDACTED])"
        );
        assert_eq!(
            format!("{guard:?}"),
            "PmReadOnlyArtifactSecretGuard([REDACTED])"
        );
    }

    #[test]
    fn secret_and_derived_collisions_in_persisted_config_are_rejected() {
        let directory = valid_directory();
        let bundle = load(directory.path()).unwrap();
        assert!(
            bundle
                .ensure_persisted_config_is_secret_free(
                    b"credential_slot_id = \"reviewed-slot-v1\"\nschema_version = 1\n"
                )
                .is_ok()
        );

        let secret = std::str::from_utf8(SECRET).unwrap();
        let derived = [
            secret.to_owned(),
            STANDARD.encode(SECRET),
            hex(SECRET, false),
        ];
        for alias in derived {
            let canonical_config = format!("credential_slot_id = \"{alias}\"\n");
            let error = bundle
                .ensure_persisted_config_is_secret_free(canonical_config.as_bytes())
                .unwrap_err();
            assert!(matches!(
                error,
                PmReadOnlyCredentialError::PersistedValueCollision
            ));
            for rendered in [format!("{error}"), format!("{error:?}")] {
                assert!(!rendered.contains(secret));
                assert!(!rendered.contains(&alias));
            }
        }
    }

    #[test]
    fn final_artifact_guard_rejects_a_venue_value_equal_to_a_valid_passphrase() {
        let directory = valid_directory();
        write(directory.path(), "passphrase", b"LIVE");
        let bundle = load(directory.path()).unwrap();
        let (_input, guard) = bundle.into_adapter_input_and_artifact_guard();
        let error = guard
            .ensure_artifact_is_secret_free(br#"{"status":"LIVE"}"#)
            .unwrap_err();

        assert!(matches!(
            error,
            PmReadOnlyCredentialError::ArtifactValueCollision
        ));
        for rendered in [
            format!("{guard:?}"),
            format!("{error}"),
            format!("{error:?}"),
        ] {
            assert!(!rendered.contains("LIVE"));
        }
    }

    #[test]
    fn final_artifact_guard_rejects_an_escaped_credential_inside_a_larger_value() {
        let guard = PmReadOnlyArtifactSecretGuard::for_test(
            std::str::from_utf8(API_KEY).unwrap(),
            std::str::from_utf8(SECRET).unwrap(),
            r#"a"b"#,
        );
        let error = guard
            .ensure_artifact_is_secret_free(br#"{"status":"prefix-a\"b-suffix"}"#)
            .unwrap_err();
        assert!(matches!(
            error,
            PmReadOnlyCredentialError::ArtifactValueCollision
        ));
    }

    #[test]
    fn final_artifact_guard_decodes_retained_public_bodies_before_scanning() {
        let guard = PmReadOnlyArtifactSecretGuard::for_test(
            std::str::from_utf8(API_KEY).unwrap(),
            std::str::from_utf8(SECRET).unwrap(),
            "LIVE",
        );
        let body = STANDARD.encode(br#"{"description":"prefix-LIVE-suffix"}"#);
        let error = guard
            .ensure_base64_artifact_value_is_secret_free(&body)
            .unwrap_err();
        assert!(matches!(
            error,
            PmReadOnlyCredentialError::ArtifactValueCollision
        ));
    }

    #[test]
    fn final_artifact_guard_scans_semantic_public_json_strings_and_keys() {
        let guard = PmReadOnlyArtifactSecretGuard::for_test(
            std::str::from_utf8(API_KEY).unwrap(),
            std::str::from_utf8(SECRET).unwrap(),
            "LIVE",
        );
        for body in [
            br#"{"nested":{"description":"prefix-L\u0049V\u0045-suffix"}}"#.as_slice(),
            br#"{"nested":{"description":"prefix-L\u0049V\u0045-suffix","description":"safe"}}"#
                .as_slice(),
            br#"{"nested":{"prefix-L\u0049V\u0045-suffix":"safe"}}"#.as_slice(),
        ] {
            let body = STANDARD.encode(body);
            assert!(matches!(
                guard.ensure_base64_artifact_value_is_secret_free(&body),
                Err(PmReadOnlyCredentialError::ArtifactValueCollision)
            ));
        }

        let clean = STANDARD
            .encode(br#"{"nested":{"description":"safe-\u0062ody","items":["alpha","beta"]}}"#);
        assert!(
            guard
                .ensure_base64_artifact_value_is_secret_free(&clean)
                .is_ok()
        );

        let malformed = STANDARD.encode(br#"{"nested":"unterminated"#);
        assert!(matches!(
            guard.ensure_base64_artifact_value_is_secret_free(&malformed),
            Err(PmReadOnlyCredentialError::ArtifactEncodingInvalid)
        ));
    }

    #[test]
    fn final_artifact_guard_rejects_standard_base64_without_padding() {
        let guard = PmReadOnlyArtifactSecretGuard::for_test(
            std::str::from_utf8(API_KEY).unwrap(),
            std::str::from_utf8(SECRET).unwrap(),
            "!!?!",
        );
        assert!(matches!(
            guard.ensure_config_is_secret_free(br#"credential_slot_id = "ISE/IQ""#),
            Err(PmReadOnlyCredentialError::PersistedValueCollision)
        ));
        assert!(matches!(
            guard.ensure_artifact_is_secret_free(br#"{"outcome":"ISE/IQ"}"#),
            Err(PmReadOnlyCredentialError::ArtifactValueCollision)
        ));
    }

    #[test]
    fn api_key_case_and_compact_aliases_are_rejected() {
        const LETTERED_API_KEY: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let guard = PmReadOnlyArtifactSecretGuard::for_test(
            LETTERED_API_KEY,
            std::str::from_utf8(SECRET).unwrap(),
            std::str::from_utf8(PASSPHRASE).unwrap(),
        );
        let compact = LETTERED_API_KEY.replace('-', "");
        for alias in [
            LETTERED_API_KEY.to_ascii_uppercase(),
            compact.clone(),
            compact.to_ascii_uppercase(),
        ] {
            let config = format!(r#"credential_slot_id = "{alias}""#);
            assert!(matches!(
                guard.ensure_config_is_secret_free(config.as_bytes()),
                Err(PmReadOnlyCredentialError::PersistedValueCollision)
            ));
            let artifact = format!(r#"{{"outcome":"{alias}"}}"#);
            assert!(matches!(
                guard.ensure_artifact_is_secret_free(artifact.as_bytes()),
                Err(PmReadOnlyCredentialError::ArtifactValueCollision)
            ));
        }
    }

    #[test]
    fn missing_entry_is_rejected_without_exposing_a_path() {
        let directory = valid_directory();
        fs::remove_file(directory.path().join("secret")).unwrap();
        let error = load(directory.path()).unwrap_err();
        assert!(matches!(
            error,
            PmReadOnlyCredentialError::Open {
                kind: PmReadOnlyCredentialKind::Secret,
                ..
            }
        ));
    }

    #[test]
    fn directory_and_entry_symlinks_are_rejected() {
        let outer = tempfile::tempdir().unwrap();
        let credentials = valid_directory();
        let directory_link = outer.path().join("credentials-link");
        symlink(credentials.path(), &directory_link).unwrap();
        assert!(matches!(
            load(&directory_link),
            Err(PmReadOnlyCredentialError::InvalidDirectory)
        ));

        let target = outer.path().join("target-secret");
        fs::write(&target, SECRET).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_file(credentials.path().join("secret")).unwrap();
        symlink(&target, credentials.path().join("secret")).unwrap();
        assert!(matches!(
            load(credentials.path()),
            Err(PmReadOnlyCredentialError::Open {
                kind: PmReadOnlyCredentialKind::Secret,
                ..
            })
        ));
    }

    #[test]
    fn nondirectory_credential_root_is_rejected() {
        let outer = tempfile::tempdir().unwrap();
        let file = outer.path().join("not-a-directory");
        fs::write(&file, b"not credentials").unwrap();
        assert!(matches!(
            load(&file),
            Err(PmReadOnlyCredentialError::InvalidDirectory)
        ));
    }

    #[test]
    fn external_hardlink_is_rejected_by_single_link_rule() {
        let directory = valid_directory();
        fs::hard_link(
            directory.path().join("secret"),
            directory.path().join("extra-link"),
        )
        .unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(PmReadOnlyCredentialError::WrongLinkCount {
                kind: PmReadOnlyCredentialKind::Secret
            })
        ));
    }

    #[test]
    fn credential_hardlinks_are_rejected_as_duplicate_inodes() {
        let directory = valid_directory();
        fs::remove_file(directory.path().join("secret")).unwrap();
        fs::hard_link(
            directory.path().join("api-key"),
            directory.path().join("secret"),
        )
        .unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(PmReadOnlyCredentialError::DuplicateInode)
        ));
    }

    #[test]
    fn wrong_mode_is_rejected() {
        let directory = valid_directory();
        fs::set_permissions(
            directory.path().join("passphrase"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(PmReadOnlyCredentialError::WrongMode {
                kind: PmReadOnlyCredentialKind::Passphrase
            })
        ));
    }

    #[test]
    fn unprotected_directory_mode_is_rejected() {
        let directory = valid_directory();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(PmReadOnlyCredentialError::WrongDirectoryMode)
        ));
    }

    #[test]
    fn newline_nul_and_oversize_values_are_rejected() {
        type CredentialFailureMatch = fn(PmReadOnlyCredentialError) -> bool;
        let cases: &[(&[u8], CredentialFailureMatch)] = &[
            (b"synthetic\n", |error| {
                matches!(
                    error,
                    PmReadOnlyCredentialError::ProhibitedByte {
                        kind: PmReadOnlyCredentialKind::Passphrase
                    }
                )
            }),
            (b"synthetic\rpassphrase", |error| {
                matches!(
                    error,
                    PmReadOnlyCredentialError::ProhibitedByte {
                        kind: PmReadOnlyCredentialKind::Passphrase
                    }
                )
            }),
            (b"synthetic\0passphrase", |error| {
                matches!(
                    error,
                    PmReadOnlyCredentialError::ProhibitedByte {
                        kind: PmReadOnlyCredentialKind::Passphrase
                    }
                )
            }),
            (
                &[b'x'; MAX_PM_READ_ONLY_CREDENTIAL_FILE_BYTES + 1],
                |error| {
                    matches!(
                        error,
                        PmReadOnlyCredentialError::TooLarge {
                            kind: PmReadOnlyCredentialKind::Passphrase
                        }
                    )
                },
            ),
        ];
        for (value, predicate) in cases {
            let directory = valid_directory();
            write(directory.path(), "passphrase", value);
            let error = load(directory.path()).unwrap_err();
            assert!(predicate(error));
        }
    }

    #[test]
    fn empty_non_utf8_and_noncanonical_values_are_rejected() {
        for (entry, value, expected) in [
            ("api-key", Vec::new(), "empty"),
            ("passphrase", vec![0xff], "UTF-8"),
            (
                "api-key",
                b"00000000-0000-4000-8000-00000000000A".to_vec(),
                "canonical",
            ),
            (
                "secret",
                b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_vec(),
                "canonical",
            ),
            ("passphrase", b"contains space".to_vec(), "canonical"),
        ] {
            let directory = valid_directory();
            write(directory.path(), entry, &value);
            let rendered = load(directory.path()).unwrap_err().to_string();
            assert!(rendered.contains(expected));
        }
    }

    #[test]
    fn unsafe_and_duplicate_entry_names_are_rejected() {
        let directory = valid_directory();
        for invalid in ["", ".", "..", "../api-key", "nested/secret", "bad name"] {
            assert!(matches!(
                load_pm_read_only_credentials(directory.path(), invalid, "secret", "passphrase"),
                Err(PmReadOnlyCredentialError::InvalidEntryName {
                    kind: PmReadOnlyCredentialKind::ApiKey
                })
            ));
        }
        assert!(matches!(
            load_pm_read_only_credentials(directory.path(), "api-key", "api-key", "passphrase"),
            Err(PmReadOnlyCredentialError::DuplicateEntryName)
        ));
    }

    #[test]
    fn error_and_debug_rendering_never_include_secret_canaries() {
        const CANARY: &str = "SECRET-CANARY-MUST-NOT-ESCAPE";
        let directory = valid_directory();
        write(
            directory.path(),
            "passphrase",
            format!("{CANARY}\n").as_bytes(),
        );
        let error = load(directory.path()).unwrap_err();
        for rendered in [format!("{error}"), format!("{error:?}")] {
            assert!(!rendered.contains(CANARY));
        }
    }
}
