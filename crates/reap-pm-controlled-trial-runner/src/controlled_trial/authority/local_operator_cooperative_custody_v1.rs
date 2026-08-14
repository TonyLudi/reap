//! Runner-private realization of the reviewed local-operator cooperative
//! credential-custody profile V1.
//!
//! This is a same-EUID cooperative protocol only. The directory `flock` is an
//! advisory lease and does not protect against a same-EUID actor which ignores
//! it. Successful construction proves only that this process continuously
//! holds one matching directory descriptor and four matching source
//! descriptors, and that the bytes read from those descriptors bind one signer
//! and one L2 bundle. It does not prove provider origin or authorship, lease or
//! rotation truth, global uniqueness, currentness, revocation state, or the
//! absence of another descriptor or credential copy.
//!
//! This module has no cleanup operation. In particular, it makes no atomic
//! exact-inode unlink or secure-erasure claim. It does not construct an actor,
//! generation, attempt, durable burn, remote-acceptance or proxy-control proof,
//! request, HMAC, network operation, dispatch owner, or mutation authority.
//! The reviewed profile and delivery evidence remain permanently DENIED.
//!
//! Every successfully parsed secret is owned by `Zeroizing` or by the existing
//! zeroizing signer/L2 types. On every pre-return failure those values are
//! dropped and zeroized. This module performs no write, rename, or removal
//! operation, so every credential basename is left unchanged. That statement
//! is limited to these userspace values and namespace operations; it is not a
//! secure-erasure or no-copy claim.

use std::{
    fmt,
    fs::{File, Metadata},
    os::unix::fs::{FileExt as _, MetadataExt as _},
    path::PathBuf,
    rc::Rc,
};

use reap_pm_controlled_trial::{
    CanonicalFreshCredentialDeliveryBindingEvidenceV1,
    CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1, FreshCredentialDeliveryLoadTokenV1,
    FreshCredentialLinuxDirectoryIdentityV1, FreshCredentialLinuxFileIdentityV1,
    FreshCredentialLinuxObjectSetV1, OfflineAuthorizationState, PM_T2_FRESH_API_KEY_ENTRY_V1,
    PM_T2_FRESH_L2_SECRET_ENTRY_V1, PM_T2_FRESH_PASSPHRASE_ENTRY_V1,
    PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1, ReviewedLocalOperatorCooperativeCustodyProfileContextV1,
    verify_fresh_credential_delivery_binding_evidence_v1,
    verify_reviewed_local_operator_cooperative_custody_profile_v1,
};
use reap_polymarket_auth::{EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials};
use rustix::fs::{FlockOperation, Mode, OFlags};
use thiserror::Error;
use zeroize::Zeroizing;

const PRIVATE_KEY_MAXIMUM_BYTES: usize = 66;
const API_KEY_MAXIMUM_BYTES: usize = 36;
const L2_SECRET_MAXIMUM_BYTES: usize = 172;
const PASSPHRASE_MAXIMUM_BYTES: usize = 128;

/// Move-only, redacted local custody. The directory descriptor itself carries
/// the continuously held cooperative lease. There is deliberately no getter,
/// split, cleanup, unlink, signing, authentication, or authority API.
#[must_use = "local credential custody must remain continuously owned"]
#[allow(
    dead_code,
    reason = "the future selected actor has not been authorized or wired"
)]
pub(super) struct LocalOperatorFreshCredentialCustodyV1 {
    _signer: FixedEoaSigner,
    _l2: L2Credentials,
    _credential_sources: [File; 4],
    _cooperative_lease_and_directory: File,
}

/// Structurally reviewed but unopened custody inputs prepared before the
/// selected actor thread is started. The whole delivery token remains intact:
/// this holder cannot project its nested locator token, Linux metadata, path,
/// signer, or any credential bytes.
///
/// The token/evidence fingerprint equality is deliberately not claimed here.
/// `FreshCredentialDeliveryLoadTokenV1` exposes no non-consuming projection;
/// the existing realization boundary checks that equality only after consuming
/// the whole token and before opening the credential directory. Keeping the
/// token unopened preserves retry semantics while the selected actor remains
/// permanently denied.
#[must_use = "prepared local credential custody must be actor-bound or deliberately dropped"]
pub(in crate::controlled_trial) struct PreparedUnopenedLocalOperatorFreshCredentialCustodyV1 {
    reviewed_profile: CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    retained_delivery_evidence: CanonicalFreshCredentialDeliveryBindingEvidenceV1,
    whole_delivery_load_token: FreshCredentialDeliveryLoadTokenV1,
}

impl fmt::Debug for PreparedUnopenedLocalOperatorFreshCredentialCustodyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PreparedUnopenedLocalOperatorFreshCredentialCustodyV1(<whole-token; retained-evidence; reviewed-profile; unopened; denied>)",
        )
    }
}

/// One inseparable unopened custody aggregate bound to the exact selected
/// actor `Rc` generation. It has no projection, realization, cleanup, signing,
/// authentication, network, dispatch, or mutation API.
#[must_use = "generation-bound unopened custody must remain actor-owned"]
pub(in crate::controlled_trial) struct GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1<
    G,
> {
    _reviewed_profile: CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    _retained_delivery_evidence: CanonicalFreshCredentialDeliveryBindingEvidenceV1,
    _whole_delivery_load_token: FreshCredentialDeliveryLoadTokenV1,
    _selected_actor_generation: Rc<G>,
}

impl<G> fmt::Debug for GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1(<whole-token; retained-evidence; reviewed-profile; Rc-generation; unopened; denied>)",
        )
    }
}

impl PreparedUnopenedLocalOperatorFreshCredentialCustodyV1 {
    /// Infallibly seal the already reviewed unopened conjunction to one
    /// caller-created `Rc` generation. The selected actor performs its own
    /// current-thread generation checks immediately before this transition.
    pub(in crate::controlled_trial) fn bind_to_actor_generation<G>(
        self,
        selected_actor_generation: Rc<G>,
    ) -> GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1<G> {
        GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1 {
            _reviewed_profile: self.reviewed_profile,
            _retained_delivery_evidence: self.retained_delivery_evidence,
            _whole_delivery_load_token: self.whole_delivery_load_token,
            _selected_actor_generation: selected_actor_generation,
        }
    }
}

impl fmt::Debug for LocalOperatorFreshCredentialCustodyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "LocalOperatorFreshCredentialCustodyV1(<five-held-descriptors; cooperative-lease; signer-and-l2-redacted; denied>)",
        )
    }
}

/// Content-free failures. No variant retains or prints a path, basename,
/// credential value, fingerprint, signer, Linux identity, or OS error string.
#[derive(Debug, PartialEq, Eq, Error)]
#[allow(
    dead_code,
    reason = "the future selected actor has not been authorized or wired"
)]
pub(in crate::controlled_trial) enum LocalOperatorCooperativeCustodyV1Error {
    #[error("the reviewed local-operator custody profile was rejected")]
    ProfileRejected,
    #[error("the retained credential-delivery evidence was rejected")]
    DeliveryEvidenceRejected,
    #[error("the whole delivery token does not match the retained delivery evidence")]
    DeliveryTokenEvidenceMismatch,
    #[error("the delivery, locator, profile, and canonical holders do not form one conjunction")]
    CanonicalHolderMismatch,
    #[error("the current effective user does not match the reviewed conjunction")]
    EffectiveUserMismatch,
    #[error("the sealed expected Linux object set is internally inconsistent")]
    InvalidExpectedObjectSet,
    #[error("the configured credential directory is not one exact canonical path")]
    NonCanonicalDirectory,
    #[error("the configured credential directory could not be opened safely")]
    DirectoryOpenFailed,
    #[error("the nonblocking exclusive cooperative directory lease is unavailable")]
    CooperativeLeaseUnavailable,
    #[error("the held credential directory is not one matching protected directory")]
    DirectoryMetadataMismatch,
    #[error("one fixed credential basename could not be opened safely")]
    CredentialOpenFailed,
    #[error("one held credential source is not a regular file")]
    CredentialNotRegular,
    #[error("one held credential source is not owned by the effective user")]
    CredentialOwnerMismatch,
    #[error("one held credential source mode is not exactly 0600")]
    CredentialModeMismatch,
    #[error("one held credential source does not have exactly one hard link")]
    CredentialLinkCountMismatch,
    #[error("one held credential source exceeds its fixed size bound")]
    CredentialTooLarge,
    #[error("one held credential source does not match its role-keyed delivery metadata")]
    CredentialMetadataMismatch,
    #[error("two credential roles resolve to one inode identity")]
    DuplicateCredentialInode,
    #[error("one held credential source could not be read with bounded pread")]
    CredentialReadFailed,
    #[error("one held credential source changed between descriptor observations")]
    CredentialChanged,
    #[error("one held credential source is not UTF-8")]
    CredentialNotUtf8,
    #[error("the private key does not bind the exact configured signer")]
    SignerBindingRejected,
    #[error("the L2 credential bundle does not bind the exact configured signer")]
    L2BindingRejected,
    #[error("the private-key and L2 signer identities disagree")]
    CredentialIdentityMismatch,
}

/// Verify the complete reviewed conjunction while retaining the whole
/// delivery token unopened. This performs no filesystem operation and binds no
/// actor generation; the selected actor completes the infallible `Rc` binding
/// only after its fallible non-secret construction and revalidation steps.
pub(in crate::controlled_trial) fn prepare_unopened_local_operator_fresh_credential_custody_v1(
    context: &ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'_>,
    reviewed_profile: CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    delivery_load_token: FreshCredentialDeliveryLoadTokenV1,
    retained_delivery_evidence: CanonicalFreshCredentialDeliveryBindingEvidenceV1,
) -> Result<
    PreparedUnopenedLocalOperatorFreshCredentialCustodyV1,
    LocalOperatorCooperativeCustodyV1Error,
> {
    verify_reviewed_conjunction(context, &reviewed_profile, &retained_delivery_evidence)?;
    Ok(PreparedUnopenedLocalOperatorFreshCredentialCustodyV1 {
        reviewed_profile,
        retained_delivery_evidence,
        whole_delivery_load_token: delivery_load_token,
    })
}

/// The sole production-shaped construction boundary. It accepts only the
/// complete reviewed conjunction, the whole one-shot delivery token, and the
/// borrowed retained delivery evidence. The nested locator token is consumed
/// only after every offline conjunction and permanent-denial check succeeds.
#[allow(
    dead_code,
    reason = "the future selected actor has not been authorized or wired"
)]
pub(super) fn realize_local_operator_fresh_credential_custody_v1(
    context: &ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'_>,
    reviewed_profile: &CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    delivery_load_token: FreshCredentialDeliveryLoadTokenV1,
    retained_delivery_evidence: &CanonicalFreshCredentialDeliveryBindingEvidenceV1,
) -> Result<LocalOperatorFreshCredentialCustodyV1, LocalOperatorCooperativeCustodyV1Error> {
    // Consuming the outer token happens exactly once. A fingerprint mismatch is
    // rejected before the nested locator token is opened or any filesystem
    // operation is attempted.
    let (reviewed_locator_load_token, expected_linux_objects, token_delivery_fingerprint) =
        delivery_load_token.into_parts();
    if token_delivery_fingerprint != retained_delivery_evidence.fingerprint() {
        return Err(LocalOperatorCooperativeCustodyV1Error::DeliveryTokenEvidenceMismatch);
    }

    verify_reviewed_conjunction(context, reviewed_profile, retained_delivery_evidence)?;
    let joined = &context.phase_a_eligibility_context;

    let effective_uid = rustix::process::geteuid().as_raw();
    if effective_uid != joined.online_authorization_v2.value().host.linux_euid
        || effective_uid != expected_linux_objects.directory.owner_uid
    {
        return Err(LocalOperatorCooperativeCustodyV1Error::EffectiveUserMismatch);
    }

    // This is the only use of the nested locator token. Its path and signer are
    // sealed immediately into the descriptor-pinned realization and are never
    // returned or retained as projections.
    let (directory, configured_signer) = reviewed_locator_load_token.into_parts();
    if configured_signer != joined.v1_config.value().account.signer {
        return Err(LocalOperatorCooperativeCustodyV1Error::CanonicalHolderMismatch);
    }
    realize_sealed_delivery(SealedDeliveryParts {
        directory,
        configured_signer,
        expected_linux_objects,
        effective_uid,
    })
}

fn verify_reviewed_conjunction(
    context: &ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'_>,
    reviewed_profile: &CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    retained_delivery_evidence: &CanonicalFreshCredentialDeliveryBindingEvidenceV1,
) -> Result<(), LocalOperatorCooperativeCustodyV1Error> {
    let joined = &context.phase_a_eligibility_context;
    let delivery_verification = verify_fresh_credential_delivery_binding_evidence_v1(
        joined.v1_config,
        joined.online_policy_v2,
        joined.online_authorization_v2,
        retained_delivery_evidence,
    )
    .map_err(|_| LocalOperatorCooperativeCustodyV1Error::DeliveryEvidenceRejected)?;
    if !delivery_verification_is_permanently_denied(&delivery_verification) {
        return Err(LocalOperatorCooperativeCustodyV1Error::DeliveryEvidenceRejected);
    }

    let profile_verification =
        verify_reviewed_local_operator_cooperative_custody_profile_v1(context, reviewed_profile)
            .map_err(|_| LocalOperatorCooperativeCustodyV1Error::ProfileRejected)?;
    if !profile_verification_is_permanently_denied(&profile_verification) {
        return Err(LocalOperatorCooperativeCustodyV1Error::ProfileRejected);
    }

    if retained_delivery_evidence.fingerprint()
        != delivery_verification.fresh_credential_delivery_binding_fingerprint
        || delivery_verification.fresh_credential_delivery_binding_fingerprint
            != joined.fresh_credential_delivery_binding_v1.fingerprint()
        || delivery_verification.reviewed_fresh_credential_slot_locator_fingerprint
            != joined
                .reviewed_fresh_credential_slot_locator_v1
                .fingerprint()
        || profile_verification.fresh_credential_delivery_binding_v1_fingerprint
            != joined.fresh_credential_delivery_binding_v1.fingerprint()
        || profile_verification.reviewed_fresh_credential_slot_locator_v1_fingerprint
            != joined
                .reviewed_fresh_credential_slot_locator_v1
                .fingerprint()
    {
        return Err(LocalOperatorCooperativeCustodyV1Error::CanonicalHolderMismatch);
    }
    Ok(())
}

fn delivery_verification_is_permanently_denied(
    verification: &reap_pm_controlled_trial::FreshCredentialDeliveryBindingVerificationV1,
) -> bool {
    verification.authorization == OfflineAuthorizationState::DENIED
        && verification.authorization.place_dispatch_allowance == 0
        && verification.exact_reviewed_locator_pins_structurally_valid
        && verification.unattested_provider_generation_labels_structurally_valid
        && verification.unattested_linux_object_metadata_labels_structurally_valid
        && verification.unattested_validity_labels_nested_within_v2
        && !verification.source_owned_current_time_checked
        && !verification.protected_credential_directory_and_four_files_checked
        && !verification.loaded_linux_objects_match_unattested_binding
        && !verification.same_loaded_holder_attested
        && !verification.globally_unique_delivery_attested
        && !verification.provider_authorship_attested
        && !verification.provider_signature_verified
        && !verification.provider_lease_fresh_and_unrevoked
        && !verification.rotation_generation_attested
        && !verification.delivery_freshness_attested
        && !verification.loaded_bundle_matches_credential_slot_generation
        && !verification.remote_api_key_owner_attested
        && !verification.locator_fingerprint_pinned_by_v2
        && !verification.delivery_binding_fingerprint_pinned_by_v2
        && !verification.delivery_consumption_durably_recorded
        && !verification.authorization_consumption_checked
        && !verification.credential_mutation_authority_attested
}

fn profile_verification_is_permanently_denied(
    verification: &reap_pm_controlled_trial::ReviewedLocalOperatorCooperativeCustodyProfileVerificationV1,
) -> bool {
    verification.authorization == OfflineAuthorizationState::DENIED
        && verification.authorization.place_dispatch_allowance == 0
        && verification.v4_reverified_unchanged_and_denied
        && !verification.profile_reviewer_authorship_attested
        && !verification.source_owned_current_time_checked
        && !verification.exact_linux_euid_observed_at_runtime
        && !verification.all_same_euid_actors_trusted_and_quiescent_observed
        && !verification.advisory_directory_lease_acquired
        && !verification.advisory_directory_lease_continuously_held
        && !verification.held_directory_descriptor_opened
        && !verification.four_held_credential_source_descriptors_opened
        && !verification.loaded_linux_objects_match_delivery_binding
        && !verification.private_key_and_l2_credentials_loaded_and_bound
        && !verification.named_l2_entries_retained_for_recovery
        && !verification.conditional_basename_identity_checked
        && !verification.basename_removal_performed
        && !verification.directory_fsync_performed
        && !verification.post_fsync_basename_absence_observed
        && !verification.atomic_unlink_if_inode_attested
        && !verification.secure_erasure_attested
        && !verification.credential_provider_origin_attested
        && !verification.globally_unique_credential_delivery_attested
        && !verification.credential_currentness_attested
        && !verification.credential_or_delivery_unrevoked_attested
        && !verification.no_other_descriptors_or_credential_copies_attested
        && !verification.selected_actor_generation_bound
        && !verification.source_owned_runtime_attempt_bound
        && !verification.same_holder_live_remote_acceptance_proof_verified
        && !verification.signer_proxy_control_proof_verified
        && !verification.durable_attempt_burn_and_no_resend_established
        && !verification.fixed_egress_single_dispatch_owner_minted
        && !verification.authenticated_request_or_hmac_constructed
        && !verification.network_dispatch_performed
        && !verification.credential_mutation_authority_attested
}

struct SealedDeliveryParts {
    directory: PathBuf,
    configured_signer: String,
    expected_linux_objects: FreshCredentialLinuxObjectSetV1,
    effective_uid: u32,
}

struct EntrySpec<'a> {
    basename: &'a str,
    maximum_bytes: usize,
    expected: &'a FreshCredentialLinuxFileIdentityV1,
}

struct HeldCredentialSource {
    file: File,
    snapshot: Snapshot,
    maximum_bytes: usize,
}

fn realize_sealed_delivery(
    sealed: SealedDeliveryParts,
) -> Result<LocalOperatorFreshCredentialCustodyV1, LocalOperatorCooperativeCustodyV1Error> {
    validate_expected_object_set(&sealed.expected_linux_objects, sealed.effective_uid)?;
    let canonical_directory = std::fs::canonicalize(&sealed.directory)
        .map_err(|_| LocalOperatorCooperativeCustodyV1Error::NonCanonicalDirectory)?;
    if canonical_directory != sealed.directory {
        return Err(LocalOperatorCooperativeCustodyV1Error::NonCanonicalDirectory);
    }

    let directory_fd = rustix::fs::open(
        &sealed.directory,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| LocalOperatorCooperativeCustodyV1Error::DirectoryOpenFailed)?;
    let directory = File::from(directory_fd);
    rustix::fs::flock(&directory, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| LocalOperatorCooperativeCustodyV1Error::CooperativeLeaseUnavailable)?;

    let directory_snapshot = Snapshot::from_metadata(
        &directory
            .metadata()
            .map_err(|_| LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch)?,
    );
    validate_directory_snapshot(
        &directory_snapshot,
        &sealed.expected_linux_objects.directory,
        sealed.effective_uid,
    )?;
    revalidate_directory_path(
        &sealed.directory,
        &directory,
        &directory_snapshot,
        &sealed.expected_linux_objects.directory,
        sealed.effective_uid,
    )?;

    let specs = [
        EntrySpec {
            basename: PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1,
            maximum_bytes: PRIVATE_KEY_MAXIMUM_BYTES,
            expected: &sealed.expected_linux_objects.files.private_key,
        },
        EntrySpec {
            basename: PM_T2_FRESH_API_KEY_ENTRY_V1,
            maximum_bytes: API_KEY_MAXIMUM_BYTES,
            expected: &sealed.expected_linux_objects.files.api_key,
        },
        EntrySpec {
            basename: PM_T2_FRESH_L2_SECRET_ENTRY_V1,
            maximum_bytes: L2_SECRET_MAXIMUM_BYTES,
            expected: &sealed.expected_linux_objects.files.l2_secret,
        },
        EntrySpec {
            basename: PM_T2_FRESH_PASSPHRASE_ENTRY_V1,
            maximum_bytes: PASSPHRASE_MAXIMUM_BYTES,
            expected: &sealed.expected_linux_objects.files.passphrase,
        },
    ];
    let sources = [
        open_credential_source(
            &directory,
            &specs[0],
            &directory_snapshot,
            sealed.effective_uid,
        )?,
        open_credential_source(
            &directory,
            &specs[1],
            &directory_snapshot,
            sealed.effective_uid,
        )?,
        open_credential_source(
            &directory,
            &specs[2],
            &directory_snapshot,
            sealed.effective_uid,
        )?,
        open_credential_source(
            &directory,
            &specs[3],
            &directory_snapshot,
            sealed.effective_uid,
        )?,
    ];
    validate_distinct_held_inodes(&sources)?;

    let mut private_key = read_utf8_credential(&sources[0])?;
    let mut api_key = read_utf8_credential(&sources[1])?;
    let mut l2_secret = read_utf8_credential(&sources[2])?;
    let mut passphrase = read_utf8_credential(&sources[3])?;

    for (source, spec) in sources.iter().zip(&specs) {
        re_resolve_credential_source(
            &directory,
            source,
            spec,
            &directory_snapshot,
            sealed.effective_uid,
        )?;
    }
    revalidate_directory_path(
        &sealed.directory,
        &directory,
        &directory_snapshot,
        &sealed.expected_linux_objects.directory,
        sealed.effective_uid,
    )?;

    let signer = FixedEoaSigner::bind(
        EoaPrivateKeyInput::new(take_secret_string(&mut private_key)),
        &sealed.configured_signer,
    )
    .map_err(|_| LocalOperatorCooperativeCustodyV1Error::SignerBindingRejected)?;
    let l2 = L2Credentials::bind(
        &sealed.configured_signer,
        L2CredentialInput::new(
            take_secret_string(&mut api_key),
            take_secret_string(&mut l2_secret),
            take_secret_string(&mut passphrase),
        ),
    )
    .map_err(|_| LocalOperatorCooperativeCustodyV1Error::L2BindingRejected)?;
    if signer.address() != l2.address() {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialIdentityMismatch);
    }

    let [
        private_key_source,
        api_key_source,
        l2_secret_source,
        passphrase_source,
    ] = sources;
    Ok(LocalOperatorFreshCredentialCustodyV1 {
        _signer: signer,
        _l2: l2,
        _credential_sources: [
            private_key_source.file,
            api_key_source.file,
            l2_secret_source.file,
            passphrase_source.file,
        ],
        _cooperative_lease_and_directory: directory,
    })
}

fn validate_expected_object_set(
    expected: &FreshCredentialLinuxObjectSetV1,
    effective_uid: u32,
) -> Result<(), LocalOperatorCooperativeCustodyV1Error> {
    if expected.directory.inode == 0
        || expected.directory.owner_uid != effective_uid
        || expected.directory.permission_mode != 0o700
    {
        return Err(LocalOperatorCooperativeCustodyV1Error::InvalidExpectedObjectSet);
    }
    let files = [
        &expected.files.private_key,
        &expected.files.api_key,
        &expected.files.l2_secret,
        &expected.files.passphrase,
    ];
    for file in files {
        if file.inode == 0
            || file.owner_uid != effective_uid
            || file.owner_uid != expected.directory.owner_uid
            || file.filesystem_device != expected.directory.filesystem_device
            || file.permission_mode != 0o600
            || file.hard_link_count != 1
            || file.inode_key() == expected.directory.inode_key()
        {
            return Err(LocalOperatorCooperativeCustodyV1Error::InvalidExpectedObjectSet);
        }
    }
    for left in 0..files.len() {
        for right in left + 1..files.len() {
            if files[left].inode_key() == files[right].inode_key() {
                return Err(LocalOperatorCooperativeCustodyV1Error::InvalidExpectedObjectSet);
            }
        }
    }
    Ok(())
}

trait ExpectedInodeKey {
    fn inode_key(&self) -> (u64, u64);
}

impl ExpectedInodeKey for FreshCredentialLinuxDirectoryIdentityV1 {
    fn inode_key(&self) -> (u64, u64) {
        (self.filesystem_device, self.inode)
    }
}

impl ExpectedInodeKey for FreshCredentialLinuxFileIdentityV1 {
    fn inode_key(&self) -> (u64, u64) {
        (self.filesystem_device, self.inode)
    }
}

fn open_credential_source(
    directory: &File,
    spec: &EntrySpec<'_>,
    directory_snapshot: &Snapshot,
    effective_uid: u32,
) -> Result<HeldCredentialSource, LocalOperatorCooperativeCustodyV1Error> {
    let fd = rustix::fs::openat(
        directory,
        spec.basename,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| LocalOperatorCooperativeCustodyV1Error::CredentialOpenFailed)?;
    let file = File::from(fd);
    let metadata = file
        .metadata()
        .map_err(|_| LocalOperatorCooperativeCustodyV1Error::CredentialMetadataMismatch)?;
    if !metadata.is_file() {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialNotRegular);
    }
    let snapshot = Snapshot::from_metadata(&metadata);
    if snapshot.uid != effective_uid {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialOwnerMismatch);
    }
    if snapshot.mode != 0o600 {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialModeMismatch);
    }
    if snapshot.nlink != 1 {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialLinkCountMismatch);
    }
    if snapshot.len > spec.maximum_bytes as u64 {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialTooLarge);
    }
    if snapshot.dev != directory_snapshot.dev || snapshot.uid != directory_snapshot.uid {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialMetadataMismatch);
    }
    if !snapshot.matches_expected_file(spec.expected) {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialMetadataMismatch);
    }
    Ok(HeldCredentialSource {
        file,
        snapshot,
        maximum_bytes: spec.maximum_bytes,
    })
}

fn read_utf8_credential(
    source: &HeldCredentialSource,
) -> Result<Zeroizing<String>, LocalOperatorCooperativeCustodyV1Error> {
    let mut bytes = Zeroizing::new(vec![0_u8; source.maximum_bytes + 1]);
    let mut length = 0_usize;
    loop {
        let read = source
            .file
            .read_at(&mut bytes[length..], length as u64)
            .map_err(|_| LocalOperatorCooperativeCustodyV1Error::CredentialReadFailed)?;
        if read == 0 {
            break;
        }
        length += read;
        if length > source.maximum_bytes {
            return Err(LocalOperatorCooperativeCustodyV1Error::CredentialTooLarge);
        }
    }
    if length as u64 != source.snapshot.len {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialChanged);
    }
    let after = Snapshot::from_metadata(
        &source
            .file
            .metadata()
            .map_err(|_| LocalOperatorCooperativeCustodyV1Error::CredentialChanged)?,
    );
    if after != source.snapshot {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialChanged);
    }
    let text = std::str::from_utf8(&bytes[..length])
        .map_err(|_| LocalOperatorCooperativeCustodyV1Error::CredentialNotUtf8)?;
    let output = Zeroizing::new(text.to_owned());
    drop(bytes);
    Ok(output)
}

fn re_resolve_credential_source(
    directory: &File,
    held: &HeldCredentialSource,
    spec: &EntrySpec<'_>,
    directory_snapshot: &Snapshot,
    effective_uid: u32,
) -> Result<(), LocalOperatorCooperativeCustodyV1Error> {
    let reopened = open_credential_source(directory, spec, directory_snapshot, effective_uid)?;
    if reopened.snapshot != held.snapshot {
        return Err(LocalOperatorCooperativeCustodyV1Error::CredentialChanged);
    }
    Ok(())
}

fn validate_distinct_held_inodes(
    sources: &[HeldCredentialSource; 4],
) -> Result<(), LocalOperatorCooperativeCustodyV1Error> {
    for left in 0..sources.len() {
        for right in left + 1..sources.len() {
            if sources[left].snapshot.inode_key() == sources[right].snapshot.inode_key() {
                return Err(LocalOperatorCooperativeCustodyV1Error::DuplicateCredentialInode);
            }
        }
    }
    Ok(())
}

fn validate_directory_snapshot(
    snapshot: &Snapshot,
    expected: &FreshCredentialLinuxDirectoryIdentityV1,
    effective_uid: u32,
) -> Result<(), LocalOperatorCooperativeCustodyV1Error> {
    if !snapshot.is_directory
        || snapshot.uid != effective_uid
        || snapshot.mode != 0o700
        || !snapshot.matches_expected_directory(expected)
    {
        return Err(LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch);
    }
    Ok(())
}

fn revalidate_directory_path(
    configured_path: &PathBuf,
    directory: &File,
    original: &Snapshot,
    expected: &FreshCredentialLinuxDirectoryIdentityV1,
    effective_uid: u32,
) -> Result<(), LocalOperatorCooperativeCustodyV1Error> {
    let canonical = std::fs::canonicalize(configured_path)
        .map_err(|_| LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch)?;
    if canonical != *configured_path {
        return Err(LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch);
    }
    let held = Snapshot::from_metadata(
        &directory
            .metadata()
            .map_err(|_| LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch)?,
    );
    let by_path_metadata = std::fs::symlink_metadata(configured_path)
        .map_err(|_| LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch)?;
    if by_path_metadata.file_type().is_symlink() {
        return Err(LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch);
    }
    let by_path = Snapshot::from_metadata(&by_path_metadata);
    validate_directory_snapshot(&held, expected, effective_uid)?;
    validate_directory_snapshot(&by_path, expected, effective_uid)?;
    if &held != original || &by_path != original {
        return Err(LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch);
    }
    Ok(())
}

fn take_secret_string(value: &mut Zeroizing<String>) -> String {
    std::mem::take(&mut **value)
}

#[derive(PartialEq, Eq)]
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
    is_directory: bool,
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
            is_directory: metadata.is_dir(),
        }
    }

    const fn inode_key(&self) -> (u64, u64) {
        (self.dev, self.ino)
    }

    fn matches_expected_directory(
        &self,
        expected: &FreshCredentialLinuxDirectoryIdentityV1,
    ) -> bool {
        self.dev == expected.filesystem_device
            && self.ino == expected.inode
            && self.uid == expected.owner_uid
            && self.mode == expected.permission_mode
            && self.mtime == expected.modified_seconds
            && self.mtime_nsec == expected.modified_nanoseconds
            && self.ctime == expected.status_changed_seconds
            && self.ctime_nsec == expected.status_changed_nanoseconds
    }

    fn matches_expected_file(&self, expected: &FreshCredentialLinuxFileIdentityV1) -> bool {
        self.dev == expected.filesystem_device
            && self.ino == expected.inode
            && self.uid == expected.owner_uid
            && self.mode == expected.permission_mode
            && self.nlink == expected.hard_link_count
            && self.mtime == expected.modified_seconds
            && self.mtime_nsec == expected.modified_nanoseconds
            && self.ctime == expected.status_changed_seconds
            && self.ctime_nsec == expected.status_changed_nanoseconds
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    // Reuse the existing synthetic ten-holder fixture grammar. The local
    // runtime fixture below replaces its lexical credential directory and
    // unattested Linux metadata with newly created synthetic objects.
    include!(
        "../../../../reap-pm-controlled-trial/tests/reviewed_local_operator_cooperative_custody_profile_v1.rs"
    );

    use super::{FlockOperation, Mode, OFlags};
    use std::os::unix::fs::{MetadataExt as _, symlink};

    const SYNTHETIC_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SYNTHETIC_API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const SYNTHETIC_L2_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const SYNTHETIC_PASSPHRASE: &str = "synthetic-passphrase-only";

    struct SyntheticCredentialDirectory {
        directory: TempDir,
        expected: FreshCredentialLinuxObjectSetV1,
    }

    impl SyntheticCredentialDirectory {
        fn new() -> Self {
            let directory = protected_dir();
            for (basename, value) in [
                (PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1, SYNTHETIC_PRIVATE_KEY),
                (PM_T2_FRESH_API_KEY_ENTRY_V1, SYNTHETIC_API_KEY),
                (PM_T2_FRESH_L2_SECRET_ENTRY_V1, SYNTHETIC_L2_SECRET),
                (PM_T2_FRESH_PASSPHRASE_ENTRY_V1, SYNTHETIC_PASSPHRASE),
            ] {
                write_0600(&directory.path().join(basename), value.as_bytes());
            }
            let expected = expected_linux_objects(directory.path());
            Self {
                directory,
                expected,
            }
        }

        fn path(&self) -> &Path {
            self.directory.path()
        }

        fn assert_all_names_unchanged(&self) {
            for (basename, value) in [
                (PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1, SYNTHETIC_PRIVATE_KEY),
                (PM_T2_FRESH_API_KEY_ENTRY_V1, SYNTHETIC_API_KEY),
                (PM_T2_FRESH_L2_SECRET_ENTRY_V1, SYNTHETIC_L2_SECRET),
                (PM_T2_FRESH_PASSPHRASE_ENTRY_V1, SYNTHETIC_PASSPHRASE),
            ] {
                assert_eq!(
                    fs::read_to_string(self.path().join(basename)).unwrap(),
                    value
                );
            }
        }
    }

    struct SyntheticRuntimeFixture {
        reviewed: StaticFixture,
        credentials: SyntheticCredentialDirectory,
        locator_path: PathBuf,
        delivery_path: PathBuf,
        delivery_record: FreshCredentialDeliveryBindingV1,
    }

    impl SyntheticRuntimeFixture {
        fn new() -> Self {
            let credentials = SyntheticCredentialDirectory::new();
            let record_directory = protected_dir();
            let root = record_directory.path().to_owned();

            let config_path = root.join("canonical-config.json");
            write_0600(&config_path, &serde_json::to_vec(&trial_config()).unwrap());
            let config = load_canonical_trial_config(&config_path).unwrap();
            let policy_path = write_json(&root, "online-policy-v2.json", &online_policy(&config));
            let online_policy = load_canonical_online_policy_v2(&policy_path).unwrap();
            let mut authorization_record = online_authorization(&config, &online_policy);
            authorization_record.host.linux_euid = rustix::process::geteuid().as_raw();
            let authorization_path =
                write_json(&root, "online-authorization-v2.json", &authorization_record);
            let online_authorization =
                load_canonical_online_authorization_v2(&authorization_path).unwrap();
            let destination_path = write_json(
                &root,
                "reviewed-destination-v1.json",
                &destination_profile(&config, &online_policy, &online_authorization),
            );
            let reviewed_destination =
                load_canonical_reviewed_production_destination_profile_v1(&destination_path)
                    .unwrap();

            let mut locator_record =
                credential_locator(&config, &online_policy, &online_authorization);
            locator_record.protected_fresh_credential_directory = credentials
                .path()
                .to_str()
                .expect("synthetic temporary path is UTF-8")
                .to_owned();
            let locator_path = write_json(&root, "reviewed-locator-v1.json", &locator_record);
            let reviewed_fresh_credential_locator =
                load_canonical_reviewed_fresh_credential_slot_locator_v1(&locator_path).unwrap();

            let mut delivery_record = delivery_binding(&reviewed_fresh_credential_locator);
            delivery_record.unattested_linux_objects = credentials.expected.clone();
            let delivery_path = write_json(&root, "delivery-binding-v1.json", &delivery_record);
            let fresh_credential_delivery =
                load_canonical_fresh_credential_delivery_binding_v1(&delivery_path).unwrap();
            let identity_path = write_json(
                &root,
                "reviewed-account-identity-v1.json",
                &reviewed_account_identity(&config, &online_policy, &online_authorization),
            );
            let reviewed_signer_proxy_identity =
                load_canonical_reviewed_signer_proxy_account_identity_v1(&identity_path).unwrap();
            let base = BaseFixture {
                _directory: record_directory,
                root,
                config,
                online_policy,
                online_authorization,
                reviewed_destination,
                reviewed_fresh_credential_locator,
                fresh_credential_delivery,
                reviewed_signer_proxy_identity,
            };
            let v1_path = write_json(
                &base.root,
                "authorization-v1.json",
                &v1_authorization(&base.config),
            );
            let v1_authorization = load_canonical_authorization(&v1_path).unwrap();
            let remote_path = write_json(
                &base.root,
                "reviewed-remote-policy-v1.json",
                &reviewed_remote_policy(&base),
            );
            let remote_policy =
                load_canonical_reviewed_remote_credential_proof_policy_v1(&remote_path).unwrap();
            Self {
                reviewed: StaticFixture {
                    base,
                    v1_authorization,
                    remote_policy,
                },
                credentials,
                locator_path,
                delivery_path,
                delivery_record,
            }
        }

        fn bind_original_delivery(
            &self,
        ) -> (
            reap_pm_controlled_trial::CanonicalFreshCredentialDeliveryBindingEvidenceV1,
            reap_pm_controlled_trial::FreshCredentialDeliveryLoadTokenV1,
        ) {
            let locator =
                load_canonical_reviewed_fresh_credential_slot_locator_v1(&self.locator_path)
                    .unwrap();
            let delivery =
                load_canonical_fresh_credential_delivery_binding_v1(&self.delivery_path).unwrap();
            reap_pm_controlled_trial::bind_fresh_credential_delivery_binding_v1(
                &self.reviewed.base.config,
                &self.reviewed.base.online_policy,
                &self.reviewed.base.online_authorization,
                locator,
                delivery,
            )
            .unwrap()
        }
    }

    #[test]
    fn exact_reviewed_token_evidence_profile_success_retains_lease_fds_and_bound_credentials() {
        let fixture = SyntheticRuntimeFixture::new();
        let static_v3 = fixture
            .reviewed
            .load_record("runtime-static-v3.json", &fixture.reviewed.record());
        let v4_context = phase_a_eligibility_context(&fixture.reviewed, &static_v3);
        let v4_record = phase_a_eligibility_record(&v4_context);
        let v4_path = write_json(
            &fixture.reviewed.base.root,
            "runtime-eligibility-v4.json",
            &v4_record,
        );
        let v4 = reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(
            &v4_path,
        )
        .unwrap();
        let context = local_operator_context(&fixture.reviewed, &static_v3, &v4);
        let profile_record = reap_pm_controlled_trial::draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(&context).unwrap();
        let profile_path = write_json(
            &fixture.reviewed.base.root,
            "runtime-local-profile-v1.json",
            &profile_record,
        );
        let profile = reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&profile_path).unwrap();
        let (evidence, token) = fixture.bind_original_delivery();

        let custody = super::realize_local_operator_fresh_credential_custody_v1(
            &context, &profile, token, &evidence,
        )
        .unwrap();
        let expected_signer = reap_polymarket_auth::EoaAddress::parse(SIGNER).unwrap();
        assert_eq!(custody._signer.address(), expected_signer);
        assert_eq!(custody._l2.address(), expected_signer);
        assert_eq!(custody._credential_sources.len(), 4);
        fixture.credentials.assert_all_names_unchanged();

        let competing_fd = rustix::fs::open(
            fixture.credentials.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        assert!(
            rustix::fs::flock(&competing_fd, FlockOperation::NonBlockingLockExclusive).is_err(),
            "custody must continuously retain the cooperative lease"
        );
        let debug = format!("{custody:?}");
        assert!(debug.contains("denied"));
        assert!(!debug.contains(SYNTHETIC_PRIVATE_KEY));
        assert!(!debug.contains(SYNTHETIC_API_KEY));
        drop(custody);
        rustix::fs::flock(&competing_fd, FlockOperation::NonBlockingLockExclusive).unwrap();
    }

    #[test]
    fn whole_delivery_token_mismatch_is_rejected_before_any_directory_open() {
        let fixture = SyntheticRuntimeFixture::new();
        let static_v3 = fixture
            .reviewed
            .load_record("mismatch-static-v3.json", &fixture.reviewed.record());
        let v4_context = phase_a_eligibility_context(&fixture.reviewed, &static_v3);
        let v4_record = phase_a_eligibility_record(&v4_context);
        let v4_path = write_json(
            &fixture.reviewed.base.root,
            "mismatch-eligibility-v4.json",
            &v4_record,
        );
        let v4 = reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(
            &v4_path,
        )
        .unwrap();
        let context = local_operator_context(&fixture.reviewed, &static_v3, &v4);
        let profile_record = reap_pm_controlled_trial::draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(&context).unwrap();
        let profile_path = write_json(
            &fixture.reviewed.base.root,
            "mismatch-local-profile-v1.json",
            &profile_record,
        );
        let profile = reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&profile_path).unwrap();
        let (evidence, original_token) = fixture.bind_original_delivery();
        drop(original_token);

        let mut drifted_record = fixture.delivery_record.clone();
        drifted_record
            .unattested_provider_generation
            .delivery_id
            .push_str("-different");
        let drifted_path = write_json(
            &fixture.reviewed.base.root,
            "mismatch-delivery-binding-v1.json",
            &drifted_record,
        );
        let drifted_delivery =
            load_canonical_fresh_credential_delivery_binding_v1(&drifted_path).unwrap();
        let drifted_locator =
            load_canonical_reviewed_fresh_credential_slot_locator_v1(&fixture.locator_path)
                .unwrap();
        let (_, drifted_token) =
            reap_pm_controlled_trial::bind_fresh_credential_delivery_binding_v1(
                &fixture.reviewed.base.config,
                &fixture.reviewed.base.online_policy,
                &fixture.reviewed.base.online_authorization,
                drifted_locator,
                drifted_delivery,
            )
            .unwrap();

        let error = super::realize_local_operator_fresh_credential_custody_v1(
            &context,
            &profile,
            drifted_token,
            &evidence,
        )
        .unwrap_err();
        assert_eq!(
            error,
            super::LocalOperatorCooperativeCustodyV1Error::DeliveryTokenEvidenceMismatch
        );
        fixture.credentials.assert_all_names_unchanged();
    }

    #[test]
    fn every_role_keyed_expected_metadata_field_and_inode_alias_is_rejected() {
        let fixture = SyntheticCredentialDirectory::new();
        let mut exercised = 0_usize;

        for field in 0..8 {
            let mut expected = fixture.expected.clone();
            perturb_directory_field(&mut expected.directory, field);
            assert!(realize_direct(fixture.path(), expected).is_err());
            exercised += 1;
        }
        for role in 0..4 {
            for field in 0..9 {
                let mut expected = fixture.expected.clone();
                perturb_file_field(expected_file_mut(&mut expected, role), field);
                assert!(realize_direct(fixture.path(), expected).is_err());
                exercised += 1;
            }
        }
        assert_eq!(exercised, 44);

        let mut swapped = fixture.expected.clone();
        let private_key = swapped.files.private_key.clone();
        swapped.files.private_key = swapped.files.api_key.clone();
        swapped.files.api_key = private_key;
        assert!(realize_direct(fixture.path(), swapped).is_err());

        let mut duplicated = fixture.expected.clone();
        duplicated.files.api_key = duplicated.files.private_key.clone();
        assert_eq!(
            realize_direct(fixture.path(), duplicated).unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::InvalidExpectedObjectSet
        );
        fixture.assert_all_names_unchanged();
    }

    #[test]
    fn unsafe_file_kinds_modes_links_sizes_and_cooperative_lock_contention_are_rejected() {
        let wrong_directory_mode = SyntheticCredentialDirectory::new();
        fs::set_permissions(
            wrong_directory_mode.path(),
            fs::Permissions::from_mode(0o750),
        )
        .unwrap();
        assert_eq!(
            realize_direct(
                wrong_directory_mode.path(),
                wrong_directory_mode.expected.clone()
            )
            .unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::DirectoryMetadataMismatch
        );

        let wrong_file_mode = SyntheticCredentialDirectory::new();
        let private_key = wrong_file_mode
            .path()
            .join(PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1);
        fs::set_permissions(&private_key, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            realize_direct(wrong_file_mode.path(), wrong_file_mode.expected.clone()).unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::CredentialModeMismatch
        );
        assert!(private_key.exists());

        let symbolic = SyntheticCredentialDirectory::new();
        let private_key = symbolic.path().join(PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1);
        fs::rename(
            &private_key,
            symbolic.path().join("synthetic-displaced-private-key"),
        )
        .unwrap();
        symlink(
            symbolic.path().join(PM_T2_FRESH_API_KEY_ENTRY_V1),
            &private_key,
        )
        .unwrap();
        let mut symbolic_expected = symbolic.expected.clone();
        symbolic_expected.directory = expected_directory(symbolic.path());
        assert_eq!(
            realize_direct(symbolic.path(), symbolic_expected).unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::CredentialOpenFailed
        );
        assert!(
            fs::symlink_metadata(&private_key)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        let hard_linked = SyntheticCredentialDirectory::new();
        let extra_link = hard_linked.path().join("synthetic-extra-hard-link");
        fs::hard_link(
            hard_linked.path().join(PM_T2_FRESH_API_KEY_ENTRY_V1),
            &extra_link,
        )
        .unwrap();
        let mut hard_link_expected = hard_linked.expected.clone();
        hard_link_expected.directory = expected_directory(hard_linked.path());
        assert_eq!(
            realize_direct(hard_linked.path(), hard_link_expected).unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::CredentialLinkCountMismatch
        );
        assert!(extra_link.exists());

        let fifo = SyntheticCredentialDirectory::new();
        let passphrase = fifo.path().join(PM_T2_FRESH_PASSPHRASE_ENTRY_V1);
        fs::rename(
            &passphrase,
            fifo.path().join("synthetic-displaced-passphrase"),
        )
        .unwrap();
        rustix::fs::mkfifoat(rustix::fs::CWD, &passphrase, Mode::RUSR | Mode::WUSR).unwrap();
        let mut fifo_expected = fifo.expected.clone();
        fifo_expected.directory = expected_directory(fifo.path());
        assert_eq!(
            realize_direct(fifo.path(), fifo_expected).unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::CredentialNotRegular
        );
        assert!(passphrase.exists());

        let oversized = SyntheticCredentialDirectory::new();
        let private_key = oversized.path().join(PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1);
        fs::rename(
            &private_key,
            oversized.path().join("synthetic-displaced-private-key"),
        )
        .unwrap();
        write_0600(&private_key, &[b'a'; super::PRIVATE_KEY_MAXIMUM_BYTES + 1]);
        let mut oversized_expected = oversized.expected.clone();
        oversized_expected.directory = expected_directory(oversized.path());
        assert_eq!(
            realize_direct(oversized.path(), oversized_expected).unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::CredentialTooLarge
        );
        assert!(private_key.exists());

        let contended = SyntheticCredentialDirectory::new();
        let competing_fd = rustix::fs::open(
            contended.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        rustix::fs::flock(&competing_fd, FlockOperation::NonBlockingLockExclusive).unwrap();
        assert_eq!(
            realize_direct(contended.path(), contended.expected.clone()).unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::CooperativeLeaseUnavailable
        );
        contended.assert_all_names_unchanged();
    }

    #[test]
    fn held_source_revalidation_rejects_name_replacement_and_post_open_metadata_drift() {
        let replaced = SyntheticCredentialDirectory::new();
        let directory_fd = rustix::fs::open(
            replaced.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let directory = std::fs::File::from(directory_fd);
        rustix::fs::flock(&directory, FlockOperation::NonBlockingLockExclusive).unwrap();
        let directory_snapshot = super::Snapshot::from_metadata(&directory.metadata().unwrap());
        let spec = super::EntrySpec {
            basename: PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1,
            maximum_bytes: super::PRIVATE_KEY_MAXIMUM_BYTES,
            expected: &replaced.expected.files.private_key,
        };
        let held = super::open_credential_source(
            &directory,
            &spec,
            &directory_snapshot,
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
        let private_key = replaced.path().join(PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1);
        let displaced = replaced.path().join("synthetic-observed-private-key");
        fs::rename(&private_key, &displaced).unwrap();
        write_0600(&private_key, SYNTHETIC_PRIVATE_KEY.as_bytes());
        assert_eq!(
            super::re_resolve_credential_source(
                &directory,
                &held,
                &spec,
                &directory_snapshot,
                rustix::process::geteuid().as_raw(),
            )
            .unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::CredentialMetadataMismatch
        );
        assert!(private_key.exists());
        assert!(displaced.exists());

        let drifted = SyntheticCredentialDirectory::new();
        let directory_fd = rustix::fs::open(
            drifted.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let directory = std::fs::File::from(directory_fd);
        rustix::fs::flock(&directory, FlockOperation::NonBlockingLockExclusive).unwrap();
        let directory_snapshot = super::Snapshot::from_metadata(&directory.metadata().unwrap());
        let spec = super::EntrySpec {
            basename: PM_T2_FRESH_API_KEY_ENTRY_V1,
            maximum_bytes: super::API_KEY_MAXIMUM_BYTES,
            expected: &drifted.expected.files.api_key,
        };
        let held = super::open_credential_source(
            &directory,
            &spec,
            &directory_snapshot,
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
        let api_key = drifted.path().join(PM_T2_FRESH_API_KEY_ENTRY_V1);
        fs::set_permissions(&api_key, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            super::read_utf8_credential(&held).unwrap_err(),
            super::LocalOperatorCooperativeCustodyV1Error::CredentialChanged
        );
        assert!(api_key.exists());
    }

    fn realize_direct(
        directory: &Path,
        expected_linux_objects: FreshCredentialLinuxObjectSetV1,
    ) -> Result<
        super::LocalOperatorFreshCredentialCustodyV1,
        super::LocalOperatorCooperativeCustodyV1Error,
    > {
        super::realize_sealed_delivery(super::SealedDeliveryParts {
            directory: directory.to_owned(),
            configured_signer: SIGNER.to_owned(),
            expected_linux_objects,
            effective_uid: rustix::process::geteuid().as_raw(),
        })
    }

    fn expected_linux_objects(directory: &Path) -> FreshCredentialLinuxObjectSetV1 {
        FreshCredentialLinuxObjectSetV1 {
            directory: expected_directory(directory),
            files: FreshCredentialLinuxFileIdentitiesV1 {
                private_key: expected_file(&directory.join(PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1)),
                api_key: expected_file(&directory.join(PM_T2_FRESH_API_KEY_ENTRY_V1)),
                l2_secret: expected_file(&directory.join(PM_T2_FRESH_L2_SECRET_ENTRY_V1)),
                passphrase: expected_file(&directory.join(PM_T2_FRESH_PASSPHRASE_ENTRY_V1)),
            },
        }
    }

    fn expected_directory(directory: &Path) -> FreshCredentialLinuxDirectoryIdentityV1 {
        let metadata = fs::metadata(directory).unwrap();
        FreshCredentialLinuxDirectoryIdentityV1 {
            filesystem_device: metadata.dev(),
            inode: metadata.ino(),
            owner_uid: metadata.uid(),
            permission_mode: metadata.mode() & 0o7777,
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            status_changed_seconds: metadata.ctime(),
            status_changed_nanoseconds: metadata.ctime_nsec(),
        }
    }

    fn expected_file(path: &Path) -> FreshCredentialLinuxFileIdentityV1 {
        let metadata = fs::metadata(path).unwrap();
        FreshCredentialLinuxFileIdentityV1 {
            filesystem_device: metadata.dev(),
            inode: metadata.ino(),
            owner_uid: metadata.uid(),
            permission_mode: metadata.mode() & 0o7777,
            hard_link_count: metadata.nlink(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            status_changed_seconds: metadata.ctime(),
            status_changed_nanoseconds: metadata.ctime_nsec(),
        }
    }

    fn expected_file_mut(
        expected: &mut FreshCredentialLinuxObjectSetV1,
        role: usize,
    ) -> &mut FreshCredentialLinuxFileIdentityV1 {
        match role {
            0 => &mut expected.files.private_key,
            1 => &mut expected.files.api_key,
            2 => &mut expected.files.l2_secret,
            3 => &mut expected.files.passphrase,
            _ => unreachable!("closed four-role test grammar"),
        }
    }

    fn perturb_directory_field(
        identity: &mut FreshCredentialLinuxDirectoryIdentityV1,
        field: usize,
    ) {
        match field {
            0 => identity.filesystem_device ^= 1,
            1 => identity.inode ^= 1,
            2 => identity.owner_uid = identity.owner_uid.wrapping_add(1),
            3 => identity.permission_mode ^= 1,
            4 => identity.modified_seconds = identity.modified_seconds.wrapping_add(1),
            5 => {
                identity.modified_nanoseconds = (identity.modified_nanoseconds + 1) % 1_000_000_000
            }
            6 => identity.status_changed_seconds = identity.status_changed_seconds.wrapping_add(1),
            7 => {
                identity.status_changed_nanoseconds =
                    (identity.status_changed_nanoseconds + 1) % 1_000_000_000;
            }
            _ => unreachable!("closed directory metadata test grammar"),
        }
    }

    fn perturb_file_field(identity: &mut FreshCredentialLinuxFileIdentityV1, field: usize) {
        match field {
            0 => identity.filesystem_device ^= 1,
            1 => identity.inode ^= 1,
            2 => identity.owner_uid = identity.owner_uid.wrapping_add(1),
            3 => identity.permission_mode ^= 1,
            4 => identity.hard_link_count = identity.hard_link_count.wrapping_add(1),
            5 => identity.modified_seconds = identity.modified_seconds.wrapping_add(1),
            6 => {
                identity.modified_nanoseconds = (identity.modified_nanoseconds + 1) % 1_000_000_000
            }
            7 => identity.status_changed_seconds = identity.status_changed_seconds.wrapping_add(1),
            8 => {
                identity.status_changed_nanoseconds =
                    (identity.status_changed_nanoseconds + 1) % 1_000_000_000;
            }
            _ => unreachable!("closed file metadata test grammar"),
        }
    }
}
