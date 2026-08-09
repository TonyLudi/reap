use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn production_sources() -> Vec<(PathBuf, String)> {
    fn visit(directory: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).expect("production source directory must be readable")
        {
            let path = entry.expect("source entry must be readable").path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    visit(&root, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path).expect("production source must be UTF-8");
            (path, without_trailing_unit_tests(&source).to_owned())
        })
        .collect()
}

fn without_trailing_unit_tests(source: &str) -> &str {
    ["\n#[cfg(test)]", "\n#[cfg(all(test,", "\n#[cfg(any(test,"]
        .into_iter()
        .filter_map(|marker| source.find(marker))
        .min()
        .map_or(source, |cutoff| &source[..cutoff])
}

fn without_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut block_depth = 0_u32;
    let mut quoted = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        if block_depth > 0 {
            if byte == b'/' && next == Some(b'*') {
                block_depth += 1;
                output.extend_from_slice(b"  ");
                index += 2;
            } else if byte == b'*' && next == Some(b'/') {
                block_depth -= 1;
                output.extend_from_slice(b"  ");
                index += 2;
            } else {
                output.push(if byte == b'\n' { b'\n' } else { b' ' });
                index += 1;
            }
        } else if quoted {
            output.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            index += 1;
        } else if byte == b'"' {
            quoted = true;
            output.push(byte);
            index += 1;
        } else if byte == b'/' && next == Some(b'/') {
            output.extend_from_slice(b"  ");
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                output.push(b' ');
                index += 1;
            }
        } else if byte == b'/' && next == Some(b'*') {
            block_depth = 1;
            output.extend_from_slice(b"  ");
            index += 2;
        } else {
            output.push(byte);
            index += 1;
        }
    }
    String::from_utf8(output).expect("sanitized Rust source remains UTF-8")
}

fn without_comments_or_literals(source: &str) -> String {
    let uncommented = without_comments(source);
    let mut code = String::with_capacity(uncommented.len());
    let mut quoted = false;
    let mut escaped = false;
    for character in uncommented.chars() {
        if quoted {
            code.push(if character == '\n' { '\n' } else { ' ' });
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else if character == '"' {
            quoted = true;
            code.push(' ');
        } else {
            code.push(character);
        }
    }
    code
}

fn code_identifiers(source: &str) -> BTreeSet<String> {
    let code = without_comments_or_literals(source);
    code.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|identifier| !identifier.is_empty())
        .map(str::to_owned)
        .collect()
}

fn identifier_words(identifier: &str) -> Vec<String> {
    let mut words = Vec::new();
    for segment in identifier.split('_').filter(|value| !value.is_empty()) {
        let mut start = 0;
        let characters: Vec<char> = segment.chars().collect();
        for index in 1..characters.len() {
            if characters[index].is_ascii_uppercase() && characters[index - 1].is_ascii_lowercase()
            {
                words.push(
                    characters[start..index]
                        .iter()
                        .collect::<String>()
                        .to_ascii_lowercase(),
                );
                start = index;
            }
        }
        words.push(
            characters[start..]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase(),
        );
    }
    words
}

fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn struct_fields(source: &str, name: &str) -> BTreeSet<String> {
    let declaration = format!("pub struct {name}");
    let start = source
        .find(&declaration)
        .unwrap_or_else(|| panic!("missing struct {name}"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .expect("struct must have a body");
    let mut depth = 0_i32;
    let mut close = None;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    source[open + 1..close.expect("struct body must close")]
        .lines()
        .filter_map(|line| {
            let field = line.trim().strip_prefix("pub ")?.split_once(':')?.0.trim();
            field
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
                .then(|| field.to_owned())
        })
        .collect()
}

#[test]
fn production_imports_and_identifiers_have_no_mutation_or_bypass_capability() {
    let forbidden_crate_tokens = [
        "reap_polymarket_auth",
        "reap_pm_live",
        "reap_pm_live_contracts",
        "reap_pm_state",
        "reap_pm_strategy",
        "reap_strategy",
        "reap_order",
        "reap_storage",
        "reap_durable_writer",
        "reap_live",
        "reap_live_contracts",
        "reap_engine",
        "reap_risk",
        "reqwest",
    ];
    let forbidden_types = [
        "EoaPrivateKeyInput",
        "FixedEoaSigner",
        "L2CredentialInput",
        "L2Credentials",
        "SignedClobV2Order",
        "SerializedPlaceRequest",
        "SerializedOwnedCancelRequest",
        "AuthenticatedL2Headers",
        "AuthenticatedPlaceRequest",
        "AuthenticatedOwnedCancelRequest",
        "PmLoopbackMutationConfig",
        "PmLoopbackMutationConnectivityOwner",
        "PmMutationServerTimeHttpRole",
        "PmMutationServerTimeValidator",
        "PmPendingMutationServerTime",
        "PmAuthorizedMutationServerTime",
        "PmRetainedPlaceRequest",
        "PmRetainedOwnedCancelRequest",
    ];

    for (path, source) in production_sources() {
        let identifiers = code_identifiers(&source);
        for forbidden in forbidden_crate_tokens {
            assert!(
                !identifiers.contains(forbidden),
                "forbidden direct crate import/reference in {}: {forbidden}",
                path.display()
            );
        }
        for forbidden in forbidden_types {
            assert!(
                !identifiers.contains(forbidden),
                "forbidden mutation/authentication type in {}: {forbidden}",
                path.display()
            );
        }
        assert!(
            !identifiers.contains("unsafe"),
            "unsafe block in {}",
            path.display()
        );

        for identifier in &identifiers {
            let words = identifier_words(identifier);
            let mutation_action = words.iter().any(|word| {
                word == "place"
                    || word == "cancel"
                    || word.starts_with("approv")
                    || word.starts_with("settl")
                    || word.starts_with("redeem")
                    || word.starts_with("withdraw")
            });
            let capability_shape = words.iter().any(|word| {
                matches!(
                    word.as_str(),
                    "request"
                        | "role"
                        | "owner"
                        | "config"
                        | "transport"
                        | "client"
                        | "route"
                        | "order"
                        | "signer"
                        | "credential"
                        | "credentials"
                        | "authentication"
                        | "serializer"
                        | "command"
                        | "dispatch"
                        | "sink"
                )
            });
            assert!(
                !(mutation_action && capability_shape),
                "mutation capability identifier in {}: {identifier}",
                path.display()
            );
            let order_construction_shape = words.iter().any(|word| word == "order")
                && words.iter().any(|word| {
                    matches!(
                        word.as_str(),
                        "signed"
                            | "unsigned"
                            | "request"
                            | "submission"
                            | "submit"
                            | "mutation"
                            | "serializer"
                            | "serialize"
                            | "dispatch"
                            | "command"
                    )
                });
            assert!(
                !order_construction_shape,
                "order construction/dispatch identifier in {}: {identifier}",
                path.display()
            );
            assert!(
                identifier == "PM_CLOB_PRODUCTION_ORIGIN"
                    || !words
                        .iter()
                        .any(|word| word == "endpoint" || word == "origin"),
                "operator endpoint override identifier in {}: {identifier}",
                path.display()
            );
            let lower = identifier.to_ascii_lowercase();
            assert!(
                !(lower.contains("private_key") || lower.contains("privatekey")),
                "private-key identifier in {}: {identifier}",
                path.display()
            );
        }

        let uncommented = without_comments(&source);
        for forbidden in [
            "\"/order",
            "\"/cancel",
            "\"/approve",
            "\"/settle",
            "\"/redeem",
            "\"/withdraw",
            ".post(",
            ".put(",
            ".patch(",
            ".delete(",
            "fn place(",
            "fn place_",
            "fn cancel(",
            "fn cancel_",
            "fn approve(",
            "fn approve_",
            "fn settle(",
            "fn settle_",
            "fn redeem(",
            "fn redeem_",
            "fn withdraw(",
            "fn withdraw_",
            ".place(",
            ".cancel(",
            ".approve(",
            ".settle(",
            ".redeem(",
            ".withdraw(",
            "\"http://",
            "\"https://",
            "\"ws://",
            "\"wss://",
        ] {
            assert!(
                !uncommented.contains(forbidden),
                "forbidden route/method/endpoint literal in {}: {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn strict_config_has_only_the_closed_secret_free_operator_fields() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/config.rs");
    let source = fs::read_to_string(path).expect("config source must be readable");
    assert!(source.contains("#[serde(deny_unknown_fields)]\npub struct PmReadOnlySmokeConfig"));
    let expected = BTreeSet::from(
        [
            "api_key_file",
            "chain_id",
            "condition_id",
            "connect_timeout_ms",
            "credential_slot_id",
            "funder_address",
            "market_id",
            "minimum_order_size",
            "negative_risk",
            "outcome",
            "passphrase_file",
            "request_timeout_ms",
            "schema_version",
            "secret_file",
            "signature_type",
            "signer_address",
            "tick",
            "token_id",
            "user_stream_dwell_ms",
            "user_stream_event_channel_capacity",
            "user_stream_idle_timeout_ms",
            "user_stream_max_reconnect_attempts",
            "user_stream_pong_timeout_ms",
            "user_stream_reconnect_backoff_ms",
        ]
        .map(str::to_owned),
    );
    assert_eq!(struct_fields(&source, "PmReadOnlySmokeConfig"), expected);
}

#[test]
fn artifact_schema_cannot_persist_raw_authenticated_or_secret_derived_material() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let source = ["schema.rs", "account_schema.rs"]
        .into_iter()
        .map(|name| fs::read_to_string(root.join(name)).expect("schema source must be readable"))
        .collect::<Vec<_>>()
        .join("\n");
    for structure in [
        "PmReadOnlyCollectionFailureEvidence",
        "PmReadOnlyAllowanceEvidence",
        "PmReadOnlyAccountEvidence",
        "PmReadOnlyOrderEvidence",
        "PmReadOnlyTradeMakerEvidence",
        "PmReadOnlyTradeEvidence",
        "PmReadOnlyReconciliationEvidence",
        "PmReadOnlyUserStreamEvidence",
        "PmReadOnlyTeardownEvidence",
        "PmReadOnlySmokeArtifact",
        "PmReadOnlyAccountSnapshotEvidence",
        "PmReadOnlyAccountTeardownEvidence",
        "PmReadOnlyAccountArtifact",
    ] {
        for field in struct_fields(&source, structure) {
            let lower = field.to_ascii_lowercase();
            assert!(
                !lower.contains("raw"),
                "raw persisted field: {structure}.{field}"
            );
            assert!(
                !lower.contains("header"),
                "header persisted field: {structure}.{field}"
            );
            assert!(
                !lower.contains("body"),
                "authenticated body persisted field: {structure}.{field}"
            );
            assert!(
                !lower.contains("subscription") || lower == "subscription_count",
                "subscription material persisted field: {structure}.{field}"
            );
            assert!(
                !lower.contains("frame") || lower == "frame_count",
                "frame material persisted field: {structure}.{field}"
            );
            assert!(
                !matches!(lower.as_str(), "api_key" | "secret" | "passphrase"),
                "secret-valued persisted field: {structure}.{field}"
            );
            assert!(
                !(lower.contains("secret")
                    && (lower.contains("sha")
                        || lower.contains("hash")
                        || lower.contains("fingerprint"))
                    && !lower.contains("nonsecret")),
                "secret-derived digest persisted field: {structure}.{field}"
            );
        }
    }
}

#[test]
fn account_only_config_is_a_distinct_transparent_view_not_a_weakened_smoke_profile() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/config.rs");
    let source = fs::read_to_string(path).expect("config source must be readable");
    assert!(source.contains("pub struct PmReadOnlyAccountConfig(PmReadOnlySmokeConfig)"));
    assert!(source.contains("account-only signature_type must be 0 (EOA) or 1 (proxy)"));
    assert!(source.contains("the fixed read-only profile requires signature_type=0"));
    assert!(source.contains("the fixed read-only profile requires signer=funder"));
}

#[test]
fn account_only_collector_has_exact_closed_request_counts_and_cancellation_fail_stop() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/account_collect.rs");
    let source = fs::read_to_string(path).expect("account collector source must be readable");
    let compact = compact(&without_comments(&source));
    assert_eq!(source.matches("fresh_read_server_time().await?").count(), 2);
    assert_eq!(source.matches("balance_allowance(").count(), 2);
    assert!(compact.contains("private_reconciliation_request_count:0"));
    assert!(compact.contains("user_stream_connection_count:0"));
    assert!(source.contains("CredentialShutdownCancellationFailStop"));
    assert!(source.contains("std::process::abort()"));
}

#[test]
fn runtime_env_lookups_are_static_and_never_secret_valued() {
    for (path, source) in production_sources() {
        let uncommented = without_comments(&source);
        assert!(
            !uncommented.contains("std::env::vars(") && !uncommented.contains("std::env::vars_os("),
            "bulk environment lookup in {}",
            path.display()
        );
        for needle in [
            "std::env::var(",
            "std::env::var_os(",
            "env::var(",
            "env::var_os(",
        ] {
            let mut remainder = uncommented.as_str();
            while let Some(start) = remainder.find(needle) {
                let after = &remainder[start + needle.len()..];
                let end = after.find(')').unwrap_or(after.len());
                let argument = &after[..end];
                let upper = argument.to_ascii_uppercase();
                assert!(
                    argument.contains("\"CREDENTIALS_DIRECTORY\"")
                        || argument.contains("\"HOSTNAME\""),
                    "runtime environment lookup must be one static non-secret name in {}: {argument}",
                    path.display()
                );
                for secret_marker in ["API_KEY", "SECRET", "PASSPHRASE", "PRIVATE_KEY", "POLY_"] {
                    assert!(
                        !upper.contains(secret_marker),
                        "secret-valued environment lookup in {}: {argument}",
                        path.display()
                    );
                }
                remainder = &after[end.min(after.len())..];
            }
        }
        for macro_name in ["env!(", "option_env!("] {
            for line in uncommented.lines().filter(|line| line.contains(macro_name)) {
                let upper = line.to_ascii_uppercase();
                for secret_marker in ["API_KEY", "SECRET", "PASSPHRASE", "PRIVATE_KEY", "POLY_"] {
                    assert!(
                        !upper.contains(secret_marker),
                        "secret-valued compile-time environment lookup in {}: {line}",
                        path.display()
                    );
                }
            }
        }
    }
}

#[test]
fn authorization_and_mutation_zeroes_are_literal_artifact_facts() {
    let sources = production_sources();
    let joined = sources
        .iter()
        .map(|(_, source)| source.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let schema =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/schema.rs"))
            .expect("schema source must be readable");
    for required in [
        "pub production_order_entry_authorized: bool",
        "pub mutation_roles_constructed: bool",
        "pub mutation_requests: u64",
    ] {
        assert!(
            schema.contains(required),
            "missing artifact field: {required}"
        );
    }
    let compact = compact(&joined);
    for required in [
        "production_order_entry_authorized:false",
        "mutation_roles_constructed:false",
        "mutation_requests:0",
    ] {
        assert!(
            compact.contains(required),
            "missing literal closed fact: {required}"
        );
    }
    assert!(!compact.contains("production_order_entry_authorized:true"));
    assert!(!compact.contains("mutation_roles_constructed:true"));
}

#[test]
fn secret_hashing_and_raw_authenticated_persistence_calls_are_absent() {
    for (path, source) in production_sources() {
        let identifiers = code_identifiers(&source);
        for forbidden in [
            "AuthenticatedL2Headers",
            "PmRetainedUserSubscription",
            "PmRetainedPlaceRequest",
            "PmRetainedOwnedCancelRequest",
        ] {
            assert!(
                !identifiers.contains(forbidden),
                "raw authenticated retention type in {}: {forbidden}",
                path.display()
            );
        }
        for identifier in &identifiers {
            let lower = identifier.to_ascii_lowercase();
            assert!(
                !((lower.contains("sha")
                    || lower.contains("hash")
                    || lower.contains("digest")
                    || lower.contains("fingerprint"))
                    && (lower.contains("api_key")
                        || lower.contains("passphrase")
                        || (lower.contains("secret") && !lower.contains("nonsecret")))),
                "secret-derived digest identifier in {}: {identifier}",
                path.display()
            );
        }
    }

    let credential_source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/credentials.rs"))
            .expect("credential source must be readable");
    let credential_code = without_comments_or_literals(&credential_source);
    assert!(credential_code.contains("pub(crate) fn into_adapter_input_and_artifact_guard"));
    assert!(!credential_code.contains("pub fn into_adapter_input("));
    for required_snapshot_guard in [
        "CredentialFileSnapshot",
        "mtime_nsec",
        "ctime_nsec",
        "validate_entry_snapshot",
    ] {
        assert!(
            credential_code.contains(required_snapshot_guard),
            "credential snapshot guard is missing: {required_snapshot_guard}"
        );
    }
    for forbidden in [
        "pub fn api_key(",
        "pub fn secret(",
        "pub fn passphrase(",
        "sha256",
        "Sha256",
        "Digest",
        "fingerprint",
    ] {
        assert!(
            !credential_code.contains(forbidden),
            "credential custody must expose no getter or digest path: {forbidden}"
        );
    }
}

#[test]
fn credential_collision_is_rejected_before_network_or_artifact_assembly() {
    let collector =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/collect.rs"))
            .expect("collector source must be readable");
    let reserve = collector
        .find("let mut output = reserve_private_output")
        .expect("collector must reserve a create-new output first");
    let load = collector
        .find("let credentials = load_pm_read_only_credentials")
        .expect("collector must load the protected credential bundle");
    let collision = collector
        .find(".ensure_config_is_secret_free(")
        .expect("collector must reject artifact-bound credential aliases");
    let public = collector
        .find("let metadata = match collect_public_metadata(&config).await")
        .expect("collector must collect fixed public metadata");

    assert!(reserve < load && load < collision && collision < public);
    let load_boundary = &collector[load..collision];
    assert!(
        load_boundary.contains(")?;"),
        "any credential-load failure must return without artifact assembly"
    );
    assert!(
        !load_boundary.contains("finish_attempt"),
        "partially loaded credential material must never reach a failure artifact"
    );
    let collision_boundary = &collector[collision..public];
    assert!(
        collision_boundary.contains(")?;"),
        "a credential collision must return without artifact assembly"
    );
    assert!(
        !collision_boundary.contains("finish_attempt"),
        "secret-bearing config evidence must never enter a failure artifact"
    );
}

#[test]
fn final_artifact_commit_scans_decoded_public_bodies_and_serialized_bytes() {
    let collector =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/collect.rs"))
            .expect("collector source must be readable");
    let persist = collector
        .find("fn persist(")
        .expect("collector must have one atomic artifact commit boundary");
    let commit = collector[persist..]
        .find(".persist(&self.target_anchor)")
        .map(|offset| persist + offset)
        .expect("collector must atomically commit the staged artifact");
    let boundary = &collector[persist..commit];

    assert!(boundary.contains("metadata.market_body_base64"));
    assert!(boundary.contains("metadata.clob_body_base64"));
    assert_eq!(
        boundary
            .matches("ensure_base64_artifact_value_is_secret_free")
            .count(),
        2
    );
    assert!(boundary.contains("ensure_artifact_is_secret_free(&bytes)"));
}

#[test]
fn authenticated_user_task_is_fail_stop_owned_from_spawn_through_join() {
    let collector =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/collect.rs"))
            .expect("collector source must be readable");
    assert!(
        collector
            .contains("let user_task = UserTaskCancellationFailStop::new(tokio::spawn(async move")
    );
    assert!(!collector.contains("let user_task = tokio::spawn"));
    assert!(collector.contains("impl Drop for UserTaskCancellationFailStop"));
    assert!(collector.contains("Err(_) => std::process::abort()"));
}
