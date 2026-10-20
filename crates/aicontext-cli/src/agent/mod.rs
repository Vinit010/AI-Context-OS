//! `aicontext agent run`: assemble the context, ask the provider for a plan, write the plan — and
//! modify nothing else (`docs/CLI_SPEC.md` §4, `TASK-055`).
//!
//! # What the command is
//!
//! Plan mode is the whole of `agent` for now (`ADR-006` #1): the run grounds one task's plan in the
//! assembled context, hands it to the configured provider, and writes the answer to
//! `.aicontext/plans/<task>.md`. Nothing in the working tree is touched — the plan lands under
//! `.aicontext/`, which `init` git-ignores, and no other file is written at all. `--execute` and
//! `--allow` are deliberately not accepted: an execution mode must not exist until plan mode has
//! proved itself (`ADR-006` #7).
//!
//! # Where the pieces come from
//!
//! The context pipeline is the `context` command's, unchanged: register, retrieval plan seeded from
//! `--task`, assembly, budget. The call is the provider layer's from `TASK-051`/`TASK-054`: the
//! packet's policy becomes the request's [`PolicyRegion`], its retrieved documents become untrusted
//! [`DataBlock`]s (T9 — the two regions are two types, so the placement cannot be got wrong by
//! accident), and the [`TokenBudget`] is a hard ceiling the adapter sends and the report quotes. A
//! profile comes from `.ai/ai.yaml` exactly as `connect` wrote it, credential *source* included, and
//! is resolved at call time through the same resolver.
//!
//! # Free means free
//!
//! A run must cost nothing when the project is pointed at a local or free server: `auth: none`
//! resolves to no credential, so no keychain is read and no environment variable is needed, and the
//! call succeeds for the same reason `connect`'s does. There is no code path where a missing paid
//! key degrades the run — a credential the profile *names* and cannot find is a failure, reported
//! by name (`RULES.md` §4.10).
//!
//! # Seams
//!
//! Environment, keychain, and the call are three ports ([`Ports`]), the same three `connect` uses.
//! Production wires the real ones; a unit test injects fakes, so no test reads the developer's
//! environment, touches the developer's keychain, or opens a socket (`RULES.md` §8).

pub(crate) mod error;
pub(crate) mod planfile;
pub(crate) mod proposal;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use aicontext_context::{
    ContextAssembler, ContextIndex, ContextPacket, PlanOptions, Register, RetrievalPlan, TaskEntry,
};
use aicontext_providers::{
    Adapter, CompletionRequest, CompletionResponse, CredentialResolver, DataBlock, Environment,
    FinishReason, Keychain, KeyringKeychain, Message, ModelOptions, PolicyBlock, PolicyRegion,
    ProcessEnvironment, ProviderError, ProviderId, Role, Secret, TokenBudget, Trust, Usage,
};
use serde::Serialize;

use crate::args::{AgentRunArgs, GlobalArgs};
use crate::connect::{AiConfig, ProviderEntry};
use crate::exit::Exit;
use crate::output::{Finding, Terminal, envelope};
use crate::project;

use error::AgentError;
use planfile::PLANS_PATH;
use proposal::Parsed;

/// The output-token ceiling on the plan call. A hard ceiling, not a hint: the adapter sends it,
/// contract test T11 proves each adapter honours it, and the run reports it beside the usage.
const PLAN_MAX_OUTPUT_TOKENS: u32 = 4000;

/// The warning a run leaves when the model hit that ceiling (`docs/CLI_SPEC.md` §6 findings).
const TRUNCATED_WARNING: &str = "AGENT-006";

/// The three effects `agent run` needs, so a test can replace each in isolation.
struct Ports<'a> {
    /// Reads the credential *source* the profile records.
    environment: &'a dyn Environment,
    /// Reads the keychain entry the profile falls back to.
    keychain: &'a dyn Keychain,
    /// Makes the one call that asks for the plan.
    planner: &'a dyn Planner,
}

/// The one plan call, behind a port so a unit test runs the whole command without a socket.
trait Planner {
    /// Builds the adapter for `entry` and asks it for the plan.
    ///
    /// # Errors
    ///
    /// Returns whatever the adapter returns: [`ProviderError::Transport`] when the server cannot be
    /// reached, [`ProviderError::Unauthorized`] when the credential is rejected, and so on. The
    /// command maps those in the provider crate's own vocabulary rather than inventing one.
    fn plan(
        &self,
        entry: &ProviderEntry,
        adapter: Adapter,
        credential: Option<&Secret>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError>;
}

/// The real planner: one `complete` call through whichever adapter the profile names.
struct HttpPlanner;

impl Planner for HttpPlanner {
    fn plan(
        &self,
        entry: &ProviderEntry,
        adapter: Adapter,
        credential: Option<&Secret>,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let id = ProviderId::new(&entry.id)?;
        let provider = adapter.build(
            id.clone(),
            &entry.base_url.clone().unwrap_or_default(),
            &entry.model.clone().unwrap_or_default(),
            credential.cloned(),
        )?;

        // The provider is asynchronous; the CLI is not. One current-thread runtime drives the one
        // call and is dropped immediately, exactly as `connect` does it.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| ProviderError::Transport {
                provider: id,
                source: Box::new(source),
            })?;
        runtime.block_on(provider.complete(request))
    }
}

/// Runs `agent run` and returns the exit code.
pub(crate) fn run(args: &AgentRunArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return AgentError::from(error).report(terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return AgentError::from(error).report(terminal),
    };

    let environment = ProcessEnvironment;
    let keychain = KeyringKeychain;
    let planner = HttpPlanner;
    let ports = Ports {
        environment: &environment,
        keychain: &keychain,
        planner: &planner,
    };

    let mut findings = Vec::new();
    match execute(args, global.offline, &root, &ports, &mut findings) {
        Ok(outcome) => {
            if global.json {
                report_json(&outcome, findings, terminal)
            } else {
                print_human(&outcome, &name, &root, findings, terminal);
                Exit::Ok
            }
        }
        Err(error) => error.report(terminal),
    }
}

/// The whole command, with the three ports supplied. `run` wraps this with the real ones.
fn execute(
    args: &AgentRunArgs,
    offline: bool,
    root: &Path,
    ports: &Ports<'_>,
    findings: &mut Vec<Finding>,
) -> Result<Outcome, AgentError> {
    let register = aicontext_context::register::read(root);
    let task = task_of(&register, &args.task)?;

    let config = AiConfig::load(root)?;
    let config_path = config.display();
    let known = known_providers(&config)?;
    let provider_id = config
        .default_provider()
        .ok_or_else(|| AgentError::NoDefaultProvider {
            path: config_path.clone(),
            known: known.clone(),
        })?;
    let entry = config
        .provider(&provider_id)?
        .ok_or_else(|| AgentError::DefaultNotListed {
            id: provider_id,
            path: config_path,
            known,
        })?;
    let adapter = Adapter::from_name(&entry.adapter).ok_or_else(|| {
        AgentError::UnsupportedAdapter {
            adapter: entry.adapter.clone(),
            known: Adapter::ALL.map(Adapter::name).join(", "),
        }
    })?;

    // The same cache-warm scan `context` uses: the only file it may write is the rebuildable,
    // git-ignored index under `.aicontext/`.
    let index = crate::context::read_index(root, findings);
    let plan = RetrievalPlan::for_task(&register, &args.task, &PlanOptions::default());
    let packet = ContextAssembler::new().assemble(&plan, &index)?;

    let context = ContextCounts {
        policy: packet.policy.len(),
        retrieved: packet.retrieved.len(),
        budget_requested: packet.budget.requested,
        budget_used: packet.budget.used,
    };

    if args.dry_run {
        return Ok(Outcome::dry_run(&args.task, &entry, context));
    }
    if offline {
        return Err(AgentError::Offline);
    }

    let source = entry.auth.source(&entry.id);
    let credential = CredentialResolver::new(ports.environment, ports.keychain)
        .resolve(&source, None)?;

    let request = build_request(task, &packet, &index);
    let response = ports
        .planner
        .plan(&entry, adapter, credential.as_ref(), request)?;

    // The finish reason is read before the text: a refusal is reported with the model's words and
    // never turned into a file (`docs/AI_PROVIDER_SPEC.md` §6), and a tool call cannot be honoured
    // by a run that offered no tools.
    match &response.finish_reason {
        FinishReason::Refusal => {
            return Err(AgentError::Declined {
                text: response.text.clone(),
            });
        }
        FinishReason::Error => {
            return Err(AgentError::UnexpectedResponse {
                detail: "the provider reported a failure in-band".to_string(),
            });
        }
        FinishReason::ToolUse => {
            return Err(AgentError::UnexpectedResponse {
                detail: "the model proposed tool calls, and plan mode offers none".to_string(),
            });
        }
        _ => {}
    }
    let truncated = response.finish_reason == FinishReason::Length;
    if truncated {
        findings.push(Finding::new(
            TRUNCATED_WARNING,
            "warning",
            PLANS_PATH,
            "the model hit the output-token ceiling; the plan may be incomplete",
            format!("ask again with a smaller task, or raise the ceiling above {PLAN_MAX_OUTPUT_TOKENS} tokens"),
        ));
    }

    // §9, as the schema states it: prose is a plan, and a claim of being a proposal must prove it.
    let (body, structured) = match proposal::parse(&response.text) {
        Parsed::Structured(proposal) => (planfile::Body::Structured(proposal), true),
        Parsed::Prose(text) => (planfile::Body::Prose(text), false),
        Parsed::Invalid(problems) => return Err(AgentError::MalformedProposal { problems }),
    };

    let usage = UsageSummary::of(&response.usage);
    let relative = PathBuf::from(PLANS_PATH).join(planfile::filename(&args.task));
    let document = planfile::render(
        &planfile::Meta {
            task_id: &args.task,
            task_title: task.title.as_deref(),
            task_status: task.status.as_deref(),
            provider: &provider_line(&entry),
            sources: &sources(&packet),
            usage: &usage_text(&usage),
            truncated,
        },
        &body,
    );
    write_plan(root, &relative, &document)?;

    Ok(Outcome::planned(
        &args.task,
        task.title.as_deref(),
        &entry,
        context,
        project::display(&relative),
        structured,
        usage,
        truncated,
    ))
}

/// The task `--task` names, when the register declares it.
///
/// An id the register does not declare is refused rather than planned blind: the packet would be
/// assembled from no task's edges, and a plan for a task that does not exist is a silent wrong
/// answer (`RULES.md` §4.10).
fn task_of<'a>(register: &'a Register, task_id: &str) -> Result<&'a TaskEntry, AgentError> {
    register
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .ok_or_else(|| AgentError::UnknownTask {
            id: task_id.to_string(),
            known: {
                let ids: Vec<_> = register.tasks.iter().map(|task| task.id.clone()).collect();
                if ids.is_empty() {
                    "none".to_string()
                } else {
                    ids.join(", ")
                }
            },
        })
}

/// The ids `.ai/ai.yaml` lists, or `none`.
fn known_providers(config: &AiConfig) -> Result<String, AgentError> {
    let ids = config.provider_ids()?;
    Ok(if ids.is_empty() {
        "none".to_string()
    } else {
        ids.join(", ")
    })
}

/// Writes the plan document, creating the plans directory when it does not exist yet.
fn write_plan(root: &Path, relative: &Path, document: &str) -> Result<(), AgentError> {
    let path = root.join(relative);
    let spell = || project::display(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| AgentError::PlanWrite {
            path: spell(),
            reason: error.to_string(),
        })?;
    }
    std::fs::write(&path, document).map_err(|error| AgentError::PlanWrite {
        path: spell(),
        reason: error.to_string(),
    })
}

/// The provider line the plan file and the human report both lead with.
fn provider_line(entry: &ProviderEntry) -> String {
    format!(
        "{} ({}) {}",
        entry.id,
        entry.adapter,
        entry.model.clone().unwrap_or_default()
    )
}

/// The context sources the plan was grounded in, one line per document the packet held.
fn sources(packet: &ContextPacket) -> Vec<String> {
    packet
        .policy
        .iter()
        .map(|item| {
            format!(
                "{} (tier {}) {}",
                item.id,
                item.tier,
                item.content_ref.display()
            )
        })
        .chain(packet.retrieved.iter().map(|item| {
            format!(
                "{} (tier {}) {}",
                item.id,
                item.tier,
                item.content_ref.display()
            )
        }))
        .collect()
}

/// The request one plan run makes: policy in the policy region, documents as untrusted data.
///
/// The instruction is the host's own words (Z3): it states the mode, the output contract, and the
/// plan contract. The documents never enter it — `AI_PROVIDER_SPEC.md` §2.1 puts them in
/// [`DataBlock`]s, and T9 is what proves each adapter keeps them there.
fn build_request(task: &TaskEntry, packet: &ContextPacket, index: &ContextIndex) -> CompletionRequest {
    CompletionRequest {
        policy: PolicyRegion {
            instruction: Some(instruction(task)),
            policy: policy_blocks(packet, index),
        },
        messages: vec![Message::new(
            Role::User,
            format!(
                "Produce the implementation plan for {}{}.",
                task.id,
                task.title
                    .as_deref()
                    .map_or_else(String::new, |title| format!(" — {title}"))
            ),
        )],
        data: data_blocks(packet, index),
        tools: Vec::new(),
        budget: TokenBudget {
            max_output_tokens: PLAN_MAX_OUTPUT_TOKENS,
        },
        options: ModelOptions::default(),
        // No idempotency key: with none, the transport attempts the call exactly once, and a plan
        // is worth more predictable than resilient — a silent retry is a second spend of the
        // developer's tokens with a different answer attached to the same request.
        idempotency_key: String::new(),
    }
}

/// The instruction a plan run is bounded by.
fn instruction(task: &TaskEntry) -> String {
    let title = task.title.as_deref().unwrap_or("(untitled)");
    let status = task.status.as_deref().unwrap_or("unrecorded");
    let touches = if task.touches.is_empty() {
        String::new()
    } else {
        format!(
            "\nTouches: {} — the paths this task may change.",
            task.touches.join(", ")
        )
    };
    format!(
        "You are `aicontext` in plan mode for the task below. You write a plan; you cannot execute \
         anything, and nothing you propose runs in this session.\n\
         The plan must list the files to modify, the files to create, and the files to delete; the \
         dependencies affected; the schema and data impact; and the security impact. Ground it in \
         the policy and the retrieved documents attached to this request, and prefer citing an \
         attached document over asserting from memory.\n\
         Reply with a JSON `action_proposal` — kind `action_proposal`, version 1, actions of type \
         `plan` (summary, files_modify, files_create, files_delete, risks) or `tool_call` (tool, \
         arguments, reason) — or reply with a prose plan. A proposed tool call is recorded in the \
         plan; this run executes none.\n\
         Task: {} — {title} (status {status}).{touches}",
        task.id
    )
}

/// The packet's policy documents, bodies read from the same index assembly read.
fn policy_blocks(packet: &ContextPacket, index: &ContextIndex) -> Vec<PolicyBlock> {
    packet
        .policy
        .iter()
        .map(|item| PolicyBlock {
            source: item.id.clone(),
            path: item.content_ref.display().to_string(),
            content: content_of(index, &item.content_ref),
        })
        .collect()
}

/// The packet's retrieved documents as untrusted data, plus the conflicts the ladder found.
fn data_blocks(packet: &ContextPacket, index: &ContextIndex) -> Vec<DataBlock> {
    let mut blocks: Vec<DataBlock> = packet
        .retrieved
        .iter()
        .map(|item| DataBlock {
            source: item.id.clone(),
            path: item.content_ref.display().to_string(),
            tier: item.tier.number(),
            trust: Trust::Untrusted,
            content: content_of(index, &item.content_ref),
        })
        .collect();
    if !packet.conflicts.is_empty() {
        let mut text = String::new();
        for conflict in &packet.conflicts {
            let _ = writeln!(
                text,
                "{}: {} (tier {}) vs {} (tier {})",
                conflict.key,
                conflict.asserted_by.id,
                conflict.asserted_by.tier,
                conflict.conflicts_with.id,
                conflict.conflicts_with.tier
            );
        }
        blocks.push(DataBlock {
            source: "aicontext-conflicts".to_string(),
            path: String::new(),
            // The winning assertion's tier: the conflicts are a fact about the documents that are
            // here, so the block carries the rung they were asserted on.
            tier: packet.conflicts[0].asserted_by.tier.number(),
            trust: Trust::Untrusted,
            content: text.trim_end().to_string(),
        });
    }
    blocks
}

/// The body of an indexed document, or the same placeholder `render_prompt` uses.
///
/// The helper is private to the context crate; a second user is not yet three, so it is written
/// here rather than shared (`RULES.md` §2).
fn content_of(index: &ContextIndex, reference: &Path) -> String {
    index.get(&reference.to_string_lossy()).map_or_else(
        || format!("[not indexed: {}]", reference.display()),
        |entry| entry.body.trim_end().to_string(),
    )
}

/// What one run produced, in the terms the report and the JSON payload both use.
#[derive(Debug)]
struct Outcome {
    task: String,
    task_title: Option<String>,
    provider: String,
    adapter: String,
    model: String,
    dry_run: bool,
    plan_path: Option<String>,
    structured: Option<bool>,
    usage: Option<UsageSummary>,
    context: ContextCounts,
    truncated: bool,
}

impl Outcome {
    /// The `--dry-run` outcome: context assembled, no call, nothing written.
    fn dry_run(task_id: &str, entry: &ProviderEntry, context: ContextCounts) -> Self {
        Self {
            task: task_id.to_string(),
            task_title: None,
            provider: entry.id.clone(),
            adapter: entry.adapter.clone(),
            model: entry.model.clone().unwrap_or_default(),
            dry_run: true,
            plan_path: None,
            structured: None,
            usage: None,
            context,
            truncated: false,
        }
    }

    /// The planned outcome: the call answered and the document is on disk.
    #[allow(clippy::too_many_arguments, reason = "one outcome, one constructor")]
    fn planned(
        task_id: &str,
        task_title: Option<&str>,
        entry: &ProviderEntry,
        context: ContextCounts,
        plan_path: String,
        structured: bool,
        usage: UsageSummary,
        truncated: bool,
    ) -> Self {
        Self {
            task: task_id.to_string(),
            task_title: task_title.map(str::to_string),
            provider: entry.id.clone(),
            adapter: entry.adapter.clone(),
            model: entry.model.clone().unwrap_or_default(),
            dry_run: false,
            plan_path: Some(plan_path),
            structured: Some(structured),
            usage: Some(usage),
            context,
            truncated,
        }
    }
}

/// The context counts a run reports, in both output shapes.
#[derive(Clone, Copy, Debug, Serialize)]
struct ContextCounts {
    /// Trusted policy documents in the packet.
    policy: usize,
    /// Untrusted retrieved documents.
    retrieved: usize,
    /// The context budget the plan asked for.
    budget_requested: u32,
    /// What assembly spent of it.
    budget_used: u32,
}

/// Token usage as the report spells it, with the estimate flag kept: a number the host derived and
/// a number the provider reported are never conflated (`AI_PROVIDER_SPEC.md` §7).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UsageSummary {
    input: Option<u32>,
    output: Option<u32>,
    cached: Option<u32>,
    estimated: bool,
}

impl UsageSummary {
    /// The provider's own usage, verbatim.
    const fn of(usage: &Usage) -> Self {
        Self {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cached: usage.cached_input_tokens,
            estimated: usage.estimated,
        }
    }
}

/// The usage line: what the provider reported, or what it did not.
fn usage_text(usage: &UsageSummary) -> String {
    let (Some(input), Some(output)) = (usage.input, usage.output) else {
        return "no token counts reported by the server".to_string();
    };
    let cached = usage.cached.unwrap_or(0);
    let origin = if usage.estimated {
        "host estimate"
    } else {
        "provider-reported"
    };
    format!("in {input}, out {output}, cached {cached} — {origin}")
}

/// The `--json` payload: names, counts, and usage — never the model's text, which the plan file
/// already holds, and never a credential, which the request never had reason to echo.
#[derive(Debug, Serialize)]
struct AgentData {
    task: String,
    task_title: Option<String>,
    provider: String,
    adapter: String,
    model: String,
    dry_run: bool,
    /// The plan's path, repository-relative, when one was written.
    plan: Option<String>,
    /// Whether the plan came from a schema-validated `action_proposal`, when a call was made.
    structured: Option<bool>,
    context: ContextCounts,
    /// Token usage, when a call was made.
    usage: Option<UsageData>,
    /// Whether the output-token ceiling was reached.
    truncated: bool,
}

/// Token usage in the JSON shape.
#[derive(Debug, Serialize)]
struct UsageData {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    cached_input_tokens: Option<u32>,
    /// Whether the host derived these numbers rather than the provider reporting them.
    estimated: bool,
}

/// Writes the JSON envelope for a successful run.
fn report_json(outcome: &Outcome, findings: Vec<Finding>, terminal: &mut Terminal) -> Exit {
    let data = AgentData {
        task: outcome.task.clone(),
        task_title: outcome.task_title.clone(),
        provider: outcome.provider.clone(),
        adapter: outcome.adapter.clone(),
        model: outcome.model.clone(),
        dry_run: outcome.dry_run,
        plan: outcome.plan_path.clone(),
        structured: outcome.structured,
        context: outcome.context,
        usage: outcome.usage.map(|usage| UsageData {
            input_tokens: usage.input,
            output_tokens: usage.output,
            cached_input_tokens: usage.cached,
            estimated: usage.estimated,
        }),
        truncated: outcome.truncated,
    };
    let summary = if outcome.dry_run {
        "dry run: no call was made, nothing was written".to_string()
    } else {
        format!(
            "plan written to {}",
            outcome.plan_path.clone().unwrap_or_default()
        )
    };
    let body = envelope("agent", Exit::Ok.code(), data, findings, summary);
    if let Err(error) = terminal.json(&body) {
        terminal.error_report(
            "AGENT-010",
            &format!("the JSON report could not be written: {error}"),
            "this is a defect in aicontext; please report it",
        );
        return Exit::General;
    }
    Exit::Ok
}

/// Prints the human report: the identity line, the task, the context, the plan, and the usage.
fn print_human(
    outcome: &Outcome,
    name: &str,
    root: &Path,
    findings: Vec<Finding>,
    terminal: &mut Terminal,
) {
    let info = terminal.mark_info();
    let ok = terminal.mark_ok();
    let warn = terminal.mark_warn();

    terminal.identity("aicontext", name, project::vcs_label(root));
    let task = match &outcome.task_title {
        Some(title) => format!("{} — {title}", outcome.task),
        None => outcome.task.clone(),
    };
    terminal.row(&info, "task", &task);
    terminal.row(
        &info,
        "provider",
        &format!(
            "{} ({}) {}",
            outcome.provider, outcome.adapter, outcome.model
        ),
    );
    terminal.row(
        &info,
        "context",
        &format!(
            "{} policy, {} retrieved, {}/{} token budget",
            outcome.context.policy,
            outcome.context.retrieved,
            outcome.context.budget_used,
            outcome.context.budget_requested
        ),
    );
    if let Some(path) = &outcome.plan_path {
        let shape = if outcome.structured == Some(true) {
            "validated action proposal"
        } else {
            "prose plan"
        };
        terminal.row(&ok, "plan", &format!("{path} ({shape})"));
    } else {
        terminal.row(&ok, "dry-run", "no call was made, nothing was written");
    }
    if let Some(usage) = &outcome.usage {
        terminal.row(&ok, "usage", &usage_text(usage));
    }
    for finding in findings {
        terminal.row(
            &warn,
            "warning",
            &format!("{} {}", finding.code, finding.message),
        );
    }
    if outcome.dry_run {
        terminal.summary("dry run", Exit::Ok, "`aicontext agent run --task <ID>`");
    } else {
        let next = outcome
            .plan_path
            .as_deref()
            .map_or_else(|| String::new(), |path| format!("review {path}"));
        terminal.summary("plan written", Exit::Ok, &next);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use aicontext_providers::{
        Adapter, CompletionRequest, CompletionResponse, Environment, FinishReason, Keychain,
        KeychainError, KeychainReference, ProviderError, Usage,
    };

    use super::{
        AgentRunArgs, Exit, Finding, Planner, Ports, ProviderEntry, Trust, UsageSummary, execute,
    };

    /// A project with the four documents every fixture needs, and the profile under test.
    fn fixture(ai_yaml: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let outer = tempfile::TempDir::new().expect("temp dir");
        let root = outer.path().to_path_buf();
        std::fs::create_dir_all(root.join(".ai")).expect("create .ai");
        std::fs::write(root.join(".ai/AI.md"), "# AI\n\nThe entry point.\n").expect("write AI.md");
        std::fs::write(root.join(".ai/RULES.md"), "# Rules\n\nBe correct.\n")
            .expect("write RULES.md");
        std::fs::write(
            root.join(".ai/TASKS.md"),
            "# Tasks\n\n**Current phase:** Phase 1\n**Current task:** TASK-001\n\n\
             ### TASK-001 — Write the fixture plan\n\n```yaml\nid: TASK-001\n\
             title: Write the fixture plan\nstatus: TODO\npriority: HIGH\n\
             touches: [\"crates/example/**\"]\n```\n",
        )
        .expect("write TASKS.md");
        std::fs::write(root.join(".ai/ai.yaml"), ai_yaml).expect("write ai.yaml");
        (outer, root)
    }

    /// A free local server: no credential is recorded, so none can be missing.
    const FREE_YAML: &str = "default_provider: local\nproviders:\n  - id: local\n    \
        adapter: openai-compatible\n    base_url: http://127.0.0.1:9/v1\n    model: test-model\n    \
        auth:\n      type: none\n";

    /// The environment a test sees: exactly the variables the test wrote into it.
    #[derive(Debug, Default)]
    struct FakeEnvironment {
        variables: BTreeMap<String, aicontext_providers::Secret>,
    }

    impl Environment for FakeEnvironment {
        fn variable(&self, name: &str) -> Option<aicontext_providers::Secret> {
            self.variables.get(name).cloned()
        }
    }

    /// The keychain a test sees: exactly the entries the test wrote into it.
    #[derive(Debug, Default)]
    struct FakeKeychain {
        entries: RefCell<BTreeMap<String, aicontext_providers::Secret>>,
    }

    impl Keychain for FakeKeychain {
        fn get(
            &self,
            reference: &KeychainReference,
        ) -> Result<Option<aicontext_providers::Secret>, KeychainError> {
            Ok(self.entries.borrow().get(reference.account()).cloned())
        }

        fn set(
            &self,
            reference: &KeychainReference,
            secret: &aicontext_providers::Secret,
        ) -> Result<(), KeychainError> {
            self.entries
                .borrow_mut()
                .insert(reference.account().to_string(), secret.clone());
            Ok(())
        }

        fn delete(&self, reference: &KeychainReference) -> Result<bool, KeychainError> {
            Ok(self
                .entries
                .borrow_mut()
                .remove(reference.account())
                .is_some())
        }
    }

    /// The call a test makes, kept so a test can assert on the request it built.
    #[derive(Debug)]
    struct FakePlanner {
        answer: RefCell<Option<Result<aicontext_providers::CompletionResponse, ProviderError>>>,
        credential: RefCell<Option<Option<aicontext_providers::Secret>>>,
        requests: RefCell<Vec<CompletionRequest>>,
    }

    impl FakePlanner {
        /// A planner that answers `answer`, once.
        fn answering(
            answer: Result<aicontext_providers::CompletionResponse, ProviderError>,
        ) -> Self {
            Self {
                answer: RefCell::new(Some(answer)),
                credential: RefCell::new(None),
                requests: RefCell::new(Vec::new()),
            }
        }
    }

    impl Planner for FakePlanner {
        fn plan(
            &self,
            _entry: &ProviderEntry,
            _adapter: Adapter,
            credential: Option<&aicontext_providers::Secret>,
            request: CompletionRequest,
        ) -> Result<aicontext_providers::CompletionResponse, ProviderError> {
            self.credential
                .borrow_mut()
                .replace(credential.cloned());
            self.requests.borrow_mut().push(request);
            self.answer
                .borrow_mut()
                .take()
                .expect("the fake answers exactly once")
        }
    }

    /// A response that is a schema-valid proposal.
    fn proposal_response() -> CompletionResponse {
        CompletionResponse {
            text: r#"{"kind":"action_proposal","version":1,"actions":[{"type":"plan","summary":"Update the register and the fixture.","files_modify":[".ai/TASKS.md"],"files_create":[],"files_delete":[],"risks":["none"]}]}"#.to_string(),
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Stop,
            usage: Usage::reported(1200, 180),
            provider_meta: serde_json::Value::Null,
        }
    }

    fn args(task: &str) -> AgentRunArgs {
        AgentRunArgs {
            task: task.to_string(),
            dry_run: false,
        }
    }

    #[test]
    fn a_plan_run_writes_the_plan_and_reports_usage() {
        let (_outer, root) = fixture(FREE_YAML);
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let mut findings = Vec::new();

        let outcome = execute(&args("TASK-001"), false, &root, &ports, &mut findings)
            .expect("the run succeeds");

        assert_eq!(outcome.plan_path.as_deref(), Some(".aicontext/plans/TASK-001.md"));
        assert_eq!(outcome.structured, Some(true));
        assert_eq!(outcome.usage, Some(UsageSummary { input: Some(1200), output: Some(180), cached: None, estimated: false }));
        assert!(findings.is_empty());

        let document = std::fs::read_to_string(root.join(".aicontext/plans/TASK-001.md"))
            .expect("the plan was written");
        assert!(document.contains("# Plan — TASK-001: Write the fixture plan"));
        assert!(document.contains("## Files to modify"));
        assert!(document.contains("- `.ai/TASKS.md`"));
        assert!(document.contains("in 1200, out 180, cached 0 — provider-reported"));

        // The request it built: instruction in the policy region, RULES.md as trusted policy, and
        // a hard output ceiling.
        let request = &planner.requests.borrow()[0];
        let instruction = request.policy.instruction.as_deref().expect("an instruction");
        assert!(instruction.contains("plan mode"));
        assert!(instruction.contains("TASK-001"));
        assert!(
            request.policy.policy.iter().any(|block| block.content.contains("Be correct.")),
            "RULES.md is mandatory policy and must reach the policy region"
        );
        assert!(request.data.iter().all(|block| block.trust == Trust::Untrusted));
        assert_eq!(request.budget.max_output_tokens, 4000);
        assert!(request.idempotency_key.is_empty(), "one attempt, by design");
    }

    #[test]
    fn a_free_local_provider_runs_with_no_credential_at_all() {
        let (_outer, root) = fixture(FREE_YAML);
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let outcome = execute(&args("TASK-001"), false, &root, &ports, &mut Vec::new())
            .expect("a free provider needs no credential");
        assert!(outcome.plan_path.is_some());
        assert!(
            planner
                .credential
                .borrow()
                .as_ref()
                .is_some_and(Option::is_none),
            "auth: none resolves to no credential — no keychain, no variable, no key"
        );
    }

    #[test]
    fn a_dry_run_makes_no_call_and_writes_nothing() {
        let (_outer, root) = fixture(FREE_YAML);
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let mut dry = args("TASK-001");
        dry.dry_run = true;

        let outcome = execute(&dry, true, &root, &ports, &mut Vec::new())
            .expect("a dry run succeeds, offline included");

        assert!(outcome.dry_run);
        assert!(outcome.usage.is_none());
        assert!(planner.requests.borrow().is_empty(), "no call was made");
        assert!(
            !root.join(".aicontext/plans").exists(),
            "nothing was written"
        );
    }

    #[test]
    fn an_offline_run_refuses_the_call_but_a_dry_run_still_works() {
        let (_outer, root) = fixture(FREE_YAML);
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let error = execute(&args("TASK-001"), true, &root, &ports, &mut Vec::new())
            .expect_err("`--offline` forbids the call");
        assert_eq!(error.code(), "AGENT-003");
        assert_eq!(error.exit(), Exit::Usage);
        assert!(planner.requests.borrow().is_empty());
    }

    #[test]
    fn no_provider_configured_fails_before_anything_is_built() {
        let (_outer, root) = fixture("providers:\n  - id: local\n    adapter: openai-compatible\n");
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let error = execute(&args("TASK-001"), false, &root, &ports, &mut Vec::new())
            .expect_err("there is no default provider");
        assert_eq!(error.code(), "AGENT-001");
        assert_eq!(error.exit(), Exit::Validation);
        assert!(error.remediation().contains("aicontext connect"));
        assert!(planner.requests.borrow().is_empty());
    }

    #[test]
    fn an_unknown_task_is_refused_naming_the_ones_that_exist() {
        let (_outer, root) = fixture(FREE_YAML);
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let error = execute(&args("TASK-999"), false, &root, &ports, &mut Vec::new())
            .expect_err("the register declares no TASK-999");
        assert_eq!(error.code(), "AGENT-002");
        assert_eq!(error.exit(), Exit::Usage);
        assert!(error.to_string().contains("TASK-001"));
    }

    #[test]
    fn a_failed_call_writes_nothing_and_reports_the_provider() {
        let (_outer, root) = fixture(FREE_YAML);
        let planner = FakePlanner::answering(Err(ProviderError::Unauthorized {
            provider: aicontext_providers::ProviderId::new("local").expect("valid id"),
        }));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let error = execute(&args("TASK-001"), false, &root, &ports, &mut Vec::new())
            .expect_err("the provider rejected the call");
        assert_eq!(error.code(), "PROVIDER_UNAUTHORIZED");
        assert_eq!(error.exit(), Exit::Provider);
        assert!(
            !root.join(".aicontext/plans").exists(),
            "a failure is not a degraded success: no plan is written"
        );
    }

    #[test]
    fn a_malformed_proposal_is_rejected_and_nothing_is_written() {
        let (_outer, root) = fixture(FREE_YAML);
        let mut response = proposal_response();
        response.text =
            r#"{"kind":"action_proposal","version":1,"actions":[{"type":"dance"}]}"#.to_string();
        let planner = FakePlanner::answering(Ok(response));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let error = execute(&args("TASK-001"), false, &root, &ports, &mut Vec::new())
            .expect_err("an unknown action is rejected whole");
        assert_eq!(error.code(), "AGENT-005");
        assert_eq!(error.exit(), Exit::Validation);
        assert!(!root.join(".aicontext/plans").exists());
    }

    #[test]
    fn a_refusal_is_reported_with_the_models_words_and_writes_no_file() {
        let (_outer, root) = fixture(FREE_YAML);
        let mut response = proposal_response();
        response.text = "I will not plan that change.".to_string();
        response.finish_reason = FinishReason::Refusal;
        let planner = FakePlanner::answering(Ok(response));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let error = execute(&args("TASK-001"), false, &root, &ports, &mut Vec::new())
            .expect_err("a refusal is a failure to plan, not a plan");
        assert_eq!(error.code(), "AGENT-007");
        assert!(error.to_string().contains("I will not plan that change."));
        assert!(!root.join(".aicontext/plans").exists());
    }

    #[test]
    fn a_truncated_run_warns_and_writes_the_partial_plan() {
        let (_outer, root) = fixture(FREE_YAML);
        let mut response = proposal_response();
        response.finish_reason = FinishReason::Length;
        response.text = "Start the plan".to_string();
        let planner = FakePlanner::answering(Ok(response));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let mut findings = Vec::new();
        let outcome = execute(&args("TASK-001"), false, &root, &ports, &mut findings)
            .expect("a truncated plan is still a plan, warned");
        assert!(outcome.truncated);
        assert_eq!(outcome.structured, Some(false));
        let warning: Vec<&Finding> = findings.iter().collect();
        assert_eq!(warning.len(), 1);
        assert_eq!(warning[0].code, "AGENT-006");
        let document = std::fs::read_to_string(root.join(".aicontext/plans/TASK-001.md"))
            .expect("the partial plan was written");
        assert!(document.contains("may be incomplete"));
    }

    #[test]
    fn an_env_credential_is_resolved_by_name_and_never_reaches_the_file() {
        let (_outer, root) = fixture(
            "default_provider: hosted\nproviders:\n  - id: hosted\n    \
             adapter: openai-compatible\n    base_url: http://127.0.0.1:9/v1\n    model: m\n    \
             auth:\n      type: env_or_keychain\n      env: AICON_TEST_KEY\n",
        );
        let before =
            std::fs::read_to_string(root.join(".ai/ai.yaml")).expect("the profile is readable");
        let environment = FakeEnvironment {
            variables: [(
                "AICON_TEST_KEY".to_string(),
                aicontext_providers::Secret::new("sk-test".to_string()),
            )]
            .into_iter()
            .collect(),
        };
        let keychain = FakeKeychain::default();
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let outcome = execute(&args("TASK-001"), false, &root, &ports, &mut Vec::new())
            .expect("the named variable resolves");
        assert!(outcome.plan_path.is_some());
        let credential = planner.credential.borrow();
        let secret = credential
            .as_ref()
            .expect("the call carried a credential")
            .as_ref()
            .expect("and it was there");
        assert_eq!(secret.expose(), "sk-test");
        let after =
            std::fs::read_to_string(root.join(".ai/ai.yaml")).expect("the profile is readable");
        assert_eq!(before, after, "the value never reaches the committed file");
    }

    #[test]
    fn a_missing_named_credential_is_a_failure_not_a_bare_call() {
        let (_outer, root) = fixture(
            "default_provider: hosted\nproviders:\n  - id: hosted\n    \
             adapter: openai-compatible\n    base_url: http://127.0.0.1:9/v1\n    model: m\n    \
             auth:\n      type: env_or_keychain\n      env: AICON_ABSENT_KEY\n",
        );
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let error = execute(&args("TASK-001"), false, &root, &ports, &mut Vec::new())
            .expect_err("a credential the profile names must be found, not skipped");
        assert_eq!(
            error.code(),
            "CREDENTIAL_KEYCHAIN_MISSING",
            "env_or_keychain falls through to the keychain when the variable is unset, and the \
             entry is not there either"
        );
        assert!(planner.requests.borrow().is_empty(), "the call was never made");
    }

    #[test]
    fn an_unknown_adapter_in_the_profile_is_refused_naming_the_known_ones() {
        let (_outer, root) = fixture(
            "default_provider: local\nproviders:\n  - id: local\n    \
             adapter: telepathy\n    base_url: http://127.0.0.1:9/v1\n    model: m\n    \
             auth:\n      type: none\n",
        );
        let planner = FakePlanner::answering(Ok(proposal_response()));
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let ports = Ports {
            environment: &environment,
            keychain: &keychain,
            planner: &planner,
        };
        let error = execute(&args("TASK-001"), false, &root, &ports, &mut Vec::new())
            .expect_err("telepathy is not an adapter");
        assert_eq!(error.code(), "AGENT-004");
        assert!(error.to_string().contains("openai-compatible"));
    }
}
