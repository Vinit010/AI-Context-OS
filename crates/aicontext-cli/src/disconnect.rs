//! `aicontext disconnect`: remove a committed provider reference and the credential it named.
//!
//! `docs/AI_PROVIDER_SPEC.md` §4 is one sentence long — *`disconnect` removes the reference and
//! leaves no residue* — and the sentence is the contract. Residue would be any of three things: the
//! provider entry left in `.ai/ai.yaml`, its `default_provider` left dangling, or the credential left
//! in the OS keychain. This command removes all three, and removes the file's copy first: if the
//! write fails, the keychain entry survives and reconnecting restores a working profile, which is the
//! recoverable direction to fail in.
//!
//! The last provider is the one case where the file cannot be written at all:
//! `schemas/ai-config.schema.json` requires `providers` to hold at least one item, so an emptied list
//! would be a document that fails the schema other tools read it by. Since `init` never creates this
//! file, the state a project with no providers is *in* is "no `.ai/ai.yaml`", and removing the file is
//! how `disconnect` returns to it.
//!
//! The two writes cannot be made atomic, so the order is chosen rather than assumed. What is *not*
//! negotiable is the error path: an id the file does not list is a failure that names the ids it does,
//! never a silent success (`RULES.md` §4.10).

use std::path::Path;

use aicontext_providers::{Keychain, KeychainReference, KeyringKeychain};
use serde::Serialize;

use crate::args::{DisconnectArgs, GlobalArgs};
use crate::connect::{AiConfig, ConnectError, KEYCHAIN_SERVICE, ProviderEntry};
use crate::exit::Exit;
use crate::output::{Terminal, envelope};
use crate::project;

/// Runs `disconnect` and returns the exit code.
pub(crate) fn run(args: &DisconnectArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return ConnectError::from(error).report(terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return ConnectError::from(error).report(terminal),
    };

    let keychain = KeyringKeychain;
    match execute(args, &root, &keychain) {
        Ok(outcome) => {
            if global.json {
                report_json(&outcome, terminal)
            } else {
                print_human(&outcome, &name, &root, terminal);
                Exit::Ok
            }
        }
        Err(error) => error.report(terminal),
    }
}

/// The whole command, with the keychain port supplied. `run` wraps this with the real one.
fn execute(
    args: &DisconnectArgs,
    root: &Path,
    keychain: &dyn Keychain,
) -> Result<Outcome, ConnectError> {
    let mut config = AiConfig::load(root)?;
    let config_path = config.display();

    let known = config.provider_ids()?;
    let Some(entry) = config.remove(&args.provider)? else {
        return Err(ConnectError::UnknownProvider {
            id: args.provider.clone(),
            path: config_path,
            known: known_hint(&known),
        });
    };
    config.clear_default(&args.provider)?;

    let remaining = config.provider_ids()?.len();
    let config_deleted = if remaining == 0 {
        config.discard()?;
        true
    } else {
        config.save()?;
        false
    };

    let keychain_deleted = delete_credential(keychain, &entry)?;

    Ok(Outcome {
        provider: entry.id.clone(),
        config_path,
        config_deleted,
        keychain_entry: entry.auth.keychain_entry.clone(),
        keychain_deleted,
        remaining,
    })
}

/// The list a "no such provider" error shows, or `none` when the file lists nothing.
fn known_hint(ids: &[String]) -> String {
    if ids.is_empty() {
        "none".to_string()
    } else {
        ids.join(", ")
    }
}

/// Removes the keychain entry the provider named, answering whether one was there.
fn delete_credential(keychain: &dyn Keychain, entry: &ProviderEntry) -> Result<bool, ConnectError> {
    let Some(account) = entry.auth.keychain_entry.as_ref() else {
        return Ok(false);
    };
    let reference = KeychainReference::new(KEYCHAIN_SERVICE, account.clone());
    Ok(keychain.delete(&reference)?)
}

/// What a successful `disconnect` produced.
#[derive(Debug)]
struct Outcome {
    provider: String,
    config_path: String,
    keychain_entry: Option<String>,
    keychain_deleted: bool,
    remaining: usize,
}

/// The `--json` payload.
#[derive(Debug, Serialize)]
struct DisconnectData {
    provider: String,
    config_path: String,
    /// The keychain entry the profile named, when it named one.
    keychain_entry: Option<String>,
    /// Whether an entry was actually removed from the OS keychain.
    keychain_deleted: bool,
    /// How many providers the file still lists.
    remaining_providers: usize,
}

/// Writes the JSON envelope for a successful run.
fn report_json(outcome: &Outcome, terminal: &mut Terminal) -> Exit {
    let data = DisconnectData {
        provider: outcome.provider.clone(),
        config_path: outcome.config_path.clone(),
        keychain_entry: outcome.keychain_entry.clone(),
        keychain_deleted: outcome.keychain_deleted,
        remaining_providers: outcome.remaining,
    };
    let body = envelope(
        "disconnect",
        Exit::Ok.code(),
        data,
        Vec::new(),
        format!("provider `{}` removed", outcome.provider),
    );
    if let Err(error) = terminal.json(&body) {
        terminal.error_report(
            "CONNECT-011",
            &format!("the JSON report could not be written: {error}"),
            "this is a defect in aicontext; please report it",
        );
        return Exit::General;
    }
    Exit::Ok
}

/// Prints the human report.
fn print_human(outcome: &Outcome, name: &str, root: &Path, terminal: &mut Terminal) {
    let info = terminal.mark_info();
    let ok = terminal.mark_ok();

    terminal.identity("aicontext", name, project::vcs_label(root));
    terminal.row(&ok, "removed", &format!("provider `{}`", outcome.provider));
    match (&outcome.keychain_entry, outcome.keychain_deleted) {
        (Some(entry), true) => terminal.row(&ok, "keychain", &format!("entry `{entry}` cleared")),
        (Some(entry), false) => terminal.row(
            &info,
            "keychain",
            &format!("no entry `{entry}` to clear"),
        ),
        (None, _) => terminal.row(&info, "keychain", "no credential was recorded"),
    }
    terminal.summary(
        &format!("{} provider(s) remain", outcome.remaining),
        Exit::Ok,
        "`aicontext connect --provider <id>`",
    );
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use aicontext_providers::{Keychain, KeychainError, KeychainReference, Secret};

    use super::{execute, known_hint};
    use crate::args::DisconnectArgs;
    use crate::connect::ConnectError;

    #[derive(Debug, Default)]
    struct FakeKeychain {
        entries: Mutex<BTreeMap<(String, String), String>>,
    }

    impl FakeKeychain {
        fn stored(&self, service: &str, account: &str) -> Option<String> {
            self.entries
                .lock()
                .expect("lock")
                .get(&(service.to_string(), account.to_string()))
                .cloned()
        }
    }

    impl Keychain for FakeKeychain {
        fn get(&self, reference: &KeychainReference) -> Result<Option<Secret>, KeychainError> {
            Ok(self
                .entries
                .lock()
                .expect("lock")
                .get(&(reference.service().to_string(), reference.account().to_string()))
                .map(|value| Secret::new(value.clone())))
        }

        fn set(&self, reference: &KeychainReference, secret: &Secret) -> Result<(), KeychainError> {
            self.entries.lock().expect("lock").insert(
                (reference.service().to_string(), reference.account().to_string()),
                secret.expose().to_string(),
            );
            Ok(())
        }

        fn delete(&self, reference: &KeychainReference) -> Result<bool, KeychainError> {
            Ok(self
                .entries
                .lock()
                .expect("lock")
                .remove(&(
                    reference.service().to_string(),
                    reference.account().to_string(),
                ))
                .is_some())
        }
    }

    fn write_ai(root: &std::path::Path, text: &str) {
        std::fs::create_dir_all(root.join(".ai")).expect("create .ai");
        std::fs::write(root.join(".ai/ai.yaml"), text).expect("write ai.yaml");
    }

    fn args(provider: &str) -> DisconnectArgs {
        DisconnectArgs {
            provider: provider.to_string(),
        }
    }

    fn file() -> String {
        "version: 1\ndefault_provider: hosted\nproviders:\n  - id: hosted\n    adapter: openai-compatible\n    auth:\n      type: keychain\n      keychain_entry: hosted\n  - id: local\n    adapter: openai-compatible\n    auth:\n      type: none\n".to_string()
    }

    #[test]
    fn removing_a_provider_deletes_its_keychain_entry_and_leaves_the_rest() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_ai(temp.path(), &file());
        let keychain = FakeKeychain {
            entries: Mutex::new(BTreeMap::from([(
                ("aicontext".to_string(), "hosted".to_string()),
                "secret".to_string(),
            )])),
        };

        let outcome = execute(&args("hosted"), temp.path(), &keychain).expect("disconnect");
        assert!(outcome.keychain_deleted);
        assert_eq!(outcome.remaining, 1);
        assert_eq!(
            keychain.stored("aicontext", "hosted"),
            None,
            "the credential is residue once the reference is gone"
        );

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(!text.contains("id: hosted"), "{text}");
        assert!(text.contains("id: local"), "{text}");
        assert!(
            !text.contains("default_provider"),
            "a dangling default is residue: {text}"
        );
        assert!(
            !text.contains("keychain_entry"),
            "the reference is gone: {text}"
        );
    }

    #[test]
    fn a_provider_with_no_credential_deletes_nothing_from_the_keychain() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_ai(temp.path(), &file());
        let keychain = FakeKeychain::default();

        let outcome = execute(&args("local"), temp.path(), &keychain).expect("disconnect");
        assert!(!outcome.keychain_deleted);
        assert_eq!(outcome.remaining, 1);

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(!text.contains("id: local"), "{text}");
        assert!(
            text.contains("default_provider: hosted"),
            "a default naming a provider that is still present must survive: {text}"
        );
    }

    #[test]
    fn removing_the_last_provider_empties_the_list_without_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_ai(
            temp.path(),
            "providers:\n  - id: local\n    adapter: openai-compatible\n    auth:\n      type: none\n",
        );
        let outcome = execute(&args("local"), temp.path(), &FakeKeychain::default()).expect("ok");
        assert_eq!(outcome.remaining, 0);

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(!text.contains("id: local"), "{text}");
    }

    #[test]
    fn an_unknown_id_names_the_ids_that_are_present() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_ai(temp.path(), &file());
        let error = execute(&args("typo"), temp.path(), &FakeKeychain::default())
            .expect_err("no such provider");
        assert!(matches!(error, ConnectError::UnknownProvider { .. }), "{error:?}");
        assert_eq!(error.code(), "CONNECT-009");
        assert_eq!(error.exit(), crate::exit::Exit::Usage);
        assert!(error.to_string().contains("hosted, local"), "{error}");
    }

    #[test]
    fn an_absent_file_lists_no_providers_rather_than_failing_to_parse() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let error = execute(&args("local"), temp.path(), &FakeKeychain::default())
            .expect_err("nothing to remove");
        assert!(error.to_string().contains("known providers: none"), "{error}");
    }

    #[test]
    fn the_known_hint_reads_naturally() {
        assert_eq!(known_hint(&[]), "none");
        assert_eq!(known_hint(&["a".to_string(), "b".to_string()]), "a, b");
    }

    #[test]
    fn a_keychain_failure_is_reported_after_the_file_is_updated() {
        // The file no longer names the provider; the failure is still surfaced rather than swallowed.
        #[derive(Debug)]
        struct Failing;
        impl Keychain for Failing {
            fn get(&self, _reference: &KeychainReference) -> Result<Option<Secret>, KeychainError> {
                Ok(None)
            }
            fn set(&self, _reference: &KeychainReference, _secret: &Secret) -> Result<(), KeychainError> {
                Ok(())
            }
            fn delete(&self, _reference: &KeychainReference) -> Result<bool, KeychainError> {
                Err(KeychainError::Denied)
            }
        }

        let temp = tempfile::tempdir().expect("tempdir");
        write_ai(temp.path(), &file());
        let error = execute(&args("hosted"), temp.path(), &Failing).expect_err("keychain denied");
        assert_eq!(error.code(), "CONNECT-010");
        assert_eq!(error.exit(), crate::exit::Exit::General);

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(!text.contains("id: hosted"), "the file was still updated: {text}");
    }
}
