//! Structural leak scans over the Rust sources themselves (plan task 07).
//!
//! The compile-fail suite in `crates/core/tests/ui/` proves `Secret<T>` cannot be
//! formatted; the runtime corpora prove what telemetry actually renders. This file
//! closes the remaining gap: source-level properties that neither a compiler nor a
//! runtime capture can pin, because they are about *which arguments the instrumentation
//! declares* and *which read APIs the provider boundary uses*.
//!
//! Scanned trees are `crates/{core,adapters,providers,server}/src` — resolved relative
//! to this crate's manifest so the test runs from any checkout depth (the same pattern
//! the server's canonical-schema test uses to reach `.specs/`).
//!
//! Every scan pairs its negative assertion with a positive control (the scan must have
//! actually found the known-good population), so a refactor that stops *parsing* can
//! never silently turn the scan into a no-op.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Workspace root, derived from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root resolves")
}

/// Every `.rs` source file under `crates/*/src`, keyed by `<crate>/<relpath>`.
fn workspace_sources() -> HashMap<String, String> {
    let root = workspace_root();
    let mut out = HashMap::new();
    for crate_name in ["core", "adapters", "providers", "server"] {
        let src = root.join("crates").join(crate_name).join("src");
        collect_rs_files(&src, &mut |path| {
            let rel = path
                .strip_prefix(&root)
                .expect("source under workspace root")
                .to_string_lossy()
                .into_owned();
            let contents =
                std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {rel}: {e}"));
            out.insert(rel, contents);
        });
    }
    assert!(
        out.len() >= 40,
        "expected to scan a real workspace tree, found {} files",
        out.len()
    );
    out
}

fn collect_rs_files(dir: &Path, f: &mut impl FnMut(&Path)) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry readable").path();
        if path.is_dir() {
            collect_rs_files(&path, f);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            f(&path);
        }
    }
}

/// All `#[instrument(...)]` attribute texts in `source`, in order. Attributes in this
/// codebase are single-line; a multi-line attribute would still be caught because the
/// matcher takes everything between `#[instrument(` and the first `)]`.
fn instrument_attrs(source: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    let mut rest = source;
    while let Some(pos) = rest.find("#[instrument(") {
        let after = &rest[pos + "#[instrument(".len()..];
        match after.find(")]") {
            Some(end) => {
                attrs.push(after[..end].to_string());
                rest = &after[end..];
            }
            None => break,
        }
    }
    attrs
}

// ---------------------------------------------------------------------------
// Scan 1 — session-repository instrumentation states its skips explicitly
// ---------------------------------------------------------------------------

/// The lookup/revoke contract: wherever a span declares the `token_hash` log-schema
/// field, the digest argument itself must be named in `skip(...)`. The bare-field
/// name collision with the parameter is a schema aid, not the control — a parameter
/// rename must fail loudly here rather than publish the session lookup key.
///
/// Positive control: all five SessionRepository backends (Dynamo, LMDB, Postgres,
/// SQLite, Valkey) declare the field on both lookup and revoke — ten attributes.
#[test]
fn token_hash_schema_fields_are_always_paired_with_explicit_skips() {
    let sources = workspace_sources();
    let mut paired = 0;
    for (name, source) in &sources {
        for attr in instrument_attrs(source) {
            if attr.contains("fields(token_hash)") {
                assert!(
                    attr.contains("skip(self, token_hash)"),
                    "{name}: `fields(token_hash)` without an explicit \
                     `skip(self, token_hash)` relies on a name match a rename defeats: \
                     #[instrument({attr})]"
                );
                paired += 1;
            }
        }
    }
    assert_eq!(
        paired, 10,
        "expected exactly ten token_hash schema fields (5 backends x lookup+revoke)"
    );
}

/// The write-path contract: a span that projects `user_id` out of the stored session
/// must skip the `session` argument itself — the session carries the hash and the
/// client provenance, none of which may become a span value.
///
/// Positive control: each of the five backends has exactly one such write span.
#[test]
fn write_path_spans_skip_the_session_argument_they_project_from() {
    let sources = workspace_sources();
    let mut projected = 0;
    for (name, source) in &sources {
        for attr in instrument_attrs(source) {
            if attr.contains("%session.") {
                assert!(
                    attr.contains("skip(self, session)"),
                    "{name}: projecting a session field while not skipping `session` \
                     would auto-record the whole struct (hash and provenance included): \
                     #[instrument({attr})]"
                );
                projected += 1;
            }
        }
    }
    assert_eq!(projected, 5, "expected one write span per session backend");
}

/// No span anywhere may give a credential-bearing or client-provenance name a *value*
/// (the `field = expr` capture forms), and the session hash/provenance field names may
/// not appear in an attribute at all — the sanctioned shape is the value-less bare
/// `token_hash` schema entry pinned by the pairing scan above.
#[test]
fn no_span_attribute_captures_a_secret_or_provenance_value() {
    // Substrings that must never occur inside an `#[instrument(...)]`: the hash under
    // any spelling, the configured-secret and Apple-assertion names, and the three
    // provenance fields. (`token_hash` bare is fine; `token_hash =` is a value capture.)
    const FORBIDDEN: [&str; 8] = [
        "refresh_token_hash",
        "client_secret",
        "access_token",
        "id_token",
        "device_id",
        "user_agent",
        "ip_address",
        "token_hash =",
    ];
    let sources = workspace_sources();
    let mut checked = 0;
    for (name, source) in &sources {
        for attr in instrument_attrs(source) {
            checked += 1;
            for needle in FORBIDDEN {
                assert!(
                    !attr.contains(needle),
                    "{name}: span attribute captures sensitive field {needle:?}: \
                     #[instrument({attr})]"
                );
            }
        }
    }
    assert!(
        checked >= 50,
        "expected to inspect a real population of instrumented spans, saw {checked}"
    );
}

// ---------------------------------------------------------------------------
// Scan 2 — the provider boundary has no unbounded body read left
// ---------------------------------------------------------------------------

/// Upstream response bodies may only enter this process through the bounded streaming
/// reader (`shared::http::read_bounded`, capped at `MAX_UPSTREAM_BODY_BYTES`). An
/// unbounded `response.text()` lets a hostile upstream choose how many bytes the
/// service retains and how much diagnostic text can later reach a log line, so the
/// call must not exist anywhere — task 05 removed the last three.
#[test]
fn no_unbounded_response_text_read_remains_at_the_provider_boundary() {
    let sources = workspace_sources();
    let mut reqwest_uses = 0;
    for (name, source) in &sources {
        for (line_no, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("//!") || trimmed.starts_with("///")
            {
                continue;
            }
            if line.contains(".text()") {
                panic!(
                    "{name}:{}: unbounded `.text()` read at the provider boundary — \
                     route the body through shared::http::read_bounded instead",
                    line_no + 1
                );
            }
            if line.contains("reqwest::") || line.contains("bytes_stream()") {
                reqwest_uses += 1;
            }
        }
    }
    // Positive control: the scanned tree really is the HTTP-speaking half of the
    // workspace, so the absence above was measured against live code.
    assert!(
        reqwest_uses >= 10,
        "expected to sweep real reqwest call sites, saw {reqwest_uses}"
    );
}

// ---------------------------------------------------------------------------
// Scan 3 — the redacting types stay non-deriving
// ---------------------------------------------------------------------------

/// `Session` must keep its hand-written `Debug` (which elides the refresh-token hash):
/// a future `derive(Debug)` re-addition would only compile if `Secret` grew formatting,
/// but the guard costs nothing and keeps the intent local.
#[test]
fn session_entity_never_derives_debug() {
    let sources = workspace_sources();
    let session_rs = sources
        .get("crates/core/src/domain/session.rs")
        .expect("session domain source present");
    for (line_no, line) in session_rs.lines().enumerate() {
        // Only attribute position counts: the hand-written impl's doc comment
        // legitimately *mentions* `derive(Debug)` when explaining why it is absent.
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[derive(") && trimmed.contains("Debug") {
            panic!(
                "crates/core/src/domain/session.rs:{}: Session must keep its \
                 hand-written redacting Debug, not a derived one",
                line_no + 1
            );
        }
    }
    // Positive control: the file really declares the entity this guards.
    assert!(
        session_rs.contains("pub struct Session"),
        "scan must run against the real Session definition"
    );
}

/// `Secret<T>` derives exactly `Clone`, `Serialize`, `Deserialize` — never `Debug`,
/// `Display`, or a general `PartialEq` (equality exists only as the constant-time
/// `Secret<String>` impl). If the derive list ever grows, this fails before a new
/// formatting side door can ship.
#[test]
fn secret_type_derives_only_clone_and_serde() {
    let sources = workspace_sources();
    let secret_rs = sources
        .get("crates/core/src/secret.rs")
        .expect("secret source present");

    let derive_line = secret_rs
        .lines()
        .find(|line| line.trim_start().starts_with("#[derive("))
        .expect("Secret's derive attribute must stay present");
    for banned in ["Debug", "Display", "PartialEq", "Eq"] {
        // Split on word boundaries so `Deserialize` cannot satisfy a `Eq`-style
        // substring match by accident; simple containment with `,`/`)]` delimiters
        // is enough for rustfmt's one-per-line derive lists.
        let item_boundary = format!("{banned},");
        let item_tail = format!("{banned})]");
        assert!(
            !derive_line.contains(&item_boundary) && !derive_line.contains(&item_tail),
            "Secret<T> derive line must not gain {banned}: {derive_line}"
        );
    }
    assert!(
        derive_line.contains("Clone")
            && derive_line.contains("Serialize")
            && derive_line.contains("Deserialize"),
        "positive control failed — derive line drifted: {derive_line}"
    );
}
