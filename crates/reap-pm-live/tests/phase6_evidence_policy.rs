use std::path::{Path, PathBuf};

#[test]
fn sealed_journal_backend_is_confined_to_the_fixed_evidence_path() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let library = read(crate_root.join("src/lib.rs"));
    let evidence = read(crate_root.join("src/evidence.rs"));
    let journal = read(crate_root.join("src/journal.rs"));
    let composition = module_source(&crate_root.join("src"), "composition");

    assert!(!library.contains("PmJournalRuntime"));
    assert!(!library.contains("PmSealedJournalLedger"));
    assert!(!library.contains("PmSealedJournalProjection"));
    assert!(!composition.contains("start_sealed_evidence"));
    assert!(journal.contains("enum PmJournalRuntime"));
    assert!(!journal.contains("pub enum PmJournalRuntime"));
    assert!(journal.contains("pub(crate) fn start_sealed_evidence"));
    assert!(!journal.contains("pub fn start_sealed_evidence"));

    assert!(library.contains("run_pm_action_path_evidence"));
    let action_api = evidence
        .split("pub fn run_pm_action_path_evidence")
        .nth(1)
        .expect("fixed action evidence API remains exported")
        .split('{')
        .next()
        .expect("fixed action evidence signature is complete");
    assert!(
        action_api.contains("() -> Result<String, PmEvidenceError>"),
        "the fixed action evidence runner must remain zero-input"
    );
}

#[test]
fn tracking_allocator_installation_is_test_and_benchmark_only() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = crate_root
        .parent()
        .and_then(Path::parent)
        .expect("PM live crate is nested under the workspace");
    let pm_source = crate_root.join("src");

    for path in rust_sources(&pm_source) {
        let source = read(&path);
        if source.contains("reap_benchmark_allocator::") {
            let relative = path.strip_prefix(&pm_source).unwrap();
            assert!(
                relative.starts_with("evidence")
                    || relative.starts_with("lanes/phase6_overload_tests"),
                "benchmark allocator escaped the fixed evidence source tree: {}",
                path.display()
            );
        }
    }

    let mut installations = rust_sources(&workspace.join("crates"))
        .into_iter()
        .filter(|path| {
            let source = read(path);
            source
                .lines()
                .any(|line| line.trim() == "#[global_allocator]")
                && (source.contains("reap_benchmark_allocator::TrackingAllocator")
                    || source.contains("use reap_benchmark_allocator::TrackingAllocator"))
        })
        .map(|path| {
            path.strip_prefix(workspace)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    installations.sort();
    assert_eq!(
        installations,
        [
            "crates/reap-pm-live/benches/pm_action_path.rs",
            "crates/reap-pm-live/src/evidence/overload_tests/mod.rs",
            "crates/reap-pm-live/tests/combined_replay.rs",
        ]
    );

    let evidence_module = read(pm_source.join("evidence.rs"));
    assert!(evidence_module.contains("#[cfg(test)]\nmod overload_tests;"));
}

fn module_source(root: &Path, module: &str) -> String {
    let mut paths = vec![root.join(format!("{module}.rs"))];
    paths.extend(rust_sources(&root.join(module)));
    paths.into_iter().map(read).collect::<Vec<_>>().join("\n")
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("source directory entry is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                sources.push(path);
            }
        }
    }
    sources.sort();
    sources
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path.as_ref())
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.as_ref().display()))
}
