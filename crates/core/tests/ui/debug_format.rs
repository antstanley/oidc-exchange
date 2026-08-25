//! Compile-fail: `Secret<T>` implements no `Debug`, so `{:?}` interpolation must be
//! rejected by the compiler rather than rendering the wrapped credential.
use oidc_exchange_core::Secret;

#[allow(dead_code)]
fn render(secret: &Secret<String>) -> String {
    format!("{secret:?}")
}

fn main() {}
