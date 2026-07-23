use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn state_crate_has_only_the_deliberate_dependencies() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(manifest_dir.join("Cargo.toml")).unwrap();
    let dependency_lines = manifest
        .split_once("[dependencies]")
        .expect("dependencies section")
        .1
        .lines()
        .take_while(|line| !line.trim_start().starts_with('['))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    assert_eq!(
        dependency_lines,
        [
            "reap-pm-core.workspace = true",
            "thiserror.workspace = true"
        ]
    );
}

#[test]
fn reducer_source_remains_pure_single_owner_and_runtime_agnostic() {
    let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rust_files(&source_root, &mut files);
    assert!(!files.is_empty());

    let forbidden = [
        ("Tokio runtime", "tokio"),
        ("network I/O", "std::net"),
        ("filesystem I/O", "std::fs"),
        ("generic I/O", "std::io"),
        ("shared Arc ownership", "Arc<"),
        ("shared mutex mutation", "Mutex<"),
        ("shared rwlock mutation", "RwLock<"),
        ("unsafe block", "unsafe {"),
        ("unsafe function", "unsafe fn"),
        ("unsafe implementation", "unsafe impl"),
    ];
    for file in files {
        let source = fs::read_to_string(&file).unwrap();
        for (capability, pattern) in forbidden {
            assert!(
                !source.contains(pattern),
                "{capability} (`{pattern}`) is forbidden in {}",
                file.display()
            );
        }
    }
}

fn collect_rust_files(directory: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rust_files(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}
