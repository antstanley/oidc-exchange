//! Compile-fail: `tracing`'s `%field` capture routes through `field::display`, whose
//! `T: Display` bound a `Secret<T>` cannot satisfy.
use oidc_exchange_core::Secret;

#[allow(dead_code)]
fn emit(secret: &Secret<String>) {
    tracing::info!(%secret, "must not compile");
}

fn main() {}
