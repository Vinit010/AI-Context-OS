//! The network egress allowlist.
//!
//! `docs/PLUGIN_SPEC.md` §7 and `docs/SECURITY.md` §7 make an empty list mean "no network" and a
//! non-empty list an explicit, host-level allowlist. The runtime stores only bare, lower-case DNS
//! hostnames: a value with a scheme, port, path, wildcard, whitespace, or trailing dot is refused,
//! because a parser that accepted `https://host/path` would be making a URL policy look like a host
//! policy and could be confused about which part it actually checked.
//!
//! The check is used by every host-mediated network operation. A plugin that contacts a host the
//! manifest did not declare is [`SandboxError::Denied`], not a generic I/O failure, so the gateway
//! can audit the attempt as a sandbox violation.

use std::collections::BTreeSet;

use crate::sandbox::SandboxError;

/// What a plugin may reach on the network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkPolicy {
    /// The manifest declared no egress. Every host is refused.
    DenyAll,
    /// Only these hosts, lower-case and sorted.
    Allow(BTreeSet<String>),
}

impl NetworkPolicy {
    /// Validate the manifest's `network.egress` list.
    pub(crate) fn build(plugin: &str, egress: &[String]) -> Result<Self, SandboxError> {
        if egress.is_empty() {
            return Ok(Self::DenyAll);
        }

        let mut hosts = BTreeSet::new();
        for raw in egress {
            let host = validate_declared(plugin, raw)?;
            hosts.insert(host);
        }
        Ok(Self::Allow(hosts))
    }

    /// Whether this policy refuses every host.
    #[must_use]
    pub const fn is_deny_all(&self) -> bool {
        matches!(self, Self::DenyAll)
    }

    /// The declared hosts, in order. Empty for [`NetworkPolicy::DenyAll`].
    pub fn hosts(&self) -> impl Iterator<Item = &str> {
        let hosts = match self {
            Self::DenyAll => None,
            Self::Allow(hosts) => Some(hosts),
        };
        hosts
            .into_iter()
            .flat_map(|hosts| hosts.iter().map(String::as_str))
    }

    /// Refuse `host` unless it is one of the declared hosts.
    ///
    /// The request is normalized the same way a declaration is, so `Example.COM` matches
    /// `example.com`. A malformed request is a denial rather than an error: the plugin supplied it,
    /// and the host was never going to be contacted.
    ///
    /// # Errors
    ///
    /// [`SandboxError::Denied`] when the policy is [`NetworkPolicy::DenyAll`] or the host is not in
    /// the allowlist.
    pub fn check(&self, plugin: &str, host: &str) -> Result<(), SandboxError> {
        let normalized = normalize(host).ok_or_else(|| SandboxError::Denied {
            plugin: plugin.to_owned(),
            operation: "network",
            target: sanitize(host),
        })?;
        match self {
            Self::Allow(hosts) if hosts.contains(&normalized) => Ok(()),
            Self::DenyAll | Self::Allow(_) => Err(SandboxError::Denied {
                plugin: plugin.to_owned(),
                operation: "network",
                target: normalized,
            }),
        }
    }

    /// Whether [`NetworkPolicy::check`] would accept `host`.
    #[must_use]
    pub fn allows(&self, host: &str) -> bool {
        self.check("", host).is_ok()
    }
}

/// Validate one manifest entry, naming the value in the error.
fn validate_declared(plugin: &str, raw: &str) -> Result<String, SandboxError> {
    normalize(raw).ok_or_else(|| SandboxError::InvalidHost {
        plugin: plugin.to_owned(),
        detail: format!(
            "`{}` is not a bare DNS hostname: no scheme, port, path, wildcard, whitespace, \
             trailing dot, or non-ASCII character",
            sanitize(raw)
        ),
    })
}

/// The shape a host must have. `None` means the value is not a bare DNS hostname.
fn normalize(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > 253 || !raw.is_ascii() {
        return None;
    }
    // A trailing dot is a valid absolute DNS name, but two spellings of one host in an allowlist is
    // exactly the ambiguity a security check must not have. Refuse it.
    if raw.starts_with('.') || raw.ends_with('.') {
        return None;
    }

    let host = raw.to_ascii_lowercase();
    if host.contains("..") {
        return None;
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        let bytes = label.as_bytes();
        if !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        {
            return None;
        }
        if bytes.first() == Some(&b'-') || bytes.last() == Some(&b'-') {
            return None;
        }
    }
    Some(host)
}

/// Replace control characters so the value cannot forge a line in a prompt or an audit record.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{FFFD}'
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{NetworkPolicy, normalize};

    fn policy(egress: &[&str]) -> NetworkPolicy {
        let entries: Vec<String> = egress.iter().map(|host| (*host).to_owned()).collect();
        NetworkPolicy::build("fixture", &entries).expect("the fixture declares valid hosts")
    }

    #[test]
    fn an_empty_list_denies_every_host() {
        let policy = policy(&[]);
        assert!(policy.is_deny_all());
        assert_eq!(policy.hosts().count(), 0);
        let error = policy
            .check("fixture", "api.example.com")
            .expect_err("an empty list means no network");
        assert!(error.to_string().contains("api.example.com"));
    }

    #[test]
    fn only_declared_hosts_are_allowed_and_case_does_not_matter() {
        let policy = policy(&["API.Example.com", "api.example.com", "s3.example.com"]);
        assert!(!policy.is_deny_all());
        assert_eq!(
            policy.hosts().collect::<Vec<_>>(),
            ["api.example.com", "s3.example.com"]
        );
        assert!(policy.allows("api.example.com"));
        assert!(policy.allows("API.EXAMPLE.COM"));
        assert!(!policy.allows("evil.example.com"));
        assert!(!policy.allows("example.com"));
        assert!(!policy.allows("api.example.com.evil"));
    }

    #[test]
    fn a_request_that_is_not_a_hostname_is_denied_without_echoing_controls() {
        let policy = policy(&["api.example.com"]);
        let error = policy
            .check("fixture", "api.example.com\nforged")
            .expect_err("a malformed request is denied");
        assert!(!error.to_string().contains('\n'));
    }

    #[test]
    fn the_declared_shape_is_a_bare_dns_hostname() {
        for good in [
            "api.example.com",
            "localhost",
            "127.0.0.1",
            "a-b.c-d.example",
            "xn--bcher-kva.example",
        ] {
            assert_eq!(normalize(good).as_deref(), Some(good), "{good}");
        }

        for bad in [
            "",
            "https://api.example.com",
            "api.example.com:443",
            "api.example.com/path",
            "*.example.com",
            "api example.com",
            "api.example.com.",
            ".api.example.com",
            "api..example.com",
            "-api.example.com",
            "api-.example.com",
            "api.example.com\n",
            "bücher.example",
            "api.example.com\t",
        ] {
            assert_eq!(normalize(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn an_invalid_declared_host_is_a_manifest_error() {
        let error = NetworkPolicy::build("fixture", &["https://api.example.com".to_owned()])
            .expect_err("a URL is not a host");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_011, "{error}");
        assert!(error.to_string().contains("fixture"));
    }

    #[test]
    fn the_policy_is_deterministic() {
        let first = policy(&["b.example", "a.example", "b.example"]);
        let second = policy(&["a.example", "b.example"]);
        assert_eq!(first, second);
        assert_eq!(
            first.hosts().collect::<Vec<_>>(),
            second.hosts().collect::<Vec<_>>()
        );
    }
}
