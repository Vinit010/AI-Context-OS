//! `aicontext connect`: prove a provider answers, then commit a reference to it.
//!
//! `docs/AI_PROVIDER_SPEC.md` §4 fixes what this command is for. A provider is described by a
//! *profile* — an adapter, a base URL, a model, and a credential *source* — and the profile is
//! committed to `.ai/ai.yaml`. The credential *value* is never committed: `connect` writes it to the
//! OS keychain (or the developer sets the named environment variable), and the file records only the
//! name (`docs/SECURITY.md` §6, `ADR-005`).
//!
//! # Why the order of operations is the contract
//!
//! `connect` does four things and they happen in this order: resolve the credential, make one
//! minimal call, store the credential, write the file. Nothing is written until the provider has
//! answered, so a mistyped `base_url`, an expired key, or an unreachable server leaves the repository
//! and the keychain exactly as they were. A failed call is a failure, not a degraded success
//! (`RULES.md` §4.10) — the whole point of `connect` is that it proves the profile works.
//!
//! # Seams
//!
//! The environment, the keychain, and the one call are three ports ([`Ports`]). Production wires the
//! real ones; a test injects fakes, so no test reads the developer's environment, touches the
//! developer's keychain, or opens a socket (`RULES.md` §8). The three are separate because they fail
//! differently and a test needs to make one of them fail without the others.

pub(crate) mod config;
pub(crate) mod error;

pub(crate) use config::{AiConfig, AuthEntry, AuthKind, KEYCHAIN_SERVICE, ProviderEntry};
pub(crate) use error::ConnectError;

use std::path::Path;

use aicontext_providers::{
    AIProvider, Auth, CompletionRequest, Environment, Keychain, KeychainReference, KeyringKeychain,
    Message, ModelOptions, OpenAiCompatible, OpenAiCompatibleConfig, PolicyRegion,
    ProcessEnvironment, ProviderError, ProviderId, Role, Secret, TokenBudget,
};
use serde::Serialize;

use crate::args::{ConnectArgs, GlobalArgs};
use crate::exit::Exit;
use crate::output::{Terminal, envelope};
use crate::project::{self, ProjectError};

/// The one adapter this release can build. `TASK-054` adds the others.
const OPENAI_COMPATIBLE: &str = "openai-compatible";

/// Adapters named in the spec but not built yet, and the task that will build them.
const SCHEDULED_ADAPTERS: &[(&str, &str)] = &[("anthropic", "TASK-054"), ("gemini", "TASK-054")];

/// The one-shot prompt the verification call sends. Small, unambiguous, and free of project content:
/// `connect` proves the transport and the credential, nothing else.
const PING: &str = "Reply with the single word: ok.";

/// The output ceiling on the verification call. One word does not need more.
const PING_MAX_TOKENS: u32 = 8;

/// The three effects `connect` needs, so a test can replace each in isolation.
struct Ports<'a> {
    /// Reads the named environment variable when `--env` is used.
    environment: &'a dyn Environment,
    /// Stores the credential value, the one thing that must not reach `.ai/`.
    keychain: &'a dyn Keychain,
    /// Makes the one minimal call that proves the profile works.
    verifier: &'a dyn Verifier,
}

/// The one verification call, behind a port so a unit test runs the whole command without a socket.
trait Verifier {
    /// Builds the adapter for `entry` and asks it for one minimal answer.
    ///
    /// `Ok(())` means the provider answered; every failure is a [`ProviderError`] the command maps
    /// onto a code and an exit code rather than inventing one.
    ///
    /// # Errors
    ///
    /// Returns whatever the adapter returns: [`ProviderError::Transport`] when the server cannot be
    /// reached, [`ProviderError::Unauthorized`] when the credential is rejected, and so on.
    fn verify(&self, entry: &ProviderEntry, credential: Option<&Secret>) -> Result<(), ProviderError>;
}

/// The real verifier: one `complete` call through the OpenAI-compatible adapter.
struct HttpVerifier;

impl Verifier for HttpVerifier {
    fn verify(&self, entry: &ProviderEntry, credential: Option<&Secret>) -> Result<(), ProviderError> {
        let id = ProviderId::new(&entry.id)?;
        let mut config = OpenAiCompatibleConfig::new(
            id.clone(),
            entry.base_url.clone().unwrap_or_default(),
            entry.model.clone().unwrap_or_default(),
        );
        if let Some(secret) = credential {
            config = config.with_auth(Auth::Bearer(secret.clone()));
        }
        let provider = OpenAiCompatible::new(config)?;

        // The provider is asynchronous; the CLI is not. One current-thread runtime drives the one
        // call and is dropped immediately.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| ProviderError::Transport {
                provider: id,
                source: Box::new(source),
            })?;
        runtime.block_on(provider.complete(ping_request())).map(|_| ())
    }
}

/// Runs `connect` and returns the exit code.
pub(crate) fn run(args: &ConnectArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return ConnectError::from(error).report(terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return ConnectError::from(error).report(terminal),
    };

    let environment = ProcessEnvironment;
    let keychain = KeyringKeychain;
    let verifier = HttpVerifier;
    let ports = Ports {
        environment: &environment,
        keychain: &keychain,
        verifier: &verifier,
    };

    match execute(args, global.offline, &root, &ports) {
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

/// The whole command, with the three ports supplied. `run` wraps this with the real ones.
fn execute(
    args: &ConnectArgs,
    offline: bool,
    root: &Path,
    ports: &Ports<'_>,
) -> Result<Outcome, ConnectError> {
    // Fail every check that does not need a call before making one.
    let id = ProviderId::new(&args.provider)?;
    validate_adapter(&args.adapter)?;
    require_ai_tree(root)?;
    if offline {
        return Err(ConnectError::Offline);
    }

    let credential = resolve_credential(args, ports.environment)?;
    let entry = build_entry(args, credential.is_some());

    let mut config = AiConfig::load(root)?;
    let config_path = config.display();

    // Nothing below this line is reached unless the provider answered.
    ports.verifier.verify(&entry, credential.as_ref())?;

    config.upsert(&entry)?;
    config.ensure_default(&entry.id)?;

    let stored = store_credential(ports.keychain, &entry, credential.as_ref())?;
    if let Err(error) = config.save() {
        // The keychain entry was written for a profile that is not in the file. Remove it rather than
        // leave a credential the project no longer references.
        if stored {
            clear_credential(ports.keychain, &entry);
        }
        return Err(error);
    }

    Ok(Outcome {
        provider: id.as_str().to_string(),
        adapter: entry.adapter.clone(),
        base_url: entry.base_url.clone().unwrap_or_default(),
        model: entry.model.clone().unwrap_or_default(),
        auth: entry.auth.kind.as_str(),
        env: entry.auth.env.clone(),
        keychain_entry: entry.auth.keychain_entry.clone(),
        keychain_stored: stored,
        config_path,
    })
}

/// Refuses an adapter this release cannot build, naming the task that will provide it.
fn validate_adapter(adapter: &str) -> Result<(), ConnectError> {
    if adapter == OPENAI_COMPATIBLE {
        return Ok(());
    }
    let task = SCHEDULED_ADAPTERS
        .iter()
        .find(|(name, _)| *name == adapter)
        .map_or("not scheduled yet", |(_, task)| *task);
    Err(ConnectError::UnsupportedAdapter {
        adapter: adapter.to_string(),
        task,
    })
}

/// Refuses a root with no `.ai/` tree, before spending a network call on a write that cannot land.
fn require_ai_tree(root: &Path) -> Result<(), ConnectError> {
    if root.join(".ai").is_dir() {
        return Ok(());
    }
    Err(ConnectError::Root(ProjectError::UnusableRoot {
        path: project::display(root),
        reason: "it has no `.ai/` directory yet; run `aicontext init` first".to_string(),
    }))
}

/// Resolves the credential the flags name: `--token` wins, then `--env`, then none.
///
/// The order is the one `docs/SECURITY.md` §6 fixes for resolution, so the value `connect` verifies
/// with is the value a later call would resolve.
fn resolve_credential(
    args: &ConnectArgs,
    environment: &dyn Environment,
) -> Result<Option<Secret>, ConnectError> {
    if args.token.is_none() && args.env.is_none() && args.keychain_entry.is_some() {
        return Err(ConnectError::KeychainEntryWithoutCredential);
    }
    if let Some(token) = &args.token {
        if token.is_empty() {
            // Named, not echoed: an empty `--token` is a mistake, and the value must never be printed.
            return Err(ConnectError::BlankValue { flag: "--token" });
        }
        return Ok(Some(Secret::new(token.clone())));
    }
    if let Some(variable) = &args.env {
        let secret = environment
            .variable(variable)
            .ok_or_else(|| aicontext_providers::CredentialError::EnvironmentMissing {
                variable: variable.clone(),
            })?;
        return Ok(Some(secret));
    }
    Ok(None)
}

/// Builds the profile the flags describe.
fn build_entry(args: &ConnectArgs, has_credential: bool) -> ProviderEntry {
    let (kind, env) = match (&args.env, has_credential) {
        (Some(variable), _) => (AuthKind::EnvironmentOrKeychain, Some(variable.clone())),
        (None, true) => (AuthKind::Keychain, None),
        (None, false) => (AuthKind::None, None),
    };
    let keychain_entry = match (&args.keychain_entry, has_credential) {
        (Some(account), _) => Some(account.clone()),
        (None, true) => Some(args.provider.clone()),
        (None, false) => None,
    };
    ProviderEntry {
        id: args.provider.clone(),
        adapter: args.adapter.clone(),
        base_url: Some(args.base_url.clone()),
        model: Some(args.model.clone()),
        auth: AuthEntry {
            kind,
            env,
            keychain_entry,
        },
    }
}

/// Stores the credential value in the OS keychain, answering whether anything was written.
fn store_credential(
    keychain: &dyn Keychain,
    entry: &ProviderEntry,
    credential: Option<&Secret>,
) -> Result<bool, ConnectError> {
    let (Some(secret), Some(account)) = (credential, entry.auth.keychain_entry.as_ref()) else {
        return Ok(false);
    };
    let reference = KeychainReference::new(KEYCHAIN_SERVICE, account.clone());
    keychain.set(&reference, secret)?;
    Ok(true)
}

/// Best-effort removal of the entry `store_credential` just wrote, when the file write failed.
fn clear_credential(keychain: &dyn Keychain, entry: &ProviderEntry) {
    if let Some(account) = &entry.auth.keychain_entry {
        let reference = KeychainReference::new(KEYCHAIN_SERVICE, account.clone());
        // The rollback cannot do more than try: the file write has already failed, and that failure
        // is the one the developer needs to see. The error it would produce is dropped deliberately.
        keychain.delete(&reference).ok();
    }
}

/// A request that asks for one word and makes retries unsafe, so the call happens exactly once.
fn ping_request() -> CompletionRequest {
    CompletionRequest {
        policy: PolicyRegion::default(),
        messages: vec![Message::new(Role::User, PING.to_string())],
        data: Vec::new(),
        tools: Vec::new(),
        budget: TokenBudget {
            max_output_tokens: PING_MAX_TOKENS,
        },
        options: ModelOptions::default(),
        idempotency_key: String::new(),
    }
}

/// What a successful `connect` produced, in the terms the report and the JSON payload both use.
struct Outcome {
    provider: String,
    adapter: String,
    base_url: String,
    model: String,
    auth: &'static str,
    env: Option<String>,
    keychain_entry: Option<String>,
    keychain_stored: bool,
    config_path: String,
}

/// The `--json` payload. Names only — the credential is absent by construction, not by redaction.
#[derive(Debug, Serialize)]
struct ConnectData {
    provider: String,
    adapter: String,
    base_url: String,
    model: String,
    /// The recorded source: `none`, `keychain`, or `env_or_keychain`.
    auth: &'static str,
    /// The environment variable's *name*, when one was recorded.
    env: Option<String>,
    /// The keychain entry's *name*, when one was recorded.
    keychain_entry: Option<String>,
    /// Whether a value was written to the OS keychain.
    keychain_stored: bool,
    /// Always true: the command makes no progress past the verification call otherwise.
    verified: bool,
    /// Where the profile was written, relative to the project root.
    config_path: String,
}

/// Writes the JSON envelope for a successful run.
fn report_json(outcome: &Outcome, terminal: &mut Terminal) -> Exit {
    let data = ConnectData {
        provider: outcome.provider.clone(),
        adapter: outcome.adapter.clone(),
        base_url: outcome.base_url.clone(),
        model: outcome.model.clone(),
        auth: outcome.auth,
        env: outcome.env.clone(),
        keychain_entry: outcome.keychain_entry.clone(),
        keychain_stored: outcome.keychain_stored,
        verified: true,
        config_path: outcome.config_path.clone(),
    };
    let body = envelope(
        "connect",
        Exit::Ok.code(),
        data,
        Vec::new(),
        format!("provider `{}` verified and stored", outcome.provider),
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

/// Prints the human report: the identity line, the profile, the proof, and the summary.
fn print_human(outcome: &Outcome, name: &str, root: &Path, terminal: &mut Terminal) {
    // Both marks are built before the first row, because `row` borrows the terminal mutably.
    let info = terminal.mark_info();
    let ok = terminal.mark_ok();

    terminal.identity("aicontext", name, project::vcs_label(root));
    terminal.row(
        &info,
        "provider",
        &format!("{} ({})", outcome.provider, outcome.adapter),
    );
    terminal.row(&info, "base_url", &outcome.base_url);
    terminal.row(&info, "model", &outcome.model);
    terminal.row(&info, "auth", &auth_line(outcome));
    terminal.row(&ok, "verified", "one minimal call answered");
    terminal.summary("stored", Exit::Ok, "`aicontext disconnect --provider <id>`");
}

/// The one-line description of the recorded credential source.
fn auth_line(outcome: &Outcome) -> String {
    match (outcome.env.as_deref(), outcome.keychain_entry.as_deref()) {
        (Some(variable), Some(entry)) => {
            format!("{} (env {variable}, keychain `{entry}`)", outcome.auth)
        }
        (Some(variable), None) => format!("{} (env {variable})", outcome.auth),
        (None, Some(entry)) => format!("{} (keychain `{entry}`)", outcome.auth),
        (None, None) => outcome.auth.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use aicontext_providers::{
        Environment, Keychain, KeychainError, KeychainReference, ProviderError, ProviderId, Secret,
    };

    use super::{Ports, Verifier, execute};
    use crate::args::ConnectArgs;
    use crate::connect::config::{AiConfig, AuthKind};
    use crate::connect::{ConnectError, ProviderEntry};

    #[derive(Debug, Default)]
    struct FakeEnvironment {
        values: BTreeMap<String, String>,
    }

    impl Environment for FakeEnvironment {
        fn variable(&self, name: &str) -> Option<Secret> {
            self.values
                .get(name)
                .filter(|value| !value.is_empty())
                .map(|value| Secret::new(value.clone()))
        }
    }

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

        fn is_empty(&self) -> bool {
            self.entries.lock().expect("lock").is_empty()
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

    fn args() -> ConnectArgs {
        ConnectArgs {
            provider: "hosted".to_string(),
            adapter: "openai-compatible".to_string(),
            base_url: "http://example.test/v1".to_string(),
            model: "gpt-test".to_string(),
            env: None,
            token: None,
            keychain_entry: None,
        }
    }

    #[test]
    fn a_token_reaches_the_keychain_and_never_the_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let verifier = FakeVerifier::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            verifier: &verifier,
        };

        let args = ConnectArgs {
            token: Some("s3cr3t-value".to_string()),
            ..args()
        };
        let outcome = execute(&args, false, temp.path(), &ports).expect("connect succeeds");

        assert_eq!(
            keychain.stored("aicontext", "hosted").as_deref(),
            Some("s3cr3t-value"),
            "the value goes to the keychain"
        );
        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(!text.contains("s3cr3t-value"), "the value never reaches .ai/: {text}");
        assert!(text.contains("type: keychain"), "{text}");
        assert!(text.contains("keychain_entry: hosted"), "{text}");
        assert!(matches!(outcome.auth, "keychain"));
        assert!(outcome.keychain_stored);
    }

    #[test]
    fn an_environment_variable_is_recorded_by_name_and_stored_by_value() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let environment = FakeEnvironment {
            values: BTreeMap::from([("AICON_AI_HOSTED_KEY".to_string(), "env-secret".to_string())]),
        };
        let keychain = FakeKeychain::default();
        let verifier = RecordingVerifier::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            verifier: &verifier,
        };

        let args = ConnectArgs {
            env: Some("AICON_AI_HOSTED_KEY".to_string()),
            ..args()
        };
        execute(&args, false, temp.path(), &ports).expect("connect succeeds");

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(text.contains("type: env_or_keychain"), "{text}");
        assert!(text.contains("env: AICON_AI_HOSTED_KEY"), "{text}");
        assert!(
            !text.contains("env-secret"),
            "the value is a name, never a secret: {text}"
        );
        assert_eq!(
            keychain.stored("aicontext", "hosted").as_deref(),
            Some("env-secret")
        );
        assert_eq!(
            verifier.seen(),
            vec![Some("env-secret".to_string())],
            "the call is verified with the same value that is stored"
        );
    }

    #[test]
    fn no_credential_records_none_and_leaves_the_keychain_alone() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let keychain = FakeKeychain::default();
        let verifier = RecordingVerifier::default();
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &keychain,
            verifier: &verifier,
        };

        let outcome = execute(&args(), false, temp.path(), &ports).expect("connect succeeds");

        assert!(keychain.is_empty(), "nothing to store, nothing stored");
        assert!(!outcome.keychain_stored);
        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(text.contains("type: none"), "{text}");
        assert!(!text.contains("keychain_entry"), "{text}");
        assert_eq!(verifier.seen(), vec![None]);
    }

    #[test]
    fn a_failed_verification_writes_neither_the_file_nor_the_keychain() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &keychain,
            verifier: &FailingVerifier,
        };

        let args = ConnectArgs {
            token: Some("s3cr3t".to_string()),
            ..args()
        };
        let error = execute(&args, false, temp.path(), &ports).expect_err("the call fails");

        assert!(matches!(error, ConnectError::Provider(_)), "{error:?}");
        assert_eq!(error.code(), "PROVIDER_UNAUTHORIZED");
        assert_eq!(error.exit(), crate::exit::Exit::Provider);
        assert!(keychain.is_empty(), "no credential for a profile that was not stored");
        assert!(
            !temp.path().join(".ai/ai.yaml").exists(),
            "a failed call must leave no profile behind"
        );
    }

    #[test]
    fn offline_refuses_before_resolving_anything() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &FakeKeychain::default(),
            verifier: &RecordingVerifier::default(),
        };
        let error = execute(&args(), true, temp.path(), &ports).expect_err("offline refuses");
        assert!(matches!(error, ConnectError::Offline), "{error:?}");
        assert_eq!(error.exit(), crate::exit::Exit::Usage);
    }

    #[test]
    fn a_keychain_entry_without_a_credential_is_refused() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &FakeKeychain::default(),
            verifier: &RecordingVerifier::default(),
        };
        let args = ConnectArgs {
            keychain_entry: Some("shared".to_string()),
            ..args()
        };
        let error = execute(&args, false, temp.path(), &ports).expect_err("refused");
        assert!(matches!(error, ConnectError::KeychainEntryWithoutCredential));
    }

    #[test]
    fn an_empty_token_is_refused_without_echoing_it() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &FakeKeychain::default(),
            verifier: &RecordingVerifier::default(),
        };
        let args = ConnectArgs {
            token: Some(String::new()),
            ..args()
        };
        let error = execute(&args, false, temp.path(), &ports).expect_err("refused");
        assert!(matches!(error, ConnectError::BlankValue { flag: "--token" }));
        assert!(!error.to_string().contains("s3cr3t"));
    }

    #[test]
    fn a_named_variable_that_is_unset_is_an_error_not_a_silent_fallback() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &FakeKeychain::default(),
            verifier: &RecordingVerifier::default(),
        };
        let args = ConnectArgs {
            env: Some("AICON_AI_HOSTED_KEY".to_string()),
            ..args()
        };
        let error = execute(&args, false, temp.path(), &ports).expect_err("no variable, no credential");
        assert_eq!(error.code(), "CREDENTIAL_ENVIRONMENT_MISSING");
        assert_eq!(error.exit(), crate::exit::Exit::Usage);
    }

    #[test]
    fn an_unbuilt_adapter_names_the_task_that_builds_it() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &FakeKeychain::default(),
            verifier: &RecordingVerifier::default(),
        };
        let args = ConnectArgs {
            adapter: "anthropic".to_string(),
            ..args()
        };
        let error = execute(&args, false, temp.path(), &ports).expect_err("refused");
        assert!(matches!(
            error,
            ConnectError::UnsupportedAdapter {
                task: "TASK-054",
                ..
            }
        ));
    }

    #[test]
    fn a_root_without_an_ai_tree_is_refused_before_any_call() {
        let temp = tempfile::tempdir().expect("tempdir");
        let verifier = RecordingVerifier::default();
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &FakeKeychain::default(),
            verifier: &verifier,
        };
        let error = execute(&args(), false, temp.path(), &ports).expect_err("no .ai/");
        assert_eq!(error.code(), "PROJ-001");
        assert!(verifier.seen().is_empty(), "no call is made for a root we cannot write");
    }

    #[test]
    fn a_reconnect_keeps_a_single_provider_and_sets_no_second_default() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &FakeKeychain::default(),
            verifier: &RecordingVerifier::default(),
        };
        execute(&args(), false, temp.path(), &ports).expect("first");
        let second = ConnectArgs {
            model: "gpt-other".to_string(),
            ..args()
        };
        execute(&second, false, temp.path(), &ports).expect("second");

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert_eq!(text.matches("id: hosted").count(), 1, "{text}");
        assert_eq!(text.matches("default_provider").count(), 1, "{text}");
        assert!(text.contains("gpt-other"), "{text}");
    }

    #[test]
    fn a_new_document_is_a_valid_provider_list() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        let ports = Ports {
            environment: &FakeEnvironment::default(),
            keychain: &FakeKeychain::default(),
            verifier: &RecordingVerifier::default(),
        };
        execute(&args(), false, temp.path(), &ports).expect("connect");

        let config = AiConfig::load(temp.path()).expect("re-reads");
        assert_eq!(config.provider_ids().expect("ids"), vec!["hosted".to_string()]);
        let mut round = config;
        let removed = round.remove("hosted").expect("remove").expect("present");
        assert_eq!(removed.auth.kind, AuthKind::None);
    }

    /// A verifier that records the credential it was handed.
    #[derive(Default)]
    struct RecordingVerifier {
        seen: Mutex<Vec<Option<String>>>,
    }

    impl RecordingVerifier {
        fn seen(&self) -> Vec<Option<String>> {
            self.seen.lock().expect("lock").clone()
        }
    }

    impl Verifier for RecordingVerifier {
        fn verify(
            &self,
            _entry: &ProviderEntry,
            credential: Option<&Secret>,
        ) -> Result<(), ProviderError> {
            self.seen
                .lock()
                .expect("lock")
                .push(credential.map(|secret| secret.expose().to_string()));
            Ok(())
        }
    }

    /// A verifier that always rejects the credential.
    struct FailingVerifier;

    impl Verifier for FailingVerifier {
        fn verify(
            &self,
            _entry: &ProviderEntry,
            _credential: Option<&Secret>,
        ) -> Result<(), ProviderError> {
            Err(ProviderError::Unauthorized {
                provider: ProviderId::new("hosted").expect("valid id"),
            })
        }
    }
}
