//! Compile-fail: `#[instrument]`'s default argument capture records every unskipped
//! argument through `tracing::field::debug`, so a function whose signature carries a
//! `Secret<T>` cannot compile until the argument is added to `skip(...)`. This is the
//! structural replacement for the name-collision redaction the store adapters
//! previously relied on.
use oidc_exchange_core::Secret;
use tracing::instrument;

#[instrument]
#[allow(dead_code)]
fn handle_request(secret: &Secret<String>) -> u8 {
    let _ = secret;
    0
}

fn main() {
    let secret = Secret::new("unused-sentinel".to_string());
    let _ = handle_request(&secret);
}
