//! A wrapper for a value that must never be printed.
//!
//! `docs/AI_PROVIDER_SPEC.md` §11 T10 requires that no credential appears in any error, log, or usage
//! record, and an adapter that holds a credential in a plain `String` cannot make that promise: a
//! `#[derive(Debug)]` three types up will print it. [`Secret`] makes the safe rendering the only one
//! that exists.
//!
//! This is the minimal piece `TASK-051` needs. `TASK-052` adds the resolver that decides *which*
//! secret to use and the redaction at the logging boundary; it builds on this wrapper rather than
//! replacing it.

use std::fmt;

/// A value — today, a credential — that renders as `[redacted]` in every `Debug` and `Display`.
///
/// The only way to read it back is [`Secret::expose`], which is deliberately a named call rather
/// than a `Deref`, so "where can this value escape?" is answerable by reading the call sites.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(String);

impl Secret {
    /// Wraps `value`.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that builds an `Authorization` header.
    ///
    /// Every caller of this method is a place a credential can leak, which is the point of making it
    /// explicit.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the wrapped value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret([redacted])")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn a_secret_never_renders_its_value() {
        let secret = Secret::new("sk-live-do-not-print");
        for rendered in [format!("{secret}"), format!("{secret:?}")] {
            assert!(
                !rendered.contains("sk-live-do-not-print"),
                "the value leaked: {rendered}"
            );
            assert!(
                rendered.contains("redacted"),
                "the reader must be told it was redacted: {rendered}"
            );
        }
    }

    #[test]
    fn a_secret_nested_in_a_derived_debug_stays_redacted() {
        // The real failure mode: not `Secret` itself, but a config that derives Debug.
        #[derive(Debug)]
        struct Holder {
            secret: Secret,
        }
        let holder = Holder {
            secret: Secret::new("sk-live-do-not-print"),
        };
        let rendered = format!("{holder:?}");
        assert!(!rendered.contains("sk-live-do-not-print"), "{rendered}");
    }

    #[test]
    fn the_value_is_reachable_only_by_asking() {
        assert_eq!(Secret::new("token").expose(), "token");
        assert!(Secret::from("").is_empty());
        assert!(!Secret::from("x").is_empty());
    }
}
