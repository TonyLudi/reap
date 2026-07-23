use std::path::Path;

const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const BOUNDED_WRITER: &str = include_str!("../src/bounded_writer.rs");
const FRAME: &str = include_str!("../src/frame.rs");
const HASH: &str = include_str!("../src/hash.rs");
const VERIFY: &str = include_str!("../src/verify.rs");

#[test]
fn production_surface_is_schema_and_venue_neutral() {
    let production = [LIB, BOUNDED_WRITER, FRAME, HASH, VERIFY].join("\n");
    for forbidden in [
        "RawCapture",
        "NormalizedEvent",
        "StorageRecord",
        "Polymarket",
        "Okx",
        "VenueAdapter",
        "reap_core",
        "reap_feed",
        "reap_pm",
        "reap_storage",
        "reap_venue",
    ] {
        assert!(
            !production.contains(forbidden),
            "neutral framing source contains forbidden product term {forbidden}"
        );
    }
    for forbidden_dependency in [
        "reap-core",
        "reap-feed",
        "reap-pm",
        "reap-storage",
        "reap-venue",
    ] {
        assert!(
            !MANIFEST.contains(forbidden_dependency),
            "neutral framing manifest contains forbidden dependency {forbidden_dependency}"
        );
    }
}

#[test]
fn writer_has_no_unbounded_queue_or_dynamic_codec() {
    assert!(!BOUNDED_WRITER.contains("unbounded_channel"));
    assert!(!BOUNDED_WRITER.contains("dyn Serialize"));
    assert!(BOUNDED_WRITER.contains("mpsc::channel(capacity.max(1))"));
    assert!(BOUNDED_WRITER.contains("T: Serialize + Send + 'static"));
    assert!(!BOUNDED_WRITER.contains("QueueByteSized"));
    assert!(BOUNDED_WRITER.contains("encode_jsonl_frame_bounded(&value, self.max_frame_bytes)"));
    assert!(BOUNDED_WRITER.contains("let frame_bytes = frame.len()"));
    assert!(BOUNDED_WRITER.contains("acquire_many_owned(self.max_frame_bytes as u32)"));
    assert!(BOUNDED_WRITER.contains("byte_reservation.shrink_to(frame_bytes)"));
    assert!(BOUNDED_WRITER.contains("max_queue_bytes"));
    assert!(BOUNDED_WRITER.contains("max_reserved_bytes"));
    assert!(BOUNDED_WRITER.contains("FrameTooLarge"));
    assert!(BOUNDED_WRITER.contains("ByteBackpressure"));
    assert!(BOUNDED_WRITER.contains("LegacyEntryCountJsonlWriter"));
    assert!(!BOUNDED_WRITER.contains("pub type BoundedJsonlWriter"));

    let send_start = BOUNDED_WRITER
        .find("pub async fn send_with_timeout")
        .expect("bounded send exists");
    let evidence_start = BOUNDED_WRITER[send_start..]
        .find("pub fn queue_byte_evidence")
        .map(|offset| send_start + offset)
        .expect("bounded byte evidence exists");
    let bounded_send = &BOUNDED_WRITER[send_start..evidence_start];
    let entry = bounded_send
        .find("acquire_owned()")
        .expect("bounded send reserves an entry before work");
    let reserve = bounded_send
        .find("acquire_many_owned(self.max_frame_bytes as u32)")
        .expect("bounded send reserves a worst-case frame");
    let encode = bounded_send
        .find("encode_jsonl_frame_bounded(&value, self.max_frame_bytes)")
        .expect("bounded send performs bounded exact encoding");
    let length = bounded_send
        .find("let frame_bytes = frame.len()")
        .expect("bounded send charges exact encoded length");
    let shrink = bounded_send
        .find("byte_reservation.shrink_to(frame_bytes)")
        .expect("bounded send releases reservation surplus");
    assert!(entry < reserve && reserve < encode && encode < length && length < shrink);

    assert!(BOUNDED_WRITER.contains("QueuedPayload::SerializeOnWriter(value)"));
    assert!(BOUNDED_WRITER.contains("encode_jsonl_frame_legacy_unbounded(value)?"));

    let writer_loop = BOUNDED_WRITER
        .split_once("async fn run_jsonl_writer")
        .map(|(_, source)| source)
        .expect("writer loop exists");
    let write = writer_loop
        .find("writer.write_all(&frame).await?")
        .expect("writer writes complete frame");
    let hash = writer_loop
        .find("hasher.update(&frame)")
        .expect("writer hashes the retained frame");
    let drop_frame = writer_loop
        .find("drop(frame)")
        .expect("writer drops the encoded frame");
    let release = writer_loop
        .find("item.release_accounting()")
        .expect("writer releases queue accounting");
    let flush = writer_loop
        .find("writer.flush().await?")
        .expect("writer flush exists");
    assert!(write < hash && hash < drop_frame && drop_frame < release && release < flush);

    let queue_accounting = BOUNDED_WRITER
        .split_once("impl QueueAccounting")
        .and_then(|(_, source)| source.split_once("impl Drop for QueueAccounting"))
        .map(|(source, _)| source)
        .expect("queue accounting release exists");
    let depth_evidence = queue_accounting
        .find("self.queued.fetch_sub")
        .expect("queue depth evidence is decremented");
    let byte_evidence = queue_accounting
        .find("queued_bytes.fetch_sub")
        .expect("queue byte evidence is decremented");
    let byte_permit = queue_accounting
        .find("drop(self.byte_reservation.take())")
        .expect("queue byte permit is released");
    let entry_permit = queue_accounting
        .find("drop(self.entry_permit.take())")
        .expect("queue entry permit is released");
    assert!(
        depth_evidence < entry_permit && byte_evidence < byte_permit,
        "evidence must be decremented before a permit can wake another producer"
    );

    let tracked_reservation = BOUNDED_WRITER
        .split_once("impl TrackedByteReservation")
        .and_then(|(_, source)| source.split_once("impl Drop for TrackedByteReservation"))
        .map(|(source, _)| source)
        .expect("tracked byte reservation implementation exists");
    let shrink_evidence = tracked_reservation
        .find("self.reserved_bytes.fetch_sub(surplus")
        .expect("reservation shrink decrements evidence");
    let shrink_permit = tracked_reservation
        .find("drop(surplus_permit)")
        .expect("reservation shrink releases its permit");
    assert!(shrink_evidence < shrink_permit);

    let reservation_drop = BOUNDED_WRITER
        .split_once("impl Drop for TrackedByteReservation")
        .map(|(_, source)| source)
        .expect("tracked byte reservation drop exists");
    let drop_evidence = reservation_drop
        .find(".fetch_sub(self.charged_bytes")
        .expect("reservation drop decrements evidence");
    let drop_permit = reservation_drop
        .find("drop(self.permit.take())")
        .expect("reservation drop releases its permit");
    assert!(drop_evidence < drop_permit);

    assert!(FRAME.contains("CountingWriter"));
    assert!(FRAME.contains("FixedCapacityWriter"));
    assert!(FRAME.contains("Vec::with_capacity(expected_bytes)"));
}

#[test]
fn bounded_verifier_never_uses_the_unbounded_read_primitive() {
    let bounded_start = VERIFY
        .find("pub fn scan_jsonl_file_bounded")
        .expect("bounded verifier exists");
    let legacy_start = VERIFY
        .find("pub fn scan_jsonl_file_legacy_unbounded")
        .expect("explicit legacy verifier exists");
    let bounded_source = &VERIFY[bounded_start..legacy_start];

    assert!(bounded_source.contains("read_frame_bounded"));
    assert!(!bounded_source.contains("read_until"));
    assert!(VERIFY[legacy_start..].contains("read_until"));
    assert!(!VERIFY.contains("pub fn scan_jsonl_file("));

    let read_start = VERIFY
        .find("pub fn read_bounded_regular_file")
        .expect("bounded whole-file reader exists");
    let canonical_start = VERIFY[read_start..]
        .find("pub fn canonical_regular_file")
        .map(|offset| read_start + offset)
        .expect("canonical reader boundary exists");
    let bounded_read = &VERIFY[read_start..canonical_start];
    assert!(bounded_read.contains("open_verified_regular_file(path)?"));
    assert!(bounded_read.contains("file.take(read_limit)"));
    assert!(!bounded_read.contains("std::fs::read"));

    let secure_open = VERIFY
        .split_once("fn open_verified_regular_file")
        .and_then(|(_, source)| source.split_once("fn regular_path_metadata"))
        .map(|(source, _)| source)
        .expect("descriptor-bound regular-file open exists");
    assert!(secure_open.contains("open_read_only_no_follow(path)"));
    assert!(secure_open.contains("opened_metadata.is_file()"));
    assert!(secure_open.contains("same_file_identity(&target_before_open, &opened_metadata)"));
    assert!(secure_open.contains("same_file_identity(&target_after_open, &opened_metadata)"));
    assert!(VERIFY.contains("libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC"));
}

#[test]
fn legacy_escape_hatches_have_an_exact_workspace_allowlist() {
    assert!(MANIFEST.contains("legacy-reap-capture = []"));
    assert!(!MANIFEST.contains("default = [\"legacy-reap-capture\"]"));
    for declaration in [
        "pub struct LegacyEntryCountWriterConfig",
        "pub struct LegacyEntryCountJsonlWriter",
    ] {
        let declaration = BOUNDED_WRITER
            .find(declaration)
            .expect("legacy writer declaration exists");
        let prefix = &BOUNDED_WRITER[..declaration];
        assert!(
            prefix.ends_with("#[cfg(feature = \"legacy-reap-capture\")]\n"),
            "legacy writer declaration is not feature-gated"
        );
    }
    let legacy_scan = VERIFY
        .find("pub fn scan_jsonl_file_legacy_unbounded")
        .expect("legacy scan declaration exists");
    assert!(VERIFY[..legacy_scan].ends_with("#[cfg(feature = \"legacy-reap-capture\")]\n"));
    let legacy_encode = FRAME
        .find("pub fn encode_jsonl_frame_legacy_unbounded")
        .expect("legacy encoder declaration exists");
    assert!(FRAME[..legacy_encode].ends_with("#[cfg(feature = \"legacy-reap-capture\")]\n"));

    let workspace_crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace crates directory");
    let workspace_manifest = workspace_crates
        .parent()
        .expect("workspace root")
        .join("Cargo.toml");
    assert!(
        !std::fs::read_to_string(&workspace_manifest)
            .expect("read workspace manifest")
            .contains("legacy-reap-capture"),
        "the workspace root must not enable or propagate the legacy capture feature"
    );
    for entry in std::fs::read_dir(workspace_crates).expect("read workspace crates") {
        let entry = entry.expect("read crate entry");
        if entry.file_name() == "reap-capture-framing" {
            continue;
        }
        inspect_workspace_path(&entry.path(), workspace_crates);
    }
}

fn inspect_workspace_path(path: &Path, workspace_crates: &Path) {
    if path.is_dir() {
        if path.file_name().and_then(|name| name.to_str()) == Some("target") {
            return;
        }
        for entry in std::fs::read_dir(path).expect("read PM source directory") {
            inspect_workspace_path(
                &entry.expect("read workspace source entry").path(),
                workspace_crates,
            );
        }
        return;
    }
    let file_name = path.file_name().and_then(|name| name.to_str());
    if file_name == Some("Cargo.toml") {
        let manifest = std::fs::read_to_string(path).expect("read workspace manifest");
        if manifest.contains("legacy-reap-capture") {
            assert_eq!(
                path,
                workspace_crates.join("reap-capture").join("Cargo.toml"),
                "only reap-capture may enable the legacy framing feature"
            );
        }
        return;
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
        return;
    }
    let source = std::fs::read_to_string(path).expect("read workspace Rust source");
    for forbidden in [
        "LegacyEntryCountJsonlWriter",
        "LegacyEntryCountWriterConfig",
        "encode_jsonl_frame_legacy_unbounded",
        "scan_jsonl_file_legacy_unbounded",
    ] {
        if source.contains(forbidden) {
            let allowed_writer = workspace_crates
                .join("reap-capture")
                .join("src")
                .join("writer.rs");
            let allowed_verification = workspace_crates
                .join("reap-capture")
                .join("src")
                .join("verification.rs");
            assert!(
                path == allowed_writer || path == allowed_verification,
                "workspace source {} uses legacy capture symbol {forbidden} outside the exact compatibility facade allowlist",
                path.display()
            );
        }
    }
}
