use super::super::*;

#[test]
fn regular_file_resolution_accepts_a_file_and_rejects_a_directory() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = directory.path().join("artifact.json");
    std::fs::write(&artifact, b"{}").unwrap();

    assert_eq!(
        resolve_regular_file(
            directory.path(),
            Path::new("artifact.json"),
            "test artifact"
        )
        .unwrap(),
        std::fs::canonicalize(&artifact).unwrap()
    );

    let nested = directory.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    assert_eq!(
        resolve_regular_file(directory.path(), Path::new("nested"), "test artifact")
            .unwrap_err()
            .to_string(),
        format!(
            "test artifact {} must be a regular file and not a symbolic link",
            nested.display()
        )
    );
}

#[cfg(unix)]
#[test]
fn regular_file_resolution_rejects_symbolic_links() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let artifact = directory.path().join("artifact.json");
    let link = directory.path().join("artifact-link.json");
    std::fs::write(&artifact, b"{}").unwrap();
    symlink("artifact.json", &link).unwrap();

    assert_eq!(
        resolve_regular_file(
            directory.path(),
            Path::new("artifact-link.json"),
            "test artifact"
        )
        .unwrap_err()
        .to_string(),
        format!(
            "test artifact {} must be a regular file and not a symbolic link",
            link.display()
        )
    );
}

#[test]
fn unique_path_resolution_rejects_duplicate_canonical_paths() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = directory.path().join("artifact.json");
    std::fs::write(&artifact, b"{}").unwrap();
    let canonical = std::fs::canonicalize(&artifact).unwrap();

    assert_eq!(
        resolve_unique_paths(
            directory.path(),
            &[
                PathBuf::from("artifact.json"),
                PathBuf::from("./artifact.json")
            ],
            "latency source report",
        )
        .unwrap_err()
        .to_string(),
        format!(
            "duplicate latency source report path {}",
            canonical.display()
        )
    );
}
