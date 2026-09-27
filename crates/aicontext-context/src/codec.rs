//! The YAML seam: the only module in this crate that knows which YAML library is in use.
//!
//! [`YamlCodec`] is two methods wide — decode a block into ordered key/value pairs, encode ordered
//! key/value pairs into a block — and [`SerdeYaml`] implements it with `yaml_serde`. Everything above
//! this module depends on the trait, so the storage contract in `docs/CONTEXT_SPEC.md` §2 is
//! expressed in terms of our own [`Value`] rather than a third party's. That is the mitigation for
//! `RISK R-1`.
//!
//! # The trait must stay thin
//!
//! This trait earns its existence by staying two methods wide. If it grows into a general YAML
//! abstraction - a builder, a node API, a multi-document reader, features that leak through - then
//! it has stopped containing the risk and become it, because every one of those additions is a
//! decision the next parser will have to reimplement. The right move at that point is to delete
//! this module and call the crate directly, accepting a larger blast radius in exchange for no
//! abstraction tax. That is stated here so the next person to touch it does not inherit a layer
//! that grew for its own sake.
//!
//! # Why `yaml_serde`
//!
//! Chosen in `ADR-007`: it is the maintained fork of the now-archived `serde_yaml`, published by
//! the official YAML organisation, pure Rust, MIT OR Apache-2.0, with an MSRV of 1.82 against our
//! floor of 1.85. `serde_yml` is deprecated and carries `RUSTSEC-2025-0068`; `serde_norway` had
//! gone 21 months without a release. Rejected candidates and their reasons are in the ADR.

use std::collections::BTreeMap;

use super::value::Value;

/// A failure from the YAML layer, with a line number where the layer could supply one.
///
/// A private error type on purpose: callers above this module handle
/// [`ContextError`](crate::ContextError), which carries a line number of its own and can express
/// every failure mode of the front-matter contract. This type exists only to be converted.
///
/// The message is passed through **verbatim**, including any position the layer wrote into it.
/// Rewriting it here would be the wrong place: this module does not know where the block sits in
/// the file, and a position edited with the wrong offset is worse than one left alone. The
/// translation happens in [`ContextError::from_yaml`](crate::ContextError), which does know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct YamlError {
    pub(crate) message: String,
    pub(crate) line: Option<usize>,
}

impl YamlError {
    /// A failure with no usable location, for a problem detected while converting a value tree - an
    /// unsupported tag, an integer too large to represent, a repeated key.
    ///
    /// Every failure that *does* have a location comes from [`yaml_error`], because the line number
    /// belongs to the YAML layer that found it and inventing one here would only be a guess.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            line: None,
        }
    }
}

impl std::fmt::Display for YamlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "line {line}: {}", self.message),
            None => formatter.write_str(&self.message),
        }
    }
}

/// Decodes and encodes a front-matter block, over [`Value`] and nothing else.
pub(crate) trait YamlCodec {
    /// Parses `input` as a single YAML document, returning its top-level key/value pairs in the
    /// order they were written.
    ///
    /// Returns a list rather than a map on purpose. A [`BTreeMap`] would either drop a repeated key
    /// or refuse the whole document with a message that does not name the key, and
    /// `docs/CONTEXT_SPEC.md` §2 rule 5 is emphatic that nothing written by hand may be silently
    /// lost. The caller decides what a repeated key means, and can say which key it was.
    ///
    /// A document whose top level is not a mapping - a list, a bare scalar - is an error, because
    /// front matter with no keys has no fields to read.
    ///
    /// `input` is the block with the fences already removed, so a line number in the returned error
    /// is relative to the block's first line. The caller adds the offset, which is why this returns
    /// a line rather than a span.
    fn decode_mapping(&self, input: &str) -> Result<Vec<(String, Value)>, YamlError>;

    /// Renders `entries` as a single YAML document, keeping the given order, without a fence.
    ///
    /// Order-preserving because `docs/CONTEXT_SPEC.md` §2 rule 4 prescribes a key order for the
    /// well-known keys, and a sorted map cannot express it.
    fn encode_mapping(&self, entries: &[(String, Value)]) -> Result<String, YamlError>;
}

/// The codec this crate ships: `yaml_serde`, per `ADR-007`.
pub(crate) struct SerdeYaml;

impl YamlCodec for SerdeYaml {
    fn decode_mapping(&self, input: &str) -> Result<Vec<(String, Value)>, YamlError> {
        // Parsed into the crate's own value type first, then walked here, rather than deserialised
        // straight into a `BTreeMap` or a `Vec` of pairs. Both of those were tried: a `BTreeMap`
        // drops a repeated key without saying which one, and this YAML layer will not produce a
        // pair list from a mapping at all. Walking the mapping keeps the written order, keeps both
        // copies of a repeated key so the caller can name it, and still gets a line number out of
        // the layer for a genuine syntax error.
        let document: yaml_serde::Value =
            yaml_serde::from_str(input).map_err(|error| yaml_error(&error))?;

        let yaml_serde::Value::Mapping(entries) = document else {
            return Err(YamlError::new(format!(
                "expected a mapping of keys to values, found {}",
                describe(&document)
            )));
        };

        entries
            .into_iter()
            .map(|(key, value)| Ok((stringify_key(key)?, convert(value)?)))
            .collect()
    }

    fn encode_mapping(&self, entries: &[(String, Value)]) -> Result<String, YamlError> {
        let mut native = yaml_serde::Mapping::new();
        for (key, value) in entries {
            native.insert(yaml_serde::Value::String(key.clone()), to_native(value)?);
        }
        yaml_serde::to_string(&native).map_err(|error| YamlError::new(error.to_string()))
    }
}

/// Translates a `yaml_serde` error into ours, keeping its line number when it has one.
fn yaml_error(error: &yaml_serde::Error) -> YamlError {
    YamlError {
        message: error.to_string(),
        line: error.location().map(|location| location.line()),
    }
}

/// Converts one value of the foreign tree into ours, recursively.
fn convert(value: yaml_serde::Value) -> Result<Value, YamlError> {
    match value {
        yaml_serde::Value::Null => Ok(Value::Null),
        yaml_serde::Value::Bool(inner) => Ok(Value::Bool(inner)),
        yaml_serde::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Ok(Value::Int(integer))
            } else if number.as_u64().is_some() {
                // A precise integer this large would have to become a float to be stored, and a
                // float that quietly stands in for an integer is exactly the kind of imprecision a
                // storage contract must not have. Refuse it.
                Err(YamlError::new(format!(
                    "integer `{number}` is larger than this crate can represent exactly; front \
                     matter holds identifiers and sizes, not big numbers"
                )))
            } else if let Some(float) = number.as_f64() {
                Ok(Value::Float(float))
            } else {
                Err(YamlError::new(format!(
                    "number `{number}` cannot be represented; write it as a string if the exact \
                     value matters"
                )))
            }
        }
        yaml_serde::Value::String(text) => Ok(Value::Str(text)),
        yaml_serde::Value::Sequence(items) => {
            let items = items
                .into_iter()
                .map(convert)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::List(items))
        }
        yaml_serde::Value::Mapping(entries) => {
            let mut converted = BTreeMap::new();
            for (key, value) in entries {
                let key = stringify_key(key)?;
                if converted.insert(key.clone(), convert(value)?).is_some() {
                    return Err(YamlError::new(format!(
                        "`{key}` appears twice in one nested mapping, so one value would be lost"
                    )));
                }
            }
            Ok(Value::Map(converted))
        }
        // A tagged node such as `!Foo` is a schema question, not a syntax one, and the schema for
        // each document type arrives in TASK-015. Refusing is the honest response: accepting a tag
        // and ignoring it would mean storing a document's structure and quietly discarding the part
        // that gave it meaning.
        yaml_serde::Value::Tagged(tagged) => Err(YamlError::new(format!(
            "YAML tag `!{}` is not supported in front matter",
            tagged.tag
        ))),
    }
}

/// Converts one of ours into theirs, recursively, for encoding.
fn to_native(value: &Value) -> Result<yaml_serde::Value, YamlError> {
    match value {
        Value::Null => Ok(yaml_serde::Value::Null),
        Value::Bool(inner) => Ok(yaml_serde::Value::Bool(*inner)),
        Value::Int(inner) => Ok(yaml_serde::Value::Number((*inner).into())),
        Value::Float(inner) if inner.is_finite() => Ok(yaml_serde::Value::Number((*inner).into())),
        Value::Float(inner) => Err(YamlError::new(format!(
            "float `{inner}` has no YAML representation, because YAML cannot encode a non-finite \
             number"
        ))),
        Value::Str(inner) => Ok(yaml_serde::Value::String(inner.clone())),
        Value::List(items) => {
            let items = items.iter().map(to_native).collect::<Result<Vec<_>, _>>()?;
            Ok(yaml_serde::Value::Sequence(items))
        }
        Value::Map(entries) => {
            let mut native = yaml_serde::Mapping::new();
            for (key, value) in entries {
                native.insert(yaml_serde::Value::String(key.clone()), to_native(value)?);
            }
            Ok(yaml_serde::Value::Mapping(native))
        }
    }
}

/// Renders a mapping key as the string this crate stores it under.
///
/// YAML permits non-string keys. A front-matter key is a field name, so a numeric, boolean, or null
/// key is refused instead of being stringified: `{1: x}` stored as `"1"` would collide with a
/// genuine `"1"` key and turn an authoring mistake into silent data loss.
fn stringify_key(key: yaml_serde::Value) -> Result<String, YamlError> {
    match key {
        yaml_serde::Value::String(text) => Ok(text),
        other => Err(YamlError::new(format!(
            "a front-matter key is {}, and keys name fields, so they must be strings",
            describe(&other)
        ))),
    }
}

/// Names a value of the foreign tree in words an author would use.
///
/// Used only in error messages, which is why it does not need to handle the mapping case: by the
/// time anything is reported, the shape is already known.
fn describe(value: &yaml_serde::Value) -> String {
    match value {
        yaml_serde::Value::Null => "null".to_string(),
        yaml_serde::Value::Bool(inner) => format!("the boolean `{inner}`"),
        yaml_serde::Value::Number(inner) => format!("the number `{inner}`"),
        yaml_serde::Value::String(inner) => format!("the string `{inner}`"),
        yaml_serde::Value::Sequence(_) => "a list".to_string(),
        yaml_serde::Value::Mapping(_) => "a mapping".to_string(),
        yaml_serde::Value::Tagged(tagged) => format!("the tagged value `!{}`", tagged.tag),
    }
}

#[cfg(test)]
mod tests {
    use super::{SerdeYaml, YamlCodec, YamlError};
    use crate::value::Value;

    fn decode(input: &str) -> Result<Vec<(String, Value)>, YamlError> {
        SerdeYaml.decode_mapping(input)
    }

    #[test]
    fn a_simple_block_decodes_into_our_own_value_type() {
        let decoded = decode("id: TASK-014\nphase: 2\ntags: [a, b]\n").expect("valid front matter");

        assert_eq!(decoded.len(), 3, "one pair per key: {decoded:?}");
        assert_eq!(decoded[0].0, "id");
        assert_eq!(decoded[0].1, Value::Str("TASK-014".into()));
        assert_eq!(decoded[1].1, Value::Int(2));
        assert_eq!(decoded[2].1.as_list().map(<[Value]>::len), Some(2));
    }

    #[test]
    fn keys_come_back_in_the_order_they_were_written() {
        let decoded = decode("zeta: 1\nalpha: 2\nmiddle: 3\n").expect("valid");

        let order: Vec<&str> = decoded.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            order,
            ["zeta", "alpha", "middle"],
            "the writer needs the written order, so a sorted map would lose it"
        );
    }

    #[test]
    fn a_failure_reports_the_line_it_is_on() {
        // `title: a: b` is a mapping value in a place YAML does not allow one, and the layer puts
        // the failure on the line that wrote it.
        let error = decode("id: TASK-014\ntitle: a: b\n").expect_err("not valid YAML");

        assert_eq!(error.line, Some(2), "the second line is the broken one");
        assert!(
            error.to_string().starts_with("line 2: "),
            "the line must lead the message, got: {error}"
        );
    }

    #[test]
    fn a_failure_in_an_unterminated_construct_reports_where_the_scanner_gave_up() {
        // Worth pinning down, because the line is not the line the author must fix: an unclosed
        // quote or bracket is only discovered at the end of the block, so the layer points there.
        // The message still carries the real position ("while scanning a quoted scalar at line 2
        // column 8"), and it is passed through rather than rewritten, so nothing is lost.
        let error = decode("id: TASK-014\ntitle: \"unterminated\n").expect_err("unclosed quote");

        assert_eq!(
            error.line,
            Some(3),
            "the end of the block is where it is noticed"
        );
        assert!(
            error.to_string().contains("line 2 column"),
            "the real position must survive in the message, got: {error}"
        );
    }

    #[test]
    fn a_repeated_key_is_refused_by_the_parser_rather_than_dropped() {
        // Last-wins is what most YAML libraries do, which would quietly discard the first value.
        // Front matter records intent, so a duplicate is refused and named. This is the layer's own
        // check firing; `FrontMatter` reports it more precisely, and this is the backstop for the
        // spellings its scan cannot see.
        let error = decode("id: TASK-014\nid: TASK-015\n").expect_err("a repeated key must fail");

        assert!(
            error.to_string().contains("duplicate"),
            "the error must name the problem, got: {error}"
        );
        assert!(
            error.to_string().contains("id"),
            "the error must name the key, got: {error}"
        );
    }

    #[test]
    fn a_non_mapping_block_is_refused() {
        let error = decode("- one\n- two\n").expect_err("front matter must be a mapping");

        assert!(
            error.to_string().to_lowercase().contains("map"),
            "the error must say a mapping was expected, got: {error}"
        );
    }

    #[test]
    fn a_tagged_value_is_refused_rather_than_half_understood() {
        let error = decode("value: !Custom {a: 1}\n").expect_err("a tag cannot be honoured");

        assert!(
            error.to_string().contains("!Custom"),
            "the error must name the tag, got: {error}"
        );
    }

    #[test]
    fn a_float_is_kept_as_a_float_rather_than_refused_as_an_integer() {
        // `as_i64` returns nothing for a float, so the obvious implementation reports `1.5` as an
        // unrepresentable integer. That would refuse ordinary front matter.
        let decoded = decode("ratio: 1.5\nnegative: -0.25\n").expect("floats are representable");

        assert_eq!(decoded[0].1, Value::Float(1.5));
        assert_eq!(decoded[1].1, Value::Float(-0.25));
    }

    #[test]
    fn a_precise_integer_too_large_for_i64_is_refused_rather_than_rounded() {
        // u64's maximum fits u64 but not i64. Storing it as a float would lose the low digits, so
        // the value would come back as a different number.
        let error = decode("size: 18446744073709551615\n")
            .expect_err("an integer beyond i64 must not be silently made a float");

        assert!(
            error
                .to_string()
                .contains("larger than this crate can represent"),
            "the error must explain the refusal, got: {error}"
        );
    }

    #[test]
    fn a_non_string_key_is_refused_rather_than_stringified() {
        let error = decode("1: one\n").expect_err("a numeric key would collide with \"1\"");

        assert!(
            error.to_string().contains("must be strings"),
            "the error must explain the refusal, got: {error}"
        );
        assert!(
            error.to_string().contains("the number `1`"),
            "the error must name what the key was, got: {error}"
        );
    }

    #[test]
    fn a_repeated_nested_key_is_refused_too() {
        let error =
            decode("outer: {a: 1, a: 2}\n").expect_err("a nested duplicate is still a loss");

        assert!(
            error.to_string().to_lowercase().contains("duplicate"),
            "the error must say the key repeats, got: {error}"
        );
    }

    #[test]
    fn encoding_escapes_what_would_otherwise_break_the_block() {
        // A title holding the fence itself, or a colon, must survive being written back out.
        let entries = vec![
            ("id".to_string(), Value::Str("TASK-014".into())),
            ("title".to_string(), Value::Str("a: b # c".into())),
        ];
        let encoded = SerdeYaml.encode_mapping(&entries).expect("encodable");

        let decoded = decode(&encoded).expect("what we encode, we can read back");
        assert_eq!(decoded[1].1.as_str(), Some("a: b # c"));
    }

    #[test]
    fn encoding_keeps_the_given_order() {
        let entries = vec![
            ("zeta".to_string(), Value::Int(1)),
            ("alpha".to_string(), Value::Int(2)),
        ];
        let encoded = SerdeYaml.encode_mapping(&entries).expect("encodable");

        let decoded = decode(&encoded).expect("re-readable");
        let order: Vec<&str> = decoded.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            order,
            ["zeta", "alpha"],
            "rule 4 order must survive a write"
        );
    }

    #[test]
    fn a_non_finite_float_is_refused_at_encode_time() {
        // YAML cannot represent NaN, so encoding would have to emit something that reads back as a
        // different number. Refusing is the only honest option.
        let entries = vec![("ratio".to_string(), Value::Float(f64::NAN))];
        let error = SerdeYaml
            .encode_mapping(&entries)
            .expect_err("NaN has no encoding");

        assert!(
            error.to_string().contains("non-finite"),
            "the error must explain the refusal, got: {error}"
        );
    }

    #[test]
    fn a_yaml_error_without_a_location_still_renders_readably() {
        assert_eq!(
            YamlError::new("no location").to_string(),
            "no location",
            "a message with no line must not invent one"
        );
        assert_eq!(
            YamlError {
                message: "a message".into(),
                line: Some(7),
            }
            .to_string(),
            "line 7: a message"
        );
    }

    #[test]
    fn a_position_in_the_layers_own_words_is_passed_through_untouched() {
        // The layer's messages carry positions, sometimes more than one, and they are numbered from
        // the start of the block it was handed. Cutting or editing them here would be the wrong
        // place: this module does not know where the block sits in the file, and a position moved
        // by the wrong offset points the author at the wrong line. `ContextError::from_yaml` does
        // the translation, because it is the one that knows.
        let error = decode("id: TASK-014\ntitle: \"unterminated\n").expect_err("unclosed quote");

        assert!(
            error.message.contains("at line 3 column"),
            "the position the layer reported must survive, got: {}",
            error.message
        );
        assert!(
            error.message.contains("line 2 column"),
            "the position the author has to fix is in the prose too, got: {}",
            error.message
        );
    }
}
