//! Private durable burn lineage for one loopback credential-proof attempt.
//!
//! The two fixed artifacts contain public commitments and state links only.
//! They never contain source timestamps, authenticated headers, signatures,
//! response bytes or digests, or credential material. Every state is denied,
//! claim presence is a permanent burn signal, and there is no reopen or resume
//! path. The cooperative directory lease assumes trusted local storage and
//! cannot stop a same-EUID process that deliberately bypasses the advisory
//! lock or rolls back the whole directory.

use std::{
    fmt,
    fs::{File, Metadata, OpenOptions},
    io::{Read as _, Seek as _, Write as _},
    os::unix::{
        fs::{MetadataExt as _, OpenOptionsExt as _},
        io::AsRawFd as _,
    },
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};

pub(crate) const ATTEMPT_LEDGER_FILE: &str = "pm-t2-phase-a-credential-proof-attempt-v1.jsonl";
pub(crate) const ATTEMPT_BURN_CLAIM_FILE: &str =
    "pm-t2-phase-a-credential-proof-attempt-burn-claim-v1.json";

const SCHEMA: &str = "pm-t2-phase-a-credential-proof-attempt-v1";
const CLAIM_SCHEMA: &str = "pm-t2-phase-a-credential-proof-attempt-burn-claim-v1";
const MAX_LEDGER_BYTES: usize = 16 * 1024;
const MAX_CLAIM_BYTES: usize = 4 * 1024;
const ZERO_FINGERPRINT: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const BINDING_DOMAIN: &[u8] = b"reap.pm-t2.phase-a.credential-proof.binding.v1\0";
const PREPARED_DOMAIN: &[u8] = b"reap.pm-t2.phase-a.credential-proof.prepared.v1\0";
const CLAIM_DOMAIN: &[u8] = b"reap.pm-t2.phase-a.credential-proof.burn-claim.v1\0";
const CONSUMED_DOMAIN: &[u8] = b"reap.pm-t2.phase-a.credential-proof.consumed.v1\0";
const FINAL_DOMAIN: &[u8] = b"reap.pm-t2.phase-a.credential-proof.final.v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttemptPublicBinding {
    pub(crate) policy_commitment: String,
    pub(crate) source_commitment: String,
    pub(crate) destination_selection_commitment: String,
    pub(crate) local_egress_selection_commitment: String,
    pub(crate) signer_identity_commitment: String,
    pub(crate) selected_actor_generation_commitment: String,
    pub(crate) attempt_commitment: String,
}

impl AttemptPublicBinding {
    fn validate(&self) -> Result<(), LineageError> {
        for value in [
            &self.policy_commitment,
            &self.source_commitment,
            &self.destination_selection_commitment,
            &self.local_egress_selection_commitment,
            &self.signer_identity_commitment,
            &self.selected_actor_generation_commitment,
            &self.attempt_commitment,
        ] {
            if value.len() != 64
                || value == ZERO_FINGERPRINT
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(LineageError::InvalidBinding);
            }
        }
        Ok(())
    }

    fn canonical_basis(&self) -> String {
        format!(
            concat!(
                "{{\"policy_commitment\":\"{}\",",
                "\"source_commitment\":\"{}\",",
                "\"destination_selection_commitment\":\"{}\",",
                "\"local_egress_selection_commitment\":\"{}\",",
                "\"signer_identity_commitment\":\"{}\",",
                "\"selected_actor_generation_commitment\":\"{}\",",
                "\"attempt_commitment\":\"{}\"}}"
            ),
            self.policy_commitment,
            self.source_commitment,
            self.destination_selection_commitment,
            self.local_egress_selection_commitment,
            self.signer_identity_commitment,
            self.selected_actor_generation_commitment,
            self.attempt_commitment,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedRecord {
    schema: String,
    sequence: u8,
    record: String,
    authorization: String,
    production_permit: bool,
    resume_allowed: bool,
    binding_fingerprint: String,
    binding: AttemptPublicBinding,
    previous_record_fingerprint: String,
    record_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BurnClaimRecord {
    schema: String,
    ledger_file: String,
    authorization: String,
    production_permit: bool,
    resume_allowed: bool,
    attempt_commitment: String,
    binding_fingerprint: String,
    prepared_record_fingerprint: String,
    claim_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsumedRecord {
    schema: String,
    sequence: u8,
    record: String,
    authorization: String,
    production_permit: bool,
    resume_allowed: bool,
    binding_fingerprint: String,
    prepared_record_fingerprint: String,
    claim_fingerprint: String,
    previous_record_fingerprint: String,
    record_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinalRecord {
    schema: String,
    sequence: u8,
    record: String,
    authorization: String,
    production_permit: bool,
    resume_allowed: bool,
    binding_fingerprint: String,
    prepared_record_fingerprint: String,
    claim_fingerprint: String,
    consumed_record_fingerprint: String,
    first_loopback_time_completed: bool,
    derive_tuple_local_equality_completed: bool,
    second_loopback_time_completed: bool,
    same_holder_closed_only_loopback_completed: bool,
    remote_acceptance_proven: bool,
    mutation_authority: bool,
    previous_record_fingerprint: String,
    record_fingerprint: String,
}

/// Private read-only crash-shape classification. Every class is denied and
/// nonresumable. Only `Pristine` may be followed by the create-new writer in
/// the same held cooperative lease; inspection itself grants no capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptLineageInspection {
    Pristine,
    PreparedUnclaimed,
    BurnClaimEmpty,
    BurnClaimPartial,
    BurnClaimDurable,
    ConsumedDenied,
    CompleteDenied,
}

impl AttemptLineageInspection {
    pub(crate) const fn authorization(&self) -> &'static str {
        "DENIED"
    }

    pub(crate) const fn resume_allowed(&self) -> bool {
        false
    }

    const fn create_new_eligible(&self) -> bool {
        matches!(self, Self::Pristine)
    }
}

pub(crate) struct PreparedAttemptLineage {
    ledger: ProtectedFile,
    expected_ledger: Vec<u8>,
    binding_fingerprint: String,
    prepared_fingerprint: String,
    attempt_commitment: String,
    lease: ProtectedDirectoryLease,
}

pub(crate) struct BurnedAttemptLineage {
    ledger: ProtectedFile,
    claim: ProtectedFile,
    expected_ledger: Vec<u8>,
    expected_claim: Vec<u8>,
    binding_fingerprint: String,
    prepared_fingerprint: String,
    claim_fingerprint: String,
    consumed_fingerprint: String,
    lease: ProtectedDirectoryLease,
}

pub(crate) struct FinalAttemptLineage {
    _ledger: ProtectedFile,
    _claim: ProtectedFile,
    _expected_ledger: Vec<u8>,
    _expected_claim: Vec<u8>,
    _lease: ProtectedDirectoryLease,
}

impl fmt::Debug for PreparedAttemptLineage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedAttemptLineage(<DENIED; NO_RESUME>)")
    }
}

impl fmt::Debug for BurnedAttemptLineage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BurnedAttemptLineage(<DENIED; NO_RESUME>)")
    }
}

impl fmt::Debug for FinalAttemptLineage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FinalAttemptLineage(<DENIED; NO_AUTHORITY>)")
    }
}

impl PreparedAttemptLineage {
    pub(crate) fn create_new(
        artifact_directory: &Path,
        binding: AttemptPublicBinding,
    ) -> Result<Self, LineageError> {
        binding.validate()?;
        let mut lease = ProtectedDirectoryLease::acquire(artifact_directory)?;
        let claim_present = lease.entry_present(ATTEMPT_BURN_CLAIM_FILE)?;
        let inspected = inspect_while_leased(&lease);
        if claim_present {
            return Err(LineageError::AlreadyBurned);
        }
        let initial = inspected?;
        if !initial.create_new_eligible() {
            return Err(LineageError::InvalidInspection);
        }

        let binding_basis = binding.canonical_basis();
        let binding_fingerprint = fingerprint(BINDING_DOMAIN, binding_basis.as_bytes());
        let prepared_prefix = format!(
            concat!(
                "{{\"schema\":\"{}\",\"sequence\":0,\"record\":\"Prepared\",",
                "\"authorization\":\"DENIED\",\"production_permit\":false,",
                "\"resume_allowed\":false,\"binding_fingerprint\":\"{}\",",
                "\"binding\":{},\"previous_record_fingerprint\":\"{}\",",
                "\"record_fingerprint\":\""
            ),
            SCHEMA, binding_fingerprint, binding_basis, ZERO_FINGERPRINT,
        );
        let (prepared_fingerprint, prepared_bytes) = seal_record(PREPARED_DOMAIN, &prepared_prefix);

        let ledger_path = artifact_directory.join(ATTEMPT_LEDGER_FILE);
        let mut ledger = ProtectedFile::create_new(&ledger_path, MAX_LEDGER_BYTES)?;
        lease.refresh_after_create()?;
        ledger.append_durable(&[], &prepared_bytes)?;
        ledger.validate_exact(&prepared_bytes)?;
        lease.validate()?;

        Ok(Self {
            ledger,
            expected_ledger: prepared_bytes,
            binding_fingerprint,
            prepared_fingerprint,
            attempt_commitment: binding.attempt_commitment,
            lease,
        })
    }

    pub(crate) fn burn(self) -> Result<BurnedAttemptLineage, LineageError> {
        let Self {
            mut ledger,
            mut expected_ledger,
            binding_fingerprint,
            prepared_fingerprint,
            attempt_commitment,
            mut lease,
        } = self;
        ledger.validate_exact(&expected_ledger)?;
        lease.validate()?;

        let claim_prefix = format!(
            concat!(
                "{{\"schema\":\"{}\",\"ledger_file\":\"{}\",",
                "\"authorization\":\"DENIED\",\"production_permit\":false,",
                "\"resume_allowed\":false,\"attempt_commitment\":\"{}\",",
                "\"binding_fingerprint\":\"{}\",",
                "\"prepared_record_fingerprint\":\"{}\",",
                "\"claim_fingerprint\":\""
            ),
            CLAIM_SCHEMA,
            ATTEMPT_LEDGER_FILE,
            attempt_commitment,
            binding_fingerprint,
            prepared_fingerprint,
        );
        let (claim_fingerprint, claim_bytes) = seal_record(CLAIM_DOMAIN, &claim_prefix);
        let claim_path = lease.parent.join(ATTEMPT_BURN_CLAIM_FILE);

        // Successful create-new plus file and directory fsync is the permanent
        // burn point. An empty or partial claim is still burned and never resumed.
        let mut claim = ProtectedFile::create_new(&claim_path, MAX_CLAIM_BYTES)?;
        let ledger_refresh = ledger.refresh_parent_after_bound_create();
        let lease_refresh = lease.refresh_after_create();
        let claim_validation = claim.validate_exact(&[]);
        ledger_refresh?;
        lease_refresh?;
        claim_validation?;
        claim.append_durable(&[], &claim_bytes)?;

        ledger.validate_exact(&expected_ledger)?;
        claim.validate_exact(&claim_bytes)?;
        lease.validate()?;

        let consumed_prefix = format!(
            concat!(
                "{{\"schema\":\"{}\",\"sequence\":1,\"record\":\"Consumed\",",
                "\"authorization\":\"DENIED\",\"production_permit\":false,",
                "\"resume_allowed\":false,\"binding_fingerprint\":\"{}\",",
                "\"prepared_record_fingerprint\":\"{}\",",
                "\"claim_fingerprint\":\"{}\",",
                "\"previous_record_fingerprint\":\"{}\",",
                "\"record_fingerprint\":\""
            ),
            SCHEMA,
            binding_fingerprint,
            prepared_fingerprint,
            claim_fingerprint,
            prepared_fingerprint,
        );
        let (consumed_fingerprint, consumed_bytes) = seal_record(CONSUMED_DOMAIN, &consumed_prefix);
        ledger.append_durable(&expected_ledger, &consumed_bytes)?;
        expected_ledger.extend_from_slice(&consumed_bytes);

        ledger.validate_exact(&expected_ledger)?;
        claim.validate_exact(&claim_bytes)?;
        lease.validate()?;
        Ok(BurnedAttemptLineage {
            ledger,
            claim,
            expected_ledger,
            expected_claim: claim_bytes,
            binding_fingerprint,
            prepared_fingerprint,
            claim_fingerprint,
            consumed_fingerprint,
            lease,
        })
    }
}

impl BurnedAttemptLineage {
    /// Revalidate both descriptor-pinned files, their exact bytes, and the held
    /// cooperative directory lease. The attempt calls this immediately before
    /// each operation that may send on loopback.
    pub(crate) fn validate_exact(&mut self) -> Result<(), LineageError> {
        let ledger = self.ledger.validate_exact(&self.expected_ledger);
        let claim = self.claim.validate_exact(&self.expected_claim);
        let lease = self.lease.validate();
        ledger?;
        claim?;
        lease
    }

    pub(crate) fn finish(mut self) -> Result<FinalAttemptLineage, LineageError> {
        self.validate_exact()?;
        let final_prefix = format!(
            concat!(
                "{{\"schema\":\"{}\",\"sequence\":2,\"record\":\"Final\",",
                "\"authorization\":\"DENIED\",\"production_permit\":false,",
                "\"resume_allowed\":false,\"binding_fingerprint\":\"{}\",",
                "\"prepared_record_fingerprint\":\"{}\",",
                "\"claim_fingerprint\":\"{}\",",
                "\"consumed_record_fingerprint\":\"{}\",",
                "\"first_loopback_time_completed\":true,",
                "\"derive_tuple_local_equality_completed\":true,",
                "\"second_loopback_time_completed\":true,",
                "\"same_holder_closed_only_loopback_completed\":true,",
                "\"remote_acceptance_proven\":false,\"mutation_authority\":false,",
                "\"previous_record_fingerprint\":\"{}\",",
                "\"record_fingerprint\":\""
            ),
            SCHEMA,
            self.binding_fingerprint,
            self.prepared_fingerprint,
            self.claim_fingerprint,
            self.consumed_fingerprint,
            self.consumed_fingerprint,
        );
        let (_, final_bytes) = seal_record(FINAL_DOMAIN, &final_prefix);
        self.ledger
            .append_durable(&self.expected_ledger, &final_bytes)?;
        self.expected_ledger.extend_from_slice(&final_bytes);
        self.validate_exact()?;
        Ok(FinalAttemptLineage {
            _ledger: self.ledger,
            _claim: self.claim,
            _expected_ledger: self.expected_ledger,
            _expected_claim: self.expected_claim,
            _lease: self.lease,
        })
    }
}

pub(crate) fn inspect_attempt_lineage(
    artifact_directory: &Path,
) -> Result<AttemptLineageInspection, LineageError> {
    let lease = ProtectedDirectoryLease::acquire(artifact_directory)?;
    let inspected = inspect_while_leased(&lease);
    let final_validation = lease.validate();
    final_validation?;
    inspected
}

fn inspect_while_leased(
    lease: &ProtectedDirectoryLease,
) -> Result<AttemptLineageInspection, LineageError> {
    let ledger = lease.read_entry(ATTEMPT_LEDGER_FILE, MAX_LEDGER_BYTES)?;
    let claim = lease.read_entry(ATTEMPT_BURN_CLAIM_FILE, MAX_CLAIM_BYTES)?;
    let result = match (ledger, claim) {
        (InspectedEntry::Absent, InspectedEntry::Absent) => AttemptLineageInspection::Pristine,
        (InspectedEntry::Absent, InspectedEntry::Present(_)) => {
            return Err(LineageError::InvalidInspection);
        }
        (InspectedEntry::Present(ledger), claim) => classify_present_ledger(&ledger, claim)?,
    };
    lease.validate()?;
    Ok(result)
}

fn classify_present_ledger(
    ledger_bytes: &[u8],
    claim: InspectedEntry,
) -> Result<AttemptLineageInspection, LineageError> {
    let lines = exact_lines(ledger_bytes)?;
    let prepared = parse_exact_line::<PreparedRecord>(lines[0])?;
    validate_prepared(&prepared)?;
    let (_, expected_claim_bytes) = expected_claim(&prepared)?;

    match claim {
        InspectedEntry::Absent if lines.len() == 1 => {
            Ok(AttemptLineageInspection::PreparedUnclaimed)
        }
        InspectedEntry::Absent => Err(LineageError::InvalidInspection),
        InspectedEntry::Present(claim_bytes) if claim_bytes.is_empty() && lines.len() == 1 => {
            Ok(AttemptLineageInspection::BurnClaimEmpty)
        }
        InspectedEntry::Present(claim_bytes)
            if lines.len() == 1
                && !claim_bytes.is_empty()
                && claim_bytes.len() < expected_claim_bytes.len()
                && expected_claim_bytes.starts_with(&claim_bytes) =>
        {
            Ok(AttemptLineageInspection::BurnClaimPartial)
        }
        InspectedEntry::Present(claim_bytes) if claim_bytes == expected_claim_bytes => {
            let claim = parse_exact_line::<BurnClaimRecord>(&claim_bytes)?;
            validate_claim(&claim, &prepared)?;
            match lines.as_slice() {
                [_] => Ok(AttemptLineageInspection::BurnClaimDurable),
                [_, consumed_line] => {
                    let consumed = parse_exact_line::<ConsumedRecord>(consumed_line)?;
                    validate_consumed(&consumed, &prepared, &claim)?;
                    Ok(AttemptLineageInspection::ConsumedDenied)
                }
                [_, consumed_line, final_line] => {
                    let consumed = parse_exact_line::<ConsumedRecord>(consumed_line)?;
                    validate_consumed(&consumed, &prepared, &claim)?;
                    let final_record = parse_exact_line::<FinalRecord>(final_line)?;
                    validate_final(&final_record, &prepared, &claim, &consumed)?;
                    Ok(AttemptLineageInspection::CompleteDenied)
                }
                _ => Err(LineageError::InvalidInspection),
            }
        }
        InspectedEntry::Present(_) => Err(LineageError::InvalidInspection),
    }
}

fn exact_lines(bytes: &[u8]) -> Result<Vec<&[u8]>, LineageError> {
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err(LineageError::InvalidInspection);
    }
    let lines = bytes
        .split_inclusive(|byte| *byte == b'\n')
        .collect::<Vec<_>>();
    if lines.is_empty() || lines.len() > 3 || lines.iter().any(|line| line.len() <= 1) {
        return Err(LineageError::InvalidInspection);
    }
    Ok(lines)
}

fn parse_exact_line<T>(line: &[u8]) -> Result<T, LineageError>
where
    T: DeserializeOwned + Serialize,
{
    let document = line
        .strip_suffix(b"\n")
        .ok_or(LineageError::InvalidInspection)?;
    let parsed =
        serde_json::from_slice::<T>(document).map_err(|_| LineageError::InvalidInspection)?;
    if canonical_line(&parsed)? != line {
        return Err(LineageError::InvalidInspection);
    }
    Ok(parsed)
}

fn canonical_line(value: &impl Serialize) -> Result<Vec<u8>, LineageError> {
    let mut encoded = serde_json::to_vec(value).map_err(|_| LineageError::InvalidBinding)?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn validate_prepared(prepared: &PreparedRecord) -> Result<(), LineageError> {
    prepared.binding.validate()?;
    let expected_binding = fingerprint(
        BINDING_DOMAIN,
        prepared.binding.canonical_basis().as_bytes(),
    );
    if prepared.schema != SCHEMA
        || prepared.sequence != 0
        || prepared.record != "Prepared"
        || prepared.authorization != "DENIED"
        || prepared.production_permit
        || prepared.resume_allowed
        || prepared.previous_record_fingerprint != ZERO_FINGERPRINT
        || prepared.binding_fingerprint != expected_binding
    {
        return Err(LineageError::InvalidInspection);
    }
    let mut basis = prepared.clone();
    basis.record_fingerprint = ZERO_FINGERPRINT.to_owned();
    if prepared.record_fingerprint != fingerprint(PREPARED_DOMAIN, &canonical_line(&basis)?) {
        return Err(LineageError::InvalidInspection);
    }
    Ok(())
}

fn expected_claim(prepared: &PreparedRecord) -> Result<(BurnClaimRecord, Vec<u8>), LineageError> {
    let mut claim = BurnClaimRecord {
        schema: CLAIM_SCHEMA.to_owned(),
        ledger_file: ATTEMPT_LEDGER_FILE.to_owned(),
        authorization: "DENIED".to_owned(),
        production_permit: false,
        resume_allowed: false,
        attempt_commitment: prepared.binding.attempt_commitment.clone(),
        binding_fingerprint: prepared.binding_fingerprint.clone(),
        prepared_record_fingerprint: prepared.record_fingerprint.clone(),
        claim_fingerprint: ZERO_FINGERPRINT.to_owned(),
    };
    claim.claim_fingerprint = fingerprint(CLAIM_DOMAIN, &canonical_line(&claim)?);
    let encoded = canonical_line(&claim)?;
    Ok((claim, encoded))
}

fn validate_claim(claim: &BurnClaimRecord, prepared: &PreparedRecord) -> Result<(), LineageError> {
    let (expected, _) = expected_claim(prepared)?;
    if claim != &expected {
        return Err(LineageError::InvalidInspection);
    }
    Ok(())
}

fn validate_consumed(
    consumed: &ConsumedRecord,
    prepared: &PreparedRecord,
    claim: &BurnClaimRecord,
) -> Result<(), LineageError> {
    if consumed.schema != SCHEMA
        || consumed.sequence != 1
        || consumed.record != "Consumed"
        || consumed.authorization != "DENIED"
        || consumed.production_permit
        || consumed.resume_allowed
        || consumed.binding_fingerprint != prepared.binding_fingerprint
        || consumed.prepared_record_fingerprint != prepared.record_fingerprint
        || consumed.claim_fingerprint != claim.claim_fingerprint
        || consumed.previous_record_fingerprint != prepared.record_fingerprint
    {
        return Err(LineageError::InvalidInspection);
    }
    let mut basis = consumed.clone();
    basis.record_fingerprint = ZERO_FINGERPRINT.to_owned();
    if consumed.record_fingerprint != fingerprint(CONSUMED_DOMAIN, &canonical_line(&basis)?) {
        return Err(LineageError::InvalidInspection);
    }
    Ok(())
}

fn validate_final(
    final_record: &FinalRecord,
    prepared: &PreparedRecord,
    claim: &BurnClaimRecord,
    consumed: &ConsumedRecord,
) -> Result<(), LineageError> {
    if final_record.schema != SCHEMA
        || final_record.sequence != 2
        || final_record.record != "Final"
        || final_record.authorization != "DENIED"
        || final_record.production_permit
        || final_record.resume_allowed
        || final_record.binding_fingerprint != prepared.binding_fingerprint
        || final_record.prepared_record_fingerprint != prepared.record_fingerprint
        || final_record.claim_fingerprint != claim.claim_fingerprint
        || final_record.consumed_record_fingerprint != consumed.record_fingerprint
        || !final_record.first_loopback_time_completed
        || !final_record.derive_tuple_local_equality_completed
        || !final_record.second_loopback_time_completed
        || !final_record.same_holder_closed_only_loopback_completed
        || final_record.remote_acceptance_proven
        || final_record.mutation_authority
        || final_record.previous_record_fingerprint != consumed.record_fingerprint
    {
        return Err(LineageError::InvalidInspection);
    }
    let mut basis = final_record.clone();
    basis.record_fingerprint = ZERO_FINGERPRINT.to_owned();
    if final_record.record_fingerprint != fingerprint(FINAL_DOMAIN, &canonical_line(&basis)?) {
        return Err(LineageError::InvalidInspection);
    }
    Ok(())
}

fn seal_record(domain: &[u8], prefix: &str) -> (String, Vec<u8>) {
    let basis = format!("{prefix}{ZERO_FINGERPRINT}\"}}\n");
    let record_fingerprint = fingerprint(domain, basis.as_bytes());
    let encoded = format!("{prefix}{record_fingerprint}\"}}\n").into_bytes();
    (record_fingerprint, encoded)
}

pub(crate) fn commitment(domain: &[u8], value: &[u8]) -> String {
    fingerprint(domain, value)
}

fn fingerprint(domain: &[u8], value: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(
        u64::try_from(value.len())
            .expect("bounded credential-proof fingerprint input")
            .to_be_bytes(),
    );
    digest.update(value);
    lower_hex(&digest.finalize())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineageError {
    InvalidBinding,
    InvalidDirectory,
    Protection,
    Durability,
    AlreadyBurned,
    BoundExceeded,
    AmbiguousTail,
    InvalidInspection,
}

impl fmt::Display for LineageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBinding => "credential-proof attempt binding rejected",
            Self::InvalidDirectory => "credential-proof artifact directory rejected",
            Self::Protection => "credential-proof protected artifact rejected",
            Self::Durability => "credential-proof durable write failed",
            Self::AlreadyBurned => "credential-proof attempt artifact already exists",
            Self::BoundExceeded => "credential-proof artifact bound exceeded",
            Self::AmbiguousTail => "credential-proof artifact bytes changed",
            Self::InvalidInspection => "credential-proof crash shape is ambiguous and burned",
        })
    }
}

impl std::error::Error for LineageError {}

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

struct PinnedDirectory {
    file: File,
    snapshot: Snapshot,
    descriptor_root: PathBuf,
    parent: PathBuf,
    effective_uid: u32,
}

impl PinnedDirectory {
    fn open(path: &Path) -> Result<Self, LineageError> {
        if !path.is_absolute() {
            return Err(LineageError::InvalidDirectory);
        }
        let effective_uid = effective_uid()?;
        let by_path =
            std::fs::symlink_metadata(path).map_err(|_| LineageError::InvalidDirectory)?;
        validate_directory(&by_path, effective_uid)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| LineageError::InvalidDirectory)?;
        let held = file
            .metadata()
            .map_err(|_| LineageError::InvalidDirectory)?;
        validate_directory(&held, effective_uid)?;
        if Snapshot::from(&by_path) != Snapshot::from(&held) {
            return Err(LineageError::InvalidDirectory);
        }
        let descriptor_root = PathBuf::from("/proc/self/fd").join(file.as_raw_fd().to_string());
        Ok(Self {
            file,
            snapshot: Snapshot::from(&held),
            descriptor_root,
            parent: path.to_owned(),
            effective_uid,
        })
    }

    fn refresh_after_create(&mut self) -> Result<(), LineageError> {
        let held = self.file.metadata().map_err(|_| LineageError::Protection)?;
        let by_path =
            std::fs::symlink_metadata(&self.parent).map_err(|_| LineageError::Protection)?;
        validate_directory(&held, self.effective_uid)?;
        validate_directory(&by_path, self.effective_uid)?;
        if held.dev() != by_path.dev() || held.ino() != by_path.ino() {
            return Err(LineageError::Protection);
        }
        self.snapshot = Snapshot::from(&held);
        if Snapshot::from(&by_path) != self.snapshot {
            return Err(LineageError::Protection);
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), LineageError> {
        let held = self.file.metadata().map_err(|_| LineageError::Protection)?;
        let by_path =
            std::fs::symlink_metadata(&self.parent).map_err(|_| LineageError::Protection)?;
        if Snapshot::from(&held) != self.snapshot
            || Snapshot::from(&by_path) != self.snapshot
            || by_path.file_type().is_symlink()
        {
            return Err(LineageError::Protection);
        }
        Ok(())
    }
}

struct ProtectedDirectoryLease {
    directory: PinnedDirectory,
    parent: PathBuf,
}

enum InspectedEntry {
    Absent,
    Present(Vec<u8>),
}

impl ProtectedDirectoryLease {
    fn acquire(path: &Path) -> Result<Self, LineageError> {
        let directory = PinnedDirectory::open(path)?;
        match directory.file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(LineageError::AlreadyBurned),
            Err(std::fs::TryLockError::Error(_)) => return Err(LineageError::Protection),
        }
        Ok(Self {
            directory,
            parent: path.to_owned(),
        })
    }

    fn read_entry(&self, name: &str, maximum_bytes: usize) -> Result<InspectedEntry, LineageError> {
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.directory.descriptor_root.join(name))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(InspectedEntry::Absent);
            }
            Err(_) => return Err(LineageError::Protection),
        };
        let before = file.metadata().map_err(|_| LineageError::Protection)?;
        validate_file(&before, self.directory.effective_uid, maximum_bytes)?;
        let expected = Snapshot::from(&before);
        let mut bytes = Vec::with_capacity((before.len() as usize).min(maximum_bytes));
        std::io::Read::by_ref(&mut file)
            .take((maximum_bytes + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| LineageError::Protection)?;
        if bytes.len() > maximum_bytes {
            return Err(LineageError::BoundExceeded);
        }
        let after = file.metadata().map_err(|_| LineageError::Protection)?;
        if Snapshot::from(&after) != expected || after.len() != bytes.len() as u64 {
            return Err(LineageError::Protection);
        }
        let reopened = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.directory.descriptor_root.join(name))
            .map_err(|_| LineageError::Protection)?;
        let reopened = reopened.metadata().map_err(|_| LineageError::Protection)?;
        validate_file(&reopened, self.directory.effective_uid, maximum_bytes)?;
        if Snapshot::from(&reopened) != expected {
            return Err(LineageError::Protection);
        }
        self.validate()?;
        Ok(InspectedEntry::Present(bytes))
    }

    fn entry_present(&self, name: &str) -> Result<bool, LineageError> {
        match std::fs::symlink_metadata(self.directory.descriptor_root.join(name)) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(LineageError::Protection),
        }
    }

    fn refresh_after_create(&mut self) -> Result<(), LineageError> {
        self.directory.refresh_after_create()
    }

    fn validate(&self) -> Result<(), LineageError> {
        if self.parent != self.directory.parent {
            return Err(LineageError::Protection);
        }
        self.directory.validate()
    }
}

struct ProtectedFile {
    file: File,
    directory: PinnedDirectory,
    name: std::ffi::OsString,
    identity: (u64, u64),
    maximum_bytes: usize,
}

impl ProtectedFile {
    fn create_new(path: &Path, maximum_bytes: usize) -> Result<Self, LineageError> {
        let parent = path.parent().ok_or(LineageError::Protection)?;
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
                    LineageError::AlreadyBurned
                } else {
                    LineageError::Protection
                }
            })?;
        file.sync_all()
            .and_then(|()| directory.file.sync_all())
            .map_err(|_| LineageError::Durability)?;
        directory.refresh_after_create()?;
        let metadata = file.metadata().map_err(|_| LineageError::Protection)?;
        validate_file(&metadata, directory.effective_uid, maximum_bytes)?;
        Ok(Self {
            file,
            directory,
            name,
            identity: (metadata.dev(), metadata.ino()),
            maximum_bytes,
        })
    }

    fn refresh_parent_after_bound_create(&mut self) -> Result<(), LineageError> {
        self.directory.refresh_after_create()
    }

    fn append_durable(&mut self, expected: &[u8], suffix: &[u8]) -> Result<(), LineageError> {
        let final_length = expected
            .len()
            .checked_add(suffix.len())
            .ok_or(LineageError::BoundExceeded)?;
        if final_length > self.maximum_bytes {
            return Err(LineageError::BoundExceeded);
        }
        self.validate_exact(expected)?;
        self.file
            .write_all(suffix)
            .and_then(|()| self.file.sync_all())
            .and_then(|()| self.directory.file.sync_all())
            .map_err(|_| LineageError::Durability)?;
        self.validate_identity(final_length as u64)
    }

    fn validate_exact(&mut self, expected: &[u8]) -> Result<(), LineageError> {
        if expected.len() > self.maximum_bytes {
            return Err(LineageError::BoundExceeded);
        }
        self.validate_identity(expected.len() as u64)?;
        self.file.rewind().map_err(|_| LineageError::Protection)?;
        let mut actual = Vec::with_capacity(expected.len());
        std::io::Read::by_ref(&mut self.file)
            .take((self.maximum_bytes + 1) as u64)
            .read_to_end(&mut actual)
            .map_err(|_| LineageError::Protection)?;
        if actual != expected {
            return Err(LineageError::AmbiguousTail);
        }
        self.validate_identity(expected.len() as u64)
    }

    fn validate_identity(&self, expected_length: u64) -> Result<(), LineageError> {
        let held = self.file.metadata().map_err(|_| LineageError::Protection)?;
        validate_file(&held, self.directory.effective_uid, self.maximum_bytes)?;
        if (held.dev(), held.ino()) != self.identity || held.len() != expected_length {
            return Err(LineageError::Protection);
        }
        let reopened = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.directory.descriptor_root.join(&self.name))
            .map_err(|_| LineageError::Protection)?;
        let metadata = reopened.metadata().map_err(|_| LineageError::Protection)?;
        validate_file(&metadata, self.directory.effective_uid, self.maximum_bytes)?;
        if (metadata.dev(), metadata.ino()) != self.identity || metadata.len() != expected_length {
            return Err(LineageError::Protection);
        }
        self.directory.validate()
    }
}

fn direct_name(path: &Path) -> Result<&std::ffi::OsStr, LineageError> {
    let name = path.file_name().ok_or(LineageError::Protection)?;
    if name.is_empty() || name == std::ffi::OsStr::new(".") || name == std::ffi::OsStr::new("..") {
        return Err(LineageError::Protection);
    }
    Ok(name)
}

fn validate_file(
    metadata: &Metadata,
    effective_uid: u32,
    maximum_bytes: usize,
) -> Result<(), LineageError> {
    if !metadata.is_file()
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(LineageError::Protection);
    }
    if metadata.len() > maximum_bytes as u64 {
        return Err(LineageError::BoundExceeded);
    }
    Ok(())
}

fn validate_directory(metadata: &Metadata, effective_uid: u32) -> Result<(), LineageError> {
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(LineageError::InvalidDirectory);
    }
    Ok(())
}

fn effective_uid() -> Result<u32, LineageError> {
    let status =
        std::fs::read_to_string("/proc/self/status").map_err(|_| LineageError::InvalidDirectory)?;
    status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_ascii_whitespace().nth(2))
        .and_then(|value| value.parse().ok())
        .ok_or(LineageError::InvalidDirectory)
}

#[cfg(test)]
mod tests {
    include!("lineage/tests.rs");
}
