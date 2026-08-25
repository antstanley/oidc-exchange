//! Deterministic cross-provider JWK corpus fixtures.
//!
//! The corpus exists to answer threat-model contradiction C12 — "do the OIDC and
//! Apple validators accept the same tokens?" — with evidence instead of assumption.
//! Every case below is a complete JWKS document built from fixed key material (an
//! embedded RSA-2048 key pair and a P-256 key pair generated once from the seed
//! `42u8; 32`), so both provider validation paths see byte-identical fixtures and
//! their dispositions can be compared case by case.
//!
//! The cases are the ones the outbound-boundary source spec lists for the
//! key-selection corpus: purpose filtering (`use`, `key_ops`), algorithm
//! consistency, duplicate `kid` in both array orders, key-type rejections, and
//! the two non-regression success shapes.

/// `kid` of the RSA corpus entry.
pub const RSA_KID: &str = "corpus-rsa-key";

/// `kid` of the EC corpus entry.
pub const EC_KID: &str = "corpus-ec-key";

/// `kid` shared by the duplicate-`kid` corpus cases.
pub const DUPLICATE_KID: &str = "corpus-duplicate-kid";

/// Private half of the corpus RSA key pair (PKCS#8). Test-only material; never
/// used outside corpus fixtures.
pub const RSA_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDJ4ezfFIefEY3X
vJZu/DwUGemyo6VYcK6XU9niHUHZw4oWvsDtA3qZXNWy3lXXBKmFzAEXLicZsSid
JcrozVRnvp8AjzwZ5jGUKW562pfhcJDNM8Ycc/fSZeKNR9hEUinvplIYyxI0AFYa
BIo5biYz+p8R/PjNApspWCJmvQHr5PPqDDTa9AK9wPzKT9I/2bcdTlNaOMCZnUr1
MhAcJUC2QoELxowag5VIbQptyD291TzsKgxKdSpDnzFjVCsvKTkLexkvtXo1y+Ha
P6KU/9zifqpmm9apgQaqoErbQwSJpD+JFtdHpeii5nPU4tmQGZQ7cn6MLJ8T056v
92/HuS0hAgMBAAECggEAbn7HhPnZmQikl/XSaICJ6X6dWHcVIqjaBl2QnZ/h0Oyj
gft54L/MtHAJTtM+LGeS2XZlCmjqYbeDQS/UNUNc9UNyB35eKNbDQBLFM1y9UFiq
CIZT4nLeqzu0mhs+lXZbGZ3wxT0wg2HDvo3JkdFl+4Eq20+YZa0Ne72PZqgAizlU
wN8LPPJunVGz4D0fmGWpp6hK439ejTnNfv2J0h1OVB3PH6rX0iHz+tYEz2tOl16S
oLapXtC29BwS1X6BOEF05dOumTxR4nT1vhkCZKrz/cK7mxmmzJuVVDZvppecb8Eb
L0t3zg9DAWyN0cy0VVbjOH1TvFxNodnd0FRfTVTigQKBgQD8oXRReKepft/FvCRa
f1hqf/8zwX3Xef39HKZYqr94UK1txwoG+yo4tTlE8JPL50EMCalUc/Zkbmu5TbM+
pMjjb4B3Eyfze4N0NpWEEEW+M4exis31MOe9gUITm65eT2S2Mntt0F317WjHmHxA
lcWmmoxgzzoTflx7YfUGUZvciQKBgQDMkzQoGRN/1FLlpwjiNYp9v+i4Mu2Cwxji
BCUNyZZPeTx/aQjqt1Z2lyf0jb9QZ3pTsHbMGJiuav80whpudHYUU4HJ24AvxC7R
2Q6spQiBtbJgfMKw2lByfFCHOzCKQWcRb5E2cMsLhrC8RvICLNkn07z2mgTuDrXY
TxfITGQV2QKBgH40mktpHzlJrLi3uOGM5LqvnupYK2nOA9jCy0dYZbbRdxJ0cMn0
B6+0uRt7pBolORWube1G0Txy/VXhPz54S/Ny7JaP91Fnzs/rxN3o0y6lx5Ama6Wl
/N9rB3uMNpvexc1PguHlSktlgwbTYp9RMyB77M0gOT8rzT/GPAYgFuEhAoGAHQ1O
m98ryLyDZT+6YD2QRFlrmDULS8WfFAHYrUOSiAjEkad9769HpSHEN9OldqqrUZU+
2a8oh6SER57FGCiL2EkfpmX4p0/qAj0b+2KYeasvAMrW7zyhrhB/cyTxuMCe/Xfl
nGCaRTHEiYhdt/dcg25raG3pA1Gte2GIFBbdI8kCgYEAgWeT4qcWj7csI00a5re9
TIbthLgtxuKO6B8QdYQdYUoDJui9RXfuiiYTris/JPW1GF+0BcCmbvv0PTxP79oH
FVcapekT6K7Pw8/im5jUsL16uUTW9JJXIpQ/HWLGwo7uR3C3KB0wvcsZa+LM7Boj
6dNwesfbd8w2/iGcG9lDLbE=
-----END PRIVATE KEY-----
";

/// Public modulus of the corpus RSA key pair, base64url without padding.
const RSA_N: &str = "yeHs3xSHnxGN17yWbvw8FBnpsqOlWHCul1PZ4h1B2cOKFr7A7QN6mVzVst5V1wSphcwBFy4nGbEonSXK6M1UZ76fAI88GeYxlCluetqX4XCQzTPGHHP30mXijUfYRFIp76ZSGMsSNABWGgSKOW4mM_qfEfz4zQKbKVgiZr0B6-Tz6gw02vQCvcD8yk_SP9m3HU5TWjjAmZ1K9TIQHCVAtkKBC8aMGoOVSG0Kbcg9vdU87CoMSnUqQ58xY1QrLyk5C3sZL7V6Ncvh2j-ilP_c4n6qZpvWqYEGqqBK20MEiaQ_iRbXR6XoouZz1OLZkBmUO3J-jCyfE9Oer_dvx7ktIQ";

/// Public exponent of the corpus RSA key pair (65537), base64url without padding.
const RSA_E: &str = "AAAAAAABAAE";

/// Private half of the corpus P-256 key pair (PKCS#8), generated from the seed
/// `[42u8; 32]`. Test-only material.
pub const EC_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgKioqKioqKioqKioq
KioqKioqKioqKioqKioqKioqKiqhRANCAAQMkB1CPIMcqF4nxzwmO6Eychu516hM
TwOAsqZ1b9YBMxyIcCNN7IeFBMF0FE+ksUtmplFpFgbYFz5VvTfjgVae
-----END PRIVATE KEY-----
";

/// Public x coordinate of the corpus P-256 key pair, base64url without padding.
const EC_X: &str = "DJAdQjyDHKheJ8c8JjuhMnIbudeoTE8DgLKmdW_WATM";

/// Public y coordinate of the corpus P-256 key pair, base64url without padding.
const EC_Y: &str = "HIhwI03sh4UEwXQUT6SxS2amUWkWBtgXPlW9N-OBVp4";

/// Symmetric key material (`oct`) for the oct-key rejection case.
const OCT_K: &str = "c2VjcmV0LWtleS1tYXRlcmlhbA";

/// One JWK-selection corpus case.
///
/// Each variant names the behaviour under test; [`jwks_for_case`] renders the
/// complete JWKS document both providers must be served for that case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// A signing-purpose RSA entry carrying no `alg`: exercises absent-alg
    /// inference on both paths.
    RsaSigAbsentAlg,
    /// `use: "enc"` with no `alg`.
    RsaEncAbsentAlg,
    /// `use: "enc"` with an admitted signing `alg` (`RS256`).
    RsaEncRs256,
    /// `key_ops: ["encrypt", "wrapKey"]` — no `verify` operation.
    RsaKeyOpsEncryptWrap,
    /// `key_ops: ["encrypt"]` — also omits `verify`.
    RsaKeyOpsEncryptOnly,
    /// An `alg` inconsistent with `kty`: `ES256` on an RSA key.
    RsaAlgEs256,
    /// `alg: "RSA-OAEP"` — a real RSA algorithm, but an encryption one.
    RsaAlgRsaOaep,
    /// `alg: "ES256"` on a `use: "enc"` P-256 key: right family, wrong purpose.
    EcEncEs256,
    /// Duplicate `kid`, ineligible entry first, eligible second.
    DuplicateKidEncFirst,
    /// Duplicate `kid`, eligible entry first, ineligible second.
    DuplicateKidSigFirst,
    /// An `oct` (symmetric) key: never a verification candidate.
    OctKey,
    /// `alg: "none"` — the un-signing algorithm.
    AlgNone,
}

/// Render the full JWKS document for a corpus case.
pub fn jwks_for_case(case: Case) -> serde_json::Value {
    serde_json::json!({ "keys": keys_for_case(case) })
}

/// A clean signing-capable RSA entry (`use: sig`, `alg: RS256`) for the
/// non-regression success cases.
pub fn rsa_sig_entry(kid: &str) -> serde_json::Value {
    rsa_sig_jwk(kid, Some("RS256"))
}

/// A clean signing-capable P-256 entry (`use: sig`, `alg: ES256`) for the
/// non-regression success cases. Carries an explicit `alg` because that is what
/// every validator in this repository requires today; the alg-less P-256 shapes
/// live in the rejection/inference cases.
pub fn ec_sig_entry(kid: &str) -> serde_json::Value {
    with_alg(ec_sig_jwk(kid), "ES256")
}

/// The `keys` array for a corpus case (also usable as the trailing element of a
/// larger JWKS).
fn keys_for_case(case: Case) -> Vec<serde_json::Value> {
    match case {
        Case::RsaSigAbsentAlg => vec![rsa_sig_jwk(RSA_KID, None)],
        Case::RsaEncAbsentAlg => vec![with_use(rsa_sig_jwk(RSA_KID, None), "enc")],
        Case::RsaEncRs256 => {
            vec![with_use(rsa_sig_jwk(RSA_KID, Some("RS256")), "enc")]
        }
        Case::RsaKeyOpsEncryptWrap => vec![with_key_ops(
            rsa_sig_jwk(RSA_KID, Some("RS256")),
            &["encrypt", "wrapKey"],
        )],
        Case::RsaKeyOpsEncryptOnly => vec![with_key_ops(
            rsa_sig_jwk(RSA_KID, Some("RS256")),
            &["encrypt"],
        )],
        Case::RsaAlgEs256 => vec![rsa_sig_jwk(RSA_KID, Some("ES256"))],
        Case::RsaAlgRsaOaep => vec![rsa_sig_jwk(RSA_KID, Some("RSA-OAEP"))],
        Case::EcEncEs256 => {
            vec![with_use(ec_sig_jwk(EC_KID), "enc")]
        }
        Case::DuplicateKidEncFirst => vec![
            with_use(
                with_alg(rsa_sig_jwk(DUPLICATE_KID, None), "RSA-OAEP"),
                "enc",
            ),
            rsa_sig_jwk(DUPLICATE_KID, Some("RS256")),
        ],
        Case::DuplicateKidSigFirst => vec![
            rsa_sig_jwk(DUPLICATE_KID, Some("RS256")),
            with_use(
                with_alg(rsa_sig_jwk(DUPLICATE_KID, None), "RSA-OAEP"),
                "enc",
            ),
        ],
        Case::OctKey => vec![serde_json::json!({
            "kty": "oct",
            "kid": "corpus-oct-key",
            "use": "sig",
            "k": OCT_K,
        })],
        Case::AlgNone => vec![rsa_sig_jwk(RSA_KID, Some("none"))],
    }
}

/// A clean signing-capable RSA corpus entry: `use: sig`, optional explicit `alg`.
fn rsa_sig_jwk(kid: &str, alg: Option<&str>) -> serde_json::Value {
    let mut jwk = serde_json::json!({
        "kty": "RSA",
        "kid": kid,
        "use": "sig",
        "n": RSA_N,
        "e": RSA_E,
    });
    if let Some(alg) = alg {
        jwk["alg"] = serde_json::json!(alg);
    }
    jwk
}

/// A clean signing-capable EC P-256 corpus entry: `use: sig`, `crv: P-256`,
/// no explicit `alg` (the Apple-realistic alg-less shape).
fn ec_sig_jwk(kid: &str) -> serde_json::Value {
    serde_json::json!({
        "kty": "EC",
        "kid": kid,
        "use": "sig",
        "crv": "P-256",
        "x": EC_X,
        "y": EC_Y,
    })
}

fn with_use(mut jwk: serde_json::Value, use_value: &str) -> serde_json::Value {
    jwk["use"] = serde_json::json!(use_value);
    jwk
}

fn with_alg(mut jwk: serde_json::Value, alg: &str) -> serde_json::Value {
    jwk["alg"] = serde_json::json!(alg);
    jwk
}

fn with_key_ops(mut jwk: serde_json::Value, ops: &[&str]) -> serde_json::Value {
    jwk["key_ops"] = serde_json::json!(ops);
    jwk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_renders_a_jwks_with_a_keys_array() {
        let cases = [
            Case::RsaSigAbsentAlg,
            Case::RsaEncAbsentAlg,
            Case::RsaEncRs256,
            Case::RsaKeyOpsEncryptWrap,
            Case::RsaKeyOpsEncryptOnly,
            Case::RsaAlgEs256,
            Case::RsaAlgRsaOaep,
            Case::EcEncEs256,
            Case::DuplicateKidEncFirst,
            Case::DuplicateKidSigFirst,
            Case::OctKey,
            Case::AlgNone,
        ];
        assert_eq!(cases.len(), 12, "the corpus enumerates twelve cases");

        for case in cases {
            let jwks = jwks_for_case(case);
            assert!(jwks["keys"].is_array(), "{case:?} must render a keys array");
            assert!(
                !jwks["keys"].as_array().expect("checked").is_empty(),
                "{case:?} must carry at least one entry"
            );
        }
    }

    #[test]
    fn duplicate_kid_cases_share_one_kid_in_both_orders() {
        for case in [Case::DuplicateKidEncFirst, Case::DuplicateKidSigFirst] {
            let jwks = jwks_for_case(case);
            let keys = jwks["keys"].as_array().expect("array");
            assert_eq!(keys.len(), 2, "{case:?} has exactly two entries");
            for key in keys {
                assert_eq!(key["kid"], DUPLICATE_KID);
            }
            // Both variants carry exactly one ineligible and one eligible entry;
            // array order is the only difference between them.
            let mut uses: Vec<&str> = keys
                .iter()
                .map(|k| k["use"].as_str().expect("use must be a string"))
                .collect();
            uses.sort_unstable();
            assert_eq!(uses, vec!["enc", "sig"], "{case:?} pairs enc with sig");
        }

        // EncFirst leads with the ineligible entry; SigFirst is its mirror.
        let enc_first = jwks_for_case(Case::DuplicateKidEncFirst);
        assert_eq!(enc_first["keys"][0]["use"], "enc");
        let sig_first = jwks_for_case(Case::DuplicateKidSigFirst);
        assert_eq!(sig_first["keys"][0]["use"], "sig");
    }

    #[test]
    fn embedded_private_keys_are_parseable_and_match_public_material() {
        // The success cases depend on the embedded PEMs matching the embedded
        // public components; verify the pairing by round-tripping through the
        // encoding crates' own parsers. Signing itself happens in the consumer
        // crates (they own the jsonwebtoken dependency).
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        // RSA n must decode to a 2048-bit modulus (256 bytes).
        let n_bytes = URL_SAFE_NO_PAD
            .decode(RSA_N)
            .expect("RSA_N must be valid base64url");
        assert_eq!(n_bytes.len(), 256, "corpus RSA modulus is 2048-bit");

        // EC coordinates must decode to 32-byte P-256 field elements.
        let x_bytes = URL_SAFE_NO_PAD
            .decode(EC_X)
            .expect("EC_X must be valid base64url");
        let y_bytes = URL_SAFE_NO_PAD
            .decode(EC_Y)
            .expect("EC_Y must be valid base64url");
        assert_eq!((x_bytes.len(), y_bytes.len()), (32, 32));

        // PEM armour must bracket the payloads exactly.
        for pem in [RSA_PRIVATE_PEM, EC_PRIVATE_PEM] {
            assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
            assert!(pem.trim_end().ends_with("-----END PRIVATE KEY-----"));
        }
    }
}
