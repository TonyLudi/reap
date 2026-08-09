//! Runner-private custody for reviewed Linux local-egress facts.
//!
//! This source captures one point-in-time mapping from the calling Linux
//! thread's network namespace to an exact interface index and assigned local
//! address. It also pins and hashes the exact absolute, reviewer-authorized
//! non-secret tunnel/gateway profile artifact. The namespace and profile file
//! descriptors remain held so the current-runtime gate can consume this
//! custody and perform its own immediate checks.
//!
//! This value is deliberately unwired and non-authoritative. It observes no
//! route, destination, DNS answer, connected socket, geoblock response, NAT
//! behavior, or public egress. It cannot construct an online runtime binding,
//! permit, credential, request, mutation, or network transport. In particular,
//! a held namespace descriptor pins an object but does not prove that a later
//! thread still inhabits it, and a held profile descriptor does not freeze
//! in-place writes. The consuming runtime recheck therefore repeats process,
//! thread, EUID, namespace, interface, address, profile-identity, and profile-
//! hash observations. Even that recheck proves only same-local-egress
//! selection; it does not create a socket or attest destination NAT behavior.

use std::{
    fmt,
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    net::IpAddr,
    os::unix::fs::MetadataExt as _,
    path::{Component, Path},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

use reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

#[cfg(target_os = "linux")]
use nix::{ifaddrs::getifaddrs, net::if_::InterfaceFlags};
#[cfg(target_os = "linux")]
use rustix::{
    fs::{Mode, OFlags, ResolveFlags},
    net::{AddressFamily, SocketFlags, SocketType},
};

const PROC_ROOT_PATH: &str = "/proc";
const PROC_THREAD_NET_NAMESPACE_ENTRY: &str = "thread-self/ns/net";
const NSFS_MAGIC: rustix::fs::FsWord = 0x6e73_6673;
const MAX_REVIEWED_PROFILE_BYTES: usize = 64 * 1024;
const MAX_LOCAL_FACT_CAPTURE_DURATION: Duration = Duration::from_secs(5);

/// Move-only custody of one point-in-time local Linux egress observation.
///
/// No fact accessor is exposed in this precursor. A later selected-connection
/// actor must take ownership and add an immediate source check plus socket
/// construction in one private operation. That future actor must invoke
/// capture on its own dedicated OS thread; the private `Rc` marker confines
/// custody to that thread because it is structurally neither `Send` nor `Sync`.
/// It must never become a stale positive bearer on its own.
#[must_use = "local-egress fact custody must remain owned or be dropped"]
pub(super) struct PmLinuxEgressLocalFactCustody {
    creating_process_id: u32,
    source_thread_id: u32,
    effective_user_id: u32,
    held_network_namespace: File,
    network_namespace_device: u64,
    network_namespace_inode: u64,
    interface_name: Box<str>,
    interface_index: u32,
    local_source_ip: IpAddr,
    reviewed_profile: HeldReviewedEgressProfile,
    online_authorization_fingerprint: Box<str>,
    wall_started: SystemTime,
    wall_completed: SystemTime,
    monotonic_started: Instant,
    monotonic_completed: Instant,
    thread_confinement: Rc<()>,
}

impl PmLinuxEgressLocalFactCustody {
    /// Capture only source-observed facts named by one exact canonical V2
    /// authorization.
    ///
    /// The path is a locator, not a caller-supplied fact: it must be absolute
    /// UTF-8 and byte-for-byte identical to the authorization's reviewed
    /// profile reference before any profile file is opened.
    pub(super) fn capture(
        authorization: &CanonicalOnlineAuthorizationV2,
        reviewed_nonsecret_profile_path: &Path,
    ) -> Result<Self, PmLinuxEgressLocalFactError> {
        #[cfg(target_os = "linux")]
        {
            let wall_started = SystemTime::now();
            let monotonic_started = Instant::now();
            let creating_process_id = std::process::id();
            let source_thread_id = current_thread_id()?;
            let effective_user_id = rustix::process::geteuid().as_raw();
            let egress = &authorization.value().host.egress;
            validate_effective_user_binding(
                effective_user_id,
                authorization.value().host.linux_euid,
            )?;
            validate_exact_reviewed_profile_path(
                reviewed_nonsecret_profile_path,
                &egress.dedicated_tunnel_or_gateway_profile_reference,
            )?;
            let expected_local_source_ip = egress
                .local_source_ip
                .parse::<IpAddr>()
                .map_err(|_| PmLinuxEgressLocalFactError::AuthorizationBinding)?;

            let (held_network_namespace, namespace_identity) = open_thread_network_namespace()?;
            validate_namespace_binding(
                namespace_identity,
                egress.network_namespace_device,
                egress.network_namespace_inode,
            )?;
            let first_interface = observe_exact_assigned_interface(
                &egress.interface_name,
                egress.interface_index,
                expected_local_source_ip,
            )?;
            let reviewed_profile = HeldReviewedEgressProfile::open(
                reviewed_nonsecret_profile_path,
                &egress.dedicated_tunnel_or_gateway_profile_sha256,
            )?;

            let final_interface = observe_exact_assigned_interface(
                &egress.interface_name,
                egress.interface_index,
                expected_local_source_ip,
            )?;
            if first_interface != final_interface {
                return Err(PmLinuxEgressLocalFactError::InterfaceChanged);
            }
            let final_namespace = observe_thread_network_namespace_identity()?;
            if final_namespace != namespace_identity
                || namespace_file_identity(&held_network_namespace)? != namespace_identity
            {
                return Err(PmLinuxEgressLocalFactError::NamespaceChanged);
            }
            if std::process::id() != creating_process_id || current_thread_id()? != source_thread_id
            {
                return Err(PmLinuxEgressLocalFactError::ProcessOrThreadChanged);
            }
            let final_effective_user_id = rustix::process::geteuid().as_raw();
            if final_effective_user_id != effective_user_id {
                return Err(PmLinuxEgressLocalFactError::EffectiveUserChanged);
            }
            validate_effective_user_binding(
                final_effective_user_id,
                authorization.value().host.linux_euid,
            )?;

            let wall_completed = SystemTime::now();
            let monotonic_completed = Instant::now();
            validate_capture_window(
                wall_started,
                wall_completed,
                monotonic_started,
                monotonic_completed,
            )?;
            Ok(Self {
                creating_process_id,
                source_thread_id,
                effective_user_id,
                held_network_namespace,
                network_namespace_device: namespace_identity.device,
                network_namespace_inode: namespace_identity.inode,
                interface_name: first_interface.name,
                interface_index: first_interface.index,
                local_source_ip: first_interface.local_source_ip,
                reviewed_profile,
                online_authorization_fingerprint: authorization.fingerprint().into(),
                wall_started,
                wall_completed,
                monotonic_started,
                monotonic_completed,
                thread_confinement: Rc::new(()),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (authorization, reviewed_nonsecret_profile_path);
            Err(PmLinuxEgressLocalFactError::UnsupportedPlatform)
        }
    }

    /// Check that this source capture occurred wholly inside the exact outer
    /// preflight window and still belongs to the supplied canonical V2
    /// authorization. This performs no current-source recheck and yields no
    /// fact view; it is used only while assembling the initial runtime
    /// typestate immediately after capture.
    pub(super) fn validate_captured_window(
        &self,
        authorization: &CanonicalOnlineAuthorizationV2,
        window_wall_started: SystemTime,
        window_wall_completed: SystemTime,
        window_monotonic_started: Instant,
        window_monotonic_completed: Instant,
        maximum_age: Duration,
    ) -> Result<(), PmLinuxEgressLocalFactError> {
        self.validate_authorization_binding(authorization)?;
        validate_nested_window(
            window_wall_started,
            window_wall_completed,
            window_monotonic_started,
            window_monotonic_completed,
            self.wall_started,
            self.wall_completed,
            self.monotonic_started,
            self.monotonic_completed,
            maximum_age,
        )
    }

    /// Re-observe every mutable local source and lend the exact facts only for
    /// the lifetime of this still-owned, thread-confined custody. The view is
    /// non-authoritative and cannot outlive the revalidated descriptors.
    pub(super) fn revalidate_for_current_runtime(
        &mut self,
        authorization: &CanonicalOnlineAuthorizationV2,
        window_wall_started: SystemTime,
        window_wall_completed: SystemTime,
        window_monotonic_started: Instant,
        window_monotonic_completed: Instant,
        maximum_age: Duration,
    ) -> Result<PmLinuxEgressLocalFactsView<'_>, PmLinuxEgressLocalFactError> {
        #[cfg(target_os = "linux")]
        {
            self.validate_captured_window(
                authorization,
                window_wall_started,
                window_wall_completed,
                window_monotonic_started,
                window_monotonic_completed,
                maximum_age,
            )?;
            let recheck_wall_started = SystemTime::now();
            let recheck_monotonic_started = Instant::now();

            if std::process::id() != self.creating_process_id
                || current_thread_id()? != self.source_thread_id
                || Rc::strong_count(&self.thread_confinement) != 1
            {
                return Err(PmLinuxEgressLocalFactError::ProcessOrThreadChanged);
            }
            let current_effective_user_id = rustix::process::geteuid().as_raw();
            if current_effective_user_id != self.effective_user_id {
                return Err(PmLinuxEgressLocalFactError::EffectiveUserChanged);
            }
            validate_effective_user_binding(
                current_effective_user_id,
                authorization.value().host.linux_euid,
            )?;

            let expected_namespace = NamespaceIdentity {
                device: self.network_namespace_device,
                inode: self.network_namespace_inode,
            };
            if namespace_file_identity(&self.held_network_namespace)? != expected_namespace
                || observe_thread_network_namespace_identity()? != expected_namespace
            {
                return Err(PmLinuxEgressLocalFactError::NamespaceChanged);
            }
            let current_interface = observe_exact_assigned_interface(
                &self.interface_name,
                self.interface_index,
                self.local_source_ip,
            )?;
            if current_interface.name.as_ref() != self.interface_name.as_ref()
                || current_interface.index != self.interface_index
                || current_interface.local_source_ip != self.local_source_ip
            {
                return Err(PmLinuxEgressLocalFactError::InterfaceChanged);
            }
            self.reviewed_profile.revalidate(
                Path::new(
                    &authorization
                        .value()
                        .host
                        .egress
                        .dedicated_tunnel_or_gateway_profile_reference,
                ),
                &authorization
                    .value()
                    .host
                    .egress
                    .dedicated_tunnel_or_gateway_profile_sha256,
            )?;

            // Close every mutable-source gap after the potentially blocking
            // full profile rehash. This final interface observation repeats
            // name/index, UP, non-loopback, and exact assigned authorized IP.
            // Namespace membership, the held namespace FD, PID, TID, and EUID
            // are then sampled again before either completion clock edge.
            let final_interface = observe_exact_assigned_interface(
                &authorization.value().host.egress.interface_name,
                authorization.value().host.egress.interface_index,
                authorization
                    .value()
                    .host
                    .egress
                    .local_source_ip
                    .parse::<IpAddr>()
                    .map_err(|_| PmLinuxEgressLocalFactError::AuthorizationBinding)?,
            )?;
            let final_namespace = observe_thread_network_namespace_identity()?;
            let final_held_namespace = namespace_file_identity(&self.held_network_namespace)?;
            let final_process_id = std::process::id();
            let final_thread_id = current_thread_id()?;
            let final_effective_user_id = rustix::process::geteuid().as_raw();
            let final_sample = validate_final_local_source_sample(
                ExpectedFinalLocalSources {
                    process_id: self.creating_process_id,
                    thread_id: self.source_thread_id,
                    effective_user_id: self.effective_user_id,
                    namespace: expected_namespace,
                    interface_name: &authorization.value().host.egress.interface_name,
                    interface_index: authorization.value().host.egress.interface_index,
                    local_source_ip: self.local_source_ip,
                },
                FinalLocalSourceSample {
                    process_id: final_process_id,
                    thread_id: final_thread_id,
                    effective_user_id: final_effective_user_id,
                    current_namespace: final_namespace,
                    held_namespace: final_held_namespace,
                    interface: final_interface,
                },
            )?;
            validate_effective_user_binding(
                final_sample.effective_user_id,
                authorization.value().host.linux_euid,
            )?;
            let wall_completed = SystemTime::now();
            let monotonic_completed = Instant::now();
            validate_elapsed_window(
                recheck_wall_started,
                recheck_monotonic_started,
                wall_completed,
                monotonic_completed,
                MAX_LOCAL_FACT_CAPTURE_DURATION,
            )?;
            validate_elapsed_window(
                window_wall_started,
                window_monotonic_started,
                wall_completed,
                monotonic_completed,
                maximum_age,
            )?;
            Ok(PmLinuxEgressLocalFactsView {
                _custody: self,
                effective_user_id: final_sample.effective_user_id,
                network_namespace_device: final_sample.current_namespace.device,
                network_namespace_inode: final_sample.current_namespace.inode,
                interface_name: final_sample.interface.name,
                interface_index: final_sample.interface.index,
                local_source_ip: final_sample.interface.local_source_ip,
                wall_completed,
                monotonic_completed,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (
                authorization,
                window_wall_started,
                window_wall_completed,
                window_monotonic_started,
                window_monotonic_completed,
                maximum_age,
            );
            Err(PmLinuxEgressLocalFactError::UnsupportedPlatform)
        }
    }

    fn validate_authorization_binding(
        &self,
        authorization: &CanonicalOnlineAuthorizationV2,
    ) -> Result<(), PmLinuxEgressLocalFactError> {
        let egress = &authorization.value().host.egress;
        let authorized_local_ip = egress
            .local_source_ip
            .parse::<IpAddr>()
            .map_err(|_| PmLinuxEgressLocalFactError::AuthorizationBinding)?;
        if self.online_authorization_fingerprint.as_ref() != authorization.fingerprint()
            || self.effective_user_id != authorization.value().host.linux_euid
            || self.network_namespace_device != egress.network_namespace_device
            || self.network_namespace_inode != egress.network_namespace_inode
            || self.interface_name.as_ref() != egress.interface_name
            || self.interface_index != egress.interface_index
            || self.local_source_ip != authorized_local_ip
            || lower_hex(&self.reviewed_profile.sha256)
                != egress.dedicated_tunnel_or_gateway_profile_sha256
        {
            return Err(PmLinuxEgressLocalFactError::AuthorizationBinding);
        }
        Ok(())
    }
}

impl fmt::Debug for PmLinuxEgressLocalFactCustody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmLinuxEgressLocalFactCustody(<move-only; held-local-facts; non-authoritative>)",
        )
    }
}

struct HeldReviewedEgressProfile {
    file: File,
    identity: ProfileFileIdentity,
    sha256: [u8; 32],
    length: u64,
}

impl HeldReviewedEgressProfile {
    #[cfg(target_os = "linux")]
    fn open(path: &Path, expected_sha256: &str) -> Result<Self, PmLinuxEgressLocalFactError> {
        let mut file = open_profile_descriptor(path)?;
        let before = profile_file_identity(&file)?;
        validate_profile_file_identity(before)?;
        let (sha256, length) = hash_profile_descriptor(&mut file)?;
        let after = profile_file_identity(&file)?;
        if before != after || length != after.length {
            return Err(PmLinuxEgressLocalFactError::ProfileChanged);
        }
        if lower_hex(&sha256) != expected_sha256 {
            return Err(PmLinuxEgressLocalFactError::ProfileHashMismatch);
        }

        // Reopen through the exact reviewed absolute path after hashing. The
        // held descriptor remains the only object a future actor may recheck;
        // this second identity check only closes a path swap during capture.
        // Its final identity check and rehash are deliberately absent here and
        // deferred to that selected actor's immediate connection boundary.
        let reopened = open_profile_descriptor(path)?;
        let reopened_identity = profile_file_identity(&reopened)?;
        validate_profile_file_identity(reopened_identity)?;
        if reopened_identity != after {
            return Err(PmLinuxEgressLocalFactError::ProfileChanged);
        }
        Ok(Self {
            file,
            identity: after,
            sha256,
            length,
        })
    }

    #[cfg(target_os = "linux")]
    fn revalidate(
        &mut self,
        reviewed_path: &Path,
        expected_sha256: &str,
    ) -> Result<(), PmLinuxEgressLocalFactError> {
        validate_exact_reviewed_profile_path(
            reviewed_path,
            reviewed_path
                .to_str()
                .ok_or(PmLinuxEgressLocalFactError::ProfilePathMismatch)?,
        )?;
        let before = profile_file_identity(&self.file)?;
        validate_profile_file_identity(before)?;
        if before != self.identity || before.length != self.length {
            return Err(PmLinuxEgressLocalFactError::ProfileChanged);
        }
        let (sha256, length) = hash_profile_descriptor(&mut self.file)?;
        let after = profile_file_identity(&self.file)?;
        if after != before
            || length != self.length
            || sha256 != self.sha256
            || lower_hex(&sha256) != expected_sha256
        {
            return Err(PmLinuxEgressLocalFactError::ProfileChanged);
        }
        let reopened = open_profile_descriptor(reviewed_path)?;
        let reopened_identity = profile_file_identity(&reopened)?;
        validate_profile_file_identity(reopened_identity)?;
        if reopened_identity != self.identity {
            return Err(PmLinuxEgressLocalFactError::ProfileChanged);
        }
        Ok(())
    }
}

impl fmt::Debug for HeldReviewedEgressProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HeldReviewedEgressProfile(<non-secret; descriptor-pinned; hashed>)")
    }
}

/// Lifetime-borrowed facts available only while the revalidated descriptor
/// custody remains owned on its creating thread. This is not an egress permit.
pub(super) struct PmLinuxEgressLocalFactsView<'a> {
    _custody: &'a PmLinuxEgressLocalFactCustody,
    effective_user_id: u32,
    network_namespace_device: u64,
    network_namespace_inode: u64,
    interface_name: Box<str>,
    interface_index: u32,
    local_source_ip: IpAddr,
    wall_completed: SystemTime,
    monotonic_completed: Instant,
}

impl PmLinuxEgressLocalFactsView<'_> {
    #[must_use]
    pub(super) const fn network_namespace_device(&self) -> u64 {
        self.network_namespace_device
    }

    #[must_use]
    pub(super) const fn network_namespace_inode(&self) -> u64 {
        self.network_namespace_inode
    }

    #[must_use]
    pub(super) fn interface_name(&self) -> &str {
        &self.interface_name
    }

    #[must_use]
    pub(super) const fn interface_index(&self) -> u32 {
        self.interface_index
    }

    #[must_use]
    pub(super) const fn local_source_ip(&self) -> IpAddr {
        self.local_source_ip
    }

    #[must_use]
    pub(super) const fn effective_user_id(&self) -> u32 {
        self.effective_user_id
    }

    #[must_use]
    pub(super) const fn wall_completed(&self) -> SystemTime {
        self.wall_completed
    }

    #[must_use]
    pub(super) const fn monotonic_completed(&self) -> Instant {
        self.monotonic_completed
    }
}

impl fmt::Debug for PmLinuxEgressLocalFactsView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmLinuxEgressLocalFactsView(<borrowed; non-authoritative>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NamespaceIdentity {
    device: u64,
    inode: u64,
}

#[derive(PartialEq, Eq)]
struct InterfaceObservation {
    name: Box<str>,
    index: u32,
    local_source_ip: IpAddr,
}

struct ExpectedFinalLocalSources<'a> {
    process_id: u32,
    thread_id: u32,
    effective_user_id: u32,
    namespace: NamespaceIdentity,
    interface_name: &'a str,
    interface_index: u32,
    local_source_ip: IpAddr,
}

struct FinalLocalSourceSample {
    process_id: u32,
    thread_id: u32,
    effective_user_id: u32,
    current_namespace: NamespaceIdentity,
    held_namespace: NamespaceIdentity,
    interface: InterfaceObservation,
}

fn validate_final_local_source_sample(
    expected: ExpectedFinalLocalSources<'_>,
    sample: FinalLocalSourceSample,
) -> Result<FinalLocalSourceSample, PmLinuxEgressLocalFactError> {
    if sample.process_id != expected.process_id
        || sample.thread_id != expected.thread_id
        || sample.effective_user_id != expected.effective_user_id
        || sample.current_namespace != expected.namespace
        || sample.held_namespace != expected.namespace
        || sample.interface.name.as_ref() != expected.interface_name
        || sample.interface.index != expected.interface_index
        || sample.interface.local_source_ip != expected.local_source_ip
    {
        return Err(PmLinuxEgressLocalFactError::SourceChangedDuringRecheck);
    }
    Ok(sample)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProfileFileIdentity {
    regular_file: bool,
    device: u64,
    inode: u64,
    mode: u32,
    owner_user_id: u32,
    link_count: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

fn validate_exact_reviewed_profile_path(
    path: &Path,
    reviewed_reference: &str,
) -> Result<(), PmLinuxEgressLocalFactError> {
    let Some(path_text) = path.to_str() else {
        return Err(PmLinuxEgressLocalFactError::ProfilePathMismatch);
    };
    if !path.is_absolute() || path_text != reviewed_reference {
        return Err(PmLinuxEgressLocalFactError::ProfilePathMismatch);
    }

    // `Path::components` normalizes redundant separators and `.` components.
    // Rebuilding only root-plus-normal components and requiring exact equality
    // therefore rejects every lexically noncanonical spelling without resolving
    // the path or consulting a mutable filesystem namespace.
    let mut components = path.components();
    if components.next() != Some(Component::RootDir) {
        return Err(PmLinuxEgressLocalFactError::ProfilePathMismatch);
    }
    let mut normalized = String::new();
    for component in components {
        let Component::Normal(segment) = component else {
            return Err(PmLinuxEgressLocalFactError::ProfilePathMismatch);
        };
        let Some(segment) = segment.to_str() else {
            return Err(PmLinuxEgressLocalFactError::ProfilePathMismatch);
        };
        normalized.push('/');
        normalized.push_str(segment);
    }
    if normalized != path_text {
        return Err(PmLinuxEgressLocalFactError::ProfilePathMismatch);
    }
    Ok(())
}

fn validate_effective_user_binding(
    observed: u32,
    authorized: u32,
) -> Result<(), PmLinuxEgressLocalFactError> {
    if observed == 0 || observed == u32::MAX || observed != authorized {
        return Err(PmLinuxEgressLocalFactError::EffectiveUserAuthorizationMismatch);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn current_thread_id() -> Result<u32, PmLinuxEgressLocalFactError> {
    u32::try_from(rustix::thread::gettid().as_raw_pid())
        .map_err(|_| PmLinuxEgressLocalFactError::ProcessOrThreadChanged)
}

#[cfg(target_os = "linux")]
fn open_proc_directory() -> Result<rustix::fd::OwnedFd, PmLinuxEgressLocalFactError> {
    let proc = rustix::fs::open(
        PROC_ROOT_PATH,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| PmLinuxEgressLocalFactError::ProcfsUnavailable)?;
    let filesystem =
        rustix::fs::fstatfs(&proc).map_err(|_| PmLinuxEgressLocalFactError::ProcfsUnavailable)?;
    if filesystem.f_type != rustix::fs::PROC_SUPER_MAGIC {
        return Err(PmLinuxEgressLocalFactError::ProcfsUnavailable);
    }
    Ok(proc)
}

#[cfg(target_os = "linux")]
fn open_thread_network_namespace() -> Result<(File, NamespaceIdentity), PmLinuxEgressLocalFactError>
{
    let proc = open_proc_directory()?;
    // `thread-self/ns/net` is intentionally a procfs magic link. Following
    // this fixed terminal component is how Linux supplies a namespace FD.
    let descriptor = rustix::fs::openat(
        &proc,
        PROC_THREAD_NET_NAMESPACE_ENTRY,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| PmLinuxEgressLocalFactError::NamespaceUnavailable)?;
    let filesystem = rustix::fs::fstatfs(&descriptor)
        .map_err(|_| PmLinuxEgressLocalFactError::NamespaceUnavailable)?;
    if filesystem.f_type != NSFS_MAGIC {
        return Err(PmLinuxEgressLocalFactError::NamespaceInvalid);
    }
    let file = File::from(descriptor);
    let identity = namespace_file_identity(&file)?;
    if identity.device == 0 || identity.inode == 0 {
        return Err(PmLinuxEgressLocalFactError::NamespaceInvalid);
    }
    Ok((file, identity))
}

#[cfg(target_os = "linux")]
fn observe_thread_network_namespace_identity()
-> Result<NamespaceIdentity, PmLinuxEgressLocalFactError> {
    let (file, identity) = open_thread_network_namespace()?;
    drop(file);
    Ok(identity)
}

fn namespace_file_identity(file: &File) -> Result<NamespaceIdentity, PmLinuxEgressLocalFactError> {
    let metadata = file
        .metadata()
        .map_err(|_| PmLinuxEgressLocalFactError::NamespaceUnavailable)?;
    Ok(NamespaceIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn validate_namespace_binding(
    observed: NamespaceIdentity,
    expected_device: u64,
    expected_inode: u64,
) -> Result<(), PmLinuxEgressLocalFactError> {
    if observed.device != expected_device || observed.inode != expected_inode {
        return Err(PmLinuxEgressLocalFactError::NamespaceAuthorizationMismatch);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn observe_exact_assigned_interface(
    expected_name: &str,
    expected_index: u32,
    expected_local_source_ip: IpAddr,
) -> Result<InterfaceObservation, PmLinuxEgressLocalFactError> {
    let ioctl = rustix::net::socket_with(
        AddressFamily::INET,
        SocketType::DGRAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .map_err(|_| PmLinuxEgressLocalFactError::InterfaceUnavailable)?;
    let observed_index = rustix::net::netdevice::name_to_index(&ioctl, expected_name)
        .map_err(|_| PmLinuxEgressLocalFactError::InterfaceUnavailable)?;
    let observed_name = rustix::net::netdevice::index_to_name(&ioctl, expected_index)
        .map_err(|_| PmLinuxEgressLocalFactError::InterfaceUnavailable)?;
    if observed_index != expected_index || observed_name != expected_name {
        return Err(PmLinuxEgressLocalFactError::InterfaceAuthorizationMismatch);
    }

    let mut saw_interface = false;
    let mut matching_addresses = 0_u8;
    for interface in getifaddrs().map_err(|_| PmLinuxEgressLocalFactError::InterfaceUnavailable)? {
        if interface.interface_name != expected_name {
            continue;
        }
        saw_interface = true;
        if !interface.flags.contains(InterfaceFlags::IFF_UP)
            || interface.flags.contains(InterfaceFlags::IFF_LOOPBACK)
        {
            return Err(PmLinuxEgressLocalFactError::InterfaceNotEligible);
        }
        let Some(address) = interface.address else {
            continue;
        };
        let observed_ip = address
            .as_sockaddr_in()
            .map(|address| IpAddr::V4(address.ip()))
            .or_else(|| {
                address
                    .as_sockaddr_in6()
                    .map(|address| IpAddr::V6(address.ip()))
            });
        if observed_ip == Some(expected_local_source_ip) {
            matching_addresses = matching_addresses
                .checked_add(1)
                .ok_or(PmLinuxEgressLocalFactError::LocalSourceAmbiguous)?;
        }
    }
    if !saw_interface {
        return Err(PmLinuxEgressLocalFactError::InterfaceUnavailable);
    }
    if matching_addresses != 1 {
        return Err(if matching_addresses == 0 {
            PmLinuxEgressLocalFactError::LocalSourceNotAssigned
        } else {
            PmLinuxEgressLocalFactError::LocalSourceAmbiguous
        });
    }
    Ok(InterfaceObservation {
        name: observed_name.into(),
        index: observed_index,
        local_source_ip: expected_local_source_ip,
    })
}

#[cfg(target_os = "linux")]
fn open_profile_descriptor(path: &Path) -> Result<File, PmLinuxEgressLocalFactError> {
    let descriptor = rustix::fs::openat2(
        rustix::fs::CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| PmLinuxEgressLocalFactError::ProfileUnavailable)?;
    Ok(File::from(descriptor))
}

fn profile_file_identity(file: &File) -> Result<ProfileFileIdentity, PmLinuxEgressLocalFactError> {
    let metadata = file
        .metadata()
        .map_err(|_| PmLinuxEgressLocalFactError::ProfileUnavailable)?;
    Ok(ProfileFileIdentity {
        regular_file: metadata.file_type().is_file(),
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode() & 0o7777,
        owner_user_id: metadata.uid(),
        link_count: metadata.nlink(),
        length: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

fn validate_profile_file_identity(
    identity: ProfileFileIdentity,
) -> Result<(), PmLinuxEgressLocalFactError> {
    #[cfg(target_os = "linux")]
    let effective_user_id = rustix::process::geteuid().as_raw();
    #[cfg(not(target_os = "linux"))]
    let effective_user_id = u32::MAX;
    if !identity.regular_file
        || identity.device == 0
        || identity.inode == 0
        || identity.owner_user_id != effective_user_id
        || identity.mode != 0o600
        || identity.link_count != 1
        || identity.length == 0
        || identity.length > MAX_REVIEWED_PROFILE_BYTES as u64
    {
        return Err(PmLinuxEgressLocalFactError::ProfileInvalid);
    }
    Ok(())
}

fn hash_profile_descriptor(
    file: &mut File,
) -> Result<([u8; 32], u64), PmLinuxEgressLocalFactError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| PmLinuxEgressLocalFactError::ProfileUnavailable)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 8 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| PmLinuxEgressLocalFactError::ProfileUnavailable)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(
                u64::try_from(read).map_err(|_| PmLinuxEgressLocalFactError::ProfileInvalid)?,
            )
            .ok_or(PmLinuxEgressLocalFactError::ProfileInvalid)?;
        if total > MAX_REVIEWED_PROFILE_BYTES as u64 {
            return Err(PmLinuxEgressLocalFactError::ProfileInvalid);
        }
        digest.update(&buffer[..read]);
    }
    if total == 0 {
        return Err(PmLinuxEgressLocalFactError::ProfileInvalid);
    }
    Ok((digest.finalize().into(), total))
}

fn validate_capture_window(
    wall_started: SystemTime,
    wall_completed: SystemTime,
    monotonic_started: Instant,
    monotonic_completed: Instant,
) -> Result<(), PmLinuxEgressLocalFactError> {
    let wall_elapsed = wall_completed
        .duration_since(wall_started)
        .map_err(|_| PmLinuxEgressLocalFactError::ClockRegression)?;
    let monotonic_elapsed = monotonic_completed
        .checked_duration_since(monotonic_started)
        .ok_or(PmLinuxEgressLocalFactError::ClockRegression)?;
    if wall_elapsed > MAX_LOCAL_FACT_CAPTURE_DURATION
        || monotonic_elapsed > MAX_LOCAL_FACT_CAPTURE_DURATION
    {
        return Err(PmLinuxEgressLocalFactError::CaptureExpired);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_nested_window(
    outer_wall_started: SystemTime,
    outer_wall_completed: SystemTime,
    outer_monotonic_started: Instant,
    outer_monotonic_completed: Instant,
    inner_wall_started: SystemTime,
    inner_wall_completed: SystemTime,
    inner_monotonic_started: Instant,
    inner_monotonic_completed: Instant,
    maximum_age: Duration,
) -> Result<(), PmLinuxEgressLocalFactError> {
    validate_elapsed_window(
        outer_wall_started,
        outer_monotonic_started,
        outer_wall_completed,
        outer_monotonic_completed,
        maximum_age,
    )?;
    inner_wall_started
        .duration_since(outer_wall_started)
        .map_err(|_| PmLinuxEgressLocalFactError::CaptureOutsidePreflightWindow)?;
    outer_wall_completed
        .duration_since(inner_wall_completed)
        .map_err(|_| PmLinuxEgressLocalFactError::CaptureOutsidePreflightWindow)?;
    inner_monotonic_started
        .checked_duration_since(outer_monotonic_started)
        .ok_or(PmLinuxEgressLocalFactError::CaptureOutsidePreflightWindow)?;
    outer_monotonic_completed
        .checked_duration_since(inner_monotonic_completed)
        .ok_or(PmLinuxEgressLocalFactError::CaptureOutsidePreflightWindow)?;
    validate_capture_window(
        inner_wall_started,
        inner_wall_completed,
        inner_monotonic_started,
        inner_monotonic_completed,
    )
}

fn validate_elapsed_window(
    wall_started: SystemTime,
    monotonic_started: Instant,
    wall_completed: SystemTime,
    monotonic_completed: Instant,
    maximum_age: Duration,
) -> Result<(), PmLinuxEgressLocalFactError> {
    if maximum_age.is_zero() {
        return Err(PmLinuxEgressLocalFactError::CaptureExpired);
    }
    let wall_elapsed = wall_completed
        .duration_since(wall_started)
        .map_err(|_| PmLinuxEgressLocalFactError::ClockRegression)?;
    let monotonic_elapsed = monotonic_completed
        .checked_duration_since(monotonic_started)
        .ok_or(PmLinuxEgressLocalFactError::ClockRegression)?;
    if wall_elapsed > maximum_age || monotonic_elapsed > maximum_age {
        return Err(PmLinuxEgressLocalFactError::CaptureExpired);
    }
    Ok(())
}

fn lower_hex(value: &[u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in value {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[derive(Debug, Error)]
pub(super) enum PmLinuxEgressLocalFactError {
    #[error("local Linux egress observation is supported only on Linux")]
    UnsupportedPlatform,
    #[error(
        "the reviewed non-secret egress profile path is not the exact authorized absolute path"
    )]
    ProfilePathMismatch,
    #[error("the canonical online authorization egress binding is invalid")]
    AuthorizationBinding,
    #[error("the observing process or Linux thread changed during capture")]
    ProcessOrThreadChanged,
    #[error("a local Linux egress source changed during consuming recheck")]
    SourceChangedDuringRecheck,
    #[error("the numeric effective Linux user differs from authorization")]
    EffectiveUserAuthorizationMismatch,
    #[error("the numeric effective Linux user changed during capture")]
    EffectiveUserChanged,
    #[error("trusted procfs is unavailable for local egress observation")]
    ProcfsUnavailable,
    #[error("the calling thread network namespace is unavailable")]
    NamespaceUnavailable,
    #[error("the calling thread network namespace descriptor is invalid")]
    NamespaceInvalid,
    #[error("the calling thread network namespace differs from authorization")]
    NamespaceAuthorizationMismatch,
    #[error("the calling thread network namespace changed during capture")]
    NamespaceChanged,
    #[error("the authorized Linux interface is unavailable")]
    InterfaceUnavailable,
    #[error("the Linux interface name or index differs from authorization")]
    InterfaceAuthorizationMismatch,
    #[error("the authorized Linux interface is down or loopback")]
    InterfaceNotEligible,
    #[error("the authorized local source IP is not assigned to the exact interface")]
    LocalSourceNotAssigned,
    #[error("the authorized local source IP assignment is ambiguous")]
    LocalSourceAmbiguous,
    #[error("the authorized interface observation changed during capture")]
    InterfaceChanged,
    #[error("the reviewed non-secret egress profile artifact is unavailable")]
    ProfileUnavailable,
    #[error("the reviewed non-secret egress profile artifact metadata is invalid")]
    ProfileInvalid,
    #[error("the reviewed non-secret egress profile artifact changed during capture")]
    ProfileChanged,
    #[error("the reviewed non-secret egress profile artifact hash differs from authorization")]
    ProfileHashMismatch,
    #[error("the local egress source clock regressed")]
    ClockRegression,
    #[error("the local egress source capture exceeded its fixed time bound")]
    CaptureExpired,
    #[error("the local egress capture falls outside the exact outer preflight window")]
    CaptureOutsidePreflightWindow,
}

#[cfg(test)]
mod tests {
    use std::{
        fs::OpenOptions,
        io::Write as _,
        os::unix::fs::OpenOptionsExt as _,
        time::{Duration, Instant, SystemTime},
    };

    use super::*;

    #[test]
    fn reviewed_profile_path_must_be_exact_absolute_utf8() {
        assert!(
            validate_exact_reviewed_profile_path(
                Path::new("/run/reap/reviewed-egress.json"),
                "/run/reap/reviewed-egress.json",
            )
            .is_ok()
        );
        assert!(matches!(
            validate_exact_reviewed_profile_path(
                Path::new("/run/reap/other.json"),
                "/run/reap/reviewed-egress.json",
            ),
            Err(PmLinuxEgressLocalFactError::ProfilePathMismatch),
        ));
        for noncanonical in [
            "run/reap/reviewed-egress.json",
            "/run/reap/./reviewed-egress.json",
            "/run/reap/../reap/reviewed-egress.json",
            "/run//reap/reviewed-egress.json",
            "/run/reap/reviewed-egress.json/",
            "/",
        ] {
            assert!(matches!(
                validate_exact_reviewed_profile_path(Path::new(noncanonical), noncanonical,),
                Err(PmLinuxEgressLocalFactError::ProfilePathMismatch),
            ));
        }
    }

    #[test]
    fn numeric_effective_user_must_match_authorization() {
        assert!(validate_effective_user_binding(1_000, 1_000).is_ok());
        for (observed, authorized) in [(0, 0), (u32::MAX, u32::MAX), (1_000, 1_001)] {
            assert!(matches!(
                validate_effective_user_binding(observed, authorized),
                Err(PmLinuxEgressLocalFactError::EffectiveUserAuthorizationMismatch),
            ));
        }
    }

    #[test]
    fn consuming_recheck_rejects_post_rehash_interface_or_ip_drift() {
        fn expected() -> ExpectedFinalLocalSources<'static> {
            ExpectedFinalLocalSources {
                process_id: 101,
                thread_id: 202,
                effective_user_id: 1_000,
                namespace: NamespaceIdentity {
                    device: 303,
                    inode: 404,
                },
                interface_name: "pm-tunnel0",
                interface_index: 505,
                local_source_ip: "192.0.2.10".parse().unwrap(),
            }
        }

        fn sample(
            interface_name: &str,
            interface_index: u32,
            local_source_ip: &str,
        ) -> FinalLocalSourceSample {
            FinalLocalSourceSample {
                process_id: 101,
                thread_id: 202,
                effective_user_id: 1_000,
                current_namespace: NamespaceIdentity {
                    device: 303,
                    inode: 404,
                },
                held_namespace: NamespaceIdentity {
                    device: 303,
                    inode: 404,
                },
                interface: InterfaceObservation {
                    name: interface_name.into(),
                    index: interface_index,
                    local_source_ip: local_source_ip.parse().unwrap(),
                },
            }
        }

        let final_sample =
            validate_final_local_source_sample(expected(), sample("pm-tunnel0", 505, "192.0.2.10"))
                .unwrap();
        assert_eq!(final_sample.interface.name.as_ref(), "pm-tunnel0");
        assert_eq!(final_sample.interface.index, 505);
        assert_eq!(
            final_sample.interface.local_source_ip,
            "192.0.2.10".parse::<IpAddr>().unwrap(),
        );
        for drifted in [
            sample("pm-tunnel1", 505, "192.0.2.10"),
            sample("pm-tunnel0", 506, "192.0.2.10"),
            sample("pm-tunnel0", 505, "192.0.2.11"),
        ] {
            assert!(matches!(
                validate_final_local_source_sample(expected(), drifted),
                Err(PmLinuxEgressLocalFactError::SourceChangedDuringRecheck),
            ));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn current_thread_namespace_source_is_descriptor_pinned_and_stable() {
        let process = std::process::id();
        let thread = current_thread_id().unwrap();
        let (held, identity) = open_thread_network_namespace().unwrap();
        assert_ne!(identity.device, 0);
        assert_ne!(identity.inode, 0);
        assert_eq!(namespace_file_identity(&held).unwrap(), identity);
        assert_eq!(
            observe_thread_network_namespace_identity().unwrap(),
            identity
        );
        assert_eq!(std::process::id(), process);
        assert_eq!(current_thread_id().unwrap(), thread);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn profile_source_holds_exact_nonsecret_bytes_and_rejects_wrong_hash() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reviewed-egress-profile.json");
        let bytes = b"{\"profile\":\"reviewed-non-secret\"}";
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let expected: [u8; 32] = Sha256::digest(bytes).into();
        let held = HeldReviewedEgressProfile::open(&path, &lower_hex(&expected)).unwrap();
        assert_eq!(held.length, bytes.len() as u64);
        assert_eq!(held.sha256, expected);
        assert_eq!(profile_file_identity(&held.file).unwrap(), held.identity);
        assert!(matches!(
            HeldReviewedEgressProfile::open(&path, &"00".repeat(32)),
            Err(PmLinuxEgressLocalFactError::ProfileHashMismatch),
        ));
    }

    #[test]
    fn capture_clock_window_rejects_regression_and_expiry() {
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let monotonic = Instant::now();
        assert!(validate_capture_window(wall, wall, monotonic, monotonic).is_ok());
        assert!(matches!(
            validate_capture_window(wall, wall - Duration::from_nanos(1), monotonic, monotonic,),
            Err(PmLinuxEgressLocalFactError::ClockRegression),
        ));
        assert!(matches!(
            validate_capture_window(
                wall,
                wall + MAX_LOCAL_FACT_CAPTURE_DURATION + Duration::from_nanos(1),
                monotonic,
                monotonic + MAX_LOCAL_FACT_CAPTURE_DURATION + Duration::from_nanos(1),
            ),
            Err(PmLinuxEgressLocalFactError::CaptureExpired),
        ));
    }

    #[test]
    fn debug_output_is_redacted_and_non_authoritative() {
        let profile = HeldReviewedEgressProfile {
            file: tempfile::tempfile().unwrap(),
            identity: ProfileFileIdentity {
                regular_file: true,
                device: 1,
                inode: 2,
                mode: 0o600,
                owner_user_id: 1_000,
                link_count: 1,
                length: 10,
                modified_seconds: 1,
                modified_nanoseconds: 2,
                changed_seconds: 3,
                changed_nanoseconds: 4,
            },
            sha256: [7; 32],
            length: 10,
        };
        assert_eq!(
            format!("{profile:?}"),
            "HeldReviewedEgressProfile(<non-secret; descriptor-pinned; hashed>)",
        );
    }
}
