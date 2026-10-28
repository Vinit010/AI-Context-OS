//! The tool gateway: the one path from a proposed tool call to a plugin.
//!
//! `docs/PLUGIN_SPEC.md` §1 states the rule this module exists to enforce: there is exactly one path
//! from an action to a plugin, and there is no second, faster, or internal path. A call reaches a
//! plugin only through [`ToolGateway::call`], and that method performs the same five steps every
//! time:
//!
//! ```text
//! arguments ── validate against input_schema ──▶ permission resolver ──▶ approval broker
//!                                                                             │
//!                                                       audit Executing ◀────┘
//!                                                              │
//!                                                         plugin call
//!                                                              │
//!                                          untrusted envelope ◀─┘
//!                                                              │
//!                                                       audit Completed
//! ```
//!
//! A failure before `Executing` is still recorded as `Completed { Refused }`, so a validation
//! error or a policy denial cannot disappear from the record. A failure to write either record
//! refuses the call with `PLG-010`, because an execution that cannot be audited is not allowed
//! (`docs/SECURITY.md` §8 and §9).
//!
//! # The manifest is the authority
//!
//! [`ToolGateway::start`] reads `tools/list` once and checks every descriptor against the manifest
//! before any call can happen. A descriptor for a tool the manifest does not declare, a declared
//! tool the plugin does not list, a duplicate tool, or a descriptor whose `effect`/`destructive`
//! disagrees with the manifest is refused. The manifest's `destructive` flag is what the resolver
//! receives, so a descriptor cannot lower the permission floor.
//!
//! # What is deliberately not here
//!
//! The sandbox (`TASK-076`) owns the working directory, environment, filesystem allowlist, and
//! network policy. The durable hash-chained log (`TASK-075`) owns storage; this module only depends
//! on its [`GatewayAudit`] seam. The CLI owns the terminal; [`Prompter`] is implemented there. No
//! shell is ever constructed here: the only process this layer can reach is the one
//! [`start_plugin`] started from the manifest's `entry.command`, with its arguments passed
//! verbatim.

mod audit;
mod envelope;
mod schema;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use aicontext_core::{AicontextError, ErrorCode, PermissionMode};
use aicontext_permissions::{CapabilityRequest, Policy};
use aicontext_plugin_sdk::{Capability, PluginManifest, ToolDefinition};
use serde_json::Value;

use crate::approval::{ApprovalBroker, ApprovalError, ApprovalRequest, Prompter};
use crate::hash::json_digest;
use crate::sandbox::{Sandbox, SandboxError, SandboxRoots};
use crate::supervisor::{PluginLog, PluginProcess, SupervisorError, start_plugin};

pub use audit::{CallSummary, GatewayAudit, GatewayEvent, RefusalStage, ResultSummary};
pub use envelope::{EnvelopeError, HostMeta, TRUST_UNTRUSTED, UntrustedToolResult};

use audit::{ApprovalBridge, CallSummary as Summary};
use envelope::normalise;
use schema::CompiledSchema;

/// One tool call proposed by a caller.
///
/// `target` is the resource shown in the approval prompt (an instance id, a repository, a path).
/// `reason` is the caller's explanation, shown verbatim and never written to the audit sink. The
/// arguments are validated before either reaches the human.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    tool: String,
    arguments: Value,
    target: String,
    reason: String,
}

impl ToolCall {
    /// A call to `tool` with `arguments`.
    #[must_use]
    pub fn new(tool: impl Into<String>, arguments: Value) -> Self {
        Self {
            tool: tool.into(),
            arguments,
            target: String::new(),
            reason: String::new(),
        }
    }

    /// Adds the resource the call targets.
    #[must_use]
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    /// Adds the caller's reason, shown in the prompt and never audited.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = reason.into();
        self
    }

    /// The capability's canonical tool name.
    #[must_use]
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The parsed call arguments.
    #[must_use]
    pub const fn arguments(&self) -> &Value {
        &self.arguments
    }

    /// The resource the call targets, or an empty string.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The caller's explanation, or an empty string.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// What a call produced.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GatewayOutcome {
    /// The plugin answered; the result is untrusted data.
    Executed(UntrustedToolResult),
    /// The policy or the human refused; the plugin was not called.
    Refused(Refusal),
}

impl GatewayOutcome {
    /// The envelope, when the call executed.
    #[must_use]
    pub const fn result(&self) -> Option<&UntrustedToolResult> {
        match self {
            Self::Executed(result) => Some(result),
            Self::Refused(_) => None,
        }
    }

    /// The refusal, when the call did not execute.
    #[must_use]
    pub const fn refusal(&self) -> Option<&Refusal> {
        match self {
            Self::Executed(_) => None,
            Self::Refused(refusal) => Some(refusal),
        }
    }

    /// Whether the plugin was called.
    #[must_use]
    pub const fn is_executed(&self) -> bool {
        matches!(self, Self::Executed(_))
    }
}

/// A policy or approval refusal, as a caller reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    stage: RefusalStage,
    reason: String,
    code: Option<ErrorCode>,
}

impl Refusal {
    const fn new(stage: RefusalStage, reason: String, code: Option<ErrorCode>) -> Self {
        Self {
            stage,
            reason,
            code,
        }
    }

    /// The first gate that refused the call.
    #[must_use]
    pub const fn stage(&self) -> RefusalStage {
        self.stage
    }

    /// A human-readable explanation. It never contains raw arguments.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The stable code, when the refusal is a `PLG` failure rather than a denial.
    #[must_use]
    pub const fn code(&self) -> Option<ErrorCode> {
        self.code
    }
}

/// A failure that prevented the gateway from producing a reliable outcome.
///
/// A policy denial and a human "no" are [`GatewayOutcome::Refused`], not errors. These variants are
/// the cases where the call cannot honestly be reported as either executed or denied: the manifest
/// and plugin disagree, the arguments cannot be validated, the process failed, or the audit record
/// could not be written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GatewayError {
    /// The manifest does not declare the capability, or the plugin did not expose it.
    #[error("the capability `{tool}` is not declared: {detail}")]
    Undeclared {
        /// The tool name.
        tool: String,
        /// Which side of the declaration is missing.
        detail: String,
    },
    /// The tool's `input_schema` is outside the subset the gateway can enforce.
    #[error("the input schema for `{tool}` cannot be enforced: {detail}")]
    InvalidSchema {
        /// The tool name.
        tool: String,
        /// The unsupported keyword or invalid value.
        detail: String,
    },
    /// The arguments do not satisfy the tool's `input_schema`.
    #[error("the arguments for `{tool}` are invalid: {detail}")]
    InvalidArguments {
        /// The tool name.
        tool: String,
        /// The first violation, with its path.
        detail: String,
    },
    /// The plugin's descriptor disagrees with the manifest or is duplicated.
    #[error("the descriptor for `{tool}` is not consistent with the manifest: {detail}")]
    Descriptor {
        /// The tool name.
        tool: String,
        /// Which field disagrees.
        detail: String,
    },
    /// The supervisor failed while starting, listing, calling, or stopping the plugin.
    #[error(transparent)]
    Supervisor(#[from] SupervisorError),
    /// The sandbox refused to prepare the plugin, before a process existed.
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    /// The approval decision could not be made or audited.
    #[error(transparent)]
    Approval(#[from] ApprovalError),
    /// The result could not be normalised into the untrusted envelope.
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
    /// The gateway's own audit sink rejected a record.
    #[error("the gateway could not audit the call: {message}")]
    Audit {
        /// The sink's own message.
        message: String,
    },
}

impl GatewayError {
    /// The stable code for this failure.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Undeclared { .. } => ErrorCode::PLG_008,
            Self::InvalidSchema { .. } | Self::InvalidArguments { .. } => ErrorCode::PLG_012,
            Self::Descriptor { .. } => ErrorCode::PLG_005,
            Self::Supervisor(error) => error.code(),
            Self::Sandbox(error) => error.code(),
            Self::Approval(error) => error.code(),
            Self::Envelope(error) => error.code(),
            Self::Audit { .. } => ErrorCode::PLG_010,
        }
    }

    /// What the caller should do about it.
    #[must_use]
    pub fn remediation(&self) -> String {
        match self {
            Self::Undeclared { .. } => {
                "declare the capability in the manifest and return the same tool from tools/list"
                    .to_owned()
            }
            Self::InvalidSchema { .. } => {
                "use only the supported JSON Schema subset: type, enum, const, properties, \
                 required, additionalProperties: false, items, minItems, maxItems, minLength, \
                 maxLength, pattern, minimum, and maximum"
                    .to_owned()
            }
            Self::InvalidArguments { .. } => {
                "correct the arguments against the tool's published input_schema".to_owned()
            }
            Self::Descriptor { .. } => {
                "make the descriptor's effect and destructive fields match the manifest, and list \
                 each tool once"
                    .to_owned()
            }
            Self::Supervisor(error) => error.remediation(),
            Self::Sandbox(error) => error.remediation(),
            Self::Approval(error) => error.remediation().to_owned(),
            Self::Envelope(error) => match error.code() {
                ErrorCode::PLG_006 => {
                    "raise limits.max_output_bytes only if the result is legitimately large; \
                     otherwise make the plugin return less"
                        .to_owned()
                }
                _ => "return well-formed content with distinct keys and no control characters"
                    .to_owned(),
            },
            Self::Audit { .. } => {
                "restore write access to the audit log and retry; the tool call was refused"
                    .to_owned()
            }
        }
    }
}

impl From<GatewayError> for AicontextError {
    fn from(error: GatewayError) -> Self {
        let code = error.code();
        let remediation = error.remediation();
        Self::new(code, error.to_string(), remediation).with_source(error)
    }
}

/// The single gateway that owns one plugin, one policy, one prompter, and one audit sink.
///
/// Construct with [`ToolGateway::start`]. The gateway lists the plugin's tools and validates every
/// descriptor before returning, so a broken contract fails before the first call.
#[derive(Debug)]
pub struct ToolGateway<P, A> {
    manifest: PluginManifest,
    sandbox: Sandbox,
    policy: Policy,
    prompter: P,
    audit: A,
    allow_once: BTreeSet<String>,
    process: PluginProcess,
    tools: BTreeMap<String, GatewayTool>,
    next_request_id: u64,
}

#[derive(Debug)]
struct GatewayTool {
    definition: ToolDefinition,
    capability: Capability,
    schema: CompiledSchema,
}

impl<P: Prompter, A: GatewayAudit> ToolGateway<P, A> {
    /// Start the plugin, list its tools, and validate every descriptor.
    ///
    /// The sandbox is prepared first. A refusal is written to the audit sink as
    /// [`GatewayEvent::SandboxRefused`] before it is returned, so a manifest that names a path,
    /// host, or environment variable outside the sandbox cannot fail silently. A sink that also
    /// refuses that record turns the failure into `PLG-010`: an unauditable refusal is not a
    /// refusal the host may act on.
    ///
    /// # Errors
    ///
    /// [`GatewayError::Sandbox`] (`PLG-009`/`PLG-011`) if the sandbox cannot be prepared, a
    /// [`SupervisorError`] if the process cannot start or `tools/list` fails, and a
    /// [`GatewayError`] if the descriptors do not match the manifest or a schema is outside the
    /// supported subset. In every failure case no plugin process is left running.
    pub fn start(
        manifest: PluginManifest,
        roots: &SandboxRoots,
        policy: Policy,
        prompter: P,
        mut audit: A,
        allow_once: impl IntoIterator<Item = String>,
    ) -> Result<Self, GatewayError> {
        let sandbox = match Sandbox::prepare(&manifest, roots) {
            Ok(sandbox) => sandbox,
            Err(error) => {
                let event = GatewayEvent::SandboxRefused {
                    plugin: manifest.name.clone(),
                    code: error.code(),
                    detail: error.to_string(),
                };
                return match audit.record(&event) {
                    Ok(()) => Err(GatewayError::Sandbox(error)),
                    Err(sink) => Err(GatewayError::Audit {
                        message: format!("{sink}; the sandbox refusal was: {error}"),
                    }),
                };
            }
        };

        let mut process = start_plugin(&manifest, &sandbox)?;
        let definitions = process.list_tools()?;
        let tools = build_tools(&manifest, definitions)?;

        Ok(Self {
            manifest,
            sandbox,
            policy,
            prompter,
            audit,
            allow_once: allow_once.into_iter().collect(),
            process,
            tools,
            next_request_id: 0,
        })
    }

    /// The manifest the gateway is enforcing.
    #[must_use]
    pub const fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    /// The sandbox the plugin process runs in.
    #[must_use]
    pub const fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    /// The permission policy the gateway is using.
    #[must_use]
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Every validated tool descriptor, in name order.
    pub fn tools(&self) -> impl Iterator<Item = &ToolDefinition> {
        self.tools.values().map(|tool| &tool.definition)
    }

    /// One validated descriptor, if the plugin exposes it.
    #[must_use]
    pub fn tool(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.get(name).map(|tool| &tool.definition)
    }

    /// Whether the plugin process is still running.
    pub fn is_running(&mut self) -> bool {
        self.process.is_running()
    }

    /// Drain the plugin's `log` notifications.
    pub fn take_logs(&mut self) -> Vec<PluginLog> {
        self.process.take_logs()
    }

    /// The tail of the plugin's stderr, for diagnosis after a failure.
    #[must_use]
    pub fn stderr_tail(&self) -> String {
        self.process.stderr_tail()
    }

    /// The terminal, for a caller that wants to inspect what it was asked.
    #[must_use]
    pub const fn prompter(&self) -> &P {
        &self.prompter
    }

    /// The audit sink, for a caller that wants to inspect what was recorded.
    #[must_use]
    pub const fn audit(&self) -> &A {
        &self.audit
    }

    /// Stop the plugin gracefully.
    ///
    /// # Errors
    ///
    /// As [`PluginProcess::shutdown`].
    pub fn shutdown(self) -> Result<(), SupervisorError> {
        self.process.shutdown()
    }

    /// Consume the gateway, returning the terminal and audit sink for inspection.
    ///
    /// The plugin process is dropped and killed; production code has no reason to call this.
    #[must_use]
    pub fn into_parts(self) -> (P, A) {
        let Self {
            prompter,
            audit,
            process,
            ..
        } = self;
        drop(process);
        (prompter, audit)
    }

    /// Validate, resolve, authorize, execute, normalise, and audit one call.
    ///
    /// # Errors
    ///
    /// [`GatewayError`] when the call cannot produce a reliable outcome: an undeclared capability
    /// (`PLG-008`), invalid arguments or an unenforceable schema (`PLG-012`), a descriptor mismatch
    /// (`PLG-005`), a supervisor failure, an envelope failure, or an audit sink that refused a
    /// record (`PLG-010`).
    ///
    /// A policy denial and a human refusal are `Ok(GatewayOutcome::Refused(_))`: they are decisions,
    /// not failures.
    pub fn call(&mut self, call: ToolCall) -> Result<GatewayOutcome, GatewayError> {
        let arguments_digest = json_digest(&call.arguments);
        let Some(tool) = self.tools.get(&call.tool) else {
            let summary = Summary::new(
                &call.tool,
                &self.manifest.name,
                &call.target,
                arguments_digest,
                None,
                None,
                false,
            );
            let error = GatewayError::Undeclared {
                tool: call.tool.clone(),
                detail: "the manifest does not declare this capability".to_owned(),
            };
            self.record_refusal(
                &summary,
                RefusalStage::Permission,
                Some(error.code()),
                &error.to_string(),
            )?;
            return Err(error);
        };

        // The borrow of `self.tools` ends when `capability` is cloned; the rest of the call needs
        // the prompter and audit sink mutably.
        let validation = tool.schema.validate(&call.arguments);
        let capability = tool.capability.clone();
        let base = Summary::new(
            &call.tool,
            &self.manifest.name,
            &call.target,
            arguments_digest,
            None,
            Some(capability.effect),
            capability.destructive,
        );

        if let Err(detail) = validation {
            let error = GatewayError::InvalidArguments {
                tool: call.tool.clone(),
                detail,
            };
            self.record_refusal(
                &base,
                RefusalStage::Validation,
                Some(error.code()),
                &error.to_string(),
            )?;
            return Err(error);
        }

        let decision = self
            .policy
            .resolve(CapabilityRequest::new(&call.tool).with_destructive(capability.destructive));
        let summary = base.with_mode(decision.mode());

        match self.authorize(&call, &summary, decision.mode())? {
            Some(refusal) => Ok(GatewayOutcome::Refused(refusal)),
            None => self.execute(call, summary),
        }
    }

    /// Resolve the decision with the broker and record a denial as a refusal.
    fn authorize(
        &mut self,
        call: &ToolCall,
        summary: &CallSummary,
        mode: PermissionMode,
    ) -> Result<Option<Refusal>, GatewayError> {
        let request = ApprovalRequest::new(
            &call.tool,
            &self.manifest.name,
            &call.target,
            call.arguments.clone(),
            mode,
        )
        .with_reason(&call.reason);
        let mut broker = ApprovalBroker::new(
            &mut self.prompter,
            ApprovalBridge::new(&mut self.audit),
            self.allow_once.iter().cloned(),
        );
        let outcome = broker.authorize(&request)?;
        let Some(reason) = outcome.denial() else {
            return Ok(None);
        };

        let stage = if mode == PermissionMode::Deny {
            RefusalStage::Permission
        } else {
            RefusalStage::Approval
        };
        let detail = reason.to_string();
        self.record_refusal(summary, stage, None, &detail)?;
        Ok(Some(Refusal::new(stage, detail, None)))
    }

    /// Dispatch an authorized call, normalise its result, and record both facts.
    fn execute(
        &mut self,
        call: ToolCall,
        summary: CallSummary,
    ) -> Result<GatewayOutcome, GatewayError> {
        self.record(&GatewayEvent::Executing {
            call: summary.clone(),
        })?;

        self.next_request_id = self.next_request_id.saturating_add(1);
        let request_id = format!("call-{}", self.next_request_id);
        let started = Instant::now();
        let result = self.process.call_tool(&call.tool, call.arguments);
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let error = GatewayError::Supervisor(error);
                self.record_refusal(
                    &summary,
                    RefusalStage::Execution,
                    Some(error.code()),
                    &error.to_string(),
                )?;
                return Err(error);
            }
        };

        let max_bytes =
            usize::try_from(self.manifest.limits.max_output_bytes).unwrap_or(usize::MAX);
        match normalise(
            &self.manifest.name,
            &call.tool,
            request_id,
            duration_ms,
            result,
            max_bytes,
        ) {
            Ok(normalised) => {
                let is_error = normalised.envelope.is_error();
                self.record(&GatewayEvent::Completed {
                    call: summary,
                    result: ResultSummary::Succeeded {
                        is_error,
                        duration_ms,
                        bytes: normalised.bytes,
                        digest: normalised.digest,
                    },
                })?;
                Ok(GatewayOutcome::Executed(normalised.envelope))
            }
            Err(error) => {
                let error = GatewayError::Envelope(error);
                self.record_refusal(
                    &summary,
                    RefusalStage::Envelope,
                    Some(error.code()),
                    &error.to_string(),
                )?;
                Err(error)
            }
        }
    }

    /// Record a terminal refusal, or turn a sink failure into `PLG-010`.
    fn record_refusal(
        &mut self,
        call: &CallSummary,
        stage: RefusalStage,
        code: Option<ErrorCode>,
        detail: &str,
    ) -> Result<(), GatewayError> {
        self.record(&GatewayEvent::Completed {
            call: call.clone(),
            result: ResultSummary::Refused {
                stage,
                code,
                detail: detail.to_owned(),
            },
        })
    }

    fn record(&mut self, event: &GatewayEvent) -> Result<(), GatewayError> {
        self.audit
            .record(event)
            .map_err(|error| GatewayError::Audit {
                message: error.to_string(),
            })
    }
}

/// Check every descriptor against the manifest and compile the schemas.
fn build_tools(
    manifest: &PluginManifest,
    definitions: Vec<ToolDefinition>,
) -> Result<BTreeMap<String, GatewayTool>, GatewayError> {
    let declared: BTreeMap<&str, &Capability> = manifest
        .capabilities
        .iter()
        .map(|capability| (capability.tool.as_str(), capability))
        .collect();
    let mut seen = BTreeSet::new();
    let mut tools = BTreeMap::new();

    for definition in definitions {
        if !seen.insert(definition.tool.clone()) {
            return Err(GatewayError::Descriptor {
                tool: definition.tool.clone(),
                detail: "the plugin listed the tool more than once".to_owned(),
            });
        }
        let Some(capability) = declared.get(definition.tool.as_str()) else {
            return Err(GatewayError::Undeclared {
                tool: definition.tool,
                detail: "the plugin listed a tool the manifest does not declare".to_owned(),
            });
        };
        if definition.effect != capability.effect {
            return Err(GatewayError::Descriptor {
                tool: definition.tool,
                detail: format!(
                    "the descriptor says `{}`, but the manifest says `{}`",
                    definition.effect.as_str(),
                    capability.effect.as_str()
                ),
            });
        }
        if definition.destructive != capability.destructive {
            return Err(GatewayError::Descriptor {
                tool: definition.tool,
                detail: format!(
                    "the descriptor says destructive = {}, but the manifest says {}",
                    definition.destructive, capability.destructive
                ),
            });
        }
        let schema = CompiledSchema::compile(&definition.input_schema).map_err(|detail| {
            GatewayError::InvalidSchema {
                tool: definition.tool.clone(),
                detail,
            }
        })?;
        tools.insert(
            definition.tool.clone(),
            GatewayTool {
                definition,
                capability: (*capability).clone(),
                schema,
            },
        );
    }

    for capability in &manifest.capabilities {
        if !seen.contains(&capability.tool) {
            return Err(GatewayError::Undeclared {
                tool: capability.tool.clone(),
                detail: "the manifest declares a capability the plugin did not list".to_owned(),
            });
        }
    }

    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::{GatewayError, RefusalStage};
    use aicontext_core::ErrorCode;

    #[test]
    fn the_refusal_stages_have_stable_wire_names() {
        assert_eq!(RefusalStage::Validation.as_str(), "validation");
        assert_eq!(RefusalStage::Permission.as_str(), "permission");
        assert_eq!(RefusalStage::Approval.as_str(), "approval");
        assert_eq!(RefusalStage::Execution.as_str(), "execution");
        assert_eq!(RefusalStage::Envelope.as_str(), "envelope");
    }

    #[test]
    fn every_error_carries_the_code_its_stage_documents() {
        let undeclared = GatewayError::Undeclared {
            tool: "fixture.extra.run".to_owned(),
            detail: "not declared".to_owned(),
        };
        assert_eq!(undeclared.code(), ErrorCode::PLG_008);

        for error in [
            GatewayError::InvalidSchema {
                tool: "fixture.echo.run".to_owned(),
                detail: "unsupported".to_owned(),
            },
            GatewayError::InvalidArguments {
                tool: "fixture.echo.run".to_owned(),
                detail: "missing".to_owned(),
            },
        ] {
            assert_eq!(error.code(), ErrorCode::PLG_012);
        }

        let descriptor = GatewayError::Descriptor {
            tool: "fixture.echo.run".to_owned(),
            detail: "effect".to_owned(),
        };
        assert_eq!(descriptor.code(), ErrorCode::PLG_005);
        assert!(!descriptor.remediation().is_empty());
    }
}
