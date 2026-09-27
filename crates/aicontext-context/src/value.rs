//! The value model this crate exposes, independent of whichever YAML crate parses it.
//!
//! [`Value`] is a small, closed enum: a null, a boolean, an integer, a float, a string, a list, or
//! a map. Every YAML value a front-matter block can hold maps onto exactly one of these, and
//! nothing from the YAML crate appears in a public signature. That is the whole point, and it is
//! the mitigation for `RISK R-1` in `ARCHITECTURE.md` §13: the storage contract is expressed in
//! terms of this type, so replacing the parser is a change to one module rather than a rewrite of
//! everything that reads a document.
//!
//! # Why maps are sorted
//!
//! A map is a [`BTreeMap`], not an insertion-ordered map. Two consequences, both wanted:
//!
//! 1. **Round-trips are canonical.** Re-rendering a document emits keys in a fixed order, so a
//!    rewrite produces a minimal diff no matter how a human ordered the file. Order is not
//!    information in this format: `docs/CONTEXT_SPEC.md` §2 rule 4 prescribes an order for the
//!    well-known keys, and [`FrontMatter`](crate::FrontMatter) honours it explicitly.
//! 2. **Equality is order-insensitive.** Two documents with the same keys and values compare
//!    equal, so the round-trip test can assert on values instead of on formatting.
//!
//! Integers are [`i64`] and not `u64`: a value above `i64::MAX` is reported as an error rather
//! than silently rounded into a float, because front matter holds identifiers and sizes, and a
//! quietly imprecise number in a storage contract is worse than a refused one.

use std::collections::BTreeMap;
use std::fmt;

/// A YAML value, in the subset that front matter needs.
///
/// `#[non_exhaustive]` on purpose, and it is worth being explicit about what that buys, because it
/// also means a downstream crate cannot *construct* a variant. The only way to obtain a [`Value`] is
/// to read one out of a parsed document. That is the invariant we want: the set of values in memory
/// is exactly the set the parser can produce, so a value that no block could ever have contained
/// cannot turn up in one, and a later variant is additive rather than a breaking change
/// (`RULES.md` §3).
///
/// The cost is that a caller who needs to build a value - a future command that edits one field -
/// has to be given a way to do it, and this task deliberately does not invent one (`RULES.md` §2).
/// Nothing needs it yet: documents are read here and nowhere written.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// An explicit `null`, or an empty value such as `key:` with nothing after it.
    ///
    /// Kept distinct from "key absent". `spec: null` states that the document has no
    /// specification; omitting the key states nothing at all, and `doctor` treats the two
    /// differently (`docs/CONTEXT_SPEC.md` §2 rule 9).
    Null,
    /// A YAML boolean.
    Bool(bool),
    /// A whole number within [`i64`].
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// A string. Dates arrive here: YAML has no date type, and `created: 2026-09-27` is a string
    /// that this crate validates against ISO-8601 rather than trusting.
    Str(String),
    /// An ordered sequence.
    List(Vec<Value>),
    /// A mapping, with keys in sorted order.
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// The YAML type name, for an error message that tells the author what they actually wrote.
    ///
    /// # Example
    ///
    /// ```
    /// use aicontext_context::Document;
    ///
    /// // A value is read out of a document rather than built: `Value` is `#[non_exhaustive]`, so
    /// // the parser is the only source of one.
    /// let document = Document::parse("---\nphase: 1\n---\n").expect("a well-formed document");
    /// let phase = document.front_matter().expect("has a block").extra("phase").expect("has phase");
    ///
    /// assert_eq!(phase.type_name(), "an integer");
    /// ```
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        // `#[non_exhaustive]` is not `#[non_exhaustive]`-exhaustive: within the defining crate a
        // match on a public enum is still exhaustive, so a new variant is caught here at compile
        // time rather than silently falling through to a wrong name.
        match self {
            Self::Null => "null",
            Self::Bool(_) => "a boolean",
            Self::Int(_) => "an integer",
            Self::Float(_) => "a float",
            Self::Str(_) => "a string",
            Self::List(_) => "a list",
            Self::Map(_) => "a mapping",
        }
    }

    /// The string, if this is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text),
            _ => None,
        }
    }

    /// The boolean, if this is one.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The integer, if this is one. A float is not an integer, and is not widened into one.
    #[must_use]
    pub const fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    /// The list, if this is one.
    #[must_use]
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    /// The map, if this is one.
    #[must_use]
    pub const fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Self::Map(entries) => Some(entries),
            _ => None,
        }
    }

    /// The value at `key`, if this is a map that has one.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        self.as_map()?.get(key)
    }
}

impl fmt::Display for Value {
    /// Renders the value in YAML syntax, for an error message that shows the author the value.
    ///
    /// Deliberately naive about quoting: this is diagnostic output, not a serialiser. A value that
    /// needs quoting to round-trip is written by the crate's YAML codec, never by this.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => formatter.write_str("null"),
            Self::Bool(value) => write!(formatter, "{value}"),
            Self::Int(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Str(text) => formatter.write_str(text),
            Self::List(items) => {
                formatter.write_str("[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{item}")?;
                }
                formatter.write_str("]")
            }
            Self::Map(entries) => {
                formatter.write_str("{")?;
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{key}: {value}")?;
                }
                formatter.write_str("}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::Value;

    #[test]
    fn a_value_reports_the_type_name_an_author_would_recognise() {
        assert_eq!(Value::Null.type_name(), "null");
        assert_eq!(Value::Bool(true).type_name(), "a boolean");
        assert_eq!(Value::Str("x".into()).type_name(), "a string");
        assert_eq!(Value::List(vec![]).type_name(), "a list");
        assert_eq!(Value::Map(BTreeMap::new()).type_name(), "a mapping");
    }

    #[test]
    fn accessors_do_not_widen_across_types() {
        // A float is not an integer. Coercing here would let `phase: 1.5` satisfy an integer
        // field, and the schema in TASK-015 would never see it.
        assert_eq!(Value::Float(1.0).as_int(), None);
        assert_eq!(Value::Int(1).as_str(), None);
        assert_eq!(Value::Null.as_bool(), None);
    }

    #[test]
    fn a_map_is_read_through_its_own_type_only() {
        let mut entries = BTreeMap::new();
        entries.insert("id".to_string(), Value::Str("TASK-014".into()));
        let value = Value::Map(entries);

        assert_eq!(value.get("id").and_then(Value::as_str), Some("TASK-014"));
        assert_eq!(value.get("missing"), None);
        assert_eq!(Value::Str("x".into()).get("id"), None);
    }

    #[test]
    fn display_renders_a_shape_an_author_can_match_against_their_file() {
        let mut entries = BTreeMap::new();
        entries.insert("id".to_string(), Value::Str("TASK-014".into()));
        entries.insert("phase".to_string(), Value::Int(2));

        assert_eq!(Value::Map(entries).to_string(), "{id: TASK-014, phase: 2}");
        assert_eq!(
            Value::List(vec![Value::Int(1), Value::Null]).to_string(),
            "[1, null]"
        );
    }
}
