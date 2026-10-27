//! The record shape and the types a caller builds before the log seals it.
//!
//! [`AuditEntry`] is what a caller supplies; [`AuditRecord`] is what the log writes. They are
//! separate types so the three fields the log owns — `seq`, `prev`, and `self` — cannot be chosen by
//! a caller, and so there is no field in which raw arguments could be stored. Security is a shape
//! here, not a reminder (`docs/SECURITY.md` §6 and §8).
//!
//! Every string in a record is bounded and the closed vocabularies are checked against the values
//! this workspace defines. `resource` is the exception: it is host- or plugin-supplied free text (a
//! path, an instance id), so it keeps its characters and the CLI sanitises it for display.

use std::str::FromStr;

use aicontext_core::PermissionMode;
use serde::{Deserialize, Serialize};

use crate::error::AuditError;
use crate::hash::{GENESIS, is_digest, record_digest};
use crate::timestamp::Timestamp;

/// The longest `resource` a record accepts, in characters.
const MAX_RESOURCE: usize = 2_048;
/// The longest identifier field (actor id, tool name, `granted_by`), in characters.
const MAX_IDENTIFIER: usize = 256;
/// The longest provider, model, or plugin name, in characters.
const MAX_NAME: usize = 128;
/// The longest vocabulary word (`event`, `kind`, `effect`, `status`, `mode`), in characters.
const MAX_WORD: usize = 32;

/// Who caused the action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    kind: String,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

impl Actor {
    /// An actor of `kind`, identified by `id`.
    #[must_use]
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            provider: None,
            model: None,
        }
    }

    /// The agent that acted on the developer's behalf.
    #[must_use]
    pub fn agent(id: impl Into<String>) -> Self {
        Self::new("agent", id)
    }

    /// The developer acting directly.
    #[must_use]
    pub fn human(id: impl Into<String>) -> Self {
        Self::new("human", id)
    }

    /// The host acting without a model in the loop.
    #[must_use]
    pub fn system(id: impl Into<String>) -> Self {
        Self::new("system", id)
    }

    /// Records the provider that served the model.
    #[must_use]
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Records the model the agent used.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// `agent`, `human`, or `system` today; the vocabulary is open so a later actor type can be
    /// recorded without breaking an older reader.
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The actor's identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The provider that served the model, when the record has one.
    #[must_use]
    pub fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    /// The model the agent used, when the record has one.
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }
}

/// What was done, to which capability.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    tool: String,
    plugin: String,
    effect: String,
}

impl Action {
    /// One tool call: the tool, the plugin that provides it, and the declared effect.
    #[must_use]
    pub fn new(
        tool: impl Into<String>,
        plugin: impl Into<String>,
        effect: impl Into<String>,
    ) -> Self {
        Self {
            tool: tool.into(),
            plugin: plugin.into(),
            effect: effect.into(),
        }
    }

    /// The fully qualified tool name, for example `aws.ec2.stop`.
    #[must_use]
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The plugin that provides the tool.
    #[must_use]
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The declared effect, for example `read` or `write`.
    #[must_use]
    pub fn effect(&self) -> &str {
        &self.effect
    }
}

/// The approval state attached to an event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    required: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    granted_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
}

impl Approval {
    /// An approval record whose required mode is the policy's answer.
    #[must_use]
    pub fn new(required: PermissionMode) -> Self {
        Self {
            required: required.as_str().to_string(),
            granted_by: None,
            mode: None,
        }
    }

    /// Who approved the invocation.
    #[must_use]
    pub fn with_granted_by(mut self, id: impl Into<String>) -> Self {
        self.granted_by = Some(id.into());
        self
    }

    /// How the decision was reached: `interactive`, `allow_once`, `not_required`, or a value a later
    /// version documents. Bounded, but open, because the decision vocabulary is still growing.
    #[must_use]
    pub fn with_mode(mut self, mode: impl Into<String>) -> Self {
        self.mode = Some(mode.into());
        self
    }

    /// The policy mode the resolver returned, as the four wire forms in `docs/PLUGIN_SPEC.md` §5.
    #[must_use]
    pub fn required(&self) -> &str {
        &self.required
    }

    /// Who approved the invocation, when it was approved.
    #[must_use]
    pub fn granted_by(&self) -> Option<&str> {
        self.granted_by.as_deref()
    }

    /// How the decision was reached, when it was recorded.
    #[must_use]
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }
}

/// What the call produced.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultSummary {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
}

impl ResultSummary {
    /// A result with its status, for example `success`, `error`, or `refused`.
    #[must_use]
    pub fn new(status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            duration_ms: None,
            digest: None,
        }
    }

    /// Records how long the call took.
    #[must_use]
    pub const fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    /// Records the digest of the untrusted result envelope, never the envelope itself.
    #[must_use]
    pub fn with_digest(mut self, digest: impl Into<String>) -> Self {
        self.digest = Some(digest.into());
        self
    }

    /// The outcome word.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// How long the call took, when it was measured.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// The result digest, when one was recorded.
    #[must_use]
    pub fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }
}

/// One event as a caller supplies it, before the log assigns `seq`, `prev`, and `self`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEntry {
    event: String,
    actor: Actor,
    action: Action,
    resource: Option<String>,
    arguments_digest: Option<String>,
    approval: Option<Approval>,
    result: Option<ResultSummary>,
}

impl AuditEntry {
    /// An event of `event` (`prompted`, `decided`, `executing`, `completed`, or a later bounded word).
    #[must_use]
    pub fn new(event: impl Into<String>, actor: Actor, action: Action) -> Self {
        Self {
            event: event.into(),
            actor,
            action,
            resource: None,
            arguments_digest: None,
            approval: None,
            result: None,
        }
    }

    /// The target of the call: an instance id, a repository, a path.
    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// The digest of the arguments. There is no method that takes the arguments themselves.
    #[must_use]
    pub fn with_arguments_digest(mut self, digest: impl Into<String>) -> Self {
        self.arguments_digest = Some(digest.into());
        self
    }

    /// The approval state for this event.
    #[must_use]
    pub fn with_approval(mut self, approval: Approval) -> Self {
        self.approval = Some(approval);
        self
    }

    /// The result of the call.
    #[must_use]
    pub fn with_result(mut self, result: ResultSummary) -> Self {
        self.result = Some(result);
        self
    }

    /// The event word.
    #[must_use]
    pub fn event(&self) -> &str {
        &self.event
    }

    /// The actor.
    #[must_use]
    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    /// The action.
    #[must_use]
    pub fn action(&self) -> &Action {
        &self.action
    }

    /// The target, when the event names one.
    #[must_use]
    pub fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }

    /// The argument digest, when the event has arguments.
    #[must_use]
    pub fn arguments_digest(&self) -> Option<&str> {
        self.arguments_digest.as_deref()
    }

    /// The approval state, when the event has one.
    #[must_use]
    pub fn approval(&self) -> Option<&Approval> {
        self.approval.as_ref()
    }

    /// The result, when the event has one.
    #[must_use]
    pub fn result(&self) -> Option<&ResultSummary> {
        self.result.as_ref()
    }
}

/// One sealed record, exactly as it appears in the JSONL file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    seq: u64,
    ts: String,
    event: String,
    actor: Actor,
    action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arguments_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<Approval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<ResultSummary>,
    prev: String,
    #[serde(rename = "self")]
    digest: String,
}

impl AuditRecord {
    /// Seals `entry` into a record at `seq`, following `prev`.
    ///
    /// The caller cannot choose `self`: it is derived from the record's canonical bytes, so a
    /// sealed record always agrees with its own digest.
    pub(crate) fn seal(
        seq: u64,
        timestamp: Timestamp,
        entry: &AuditEntry,
        prev: &str,
    ) -> Result<Self, AuditError> {
        let mut record = Self {
            seq,
            ts: timestamp.as_str(),
            event: entry.event.clone(),
            actor: entry.actor.clone(),
            action: entry.action.clone(),
            resource: entry.resource.clone(),
            arguments_digest: entry.arguments_digest.clone(),
            approval: entry.approval.clone(),
            result: entry.result.clone(),
            prev: prev.to_string(),
            digest: GENESIS.to_string(),
        };
        validate(&record).map_err(AuditError::invalid_record)?;
        record.digest = record_digest(&record)?;
        Ok(record)
    }

    /// The record's position in the chain, starting at 1.
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// The canonical UTC timestamp.
    #[must_use]
    pub fn timestamp(&self) -> &str {
        &self.ts
    }

    /// The event word.
    #[must_use]
    pub fn event(&self) -> &str {
        &self.event
    }

    /// The actor.
    #[must_use]
    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    /// The action.
    #[must_use]
    pub fn action(&self) -> &Action {
        &self.action
    }

    /// The target, when the record names one.
    #[must_use]
    pub fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }

    /// The argument digest, when the record has arguments.
    #[must_use]
    pub fn arguments_digest(&self) -> Option<&str> {
        self.arguments_digest.as_deref()
    }

    /// The approval state, when the record has one.
    #[must_use]
    pub fn approval(&self) -> Option<&Approval> {
        self.approval.as_ref()
    }

    /// The result, when the record has one.
    #[must_use]
    pub fn result(&self) -> Option<&ResultSummary> {
        self.result.as_ref()
    }

    /// The digest of the record that precedes this one.
    #[must_use]
    pub fn prev(&self) -> &str {
        &self.prev
    }

    /// This record's own digest.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Checks everything the record shape promises except its link to its neighbours.
pub(crate) fn validate(record: &AuditRecord) -> Result<(), String> {
    if record.seq == 0 {
        return Err("seq starts at 1".to_string());
    }
    Timestamp::parse(&record.ts).map_err(|error| error.to_string())?;
    word("event", &record.event)?;
    validate_actor(&record.actor)?;
    validate_action(&record.action)?;
    if let Some(resource) = &record.resource {
        text("resource", resource, MAX_RESOURCE)?;
    }
    if let Some(value) = &record.arguments_digest {
        digest("arguments_digest", value)?;
    }
    if let Some(approval) = &record.approval {
        validate_approval(approval)?;
    }
    if let Some(result) = &record.result {
        validate_result(result)?;
    }
    digest("prev", &record.prev)?;
    digest("self", &record.digest)?;
    Ok(())
}

fn validate_actor(actor: &Actor) -> Result<(), String> {
    word("actor.kind", &actor.kind)?;
    identifier("actor.id", &actor.id)?;
    if let Some(provider) = &actor.provider {
        name("actor.provider", provider)?;
    }
    if let Some(model) = &actor.model {
        name("actor.model", model)?;
    }
    Ok(())
}

fn validate_action(action: &Action) -> Result<(), String> {
    identifier("action.tool", &action.tool)?;
    name("action.plugin", &action.plugin)?;
    word("action.effect", &action.effect)?;
    Ok(())
}

fn validate_approval(approval: &Approval) -> Result<(), String> {
    match PermissionMode::from_str(&approval.required) {
        Ok(mode) if mode.as_str() == approval.required => {}
        _ => {
            return Err(format!(
                "approval.required must be one of allow, deny, approval, explicit_approval; \
                 it is `{}`",
                approval.required
            ));
        }
    }
    if let Some(granted_by) = &approval.granted_by {
        identifier("approval.granted_by", granted_by)?;
    }
    if let Some(mode) = &approval.mode {
        word("approval.mode", mode)?;
    }
    Ok(())
}

fn validate_result(result: &ResultSummary) -> Result<(), String> {
    word("result.status", &result.status)?;
    if let Some(value) = &result.digest {
        digest("result.digest", value)?;
    }
    Ok(())
}

/// A bounded non-empty vocabulary word.
fn word(field: &str, value: &str) -> Result<(), String> {
    bounded(field, value, MAX_WORD)?;
    safe_characters(field, value)
}

/// A bounded non-empty identifier.
fn identifier(field: &str, value: &str) -> Result<(), String> {
    bounded(field, value, MAX_IDENTIFIER)?;
    safe_characters(field, value)
}

/// A bounded non-empty provider, model, or plugin name.
fn name(field: &str, value: &str) -> Result<(), String> {
    bounded(field, value, MAX_NAME)?;
    safe_characters(field, value)
}

/// Free text that is displayed, but not interpreted: bounded, and permitted to carry any character
/// because a path may legitimately contain one. The CLI sanitises it for the terminal.
fn text(field: &str, value: &str, max: usize) -> Result<(), String> {
    bounded(field, value, max)
}

/// A `sha256:<lowercase hex>` digest.
fn digest(field: &str, value: &str) -> Result<(), String> {
    if is_digest(value) {
        Ok(())
    } else {
        Err(format!(
            "{field} must be sha256: followed by 64 lowercase hex digits"
        ))
    }
}

fn bounded(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if value.chars().count() > max {
        return Err(format!("{field} must be at most {max} characters"));
    }
    Ok(())
}

/// Rejects C0/C1 controls and the Unicode bidi/zero-width characters an identifier must never need.
///
/// A record cannot use these to forge a line in the JSONL file — JSON escapes them — but it can use
/// them to make a terminal lie about what was called. The CLI sanitises display too; this is the
/// second, structural half of the same control.
fn safe_characters(field: &str, value: &str) -> Result<(), String> {
    if let Some(character) = value.chars().find(|character| is_unsafe(*character)) {
        return Err(format!(
            "{field} contains the control or format character U+{:04X}",
            u32::from(character)
        ));
    }
    Ok(())
}

/// Whether a character can change how a line renders rather than what it says.
fn is_unsafe(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
        )
}

#[cfg(test)]
mod tests {
    use aicontext_core::PermissionMode;

    use super::{
        Actor, Approval, AuditEntry, AuditRecord, GENESIS, ResultSummary, Timestamp, validate,
    };
    use crate::Action;

    fn entry() -> AuditEntry {
        AuditEntry::new(
            "completed",
            Actor::agent("developer")
                .with_provider("openai-compatible")
                .with_model("test-model"),
            Action::new("aws.ec2.stop", "aws", "write"),
        )
        .with_resource("i-0abc1234")
        .with_arguments_digest(super::super::hash::sha256(b"{\"instance\":\"i-0abc1234\"}"))
        .with_approval(
            Approval::new(PermissionMode::Approval)
                .with_granted_by("developer")
                .with_mode("interactive"),
        )
        .with_result(
            ResultSummary::new("success")
                .with_duration_ms(412)
                .with_digest(super::super::hash::sha256(b"{\"status\":\"stopped\"}")),
        )
    }

    fn record() -> AuditRecord {
        AuditRecord::seal(
            1,
            Timestamp::from_unix_millis(1_790_000_000_000),
            &entry(),
            GENESIS,
        )
        .expect("a valid record")
    }

    #[test]
    fn a_record_exposes_every_field_it_was_sealed_with() {
        let record = record();
        assert_eq!(record.seq(), 1);
        assert_eq!(record.timestamp(), "2026-09-21T14:13:20.000Z");
        assert_eq!(record.event(), "completed");
        assert_eq!(record.actor().kind(), "agent");
        assert_eq!(record.actor().id(), "developer");
        assert_eq!(record.actor().provider(), Some("openai-compatible"));
        assert_eq!(record.actor().model(), Some("test-model"));
        assert_eq!(record.action().tool(), "aws.ec2.stop");
        assert_eq!(record.action().plugin(), "aws");
        assert_eq!(record.action().effect(), "write");
        assert_eq!(record.resource(), Some("i-0abc1234"));
        assert!(
            record
                .arguments_digest()
                .is_some_and(|digest| digest.starts_with("sha256:"))
        );
        let approval = record.approval().expect("approval");
        assert_eq!(approval.required(), "approval");
        assert_eq!(approval.granted_by(), Some("developer"));
        assert_eq!(approval.mode(), Some("interactive"));
        let result = record.result().expect("result");
        assert_eq!(result.status(), "success");
        assert_eq!(result.duration_ms(), Some(412));
        assert!(result.digest().is_some());
        assert_eq!(record.prev(), GENESIS);
        assert!(record.digest().starts_with("sha256:"));
        assert_eq!(record.digest().len(), "sha256:".len() + 64);
    }

    #[test]
    fn the_sealed_record_serialises_to_the_documented_shape() {
        let value = serde_json::to_value(record()).expect("serialises");
        for key in [
            "seq",
            "ts",
            "event",
            "actor",
            "action",
            "resource",
            "arguments_digest",
            "approval",
            "result",
            "prev",
            "self",
        ] {
            assert!(value.get(key).is_some(), "{key} is missing");
        }
        assert_eq!(value["actor"]["kind"], "agent");
        assert_eq!(value["action"]["tool"], "aws.ec2.stop");
        assert_eq!(value["approval"]["required"], "approval");
        assert!(value.get("self").is_some());
        assert!(value.get("digest").is_none(), "the field is named self");
    }

    #[test]
    fn absent_optional_fields_are_absent_rather_than_null() {
        let plain = AuditRecord::seal(
            1,
            Timestamp::from_unix_millis(0),
            &AuditEntry::new(
                "prompted",
                Actor::system("host"),
                Action::new("github.issue.read", "github", "read"),
            ),
            GENESIS,
        )
        .expect("valid");
        let value = serde_json::to_value(plain).expect("serialises");
        for key in ["resource", "arguments_digest", "approval", "result"] {
            assert!(value.get(key).is_none(), "{key} must be skipped");
        }
    }

    #[test]
    fn a_record_does_not_accept_the_raw_arguments_in_place_of_a_digest() {
        // The shape has no `arguments` field at all, so a hand-written record carrying one is
        // rejected rather than silently verified with the field ignored.
        let value = serde_json::to_value(record()).expect("serialises");
        let mut object = value.as_object().expect("an object").clone();
        object.insert(
            "arguments".to_string(),
            serde_json::json!({"secret": "value"}),
        );
        let text = serde_json::to_string(&object).expect("serialises");
        assert!(
            serde_json::from_str::<AuditRecord>(&text).is_err(),
            "an unknown field is a malformed record"
        );
    }

    #[test]
    fn the_required_approval_mode_is_one_of_the_four_canonical_wire_forms() {
        let mut record = record();
        record.approval.as_mut().expect("approval").required = "Approval".to_string();
        assert!(validate(&record).is_err(), "the wire form is lowercase");

        record.approval.as_mut().expect("approval").required = "explicit_approval".to_string();
        assert!(validate(&record).is_ok());
    }

    #[test]
    fn every_digest_field_has_to_be_a_sha256_and_not_an_arbitrary_string() {
        let mut broken = record();
        broken.prev = "sha256:not-a-digest".to_string();
        assert!(validate(&broken).is_err());

        let mut broken = record();
        broken.arguments_digest = Some("sha256:ABC".to_string());
        assert!(validate(&broken).is_err());

        let mut broken = record();
        broken.result.as_mut().expect("result").digest = Some("md5:abc".to_string());
        assert!(validate(&broken).is_err());
    }

    #[test]
    fn an_identifier_cannot_carry_a_control_or_bidi_override() {
        let mut broken = record();
        broken.action.tool = "aws.ec2.stop\ncompleted".to_string();
        assert!(validate(&broken).is_err());

        let mut broken = record();
        broken.actor.id = "developer\u{202e}enod".to_string();
        assert!(validate(&broken).is_err());

        // The resource is free text and may contain a control character; display sanitises it.
        let mut resource = record();
        resource.resource = Some("file\nname".to_string());
        assert!(validate(&resource).is_ok());
    }

    #[test]
    fn an_empty_or_over_long_field_is_refused() {
        let mut broken = record();
        broken.action.tool = String::new();
        assert!(validate(&broken).is_err());

        let mut broken = record();
        broken.actor.id = "x".repeat(257);
        assert!(validate(&broken).is_err());

        let mut broken = record();
        broken.resource = Some("x".repeat(2_049));
        assert!(validate(&broken).is_err());
    }

    #[test]
    fn seq_zero_is_refused_because_the_chain_starts_at_one() {
        let mut broken = record();
        broken.seq = 0;
        assert!(validate(&broken).is_err());
    }

    #[test]
    fn a_bad_timestamp_is_refused() {
        let mut broken = record();
        broken.ts = "2026-02-30T00:00:00.000Z".to_string();
        assert!(validate(&broken).is_err());
    }

    #[test]
    fn the_builders_are_chainable_and_leave_optional_fields_empty() {
        let actor = Actor::human("developer");
        assert_eq!(actor.kind(), "human");
        assert!(actor.provider().is_none());
        assert!(actor.model().is_none());

        let approval = Approval::new(PermissionMode::Deny);
        assert_eq!(approval.required(), "deny");
        assert!(approval.granted_by().is_none());
        assert!(approval.mode().is_none());

        let result = ResultSummary::new("refused");
        assert_eq!(result.status(), "refused");
        assert_eq!(result.duration_ms(), None);
        assert!(result.digest().is_none());
    }
}
