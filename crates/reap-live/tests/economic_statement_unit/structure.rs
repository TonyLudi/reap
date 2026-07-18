use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const MODULES: [&str; 7] = [
    "artifacts",
    "cash_continuity",
    "position_basis",
    "trade_bills",
    "funding_bills",
    "report",
    "support",
];
const EOF_MARKER: &str =
    "#[cfg(test)]\n#[path = \"../tests/economic_statement_unit/mod.rs\"]\nmod tests;\n";

fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn mask(bytes: &[u8], output: &mut [u8], start: usize, end: usize) {
    for index in start..end {
        output[index] = if bytes[index] == b'\n' { b'\n' } else { b' ' };
    }
}

fn raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    if bytes.get(cursor) == Some(&b'b') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'r') {
        return None;
    }
    cursor += 1;
    let hashes_start = cursor;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    let hashes = cursor - hashes_start;
    cursor += 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"'
            && bytes.get(cursor + 1..cursor + 1 + hashes)
                == Some(&bytes[hashes_start..hashes_start + hashes])
        {
            return Some(cursor + 1 + hashes);
        }
        cursor += 1;
    }
    Some(bytes.len())
}

fn char_literal_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = start + 1;
    if *bytes.get(cursor)? == b'\\' {
        cursor += 2;
        if bytes.get(cursor - 1) == Some(&b'x') {
            cursor += 2;
        } else if bytes.get(cursor - 1) == Some(&b'u') {
            if bytes.get(cursor) != Some(&b'{') {
                return None;
            }
            while !matches!(bytes.get(cursor), Some(b'}') | None) {
                cursor += 1;
            }
            cursor += 1;
        }
    } else {
        cursor += source.get(cursor..)?.chars().next()?.len_utf8();
    }
    (bytes.get(cursor) == Some(&b'\'')).then_some(cursor + 1)
}

fn rust_code(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = bytes.to_vec();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes.get(cursor..cursor + 2) == Some(b"//") {
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            mask(bytes, &mut output, start, cursor);
        } else if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            let start = cursor;
            let mut depth = 1;
            cursor += 2;
            while cursor < bytes.len() && depth > 0 {
                if bytes.get(cursor..cursor + 2) == Some(b"/*") {
                    depth += 1;
                    cursor += 2;
                } else if bytes.get(cursor..cursor + 2) == Some(b"*/") {
                    depth -= 1;
                    cursor += 2;
                } else {
                    cursor += 1;
                }
            }
            mask(bytes, &mut output, start, cursor);
        } else if let Some(end) = raw_string_end(bytes, cursor) {
            mask(bytes, &mut output, cursor, end);
            cursor = end;
        } else if bytes[cursor] == b'"' {
            let start = cursor;
            cursor += 1;
            while cursor < bytes.len() {
                if bytes[cursor] == b'\\' {
                    cursor = (cursor + 2).min(bytes.len());
                } else if bytes[cursor] == b'"' {
                    cursor += 1;
                    break;
                } else {
                    cursor += 1;
                }
            }
            mask(bytes, &mut output, start, cursor);
        } else if bytes[cursor] == b'\'' {
            if let Some(end) = char_literal_end(source, cursor) {
                mask(bytes, &mut output, cursor, end);
                cursor = end;
            } else {
                cursor += 1;
            }
        } else {
            cursor += 1;
        }
    }
    String::from_utf8(output).expect("source masking preserves UTF-8")
}

#[derive(Debug)]
struct Token {
    text: String,
    offset: usize,
}

fn tokens(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_' {
            let start = cursor;
            cursor += 1;
            while cursor < bytes.len()
                && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
            {
                cursor += 1;
            }
            result.push(Token {
                text: source[start..cursor].to_string(),
                offset: start,
            });
        } else {
            if !bytes[cursor].is_ascii_whitespace() {
                result.push(Token {
                    text: (bytes[cursor] as char).to_string(),
                    offset: cursor,
                });
            }
            cursor += 1;
        }
    }
    result
}

fn function_lines(source: &str) -> Vec<(String, usize)> {
    let code = rust_code(source);
    let syntax = tokens(&code);
    let mut result = Vec::new();
    for (index, token) in syntax
        .iter()
        .enumerate()
        .filter(|(_, token)| token.text == "fn")
    {
        let name = &syntax[index + 1];
        let open = syntax[index + 2..]
            .iter()
            .position(|candidate| matches!(candidate.text.as_str(), "{" | ";"))
            .map(|relative| index + 2 + relative)
            .expect("function body marker");
        if syntax[open].text == ";" {
            continue;
        }
        let mut depth = 0;
        let close = syntax
            .iter()
            .enumerate()
            .skip(open)
            .find_map(|(candidate_index, candidate)| {
                if candidate.text == "{" {
                    depth += 1;
                } else if candidate.text == "}" {
                    depth -= 1;
                }
                (depth == 0).then_some(candidate_index)
            })
            .expect("closed function body");
        let lines = code[token.offset..=syntax[close].offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        result.push((name.text.clone(), lines));
    }
    result
}

fn assert_order(source: &str, needles: &[&str]) {
    let mut cursor = 0;
    for needle in needles {
        let relative = source[cursor..]
            .find(needle)
            .unwrap_or_else(|| panic!("missing ordered source fragment {needle:?}"));
        cursor += relative + needle.len();
    }
}

#[test]
fn economic_unit_tests_use_the_exact_terminal_external_marker() {
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

#[test]
fn economic_sources_have_exact_topology_and_bounded_units() {
    let root = source_root();
    let facade_path = root.join("economic_statement.rs");
    let facade = read(&facade_path);
    assert!(
        facade.ends_with(EOF_MARKER),
        "external test marker must be terminal"
    );
    assert_eq!(facade.matches(EOF_MARKER.trim_end()).count(), 1);
    let production = facade
        .strip_suffix(EOF_MARKER)
        .expect("terminal marker already checked");
    assert!(!rust_code(production).contains("include!"));

    let expected = MODULES
        .iter()
        .map(|module| format!("{module}.rs"))
        .collect::<BTreeSet<_>>();
    let directory = root.join("economic_statement");
    let actual = fs::read_dir(&directory)
        .expect("economic statement source directory")
        .filter_map(|entry| {
            let entry = entry.expect("economic statement source entry");
            entry
                .file_type()
                .expect("source entry type")
                .is_file()
                .then(|| entry.file_name().to_string_lossy().into_owned())
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "economic statement module set changed");

    let declared = rust_code(production)
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("mod ")
                .and_then(|module| module.strip_suffix(';'))
                .map(str::to_string)
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        declared,
        MODULES.into_iter().map(str::to_string).collect(),
        "facade module declarations changed"
    );

    for path in std::iter::once(facade_path).chain(
        MODULES
            .iter()
            .map(|module| directory.join(format!("{module}.rs"))),
    ) {
        let source = read(&path);
        assert!(
            source.lines().count() < 1_500,
            "{} exceeds 1,499 lines",
            path.display()
        );
        for (function, lines) in function_lines(&source) {
            assert!(
                lines <= 250,
                "{}::{function} is {lines} lines",
                path.display()
            );
        }
    }
}

#[test]
fn economic_children_keep_narrow_sequential_dependencies() {
    let directory = source_root().join("economic_statement");
    for module in MODULES {
        let path = directory.join(format!("{module}.rs"));
        let code = rust_code(&read(&path));
        assert!(!code.contains("pub "), "{} has public API", path.display());
        assert!(
            !code.contains("pub(crate)"),
            "{} has crate-wide API",
            path.display()
        );
        assert!(
            !code.contains("::*"),
            "{} has a glob import",
            path.display()
        );
        assert!(
            !code.contains("include!"),
            "{} has include!",
            path.display()
        );
        let syntax = tokens(&code);
        for forbidden in [
            "async", "await", "spawn", "channel", "Arc", "Mutex", "RwLock", "dyn",
        ] {
            assert!(
                !syntax.iter().any(|token| token.text == forbidden),
                "{} contains forbidden {forbidden}",
                path.display()
            );
        }
        for (index, _token) in syntax
            .iter()
            .enumerate()
            .filter(|(_, token)| token.text == "pub")
        {
            assert_eq!(
                syntax
                    .get(index + 1..index + 4)
                    .map(|tokens| tokens.iter().map(|token| token.text.as_str()).collect()),
                Some(vec!["(", "super", ")"]),
                "{} has visibility wider than pub(super)",
                path.display()
            );
        }
    }
}

#[test]
fn economic_lifetime_orchestration_and_bill_boundaries_are_lexical() {
    let root = source_root();
    let facade = read(&root.join("economic_statement.rs"));
    let facade_code = rust_code(&facade);
    assert_eq!(
        facade_code
            .matches("pub fn reconcile_okx_economics_paths(")
            .count(),
        1
    );
    assert_eq!(facade_code.matches("pub fn ").count(), 1);
    assert_eq!(facade_code.matches("acquire_storage_lease(").count(), 1);
    assert_eq!(facade_code.matches("lease.journal_path()").count(), 1);
    assert_order(
        &facade_code,
        &[
            "acquire_storage_lease(",
            "read_input(lease.journal_path()",
            "recover_jsonl_bytes_with_visitor(&journal_bytes",
            "let report = build_report(",
            "drop(lease);",
            "Ok(report)",
        ],
    );

    let report_source = read(&root.join("economic_statement/report.rs"));
    let report_body = &report_source[report_source
        .find("pub(super) fn build_report(")
        .expect("build_report definition")..];
    let report = rust_code(report_body);
    assert_order(
        &report,
        &[
            "validate_account_balance_continuity(",
            "validate_journal_identity(",
            "validate_runtime_sessions(",
            "build_journal_trade_evidence(",
            "validate_funding_settlements(",
            "validate_funding_mark_prices(",
            "validate_trade_bill(",
            "validate_funding_bill(",
        ],
    );
    assert_order(report_body, &["\"2\" =>", "\"8\" =>", "_ =>"]);
    assert_eq!(report_body.matches("\"2\" =>").count(), 1);
    assert_eq!(report_body.matches("\"8\" =>").count(), 1);

    let funding = read(&root.join("economic_statement/funding_bills.rs"));
    let subtype = "matches!(bill.sub_type.as_str(), \"173\" | \"174\")";
    assert_eq!(
        funding.matches(subtype).count(),
        1,
        "funding subtype boundary changed"
    );
}
