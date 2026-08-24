//! Compile-fail: `Secret<T>` implements no `Display`, so `{}` interpolation (including
//! the inline `{secret}` form) must be rejected by the compiler.
use oidc_exchange_core::Secret;

#[allow(dead_code)]
fn render(secret: &Secret<String>) -> String {
    format!("{secret}")
}

fn main() {}
