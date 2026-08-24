//! Shared telemetry-capture harness for the leak-corpus regression suites.
//!
//! A leak-corpus test must assert against *rendered* telemetry — every event, plus span
//! open **and** close lines — rather than against tracing's in-memory field model,
//! because a broken `skip(...)` shows up only in what a formatter would actually emit.
//! The stock `fmt` subscriber's `FmtSpan::NONE` would let a span-leak assertion pass
//! vacuously, so [`install_span_capture`] always enables `FmtSpan::NEW | FmtSpan::CLOSE`.
//!
//! Matching helpers percent-decode before searching: the upstream redactor's motivating
//! attack is an echo carrying percent-encoded credentials (`token=1%2F%2F…`), so a
//! corpus that matched only the literal sentinel would miss exactly the leak shape the
//! controls exist for.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

/// A clonable in-memory writer the fmt subscriber renders into, so tests can assert on
/// exactly what was produced rather than scraping stdout.
#[derive(Clone, Default)]
pub struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("capture mutex must not be poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Declared span-field schema, as `(span name, field name)` pairs recorded at span
/// creation. The fmt formatter prints nothing for a declared-but-never-valued field
/// (the `fields(token_hash)` schema entries), so schema compatibility can only be
/// proven against the metadata, not the rendered stream.
pub type DeclaredFields = Arc<Mutex<HashSet<(String, String)>>>;

/// Records the *declared* field schema of every span created while installed.
struct DeclaredFieldsLayer {
    declared: DeclaredFields,
}

impl<S> Layer<S> for DeclaredFieldsLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: Context<'_, S>,
    ) {
        let span_name = attrs.metadata().name().to_string();
        let mut declared = self
            .declared
            .lock()
            .expect("capture mutex must not be poisoned");
        for field in attrs.metadata().fields() {
            declared.insert((span_name.clone(), field.name().to_string()));
        }
        assert!(
            !declared.is_empty(),
            "every instrumented span must declare its field schema"
        );
    }
}

/// The bundle a leak-corpus test needs: hold `capture` for the whole test body, read
/// rendered telemetry via [`rendered_output`], and assert declared schema via
/// `declared`.
pub struct SpanCapture {
    /// Keeps the thread-local subscriber installed for the whole test body; dropping it
    /// uninstalls the subscriber. Declared BEFORE the gate: struct fields drop in
    /// declaration order, so the subscriber must uninstall while the gate is still
    /// held — releasing the gate first would let the next capture's install race this
    /// capture's teardown of tracing's dispatcher registry.
    _guard: tracing::subscriber::DefaultGuard,
    /// Serializes capture tests process-wide; held for the capture's lifetime.
    _gate: std::sync::MutexGuard<'static, ()>,
    buffer: SharedBuffer,
    declared: DeclaredFields,
}

impl SpanCapture {
    /// The rendered telemetry stream (events plus span open/close lines), ANSI-free.
    pub fn rendered(&self) -> String {
        let bytes = self
            .buffer
            .0
            .lock()
            .expect("capture mutex must not be poisoned")
            .clone();
        String::from_utf8(bytes).expect("captured telemetry is utf-8")
    }

    /// The declared `(span name, field name)` schema pairs observed so far.
    pub fn declared(&self) -> DeclaredFields {
        self.declared.clone()
    }
}

/// Install a fmt subscriber writing into `buffer` with explicit span open/close events
/// enabled, alongside the schema-capture layer.
///
/// The capture is scoped to this workspace's own telemetry: targets beginning with
/// `oidc_exchange`. Third-party instrumentation underneath an instrumented call (the
/// AWS SDK logs entire request/response payloads at TRACE, sqlx logs statement
/// internals) would otherwise flood the rendered stream and make absence assertions
/// fail on values *dependencies* echo — material no control in this repository is
/// responsible for, and material the shipped JSON subscriber never renders anyway
/// (production defaults to INFO with env-filter). Keeping the corpus scoped the way
/// production is scoped makes every assertion about a leak this codebase could fix.
///
/// Keep the returned handle alive for the whole test body. Under a single-threaded
/// `#[tokio::test]` runtime every poll happens on the installing thread, so the
/// thread-local default subscriber sees every span open and close.
/// Process-wide gate serializing span-capture tests: concurrent thread-local
/// subscriber installs race tracing's callsite interest cache, and a capture
/// that loses the race records nothing.
pub static CAPTURE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn install_span_capture(buffer: SharedBuffer) -> SpanCapture {
    let declared: DeclaredFields = Arc::new(Mutex::new(HashSet::new()));
    // The writer closure owns a clone; the capture keeps the original for assertions.
    let writer_buffer = buffer.clone();
    // Everything the workspace emits, at any level — leaks are level-agnostic.
    let targets = tracing_subscriber::filter::Targets::new()
        .with_target("oidc_exchange", tracing::Level::TRACE);
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(move || writer_buffer.clone())
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_ansi(false)
                .with_filter(targets),
        )
        .with(DeclaredFieldsLayer {
            declared: declared.clone(),
        });
    let gate = CAPTURE_GATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    SpanCapture {
        _gate: gate,
        _guard: guard,
        buffer,
        declared,
    }
}

/// Asserts a span declared exactly one of its expected schema fields, so the log-schema
/// contract survives even though the formatter prints nothing for empty fields.
pub fn assert_declares(declared: &DeclaredFields, span_name: &str, field_name: &str) {
    let key = (span_name.to_string(), field_name.to_string());
    assert!(
        declared
            .lock()
            .expect("capture mutex must not be poisoned")
            .contains(&key),
        "span {span_name} must keep declaring the {field_name} schema field"
    );
}

/// Percent-decode `text` (`%XX` byte escapes), copying malformed sequences through
/// unchanged. Byte-level by design: the output feeds sentinel matching, not display,
/// and a lossy pass cannot panic on arbitrary input. Mirrors the production decoder in
/// `adapters::shared::upstream` so a corpus matches exactly what the redactor sees.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    // Upper bound: every three-byte escape collapses to one byte, so decoding never
    // grows the text beyond its input length.
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let (Some(&hi), Some(&lo)) = (bytes.get(i + 1), bytes.get(i + 2)) {
                let h = (hi as char).to_digit(16).map(|v| v as u8);
                let l = (lo as char).to_digit(16).map(|v| v as u8);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Negative space for one sentinel: neither its literal form nor any percent-encoded
/// form of the rendered text may contain it. Decoding the whole stream once and
/// searching that catches an echo like `token=1%2F%2FSENTINEL…` without needing to
/// enumerate encodings.
pub fn assert_absent_plain_and_encoded(rendered: &str, sentinel: &str) {
    assert!(
        !rendered.contains(sentinel),
        "sentinel {sentinel:?} must never reach telemetry, got: {rendered:?}"
    );
    let decoded = percent_decode(rendered);
    assert!(
        !decoded.contains(sentinel),
        "percent-decoded telemetry must not contain sentinel {sentinel:?}, got: {decoded:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_mirrors_the_production_decoder() {
        assert_eq!(percent_decode("token=1%2F%2Fabc"), "token=1//abc");
        assert_eq!(percent_decode("%41%42"), "AB");
        // Malformed escapes pass through literally, never panic.
        assert_eq!(percent_decode("%ZZ%2"), "%ZZ%2");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn absent_assertion_catches_both_literal_and_encoded_leaks() {
        // A byte *inside* the sentinel is percent-encoded (`%58` = `X`), so the raw
        // stream does not contain the sentinel literally while the decoded stream
        // does — exactly the echo shape that defeats a literal-only matcher.
        let stream = "before token=1%2F%2FSENTINEL-%58YZ after";
        assert!(
            !stream.contains("SENTINEL-XYZ"),
            "precondition: the encoded sentinel must be absent from the raw stream"
        );
        let result = std::panic::catch_unwind(|| {
            assert_absent_plain_and_encoded(stream, "SENTINEL-XYZ");
        });
        assert!(
            result.is_err(),
            "an encoded sentinel must be caught by the decoded match"
        );

        // A clean stream passes both checks.
        assert_absent_plain_and_encoded("nothing to see here", "SENTINEL-XYZ");
    }

    #[test]
    fn literal_sentinel_is_caught_directly() {
        let result = std::panic::catch_unwind(|| {
            assert_absent_plain_and_encoded("leak SENTINEL-LITERAL end", "SENTINEL-LITERAL");
        });
        assert!(
            result.is_err(),
            "a literal sentinel must fail the assertion"
        );
    }
}
