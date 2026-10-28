//! The minimal environment a plugin process receives.
//!
//! `docs/PLUGIN_SPEC.md` §7 says a plugin gets only the variables its manifest declares, plus a
//! minimal base set, and `docs/SECURITY.md` §7 says `HOME` is not inherited. The implementation is
//! structural rather than subtractive: the child environment starts empty (`env_clear`) and every
//! variable here is inserted deliberately. A variable the host has set but the manifest did not
//! declare is never read, so it cannot reach the child by inheritance or by an accidental copy.
//!
//! The base set exists to let a normal executable start. `HOME`, `USERPROFILE`, and the temporary
//! directory variables all point inside the per-plugin tree; the Windows system variables name the
//! operating system's own directories, never the user's home. `PATH` is a fixed system search path,
//! not the host's. An auth reference with `source: env` is copied only when it is present, and a
//! reference that names a controlled variable is refused rather than allowed to override the
//! sandbox.
//!
//! Values are kept as [`OsString`]s, so a non-Unicode value is passed through unchanged and no
//! value is ever formatted into an error or an audit record.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use aicontext_plugin_sdk::{AuthSource, PluginManifest};

use crate::sandbox::SandboxError;

/// Variables the sandbox sets and an auth reference may not replace.
const CONTROLLED: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "TMP",
    "TEMP",
    "TMPDIR",
    "PATH",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "PATHEXT",
];

/// The environment a plugin process receives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Environment {
    variables: BTreeMap<String, OsString>,
}

impl Environment {
    /// Build the environment for one plugin.
    pub(crate) fn build(
        manifest: &PluginManifest,
        home: &Path,
        temp: &Path,
    ) -> Result<Self, SandboxError> {
        Self::build_with(manifest, home, temp, |name| std::env::var_os(name))
    }

    fn build_with(
        manifest: &PluginManifest,
        home: &Path,
        temp: &Path,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, SandboxError> {
        let mut variables = base_environment(home, temp);

        if let Some(auth) = &manifest.auth {
            for reference in &auth.references {
                if reference.source != AuthSource::Env {
                    continue;
                }
                if CONTROLLED
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&reference.name))
                {
                    return Err(SandboxError::Environment {
                        plugin: manifest.name.clone(),
                        detail: format!(
                            "`{}` is set by the sandbox and cannot be forwarded as an auth \
                             variable",
                            reference.name
                        ),
                    });
                }
                if let Some(value) = lookup(&reference.name) {
                    variables.insert(reference.name.clone(), value);
                }
            }
        }

        Ok(Self { variables })
    }

    /// One variable, if the sandbox set it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.variables.get(name).map(OsString::as_os_str)
    }

    /// One variable as text, when it is valid Unicode. For tests and diagnostics.
    #[must_use]
    pub fn get_str(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(OsStr::to_str)
    }

    /// Every variable, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &OsStr)> {
        self.variables
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_os_str()))
    }

    /// Apply the environment to a command that is about to be spawned.
    pub(crate) fn apply(&self, command: &mut std::process::Command) {
        command.env_clear();
        for (name, value) in &self.variables {
            command.env(name, value);
        }
    }
}

fn base_environment(home: &Path, temp: &Path) -> BTreeMap<String, OsString> {
    let mut variables = BTreeMap::new();
    let home = path_value(home);
    let temp = path_value(temp);
    for (name, value) in [
        ("HOME", home.as_os_str()),
        ("USERPROFILE", home.as_os_str()),
        ("TMPDIR", temp.as_os_str()),
        ("TMP", temp.as_os_str()),
        ("TEMP", temp.as_os_str()),
    ] {
        variables.insert(name.to_owned(), value.to_os_string());
    }

    #[cfg(windows)]
    {
        let root = system_root();
        variables.insert("SYSTEMROOT".to_owned(), root.as_os_str().to_os_string());
        variables.insert("WINDIR".to_owned(), root.as_os_str().to_os_string());
        let drive = root
            .components()
            .next()
            .map_or_else(|| OsStr::new("C:"), |component| component.as_os_str());
        variables.insert("SYSTEMDRIVE".to_owned(), drive.to_os_string());
        variables.insert("PATHEXT".to_owned(), OsString::from(".COM;.EXE;.BAT;.CMD"));
        let path = std::env::join_paths([root.join("system32"), root])
            .unwrap_or_else(|_| OsString::from(r"C:\Windows\system32;C:\Windows"));
        variables.insert("PATH".to_owned(), path);
    }

    #[cfg(not(windows))]
    {
        variables.insert(
            "PATH".to_owned(),
            OsString::from("/usr/local/bin:/usr/bin:/bin"),
        );
    }

    variables
}

#[cfg(windows)]
fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
}

/// A canonical host path rendered the way an ordinary Windows program expects it.
///
/// `fs::canonicalize` returns a verbatim path (`\\?\C:\...`) on Windows. Most programs understand
/// it, but some older runtimes do not, so `HOME` and the temporary variables use the ordinary
/// spelling. The verbatim prefix carries no security meaning here: the path has already been
/// canonicalised and checked, and this only changes how it is written.
fn path_value(path: &Path) -> OsString {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return OsString::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return OsString::from(rest);
        }
        path.as_os_str().to_os_string()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().to_os_string()
    }
}

#[cfg(test)]
mod tests {
    use aicontext_plugin_sdk::{
        Auth, AuthKind, AuthReference, AuthSource, Capability, Effect, EntryPoint, FilesystemScope,
        Limits, NetworkScope, PluginManifest, PluginProtocol,
    };

    use super::Environment;
    use crate::sandbox::SandboxError;

    fn manifest(auth: Option<Auth>) -> PluginManifest {
        PluginManifest {
            manifest_version: 1,
            plugin_api_version: "1".to_owned(),
            protocol: PluginProtocol::AiPlugin,
            name: "fixture".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            vendor: None,
            license: None,
            entry: EntryPoint {
                command: "fixture".to_owned(),
                args: Vec::new(),
            },
            minimum_aicontext_version: None,
            capabilities: vec![Capability {
                tool: "fixture.echo.run".to_owned(),
                effect: Effect::Read,
                destructive: false,
                mcp_tool: None,
            }],
            auth,
            filesystem: FilesystemScope {
                read: Vec::new(),
                write: Vec::new(),
            },
            network: NetworkScope { egress: Vec::new() },
            limits: Limits {
                call_timeout_ms: 1_000,
                max_output_bytes: 1_024,
            },
        }
    }

    fn reference(name: &str, source: AuthSource) -> AuthReference {
        AuthReference {
            name: name.to_owned(),
            source,
        }
    }

    #[test]
    fn home_and_temp_point_into_the_sandbox() {
        let home = std::path::Path::new("/sandbox/home");
        let temp = std::path::Path::new("/sandbox/tmp");
        let environment =
            Environment::build_with(&manifest(None), home, temp, |_| None).expect("base only");

        assert_eq!(environment.get_str("HOME"), Some("/sandbox/home"));
        assert_eq!(environment.get_str("USERPROFILE"), Some("/sandbox/home"));
        assert_eq!(environment.get_str("TMPDIR"), Some("/sandbox/tmp"));
        assert_eq!(environment.get_str("TMP"), Some("/sandbox/tmp"));
        assert_eq!(environment.get_str("TEMP"), Some("/sandbox/tmp"));
        assert!(environment.get_str("PATH").is_some());
    }

    #[test]
    fn an_undeclared_variable_is_never_looked_up() {
        let environment = Environment::build_with(
            &manifest(None),
            std::path::Path::new("/sandbox/home"),
            std::path::Path::new("/sandbox/tmp"),
            |name| panic!("the sandbox read the ambient variable `{name}`"),
        )
        .expect("base only");
        assert!(environment.get_str("AICONTEXT_SANDBOX_AMBIENT").is_none());
    }

    #[test]
    fn only_declared_environment_references_are_copied() {
        let auth = Auth {
            kind: AuthKind::EnvOrCredentialCommand,
            references: vec![
                reference("DECLARED", AuthSource::Env),
                reference("MISSING", AuthSource::Env),
                reference("COMMAND", AuthSource::CredentialCommand),
            ],
            instructions: None,
        };
        let environment = Environment::build_with(
            &manifest(Some(auth)),
            std::path::Path::new("/sandbox/home"),
            std::path::Path::new("/sandbox/tmp"),
            |name| match name {
                "DECLARED" => Some("forwarded".into()),
                "COMMAND" => Some("must not be copied".into()),
                _ => None,
            },
        )
        .expect("the references are valid");

        assert_eq!(environment.get_str("DECLARED"), Some("forwarded"));
        assert!(environment.get_str("MISSING").is_none());
        assert!(environment.get_str("COMMAND").is_none());
        assert!(environment.get_str("UNDECLARED").is_none());
    }

    #[test]
    fn a_controlled_variable_cannot_be_declared_as_auth() {
        for name in ["HOME", "home", "PATH", "Tmp", "SystemRoot"] {
            let auth = Auth {
                kind: AuthKind::EnvOrCredentialCommand,
                references: vec![reference(name, AuthSource::Env)],
                instructions: None,
            };
            let error = Environment::build_with(
                &manifest(Some(auth)),
                std::path::Path::new("/sandbox/home"),
                std::path::Path::new("/sandbox/tmp"),
                |_| Some("host value".into()),
            )
            .expect_err("the sandbox owns this name");
            assert_eq!(
                error.code(),
                aicontext_core::ErrorCode::PLG_011,
                "`{name}`: {error}"
            );
            assert!(
                matches!(error, SandboxError::Environment { .. }),
                "`{name}`: {error}"
            );
            assert!(!error.to_string().contains("host value"), "{error}");
        }
    }

    #[test]
    fn the_environment_is_deterministic() {
        let home = std::path::Path::new("/sandbox/home");
        let temp = std::path::Path::new("/sandbox/tmp");
        let first =
            Environment::build_with(&manifest(None), home, temp, |_| None).expect("base only");
        let second =
            Environment::build_with(&manifest(None), home, temp, |_| None).expect("base only");
        assert_eq!(first, second);
        assert_eq!(
            first.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            second.iter().map(|(name, _)| name).collect::<Vec<_>>()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_are_rendered_without_the_verbatim_prefix() {
        use std::path::Path;

        use super::path_value;

        assert_eq!(
            path_value(Path::new(r"\\?\C:\sandbox\home")),
            std::ffi::OsString::from(r"C:\sandbox\home")
        );
        assert_eq!(
            path_value(Path::new(r"\\?\UNC\server\share\sandbox")),
            std::ffi::OsString::from(r"\\server\share\sandbox")
        );
        assert_eq!(
            path_value(Path::new(r"C:\sandbox\home")),
            std::ffi::OsString::from(r"C:\sandbox\home")
        );
    }
}
