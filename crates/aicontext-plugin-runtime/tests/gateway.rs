//! Integration tests for the tool gateway.
//!
//! Every test drives the real `misbehaving-plugin` binary through [`ToolGateway`], so the
//! validation, permission, approval, execution, and audit path is exercised as one call rather than
//! as four units wired together by the test. The assertions cover the acceptance criteria in
//! `TASK-074`: every call passes every gate, one code path reaches the plugin, results are normalised
//! into an untrusted envelope with size and time caps, and no shell path exists.
//!
//! The two source-scan tests at the end enforce the structural invariants named in
//! `docs/SECURITY.md` §5: `.call_tool(` appears once in production, in `gateway/mod.rs`, and the
//! gateway contains no process-construction code at all.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use aicontext_core::{ErrorCode, PermissionMode};
use aicontext_permissions::{Policy, Rule};
use aicontext_plugin_runtime::{
    ApprovalEvent, ApprovalGrant, ApprovalOutcome, DenialReason, GatewayAudit, GatewayError,
    GatewayEvent, PromptError, Prompter, RefusalStage, ResultSummary, SandboxRoots,
    TRUST_UNTRUSTED, ToolCall, ToolGateway,
};
use aicontext_plugin_sdk::{
    Capability, Effect, EntryPoint, FilesystemScope, Limits, NetworkScope, PluginManifest,
    PluginProtocol,
};
use serde_json::json;

const TOOL: &str = "fixture.echo.run";
const OTHER_TOOL: &str = "fixture.other.run";

fn manifest(mode: &str) -> PluginManifest {
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
            command: env!("CARGO_BIN_EXE_misbehaving-plugin").to_owned(),
            args: vec![mode.to_owned()],
        },
        minimum_aicontext_version: None,
        capabilities: vec![Capability {
            tool: TOOL.to_owned(),
            effect: Effect::Write,
            destructive: false,
            mcp_tool: None,
        }],
        auth: None,
        filesystem: FilesystemScope {
            read: Vec::new(),
            write: Vec::new(),
        },
        network: NetworkScope { egress: Vec::new() },
        limits: Limits {
            call_timeout_ms: 2_000,
            max_output_bytes: 16 * 1024,
        },
    }
}

fn destructive_manifest(mode: &str) -> PluginManifest {
    let mut manifest = manifest(mode);
    manifest.capabilities[0].destructive = true;
    manifest
}

fn with_limits(
    mut manifest: PluginManifest,
    timeout_ms: u64,
    max_output_bytes: u64,
) -> PluginManifest {
    manifest.limits.call_timeout_ms = timeout_ms;
    manifest.limits.max_output_bytes = max_output_bytes;
    manifest
}

/// A fresh state/project pair under one temporary root shared by the test binary.
///
/// The plugin's cwd must outlive the gateway that owns the process, so the root is removed only
/// when the test binary exits rather than after each `start` call.
fn sandbox_roots() -> SandboxRoots {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let root = ROOT.get_or_init(|| tempfile::tempdir().expect("a temporary sandbox root"));
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory = root.path().join(format!("case-{id}"));
    let project = directory.join("project");
    fs::create_dir_all(&project).expect("the project exists");
    SandboxRoots::new(directory.join("state"), project)
}

fn policy(mode: PermissionMode) -> Policy {
    Policy::new(mode, std::iter::empty::<Rule>()).expect("an empty rule set is valid")
}

#[derive(Debug, Default)]
struct ScriptedPrompter {
    answers: VecDeque<Result<String, PromptError>>,
    prompts: Vec<String>,
}

impl ScriptedPrompter {
    fn approving() -> Self {
        Self {
            answers: VecDeque::from([Ok("yes".to_owned())]),
            prompts: Vec::new(),
        }
    }

    fn declining() -> Self {
        Self {
            answers: VecDeque::from([Ok("no".to_owned())]),
            prompts: Vec::new(),
        }
    }
}

impl Prompter for ScriptedPrompter {
    fn ask(&mut self, prompt: &str) -> Result<String, PromptError> {
        self.prompts.push(prompt.to_owned());
        self.answers
            .pop_front()
            .unwrap_or(Err(PromptError::Unavailable))
    }
}

#[derive(Debug, Default)]
struct Recorder {
    events: Vec<GatewayEvent>,
}

impl GatewayAudit for Recorder {
    type Error = std::convert::Infallible;

    fn record(&mut self, event: &GatewayEvent) -> Result<(), Self::Error> {
        self.events.push(event.clone());
        Ok(())
    }
}

#[derive(Debug)]
struct FailingAudit {
    fail_at: usize,
    seen: usize,
}

impl GatewayAudit for FailingAudit {
    type Error = io::Error;

    fn record(&mut self, _event: &GatewayEvent) -> Result<(), Self::Error> {
        let should_fail = self.seen == self.fail_at;
        self.seen += 1;
        if should_fail {
            return Err(io::Error::other("the audit log is read-only"));
        }
        Ok(())
    }
}

/// A recorder whose events can be inspected after `ToolGateway::start` consumed the sink.
#[derive(Clone, Debug, Default)]
struct SharedRecorder(Arc<Mutex<Vec<GatewayEvent>>>);

impl GatewayAudit for SharedRecorder {
    type Error = std::convert::Infallible;

    fn record(&mut self, event: &GatewayEvent) -> Result<(), Self::Error> {
        self.0
            .lock()
            .expect("the test recorder is not poisoned")
            .push(event.clone());
        Ok(())
    }
}

fn start_with<A: GatewayAudit>(
    manifest: PluginManifest,
    policy: Policy,
    prompter: ScriptedPrompter,
    audit: A,
    allow_once: &[&str],
) -> ToolGateway<ScriptedPrompter, A> {
    ToolGateway::start(
        manifest,
        &sandbox_roots(),
        policy,
        prompter,
        audit,
        allow_once.iter().map(|tool| (*tool).to_owned()),
    )
    .expect("the fixture starts and its descriptors match the manifest")
}

fn start(
    mode: &str,
    policy: Policy,
    prompter: ScriptedPrompter,
) -> ToolGateway<ScriptedPrompter, Recorder> {
    start_with(manifest(mode), policy, prompter, Recorder::default(), &[])
}

fn call() -> ToolCall {
    ToolCall::new(TOOL, json!({"text": "hello"}))
        .with_target("i-0123456789abcdef0")
        .with_reason("the caller explained itself")
}

#[test]
fn a_call_that_passes_every_gate_executes_and_is_enveloped() {
    let mut gateway = start(
        "ok",
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
    );

    let outcome = gateway.call(call()).expect("the call succeeds");
    let envelope = outcome.result().expect("the plugin was called");

    assert_eq!(envelope.trust(), TRUST_UNTRUSTED);
    assert_eq!(envelope.tool(), TOOL);
    assert_eq!(envelope.plugin(), "fixture");
    assert!(!envelope.is_error());
    assert_eq!(envelope.meta().request_id(), "call-1");
    assert!(envelope.extra().is_empty());
    assert_eq!(
        envelope.content().first().and_then(|block| block.as_text()),
        Some("hello")
    );

    let logs = gateway.take_logs();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].message(), "fixture handled the call");

    let (prompter, audit) = gateway.into_parts();
    assert!(prompter.prompts.is_empty(), "ALLOW must not prompt");
    assert_eq!(audit.events.len(), 2);
    assert!(
        matches!(&audit.events[0], GatewayEvent::Executing { call } if call.mode() == Some(PermissionMode::Allow))
    );
    match &audit.events[1] {
        GatewayEvent::Completed {
            call,
            result:
                ResultSummary::Succeeded {
                    is_error,
                    bytes,
                    digest,
                    ..
                },
        } => {
            assert_eq!(call.tool(), TOOL);
            assert_eq!(call.target(), "i-0123456789abcdef0");
            assert_eq!(call.mode(), Some(PermissionMode::Allow));
            assert!(!is_error);
            assert!(*bytes > 0);
            assert!(digest.starts_with("sha256:"));
        }
        other => panic!("expected a successful completion, found {other:?}"),
    }
}

#[test]
fn an_approved_call_records_the_prompt_the_decision_dispatch_and_result() {
    let mut gateway = start(
        "ok",
        policy(PermissionMode::Approval),
        ScriptedPrompter::approving(),
    );

    let outcome = gateway.call(call()).expect("the approved call succeeds");
    assert!(outcome.is_executed());

    let (prompter, audit) = gateway.into_parts();
    assert_eq!(prompter.prompts.len(), 1);
    assert!(
        prompter.prompts[0].contains(TOOL),
        "{}",
        prompter.prompts[0]
    );
    assert!(
        prompter.prompts[0].contains("i-0123456789abcdef0"),
        "{}",
        prompter.prompts[0]
    );

    assert_eq!(audit.events.len(), 4);
    assert!(matches!(
        &audit.events[0],
        GatewayEvent::Approval(ApprovalEvent::Prompted { .. })
    ));
    match &audit.events[1] {
        GatewayEvent::Approval(ApprovalEvent::Decided { outcome, .. }) => assert!(matches!(
            outcome,
            ApprovalOutcome::Allowed {
                grant: ApprovalGrant::Interactive
            }
        )),
        other => panic!("expected an approval decision, found {other:?}"),
    }
    assert!(matches!(&audit.events[2], GatewayEvent::Executing { .. }));
    assert!(matches!(
        &audit.events[3],
        GatewayEvent::Completed {
            result: ResultSummary::Succeeded { .. },
            ..
        }
    ));
}

#[test]
fn a_declined_prompt_refuses_without_dispatching() {
    let mut gateway = start(
        "ok",
        policy(PermissionMode::Approval),
        ScriptedPrompter::declining(),
    );

    let outcome = gateway
        .call(call())
        .expect("a denial is a decision, not an error");
    let refusal = outcome.refusal().expect("the call was refused");
    assert_eq!(refusal.stage(), RefusalStage::Approval);
    assert_eq!(refusal.reason(), DenialReason::Declined.to_string());
    assert!(refusal.code().is_none());

    let logs = gateway.take_logs();
    assert!(logs.is_empty(), "a refused call must not reach the plugin");

    let (prompter, audit) = gateway.into_parts();
    assert_eq!(prompter.prompts.len(), 1);
    assert_eq!(audit.events.len(), 3);
    assert!(matches!(
        &audit.events[0],
        GatewayEvent::Approval(ApprovalEvent::Prompted { .. })
    ));
    assert!(matches!(
        &audit.events[1],
        GatewayEvent::Approval(ApprovalEvent::Decided { .. })
    ));
    match &audit.events[2] {
        GatewayEvent::Completed {
            result: ResultSummary::Refused { stage, code, .. },
            ..
        } => {
            assert_eq!(*stage, RefusalStage::Approval);
            assert!(code.is_none());
        }
        other => panic!("expected a refusal, found {other:?}"),
    }
}

#[test]
fn a_policy_denial_refuses_before_the_prompt() {
    let mut gateway = start(
        "ok",
        policy(PermissionMode::Deny),
        ScriptedPrompter::default(),
    );

    let outcome = gateway
        .call(call())
        .expect("a denial is a decision, not an error");
    let refusal = outcome.refusal().expect("the call was refused");
    assert_eq!(refusal.stage(), RefusalStage::Permission);
    assert_eq!(refusal.reason(), DenialReason::Policy.to_string());

    let logs = gateway.take_logs();
    assert!(logs.is_empty());

    let (prompter, audit) = gateway.into_parts();
    assert!(prompter.prompts.is_empty(), "DENY must not prompt");
    assert_eq!(audit.events.len(), 1);
    assert!(matches!(
        &audit.events[0],
        GatewayEvent::Completed {
            result: ResultSummary::Refused {
                stage: RefusalStage::Permission,
                ..
            },
            ..
        }
    ));
}

#[test]
fn an_allow_once_grant_skips_the_prompt_for_approval() {
    let mut gateway = start_with(
        manifest("ok"),
        policy(PermissionMode::Approval),
        ScriptedPrompter::default(),
        Recorder::default(),
        &[TOOL],
    );

    let outcome = gateway.call(call()).expect("the flagged call succeeds");
    assert!(outcome.is_executed());

    let (prompter, audit) = gateway.into_parts();
    assert!(prompter.prompts.is_empty(), "the flag must skip the prompt");
    assert_eq!(audit.events.len(), 3);
    match &audit.events[0] {
        GatewayEvent::Approval(ApprovalEvent::Decided { outcome, .. }) => assert!(matches!(
            outcome,
            ApprovalOutcome::Allowed {
                grant: ApprovalGrant::AllowOnce
            }
        )),
        other => panic!("expected an allow-once decision, found {other:?}"),
    }
    assert!(matches!(&audit.events[1], GatewayEvent::Executing { .. }));
    assert!(matches!(
        &audit.events[2],
        GatewayEvent::Completed {
            result: ResultSummary::Succeeded { .. },
            ..
        }
    ));
}

#[test]
fn a_destructive_capability_requires_explicit_approval_even_when_policy_allows() {
    let mut gateway = start_with(
        destructive_manifest("destructive"),
        policy(PermissionMode::Allow),
        ScriptedPrompter::approving(),
        Recorder::default(),
        &[],
    );

    let outcome = gateway
        .call(call())
        .expect("the explicitly approved call succeeds");
    assert!(outcome.is_executed());

    let (prompter, audit) = gateway.into_parts();
    assert_eq!(
        prompter.prompts.len(),
        1,
        "the destructive floor must prompt"
    );
    assert_eq!(audit.events.len(), 4);
    match &audit.events[0] {
        GatewayEvent::Approval(ApprovalEvent::Prompted { summary }) => {
            assert_eq!(summary.mode(), PermissionMode::ExplicitApproval);
        }
        other => panic!("expected a prompt, found {other:?}"),
    }
    match &audit.events[3] {
        GatewayEvent::Completed { call, .. } => {
            assert!(call.destructive());
            assert_eq!(call.mode(), Some(PermissionMode::ExplicitApproval));
        }
        other => panic!("expected a completion, found {other:?}"),
    }
}

#[test]
fn an_allow_once_grant_cannot_satisfy_explicit_approval() {
    let mut gateway = start_with(
        destructive_manifest("destructive"),
        policy(PermissionMode::Allow),
        ScriptedPrompter::declining(),
        Recorder::default(),
        &[TOOL],
    );

    let outcome = gateway
        .call(call())
        .expect("a denial is a decision, not an error");
    let refusal = outcome.refusal().expect("the call was refused");
    assert_eq!(refusal.stage(), RefusalStage::Approval);

    let (prompter, audit) = gateway.into_parts();
    assert_eq!(
        prompter.prompts.len(),
        1,
        "--allow-once must not satisfy EXPLICIT_APPROVAL"
    );
    match &audit.events[1] {
        GatewayEvent::Approval(ApprovalEvent::Decided { outcome, .. }) => assert!(matches!(
            outcome,
            ApprovalOutcome::Denied {
                reason: DenialReason::Declined
            }
        )),
        other => panic!("expected a denial, found {other:?}"),
    }
}

#[test]
fn an_undeclared_capability_is_plg_008_and_is_audited_before_dispatch() {
    let mut gateway = start(
        "ok",
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
    );

    let error = gateway
        .call(ToolCall::new(OTHER_TOOL, json!({"text": "hello"})))
        .expect_err("the manifest does not declare the tool");
    assert_eq!(error.code(), ErrorCode::PLG_008);

    let logs = gateway.take_logs();
    assert!(logs.is_empty());

    let (prompter, audit) = gateway.into_parts();
    assert!(prompter.prompts.is_empty());
    assert_eq!(audit.events.len(), 1);
    match &audit.events[0] {
        GatewayEvent::Completed {
            call,
            result: ResultSummary::Refused { stage, code, .. },
        } => {
            assert_eq!(call.tool(), OTHER_TOOL);
            assert_eq!(*stage, RefusalStage::Permission);
            assert_eq!(*code, Some(ErrorCode::PLG_008));
        }
        other => panic!("expected a refusal, found {other:?}"),
    }
}

#[test]
fn invalid_arguments_are_plg_012_before_a_prompt_or_dispatch() {
    let mut gateway = start(
        "ok",
        policy(PermissionMode::Approval),
        ScriptedPrompter::approving(),
    );

    for arguments in [
        json!({}),
        json!({"text": "hello", "shell": "rm -rf /"}),
        json!({"text": 7}),
    ] {
        let error = gateway
            .call(ToolCall::new(TOOL, arguments))
            .expect_err("the arguments do not satisfy the schema");
        assert_eq!(error.code(), ErrorCode::PLG_012);
    }

    let logs = gateway.take_logs();
    assert!(
        logs.is_empty(),
        "invalid arguments must not reach the plugin"
    );

    let (prompter, audit) = gateway.into_parts();
    assert!(
        prompter.prompts.is_empty(),
        "invalid arguments must not reach the human"
    );
    assert_eq!(audit.events.len(), 3);
    for event in &audit.events {
        assert!(matches!(
            event,
            GatewayEvent::Completed {
                result: ResultSummary::Refused {
                    stage: RefusalStage::Validation,
                    code: Some(ErrorCode::PLG_012),
                    ..
                },
                ..
            }
        ));
    }
}

#[test]
fn a_sandbox_refusal_is_audited_before_it_is_returned() {
    let mut manifest = manifest("ok");
    manifest.filesystem.read = vec!["../outside".to_owned()];
    let audit = SharedRecorder::default();
    let events = Arc::clone(&audit.0);

    let error = ToolGateway::start(
        manifest,
        &sandbox_roots(),
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
        audit,
        std::iter::empty::<String>(),
    )
    .expect_err("the declared read escapes the project");
    assert_eq!(error.code(), ErrorCode::PLG_009);
    assert!(matches!(error, GatewayError::Sandbox(_)), "{error}");

    let events = events.lock().expect("the test recorder is not poisoned");
    assert_eq!(events.len(), 1);
    match &events[0] {
        GatewayEvent::SandboxRefused {
            plugin,
            code,
            detail,
        } => {
            assert_eq!(plugin, "fixture");
            assert_eq!(*code, ErrorCode::PLG_009);
            assert!(detail.contains("../outside"), "{detail}");
        }
        other => panic!("expected a sandbox refusal, found {other:?}"),
    }
}

#[test]
fn start_refuses_an_unenforceable_schema_with_plg_012() {
    let error = ToolGateway::start(
        manifest("bad-schema"),
        &sandbox_roots(),
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
        Recorder::default(),
        std::iter::empty::<String>(),
    )
    .expect_err("the schema uses an unsupported keyword");
    assert_eq!(error.code(), ErrorCode::PLG_012);
}

#[test]
fn start_refuses_a_descriptor_that_disagrees_with_the_manifest_with_plg_005() {
    let error = ToolGateway::start(
        manifest("descriptor-mismatch"),
        &sandbox_roots(),
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
        Recorder::default(),
        std::iter::empty::<String>(),
    )
    .expect_err("the descriptor says read where the manifest says write");
    assert_eq!(error.code(), ErrorCode::PLG_005);
}

#[test]
fn start_refuses_undeclared_missing_and_duplicate_tools() {
    for (mode, expected) in [
        ("extra-tool", ErrorCode::PLG_008),
        ("missing-tool", ErrorCode::PLG_008),
        ("duplicate-tool", ErrorCode::PLG_005),
    ] {
        let error = ToolGateway::start(
            manifest(mode),
            &sandbox_roots(),
            policy(PermissionMode::Allow),
            ScriptedPrompter::default(),
            Recorder::default(),
            std::iter::empty::<String>(),
        )
        .expect_err("the tool list does not match the manifest");
        assert_eq!(error.code(), expected, "mode `{mode}`");
    }
}

#[test]
fn an_audit_failure_before_dispatch_refuses_the_call() {
    let mut gateway = start_with(
        manifest("ok"),
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
        FailingAudit {
            fail_at: 0,
            seen: 0,
        },
        &[],
    );

    let error = gateway
        .call(call())
        .expect_err("the executing record cannot be written");
    assert_eq!(error.code(), ErrorCode::PLG_010);
    assert!(matches!(error, GatewayError::Audit { .. }));

    let logs = gateway.take_logs();
    assert!(
        logs.is_empty(),
        "no plugin call may happen without a record"
    );
    assert!(gateway.is_running());
}

#[test]
fn an_audit_failure_after_execution_still_refuses_the_result() {
    let mut gateway = start_with(
        manifest("ok"),
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
        FailingAudit {
            fail_at: 1,
            seen: 0,
        },
        &[],
    );

    let error = gateway
        .call(call())
        .expect_err("the completed record cannot be written");
    assert_eq!(error.code(), ErrorCode::PLG_010);

    let logs = gateway.take_logs();
    assert!(
        !logs.is_empty(),
        "the plugin ran, but its result must not escape without a record"
    );
}

#[test]
fn a_plugin_timeout_is_plg_003_and_the_process_is_dead() {
    let mut gateway = start_with(
        with_limits(manifest("hang"), 200, 16 * 1024),
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
        Recorder::default(),
        &[],
    );

    let error = gateway.call(call()).expect_err("the fixture never answers");
    assert_eq!(error.code(), ErrorCode::PLG_003);
    assert!(!gateway.is_running());

    let (_, audit) = gateway.into_parts();
    assert_eq!(audit.events.len(), 2);
    assert!(matches!(&audit.events[0], GatewayEvent::Executing { .. }));
    match &audit.events[1] {
        GatewayEvent::Completed {
            result: ResultSummary::Refused { stage, code, .. },
            ..
        } => {
            assert_eq!(*stage, RefusalStage::Execution);
            assert_eq!(*code, Some(ErrorCode::PLG_003));
        }
        other => panic!("expected an execution refusal, found {other:?}"),
    }
}

#[test]
fn a_tools_list_frame_over_the_manifest_cap_is_plg_006_at_start() {
    let error = ToolGateway::start(
        manifest("flood"),
        &sandbox_roots(),
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
        Recorder::default(),
        std::iter::empty::<String>(),
    )
    .expect_err("the listing floods the output cap");
    assert_eq!(error.code(), ErrorCode::PLG_006);
}

#[test]
fn a_call_result_over_the_manifest_cap_is_plg_006_during_execution() {
    let mut gateway = start(
        "flood-result",
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
    );

    let error = gateway
        .call(call())
        .expect_err("the result floods the output cap");
    assert_eq!(error.code(), ErrorCode::PLG_006);
    assert!(!gateway.is_running());

    let (_, audit) = gateway.into_parts();
    match &audit.events[1] {
        GatewayEvent::Completed {
            result: ResultSummary::Refused { stage, code, .. },
            ..
        } => {
            assert_eq!(*stage, RefusalStage::Execution);
            assert_eq!(*code, Some(ErrorCode::PLG_006));
        }
        other => panic!("expected an execution refusal, found {other:?}"),
    }
}

#[test]
fn an_envelope_over_the_manifest_cap_is_refused_after_execution() {
    // The raw result frame fits inside `max_output_bytes`; the envelope's host metadata and trust
    // marker push it over. This is the case only the gateway can catch.
    let mut gateway = start_with(
        with_limits(manifest("ok"), 2_000, 2_048),
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
        Recorder::default(),
        &[],
    );

    let error = gateway
        .call(ToolCall::new(TOOL, json!({"text": "x".repeat(1_900)})))
        .expect_err("the normalised envelope is over the cap");
    assert_eq!(error.code(), ErrorCode::PLG_006);

    let (_, audit) = gateway.into_parts();
    match &audit.events[1] {
        GatewayEvent::Completed {
            result: ResultSummary::Refused { stage, code, .. },
            ..
        } => {
            assert_eq!(*stage, RefusalStage::Envelope);
            assert_eq!(*code, Some(ErrorCode::PLG_006));
        }
        other => panic!("expected an envelope refusal, found {other:?}"),
    }
}

#[test]
fn a_plugin_reported_error_is_data_not_a_gateway_error() {
    let mut gateway = start(
        "is-error",
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
    );

    let outcome = gateway
        .call(call())
        .expect("is_error is data, not a failure");
    let envelope = outcome.result().expect("the plugin answered");
    assert!(envelope.is_error());
    assert_eq!(
        envelope.content().first().and_then(|block| block.as_text()),
        Some("the fixture refused the call")
    );

    let (_, audit) = gateway.into_parts();
    assert!(matches!(
        &audit.events[1],
        GatewayEvent::Completed {
            result: ResultSummary::Succeeded { is_error: true, .. },
            ..
        }
    ));
}

#[test]
fn control_characters_and_ansi_escapes_are_stripped_from_a_result() {
    let mut gateway = start(
        "ansi",
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
    );

    let outcome = gateway.call(call()).expect("the plugin answers");
    let envelope = outcome.result().expect("the plugin answered");

    let text = envelope
        .content()
        .first()
        .and_then(|block| block.as_text())
        .expect("the fixture returns a text block");
    assert_eq!(text, "safe red  next line");
    assert!(!text.contains('\u{1b}'));
    assert!(!text.chars().any(char::is_control));

    let extra = envelope.extra();
    let value = extra.get("extra ").expect("the key is sanitised and kept");
    assert_eq!(value["value"], json!("kept "));
}

#[test]
fn two_result_keys_that_collide_after_sanitising_are_plg_005() {
    let mut gateway = start(
        "key-collision",
        policy(PermissionMode::Allow),
        ScriptedPrompter::default(),
    );

    let error = gateway
        .call(call())
        .expect_err("the keys collide after sanitising");
    assert_eq!(error.code(), ErrorCode::PLG_005);
    assert!(matches!(error, GatewayError::Envelope(_)));

    let (_, audit) = gateway.into_parts();
    match &audit.events[1] {
        GatewayEvent::Completed {
            result: ResultSummary::Refused { stage, code, .. },
            ..
        } => {
            assert_eq!(*stage, RefusalStage::Envelope);
            assert_eq!(*code, Some(ErrorCode::PLG_005));
        }
        other => panic!("expected an envelope refusal, found {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Structural invariants (`docs/SECURITY.md` §5 controls 4 and 6).
// ---------------------------------------------------------------------------

fn rust_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("the source directory is readable") {
        let path = entry.expect("the directory entry is readable").path();
        if path.is_dir() {
            rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn production_sources() -> (PathBuf, Vec<PathBuf>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    files.sort();
    (root, files)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("the source is under src")
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn no_bypass_path_to_plugin() {
    // Exactly one production call site may invoke the supervisor's `call_tool`, and it must be the
    // gateway. The supervisor test file is a test, not production source, and is not scanned.
    let (root, sources) = production_sources();
    let mut call_sites = Vec::new();
    let mut command_sites = Vec::new();

    for path in &sources {
        let text = fs::read_to_string(path).expect("the source is UTF-8");
        let name = relative(&root, path);
        for line in text.lines() {
            if line.contains(".call_tool(") {
                call_sites.push(name.clone());
            }
            if line.contains("Command::new(") {
                command_sites.push(name.clone());
            }
        }
    }

    assert_eq!(
        call_sites,
        vec!["gateway/mod.rs".to_owned()],
        "there must be exactly one production path from a tool call to a plugin"
    );
    assert_eq!(
        command_sites,
        vec!["supervisor/process.rs".to_owned()],
        "only the supervisor may construct the plugin process"
    );
}

#[test]
fn no_shell_execution_path() {
    let (root, sources) = production_sources();
    let mut problems = Vec::new();

    for path in &sources {
        let text = fs::read_to_string(path).expect("the source is UTF-8");
        let name = relative(&root, path);

        // No production code may name a hard-coded program. The plugin command comes from the
        // manifest and is passed to `Command::new` by reference in `supervisor/process.rs`.
        if text.contains("Command::new(\"") {
            problems.push(format!("{name}: hard-coded process command"));
        }
        for shell in ["cmd.exe", "powershell", "\"sh\"", "\"-c\""] {
            if text.contains(shell) {
                problems.push(format!("{name}: shell fragment {shell}"));
            }
        }
        if name.starts_with("gateway/")
            && (text.contains("std::process") || text.contains("Command"))
        {
            problems.push(format!("{name}: the gateway must not construct a process"));
        }
    }

    assert!(
        problems.is_empty(),
        "the plugin path must have no shell sink: {problems:#?}"
    );
}
