//! Plugin API version parsing and negotiation.
//!
//! `docs/PLUGIN_SPEC.md` §2 rule 8: `plugin_api_version` is negotiated at handshake, and a
//! mismatch fails with `PLG-004` while stating both versions. The version is deliberately a
//! decimal string rather than a `const` in the manifest schema: a plugin built for a newer version
//! must reach negotiation so it can be told which version the host supports, instead of failing
//! with a schema error that names no alternative.
//!
//! The host currently implements one version. [`ApiVersion::SUPPORTED`] is the single place to
//! widen that set; `negotiate` is exact-match for now and takes the host version as a parameter so
//! a future multi-version host does not have to change the plugin-side API.

use std::fmt;
use std::str::FromStr;

use crate::error::PluginError;

/// The plugin API version this SDK and host implement, as it appears in a manifest.
pub const PLUGIN_API_VERSION: &str = "1";

/// A positive plugin API version.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ApiVersion(u16);

impl ApiVersion {
    /// The version a manifest must declare to run on this host.
    pub const CURRENT: Self = Self(1);

    /// Every version this host implements, in ascending order.
    ///
    /// Kept as a slice rather than a single constant so a future host can accept more than one
    /// version without changing the shape of the public API.
    pub const SUPPORTED: &'static [Self] = &[Self::CURRENT];

    /// A version, or `None` for zero.
    ///
    /// Zero is not a version: `docs/PLUGIN_SPEC.md` numbers versions from 1, and a manifest that
    /// writes `0` is a typo rather than a pre-release.
    #[must_use]
    pub const fn new(number: u16) -> Option<Self> {
        if number == 0 {
            None
        } else {
            Some(Self(number))
        }
    }

    /// The number as written in the manifest.
    #[must_use]
    pub const fn number(self) -> u16 {
        self.0
    }

    /// Whether this host implements this version.
    #[must_use]
    pub fn is_supported(self) -> bool {
        Self::SUPPORTED.contains(&self)
    }

    /// Parse the manifest's decimal string.
    ///
    /// Only a single decimal number is accepted: no sign, no whitespace, no leading zero, and no
    /// dotted `1.0`. The plugin API version is not the plugin's `SemVer` `version`, and accepting a
    /// dotted value here would make two spellings mean one version.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidApiVersion`] with the rejected value and the host's version.
    pub fn parse(raw: &str) -> Result<Self, PluginError> {
        let invalid = || PluginError::InvalidApiVersion {
            value: raw.to_owned(),
            supported: Self::CURRENT,
        };

        if raw.is_empty() || (raw.len() > 1 && raw.starts_with('0')) {
            return Err(invalid());
        }
        if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }

        let number: u16 = raw.parse().map_err(|_| invalid())?;
        Self::new(number).ok_or_else(invalid)
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl FromStr for ApiVersion {
    type Err = PluginError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

/// Check that a plugin's version is one the host implements.
///
/// # Errors
///
/// Returns [`PluginError::ApiVersionMismatch`] naming both versions when they differ, which is the
/// `PLG-004` behaviour `docs/PLUGIN_SPEC.md` §2 rule 8 requires.
pub fn negotiate(plugin: ApiVersion, host: ApiVersion) -> Result<ApiVersion, PluginError> {
    if plugin == host {
        Ok(plugin)
    } else {
        Err(PluginError::ApiVersionMismatch { plugin, host })
    }
}

/// Parse and negotiate in one step, for a manifest's raw `plugin_api_version` string.
///
/// # Errors
///
/// Returns [`PluginError::InvalidApiVersion`] for a malformed value and
/// [`PluginError::ApiVersionMismatch`] for a valid version the host does not implement.
pub fn negotiate_raw(
    plugin_api_version: &str,
    host: ApiVersion,
) -> Result<ApiVersion, PluginError> {
    let plugin = ApiVersion::parse(plugin_api_version)?;
    negotiate(plugin, host)
}

#[cfg(test)]
mod tests {
    use aicontext_core::ErrorCode;

    use super::{ApiVersion, PLUGIN_API_VERSION, negotiate, negotiate_raw};
    use crate::error::PluginError;

    #[test]
    fn the_current_version_is_the_documented_one() {
        assert_eq!(PLUGIN_API_VERSION, "1");
        assert_eq!(ApiVersion::CURRENT.to_string(), PLUGIN_API_VERSION);
        assert!(ApiVersion::CURRENT.is_supported());
    }

    #[test]
    fn a_decimal_version_round_trips() {
        for raw in ["1", "2", "65535"] {
            let version = ApiVersion::parse(raw).expect("a decimal version");
            assert_eq!(version.to_string(), raw);
        }
    }

    #[test]
    fn malformed_versions_are_rejected_by_name() {
        for raw in ["", "0", "01", "-1", " 1", "1 ", "1.0", "one"] {
            let error = ApiVersion::parse(raw).expect_err("must reject");
            assert_eq!(error.code(), ErrorCode::PLG_011, "raw = {raw:?}");
            assert!(
                error.to_string().contains(&format!("`{raw}`")),
                "the message must quote the rejected value: {error}"
            );
        }
    }

    #[test]
    fn the_current_version_negotiates() {
        assert_eq!(
            negotiate(ApiVersion::CURRENT, ApiVersion::CURRENT).expect("same version"),
            ApiVersion::CURRENT
        );
    }

    #[test]
    fn a_mismatch_is_plg_004_and_states_both_versions() {
        let plugin = ApiVersion::new(2).expect("2 is valid");
        let error = negotiate(plugin, ApiVersion::CURRENT).expect_err("must reject");

        assert_eq!(error.code(), ErrorCode::PLG_004);
        assert!(matches!(
            error,
            PluginError::ApiVersionMismatch { plugin: p, host: h }
                if p == plugin && h == ApiVersion::CURRENT
        ));
        let message = error.to_string();
        assert!(message.contains("plugin API version 2"), "{message}");
        assert!(message.contains("version 1"), "{message}");
    }

    #[test]
    fn negotiate_raw_parses_first_and_names_the_rejected_value() {
        let error = negotiate_raw("not-a-version", ApiVersion::CURRENT).expect_err("must reject");
        assert_eq!(error.code(), ErrorCode::PLG_011);
        assert!(error.to_string().contains("not-a-version"));
    }
}
