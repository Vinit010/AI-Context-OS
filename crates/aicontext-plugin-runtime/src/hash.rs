//! SHA-256 helpers shared by the approval broker and the tool gateway.
//!
//! Both seams need a stable digest in a record that outlives the call: the broker hashes the
//! argument set because a credential can hide in it, and the gateway hashes the untrusted result
//! envelope so the audit chain can later prove which bytes a model saw (`docs/SECURITY.md` §6 and
//! §8). Keeping the rendering in one place means both records use the same `sha256:<hex>` spelling
//! and neither grows a second hex encoder.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// `sha256:<lowercase hex>` over `bytes`.
#[must_use]
pub(crate) fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();

    let mut rendered = String::with_capacity("sha256:".len() + digest.len() * 2);
    rendered.push_str("sha256:");
    for byte in digest {
        push_hex_byte(&mut rendered, byte);
    }
    rendered
}

/// `sha256:<hex>` over the compact JSON form of `value`.
///
/// `serde_json::Value` stores object keys in a sorted map, so the digest is stable across two
/// parses of the same object. It is not stable across a different JSON representation of the same
/// value (whitespace, for example), because the bytes hashed are the compact form.
#[must_use]
pub(crate) fn json_digest(value: &Value) -> String {
    sha256(value.to_string().as_bytes())
}

/// Appends two lowercase hex digits, without a `hex` dependency or a fallible `write!`.
fn push_hex_byte(text: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    text.push(char::from(HEX[usize::from(byte >> 4)]));
    text.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{json_digest, sha256};

    #[test]
    fn the_empty_input_has_the_well_known_sha256_digest() {
        assert_eq!(
            sha256(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn the_json_digest_is_stable_for_the_same_value_and_changes_with_the_value() {
        let first = json!({ "b": 2, "a": 1 });
        let second = json!({ "a": 1, "b": 2 });

        assert_eq!(json_digest(&first), json_digest(&second));
        assert_ne!(json_digest(&first), json_digest(&json!({ "a": 1 })));
        assert!(json_digest(&first).starts_with("sha256:"));
        assert_eq!(json_digest(&first).len(), "sha256:".len() + 64);
    }
}
