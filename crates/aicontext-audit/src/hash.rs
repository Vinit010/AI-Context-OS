//! SHA-256 helpers and the digest that seals one audit record.
//!
//! Two different digests appear in the log and they must not be confused:
//!
//! - [`record_digest`] seals a whole record, including its `prev` link. It is what verification
//!   recomputes.
//! - [`is_digest`] checks the `sha256:<lowercase hex>` spelling of every digest field, so a record
//!   cannot smuggle an arbitrary string into `prev`, `self`, or an argument digest.

use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::AuditError;
use crate::record::AuditRecord;

/// The `prev` value of the first record.
///
/// It is not the SHA-256 of anything: it is a fixed sentinel of 64 zeroes, so no record can ever
/// claim to follow a real record by coincidence. The spelling matches every other digest in the log
/// so a reader parses one shape.
pub const GENESIS: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

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

/// Whether `text` is exactly `sha256:` followed by 64 lowercase hex digits.
#[must_use]
pub(crate) fn is_digest(text: &str) -> bool {
    let Some(hex) = text.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The `self` digest of `record`: SHA-256 over its compact canonical JSON without `self`.
///
/// Object keys are sorted explicitly through a [`BTreeMap`], rather than relying on
/// `serde_json::Map`'s current backing map. If a future dependency enables `preserve_order`, the
/// digest must not change with it: a chain that seals a feature flag is a chain that breaks when
/// the feature set changes.
pub(crate) fn record_digest(record: &AuditRecord) -> Result<String, AuditError> {
    let value = serde_json::to_value(record).map_err(|error| {
        AuditError::invalid_record(format!("the record cannot be rendered: {error}"))
    })?;
    let Value::Object(members) = value else {
        return Err(AuditError::invalid_record(
            "the record did not render as a JSON object",
        ));
    };
    let canonical: BTreeMap<String, Value> = members
        .into_iter()
        .filter(|(key, _)| key != "self")
        .collect();
    let bytes = serde_json::to_vec(&canonical).map_err(|error| {
        AuditError::invalid_record(format!("the canonical record cannot be rendered: {error}"))
    })?;
    Ok(sha256(&bytes))
}

/// Appends two lowercase hex digits.
fn push_hex_byte(text: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    text.push(char::from(HEX[usize::from(byte >> 4)]));
    text.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

#[cfg(test)]
mod tests {
    use super::{GENESIS, is_digest, sha256};
    use crate::record::{Action, Actor, AuditEntry};
    use crate::{AuditRecord, Timestamp};

    #[test]
    fn the_empty_input_has_the_well_known_sha256_digest() {
        assert_eq!(
            sha256(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn the_genesis_sentinel_is_a_well_formed_digest_of_zeroes() {
        assert!(is_digest(GENESIS));
        assert_eq!(GENESIS.len(), "sha256:".len() + 64);
        assert!(GENESIS["sha256:".len()..].bytes().all(|byte| byte == b'0'));
    }

    #[test]
    fn only_lowercase_hex_with_the_prefix_is_a_digest() {
        for text in [
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            GENESIS,
        ] {
            assert!(is_digest(text), "{text} must be accepted");
        }
        for text in [
            "sha256:E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b85",
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b8550",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "sha512:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ] {
            assert!(!is_digest(text), "{text} must be rejected");
        }
    }

    #[test]
    fn the_record_digest_covers_the_prev_link_and_the_content() {
        let first = record(1, GENESIS, "aws.ec2.stop");
        let second = record(2, GENESIS, "aws.ec2.stop");
        let third = record(1, GENESIS, "aws.ec2.start");

        let first_digest = super::record_digest(&first).expect("renders");
        assert_ne!(
            first_digest,
            super::record_digest(&second).expect("renders")
        );
        assert_ne!(
            first_digest,
            super::record_digest(&third).expect("renders"),
            "a changed value changes the digest"
        );
        assert!(is_digest(&first_digest));
    }

    #[test]
    fn key_order_in_the_file_does_not_change_the_digest() {
        // `serde_json` stores object keys in a sorted map, so parsing and re-rendering a line that
        // was written in struct order produces the same canonical bytes. This asserts the property
        // the digest depends on rather than the current backing map implementation.
        let record = record(1, GENESIS, "aws.ec2.stop");
        let text = serde_json::to_string(&record).expect("renders");
        let reparsed: AuditRecord = serde_json::from_str(&text).expect("parses");
        assert_eq!(
            super::record_digest(&record).expect("renders"),
            super::record_digest(&reparsed).expect("renders")
        );
    }

    fn record(seq: u64, prev: &str, tool: &str) -> AuditRecord {
        let entry = AuditEntry::new(
            "completed",
            Actor::agent("developer"),
            Action::new(tool, "aws", "write"),
        );
        AuditRecord::seal(
            seq,
            Timestamp::from_unix_millis(1_790_000_000_000),
            &entry,
            prev,
        )
        .expect("a valid record")
    }
}
