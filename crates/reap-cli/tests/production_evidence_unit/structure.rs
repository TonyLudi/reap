use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const MODULES: [&str; 6] = [
    "bindings",
    "canonical",
    "manifest",
    "policy_time",
    "report",
    "source_verifiers",
];
const TEST_MARKER: &str =
    "#[cfg(test)]\n#[path = \"../tests/production_evidence_unit/mod.rs\"]\nmod tests;";

#[derive(Debug)]
struct Token {
    text: String,
    offset: usize,
}

#[derive(Debug)]
struct Function {
    name: String,
    start: usize,
    end: usize,
    lines: usize,
}

fn src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn facade_path() -> PathBuf {
    src_root().join("production_evidence.rs")
}

fn child_path(module: &str) -> PathBuf {
    src_root()
        .join("production_evidence")
        .join(format!("{module}.rs"))
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
    let first = *bytes.get(cursor)?;
    if first == b'\\' {
        cursor += 1;
        match *bytes.get(cursor)? {
            b'x' => cursor += 3,
            b'u' => {
                cursor += 1;
                if bytes.get(cursor) != Some(&b'{') {
                    return None;
                }
                cursor += 1;
                while !matches!(bytes.get(cursor), Some(b'}') | None) {
                    cursor += 1;
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    } else {
        let width = source.get(cursor..)?.chars().next()?.len_utf8();
        cursor += width;
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
            cursor += 2;
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
    String::from_utf8(output).expect("masking preserves UTF-8")
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

fn functions(source: &str) -> Vec<Function> {
    let code = rust_code(source);
    let syntax = tokens(&code);
    let mut result = Vec::new();
    for (index, token) in syntax.iter().enumerate() {
        if token.text != "fn" {
            continue;
        }
        let Some(name) = syntax.get(index + 1) else {
            continue;
        };
        if !name
            .text
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        {
            continue;
        }
        let Some(open_index) = syntax[index + 2..]
            .iter()
            .position(|candidate| candidate.text == "{" || candidate.text == ";")
            .map(|relative| index + 2 + relative)
        else {
            continue;
        };
        if syntax[open_index].text == ";" {
            continue;
        }
        let mut depth = 0;
        let mut close_index = None;
        for (candidate_index, candidate) in syntax.iter().enumerate().skip(open_index) {
            if candidate.text == "{" {
                depth += 1;
            } else if candidate.text == "}" {
                depth -= 1;
                if depth == 0 {
                    close_index = Some(candidate_index);
                    break;
                }
            }
        }
        let close_index = close_index.unwrap_or_else(|| panic!("unclosed function {}", name.text));
        let start = token.offset;
        let end = syntax[close_index].offset;
        result.push(Function {
            name: name.text.clone(),
            start,
            end,
            lines: code[start..=end]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1,
        });
    }
    result
}

fn function_code(source: &str, name: &str) -> String {
    let matches = functions(source)
        .into_iter()
        .filter(|function| function.name == name)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one function named {name}");
    rust_code(&source[matches[0].start..=matches[0].end])
}

fn filtered_calls(source: &str, names: &[&str]) -> Vec<String> {
    let syntax = tokens(source);
    syntax
        .windows(2)
        .filter(|pair| pair[1].text == "(" && names.contains(&pair[0].text.as_str()))
        .map(|pair| pair[0].text.clone())
        .collect()
}

fn count_sequence(source: &str, sequence: &[&str]) -> usize {
    let syntax = tokens(source);
    syntax
        .windows(sequence.len())
        .filter(|window| {
            window
                .iter()
                .zip(sequence)
                .all(|(token, expected)| token.text == *expected)
        })
        .count()
}

#[test]
fn production_evidence_has_exact_topology_and_bounded_files_and_functions() {
    let facade = read(&facade_path());
    assert!(
        facade.trim_end().ends_with(TEST_MARKER),
        "external test marker must be exact and terminal"
    );
    assert_eq!(facade.matches(TEST_MARKER).count(), 1);
    let production = facade
        .trim_end()
        .strip_suffix(TEST_MARKER)
        .expect("terminal marker already checked");
    assert!(!production.contains("mod tests"));
    assert!(!production.contains("include!"));

    let declared = production
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("mod ")
                .and_then(|value| value.strip_suffix(';'))
        })
        .collect::<Vec<_>>();
    assert_eq!(declared, MODULES);

    let directory = src_root().join("production_evidence");
    let actual = fs::read_dir(&directory)
        .expect("read production-evidence directory")
        .map(|entry| {
            let entry = entry.expect("read production-evidence entry");
            assert!(entry.file_type().expect("entry type").is_file());
            entry.file_name().to_string_lossy().into_owned()
        })
        .collect::<BTreeSet<_>>();
    let expected = MODULES
        .iter()
        .map(|module| format!("{module}.rs"))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert!(!directory.join("tests").exists());

    let files =
        std::iter::once(facade_path()).chain(MODULES.iter().map(|module| child_path(module)));
    for path in files {
        let source = read(&path);
        assert!(
            source.lines().count() < 1_500,
            "{} exceeds 1,499 lines",
            path.display()
        );
        for function in functions(&source) {
            assert!(
                function.lines <= 250,
                "{}::{} is {} lines",
                path.display(),
                function.name,
                function.lines
            );
        }
    }
}

#[test]
fn child_modules_keep_narrow_visibility_and_sequential_dependencies() {
    for module in MODULES {
        let path = child_path(module);
        let source = read(&path);
        let code = rust_code(&source);
        let syntax = tokens(&code);
        for (index, token) in syntax.iter().enumerate() {
            if token.text == "pub" {
                let visibility = syntax
                    .get(index + 1..index + 4)
                    .map(|tokens| tokens.iter().map(|token| token.text.as_str()).collect());
                assert_eq!(
                    visibility,
                    Some(vec!["(", "super", ")"]),
                    "{} has visibility wider than pub(super)",
                    path.display()
                );
            }
        }
        for forbidden in [
            "async",
            "await",
            "spawn",
            "par_iter",
            "par_bridge",
            "channel",
            "sync_channel",
            "mpsc",
            "oneshot",
            "broadcast",
            "Arc",
            "Mutex",
            "RwLock",
            "dyn",
            "include",
        ] {
            assert!(
                !syntax.iter().any(|token| token.text == forbidden),
                "{} contains forbidden {forbidden}",
                path.display()
            );
        }
        for line in code
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("use "))
        {
            assert!(
                !line.contains("::*"),
                "{} has a glob import",
                path.display()
            );
        }
    }

    let all_children = MODULES
        .iter()
        .map(|module| rust_code(&read(&child_path(module))))
        .collect::<String>();
    assert_eq!(
        tokens(&all_children)
            .iter()
            .filter(|token| token.text == "join")
            .count(),
        1
    );
    assert!(all_children.contains("base.join(value)"));
}

#[test]
fn verifier_and_binding_stages_remain_ordered_single_threaded_and_unauthorized() {
    let facade = read(&facade_path());
    let entry = function_code(&facade, "verify_production_evidence_manifest_path");
    let stages = [
        "load_manifest",
        "validate_manifest",
        "resolve_manifest",
        "expected_identity",
        "current_executable_sha256",
        "host_identity_sha256",
        "load_initial_configs",
        "reconstruct_sources",
        "reopen_verified_configs",
        "summarize_sources",
        "unix_time_ms",
        "evaluate_freshness",
        "build_gate_reports",
        "evaluate_bindings",
    ];
    assert_eq!(filtered_calls(&entry, &stages), stages);

    let bindings = read(&child_path("bindings"));
    let evaluator = function_code(&bindings, "evaluate_bindings");
    let binding_stages = [
        "check_verifier_and_config_bindings",
        "check_transition_and_research",
        "check_demo",
        "check_fault",
        "check_latency",
        "check_account",
        "check_deadman",
        "check_emergency",
        "check_fill",
        "check_economic",
    ];
    assert_eq!(filtered_calls(&evaluator, &binding_stages), binding_stages);

    assert_eq!(
        count_sequence(
            &rust_code(&facade),
            &["production_order_entry_authorized", ":", "false"]
        ),
        1
    );
    assert_eq!(
        count_sequence(
            &rust_code(&facade),
            &["production_order_entry_authorized", ":", "true"]
        ),
        0
    );
    assert_eq!(
        count_sequence(
            &rust_code(&facade),
            &["production_order_entry_authorized", "="]
        ),
        0
    );
    assert_eq!(filtered_calls(&entry, &["unix_time_ms"]), ["unix_time_ms"]);

    let policy = rust_code(&read(&child_path("policy_time")));
    assert_eq!(
        count_sequence(&policy, &["SystemTime", ":", ":", "now", "(", ")"]),
        1
    );
    for module in MODULES
        .into_iter()
        .filter(|module| *module != "policy_time")
    {
        assert_eq!(
            tokens(&rust_code(&read(&child_path(module))))
                .iter()
                .filter(|token| token.text == "SystemTime")
                .count(),
            0
        );
    }
}

#[test]
fn source_masker_ignores_spoofed_braces_and_functions() {
    let sample = r##"
fn real() {
    let normal = "} fn fake() {";
    let raw = r#"{ fn raw_fake() }"#;
    let brace = '}';
    // } fn comment_fake() {
    /* { /* fn block_fake() {} */ } */
}
fn second() {}
"##;
    let found = functions(sample);
    assert_eq!(
        found
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        ["real", "second"]
    );
}
