#[test]
fn economic_unit_tests_use_the_exact_terminal_external_marker() {
    const EOF_MARKER: &str =
        "#[cfg(test)]\n#[path = \"../tests/economic_statement_unit/mod.rs\"]\nmod tests;\n";

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let facade = std::fs::read_to_string(manifest.join("src/economic_statement.rs"))
        .expect("economic statement facade source");
    assert!(
        facade.ends_with(EOF_MARKER),
        "economic statement facade must end with the exact external test marker"
    );
    assert_eq!(
        facade.matches(EOF_MARKER.trim_end()).count(),
        1,
        "economic statement facade must contain the external test marker exactly once"
    );
    assert!(
        !facade.contains("#[path = \"economic_statement/tests/mod.rs\"]"),
        "economic statement facade must not retain the old src test marker"
    );

    let old_test_tree = manifest.join("src/economic_statement/tests");
    assert!(
        !old_test_tree.join("mod.rs").exists(),
        "the old src economic test module must not exist"
    );
    if old_test_tree.exists() {
        let remaining_sources = std::fs::read_dir(&old_test_tree)
            .expect("old economic test directory must be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
            .collect::<Vec<_>>();
        assert!(
            remaining_sources.is_empty(),
            "the old src economic test tree must contain no Rust sources: {remaining_sources:?}"
        );
    }
}
