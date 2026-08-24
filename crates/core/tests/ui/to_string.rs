//! Compile-fail: with no `Display` there is no `ToString` blanket impl, so
//! `.to_string()` — the most reflexive leak path of all — must not resolve.
use oidc_exchange_core::Secret;

#[allow(dead_code)]
fn render(secret: &Secret<String>) {
    let _ = secret.to_string();
}

fn main() {}
