//! Structural non-leakage proof: `Secret<T>` refuses every formatting and tracing
//! capture path, so credential-derived values cannot reach a log line, a span field,
//! or an error string by accident — the compiler rejects the attempt.
//!
//! Each case under `tests/ui/` is a program that must FAIL to compile; the committed
//! `.stderr` fixture pins the exact rustc diagnostic (re-bless with
//! `TRYBUILD=overwrite cargo test -p oidc-exchange-core --test compile_fail`). This is
//! the proof that the control is structural — a type-system guarantee — rather than a
//! redaction convention that a reviewer has to remember.
//!
//! Runtime leak behavior (what telemetry actually renders when the controls hold) is
//! covered separately by the cross-store, service, provider, and HTTP leak corpora in
//! the adapters, core, providers, and server crates.

#[test]
fn secret_refuses_every_formatting_and_capture_path() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
