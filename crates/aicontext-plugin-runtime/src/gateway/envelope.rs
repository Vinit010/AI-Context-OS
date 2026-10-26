//! The untrusted data envelope a plugin result is normalised into.
//!
//! `docs/SECURITY.md` §5 control 1 requires tool-returned content to sit in a data role with an
//! explicit `trust: untrusted` marker, and control 3 requires it to be length-, depth-, and
//! character-capped. The supervisor already enforces the frame size and depth; this module adds the
//! marker, strips control characters and ANSI escapes, drops plugin-supplied metadata, and measures
//! the final document against the manifest's `max_output_bytes`.
//!
//! The envelope is the only form of a plugin result the gateway returns. A caller cannot
//! accidentally read the raw [`CallToolResult`](aicontext_plugin_sdk::CallToolResult) instead,
//! because the gateway never exposes it. Plugin-supplied `meta` is deliberately discarded and
//! replaced with host-recorded `request_id` and `duration_ms`: the plugin does not get to describe
//! the call that just happened.
//!
//! # Sanitising
//!
//! Every string and every object key in `content` and `extra` passes through [`sanitise_text`].
//! ANSI escape sequences are removed, and C0/C1 control characters and the Unicode bidi/zero-width
//! format controls are replaced with a space. If two object keys become the same key after
//! sanitising, the whole result is refused: merging them would silently change the data the model
//! sees.

use std::iter::Peekable;
use std::str::Chars;

use aicontext_core::ErrorCode;
use aicontext_plugin_sdk::{CallToolResult, ContentBlock};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::hash::sha256;

/// The marker every envelope carries.
pub const TRUST_UNTRUSTED: &str = "untrusted";

/// One plugin result, normalised for a model to read as data.
///
/// The fields are private and there is no way to construct this type without going through the
/// gateway, so `trust` cannot be forged by a plugin or a caller.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UntrustedToolResult {
    trust: String,
    tool: String,
    plugin: String,
    content: Vec<ContentBlock>,
    extra: Map<String, Value>,
    is_error: bool,
    meta: HostMeta,
}

impl UntrustedToolResult {
    /// The literal trust marker, always [`TRUST_UNTRUSTED`].
    #[must_use]
    pub fn trust(&self) -> &str {
        &self.trust
    }

    /// The capability that was called.
    #[must_use]
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The plugin that provided it.
    #[must_use]
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The sanitised content blocks.
    #[must_use]
    pub fn content(&self) -> &[ContentBlock] {
        &self.content
    }

    /// Unknown result fields, sanitised and carried through.
    #[must_use]
    pub const fn extra(&self) -> &Map<String, Value> {
        &self.extra
    }

    /// Whether the plugin reported the tool call as failed.
    ///
    /// `docs/SECURITY.md` §5 control 5: a refusal is data, not an error to be retried.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        self.is_error
    }

    /// Host-recorded metadata. Plugin-supplied metadata is never present.
    #[must_use]
    pub const fn meta(&self) -> &HostMeta {
        &self.meta
    }
}

/// Metadata the host attaches to a call result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HostMeta {
    request_id: String,
    duration_ms: u64,
}

impl HostMeta {
    /// The host's identifier for this call.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// How long the plugin call took, in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
}

/// A result that could not be normalised.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnvelopeError {
    /// Two object keys became the same key after sanitising.
    #[error("the result has two keys that become `{key}` after sanitising")]
    KeyCollision {
        /// The colliding key.
        key: String,
    },
    /// The normalised document is larger than the manifest allows.
    #[error("the normalised result is {bytes} bytes; the manifest allows {limit}")]
    TooLarge {
        /// The serialised envelope size.
        bytes: usize,
        /// `limits.max_output_bytes`.
        limit: usize,
    },
    /// The result could not be serialised or contains an unusable content block.
    #[error("the result could not be normalised: {detail}")]
    Shape {
        /// The reason.
        detail: String,
    },
}

impl EnvelopeError {
    /// The stable code for this failure.
    ///
    /// A shape or key problem is `PLG-005`; exceeding the manifest's output cap is `PLG-006`.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::KeyCollision { .. } | Self::Shape { .. } => ErrorCode::PLG_005,
            Self::TooLarge { .. } => ErrorCode::PLG_006,
        }
    }
}

/// The envelope plus the measurements the audit record needs.
#[derive(Debug)]
pub(crate) struct Normalised {
    pub(crate) envelope: UntrustedToolResult,
    pub(crate) bytes: usize,
    pub(crate) digest: String,
}

/// Normalise one result, dropping plugin metadata and enforcing `max_bytes`.
pub(crate) fn normalise(
    plugin: &str,
    tool: &str,
    request_id: String,
    duration_ms: u64,
    result: CallToolResult,
    max_bytes: usize,
) -> Result<Normalised, EnvelopeError> {
    let content = result
        .content
        .into_iter()
        .map(sanitise_block)
        .collect::<Result<Vec<_>, _>>()?;
    let extra = sanitise_map(result.extra)?;

    let envelope = UntrustedToolResult {
        trust: TRUST_UNTRUSTED.to_owned(),
        tool: tool.to_owned(),
        plugin: plugin.to_owned(),
        content,
        extra,
        is_error: result.is_error,
        meta: HostMeta {
            request_id,
            duration_ms,
        },
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|error| EnvelopeError::Shape {
        detail: format!("the envelope could not be serialised: {error}"),
    })?;
    if bytes.len() > max_bytes {
        return Err(EnvelopeError::TooLarge {
            bytes: bytes.len(),
            limit: max_bytes,
        });
    }

    Ok(Normalised {
        bytes: bytes.len(),
        digest: sha256(&bytes),
        envelope,
    })
}

fn sanitise_block(block: ContentBlock) -> Result<ContentBlock, EnvelopeError> {
    let kind = sanitise_text(&block.kind);
    if kind.is_empty() {
        return Err(EnvelopeError::Shape {
            detail: "a content block has an empty `type`".to_owned(),
        });
    }
    Ok(ContentBlock {
        kind,
        fields: sanitise_map(block.fields)?,
    })
}

fn sanitise_map(map: Map<String, Value>) -> Result<Map<String, Value>, EnvelopeError> {
    let mut clean = Map::with_capacity(map.len());
    for (key, value) in map {
        let key = sanitise_text(&key);
        if clean.contains_key(&key) {
            return Err(EnvelopeError::KeyCollision { key });
        }
        clean.insert(key, sanitise_value(value)?);
    }
    Ok(clean)
}

fn sanitise_value(value: Value) -> Result<Value, EnvelopeError> {
    Ok(match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value,
        Value::String(text) => Value::String(sanitise_text(&text)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(sanitise_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Object(map) => Value::Object(sanitise_map(map)?),
    })
}

/// Remove ANSI escapes and replace control and bidi/zero-width format characters with spaces.
///
/// Replacing rather than deleting keeps two words from being concatenated when a control character
/// sat between them. An escape sequence is replaced by one space as a unit.
#[must_use]
pub(crate) fn sanitise_text(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            skip_escape(&mut chars);
            clean.push(' ');
        } else if character.is_control() || is_unsafe_format(character) {
            clean.push(' ');
        } else {
            clean.push(character);
        }
    }
    clean
}

fn is_unsafe_format(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
    )
}

/// Consume one escape sequence after its `ESC`.
fn skip_escape(chars: &mut Peekable<Chars<'_>>) {
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            // CSI: parameter and intermediate bytes, then one final byte in 0x40..=0x7e.
            for character in chars.by_ref() {
                if ('@'..='~').contains(&character) {
                    break;
                }
            }
        }
        Some(']' | 'P' | 'X' | '^' | '_') => {
            chars.next();
            skip_string_terminated(chars);
        }
        Some('(' | ')' | '*' | '+' | '-' | '.' | '/') => {
            chars.next();
            // A character-set designation has one following byte.
            chars.next();
        }
        Some(_) => {
            // A two-byte escape: ESC followed by one final byte.
            chars.next();
        }
        None => {}
    }
}

/// Consume an OSC/DCS/SOS/PM/APC payload up to BEL or ST (`ESC \`).
fn skip_string_terminated(chars: &mut Peekable<Chars<'_>>) {
    while let Some(character) = chars.next() {
        if character == '\u{7}' {
            break;
        }
        if character == '\u{1b}' && chars.peek() == Some(&'\\') {
            chars.next();
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use aicontext_plugin_sdk::{CallToolResult, ContentBlock};
    use serde_json::{Map, Value, json};

    use super::{EnvelopeError, HostMeta, TRUST_UNTRUSTED, normalise, sanitise_text};

    fn result() -> CallToolResult {
        CallToolResult {
            content: vec![ContentBlock::text("hello")],
            is_error: false,
            meta: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn an_envelope_carries_the_trust_marker_and_host_metadata() {
        let normalised = normalise(
            "fixture",
            "fixture.echo.run",
            "call-1".to_owned(),
            12,
            result(),
            16 * 1024,
        )
        .expect("the result normalises");

        let envelope = normalised.envelope;
        assert_eq!(envelope.trust(), TRUST_UNTRUSTED);
        assert_eq!(envelope.tool(), "fixture.echo.run");
        assert_eq!(envelope.plugin(), "fixture");
        assert!(!envelope.is_error());
        assert_eq!(envelope.meta().request_id(), "call-1");
        assert_eq!(envelope.meta().duration_ms(), 12);
        assert_eq!(
            envelope.content().first().and_then(ContentBlock::as_text),
            Some("hello")
        );
        assert!(normalised.digest.starts_with("sha256:"));
        assert!(normalised.bytes > 0);
    }

    #[test]
    fn plugin_supplied_meta_is_dropped_and_host_meta_wins() {
        let mut raw = result();
        raw.meta = Some(aicontext_plugin_sdk::ResultMeta {
            request_id: Some("plugin-lied".to_owned()),
            duration_ms: Some(999),
            extra: Map::new(),
        });

        let envelope = normalise(
            "fixture",
            "fixture.echo.run",
            "call-7".to_owned(),
            3,
            raw,
            1024,
        )
        .expect("the result normalises")
        .envelope;
        assert_eq!(
            envelope.meta(),
            &HostMeta {
                request_id: "call-7".to_owned(),
                duration_ms: 3,
            }
        );
    }

    #[test]
    fn control_characters_and_ansi_escapes_are_replaced() {
        assert_eq!(sanitise_text("plain"), "plain");
        assert_eq!(
            sanitise_text("before\u{1b}[31mred\u{1b}[0mafter"),
            "before red after"
        );
        assert_eq!(sanitise_text("a\nb\tc\u{7}d"), "a b c d");
        assert_eq!(sanitise_text("x\u{1b}]0;title\u{7}y"), "x y");
        assert_eq!(sanitise_text("ltr\u{202e}rtl"), "ltr rtl");
        assert_eq!(sanitise_text("zero\u{200b}width"), "zero width");
        assert_eq!(sanitise_text("line\u{2028}break"), "line break");
        assert_eq!(sanitise_text("word\u{2060}joiner"), "word joiner");
    }

    #[test]
    fn sanitising_reaches_every_field_and_key() {
        let mut extra = Map::new();
        extra.insert(
            "ansi\u{1b}[31m".to_owned(),
            json!({"nested": ["ok\u{1b}[0m", {"\u{7}": "value"}]}),
        );
        let raw = CallToolResult {
            content: vec![ContentBlock {
                kind: "text".to_owned(),
                fields: {
                    let mut fields = Map::new();
                    fields.insert("text".to_owned(), Value::String("hi\u{1b}[31m".to_owned()));
                    fields
                },
            }],
            is_error: true,
            meta: None,
            extra,
        };

        let envelope = normalise(
            "fixture",
            "fixture.echo.run",
            "call-1".to_owned(),
            1,
            raw,
            4096,
        )
        .expect("the result normalises")
        .envelope;
        assert!(envelope.is_error());
        assert_eq!(
            envelope.content()[0].fields.get("text"),
            Some(&Value::String("hi ".to_owned()))
        );
        let extra = envelope.extra();
        assert!(extra.contains_key("ansi "));
        let nested = extra.get("ansi ").expect("the key was sanitised");
        assert_eq!(nested["nested"][0], json!("ok "));
        assert!(nested["nested"][1].get(" ").is_some());
    }

    #[test]
    fn two_keys_that_collide_after_sanitising_refuse_the_result() {
        let mut extra = Map::new();
        extra.insert("a\u{7}".to_owned(), json!(1));
        extra.insert("a\u{8}".to_owned(), json!(2));
        let raw = CallToolResult {
            content: Vec::new(),
            is_error: false,
            meta: None,
            extra,
        };

        let error = normalise(
            "fixture",
            "fixture.echo.run",
            "call-1".to_owned(),
            1,
            raw,
            4096,
        )
        .expect_err("the keys collide");
        assert!(matches!(error, EnvelopeError::KeyCollision { ref key } if key == "a "));
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_005);
    }

    #[test]
    fn a_result_larger_than_the_manifest_cap_is_refused() {
        let error = normalise(
            "fixture",
            "fixture.echo.run",
            "call-1".to_owned(),
            1,
            result(),
            16,
        )
        .expect_err("the envelope is larger than 16 bytes");
        assert!(matches!(error, EnvelopeError::TooLarge { limit: 16, .. }));
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_006);
    }
}
