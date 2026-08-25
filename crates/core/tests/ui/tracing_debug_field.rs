//! Compile-fail: `tracing`'s `?field` capture routes through `field::debug`, whose
//! `T: Debug` bound a `Secret<T>` cannot satisfy — the span-field leak path is a
//! compile error, not a redaction convention.
use oidc_exchange_core::Secret;

#[allow(dead_code)]
fn emit(secret: &Secret<String>) {
    tracing::info!(?secret, "must not compile");
}

fn main() {}
