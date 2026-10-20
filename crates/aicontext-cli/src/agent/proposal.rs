//! What the model answered, parsed the way `docs/AI_PROVIDER_SPEC.md` §9 asks.
//!
//! A plan-mode run accepts two shapes of answer, because `ARCHITECTURE.md` Flow D says the model
//! returns *either* text *or* a typed `action_proposal`: prose is a plan, and so is a proposal. The
//! parse is deliberately asymmetric. Prose is taken as it stands; a response that *claims* to be a
//! proposal must prove every part of that claim, because a half-understood action is how a plan
//! silently loses a file (`RULES.md` §4.10). A proposal with an unknown action type, a missing
//! argument object, or a duplicated path is rejected whole, and the caller records exactly which
//! parts failed.
//!
//! The validation is hand-written against the committed schema rather than evaluated from it: the
//! binary ships no JSON Schema engine, and `jsonschema` stays a dev-dependency. The tests in this
//! module feed the same fixtures through *both* the schema and [`parse`], so the two definitions
//! cannot drift apart without `cargo test` saying so.

use std::collections::BTreeSet;

use serde_json::Value;

/// The `kind` a proposal must carry; a const of `schemas/action-proposal.schema.json`.
const KIND: &str = "action_proposal";

/// The `version` a proposal must carry; likewise a const of the schema.
const VERSION: u64 = 1;

/// A parsed answer: a validated proposal, prose, or a proposal that failed validation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Parsed {
    /// A schema-valid `action_proposal`.
    Structured(Proposal),
    /// Plain text, kept verbatim as the plan's body.
    Prose(String),
    /// A response that claimed to be a proposal and could not prove it. Every problem, so the
    /// report can name all of them rather than the first.
    Invalid(Vec<String>),
}

/// A validated `action_proposal`: the actions, each one exactly one of the two shapes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Proposal {
    /// The actions, in the order the model wrote them.
    pub(crate) actions: Vec<Action>,
}

/// One validated action, mirroring the schema's `oneOf`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// A tool the model proposes. Recorded in the plan; plan mode executes none.
    ToolCall {
        /// The capability name, for example `aws.ec2.describe`.
        tool: String,
        /// The arguments, an object per the schema.
        arguments: Value,
        /// Why the model wants it, when it said.
        reason: Option<String>,
    },
    /// A step of the plan: what to change and what it endangers.
    Plan {
        /// One paragraph saying what this step does.
        summary: String,
        /// Files this step modifies.
        files_modify: Vec<String>,
        /// Files this step creates.
        files_create: Vec<String>,
        /// Files this step deletes.
        files_delete: Vec<String>,
        /// What could go wrong, in the model's words.
        risks: Vec<String>,
    },
}

/// Parses a provider response's text.
///
/// Anything that is not a JSON object claiming `kind: action_proposal` is prose — including other
/// JSON, because a model echoing some unrelated object has not failed a contract it was never told
/// to keep. A claim it *was* told to keep is validated strictly, here against the same rules
/// `schemas/action-proposal.schema.json` states.
#[must_use]
pub(crate) fn parse(text: &str) -> Parsed {
    let trimmed = text.trim();
    let Some(value) = as_json(trimmed) else {
        return Parsed::Prose(trimmed.to_string());
    };
    if value.get("kind").and_then(Value::as_str) != Some(KIND) {
        return Parsed::Prose(trimmed.to_string());
    }
    match validate(&value) {
        Ok(actions) => Parsed::Structured(Proposal { actions }),
        Err(problems) => Parsed::Invalid(problems),
    }
}

/// The response text as JSON, looking in the three places a model actually puts it.
///
/// A model asked for JSON usually answers with bare JSON; one that wants to be helpful fences it in
/// backticks; one that is chatty puts it in the middle of a sentence. All three are accepted, so a
/// formatting habit never decides whether a run succeeds.
fn as_json(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Some(value);
    }
    for candidate in fenced(text).into_iter().chain(wrapped(text)) {
        if let Ok(value) = serde_json::from_str::<Value>(candidate) {
            return Some(value);
        }
    }
    None
}

/// The body of a fenced code block, when the text is one.
fn fenced(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("```")?;
    let rest = rest.strip_prefix("json").unwrap_or(rest).trim_start_matches('\n');
    let end = rest.rfind("```")?;
    Some(rest[..end].trim())
}

/// The outermost `{ … }` of a text that is not itself JSON.
fn wrapped(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

/// Validates a claimed proposal, returning its actions or every problem found.
fn validate(value: &Value) -> Result<Vec<Action>, Vec<String>> {
    let mut problems = Vec::new();
    if value.get("version").and_then(Value::as_u64) != Some(VERSION) {
        problems.push(format!(
            "`version` must be {VERSION}, found {}",
            value
                .get("version")
                .map_or("(missing)".to_string(), |other| other.to_string())
        ));
    }
    let Some(actions) = value.get("actions").and_then(Value::as_array) else {
        problems.push("`actions` must be an array".to_string());
        return Err(problems);
    };
    if actions.is_empty() {
        problems.push("`actions` must hold at least one action".to_string());
    }
    let mut parsed = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        match parse_action(action) {
            Ok(action) => parsed.push(action),
            Err(why) => problems.extend(why.into_iter().map(|problem| format!("actions[{index}]: {problem}"))),
        }
    }
    if problems.is_empty() {
        Ok(parsed)
    } else {
        Err(problems)
    }
}

/// Validates one action against the schema's `oneOf`: `tool_call` or `plan`, nothing else.
fn parse_action(action: &Value) -> Result<Action, Vec<String>> {
    match action.get("type").and_then(Value::as_str) {
        Some("tool_call") => {
            let mut problems = Vec::new();
            let tool = non_empty_string(action, "tool", &mut problems);
            // `arguments` is required by the schema as well as type-checked here: an absent key
            // must produce a problem too, or the action would be rejected with an empty list and
            // the report would say nothing about why.
            let arguments = match action.get("arguments") {
                Some(value) if value.is_object() => Some(value.clone()),
                Some(_) => {
                    problems.push("`arguments` must be an object".to_string());
                    None
                }
                None => {
                    problems.push("`arguments` must be an object, and it is missing".to_string());
                    None
                }
            };
            let reason = present_string(action, "reason", &mut problems);
            match (tool, arguments) {
                (Some(tool), Some(arguments)) if problems.is_empty() => Ok(Action::ToolCall {
                    tool,
                    arguments,
                    reason,
                }),
                _ => Err(problems),
            }
        }
        Some("plan") => {
            let mut problems = Vec::new();
            let summary = non_empty_string(action, "summary", &mut problems);
            let files_modify = string_list(action, "files_modify", &mut problems);
            let files_create = string_list(action, "files_create", &mut problems);
            let files_delete = string_list(action, "files_delete", &mut problems);
            let risks = string_list(action, "risks", &mut problems);
            match summary {
                Some(summary) if problems.is_empty() => Ok(Action::Plan {
                    summary,
                    files_modify,
                    files_create,
                    files_delete,
                    risks,
                }),
                _ => Err(problems),
            }
        }
        Some(other) => Err(vec![format!(
            "unknown action type `{other}` — an action is either `tool_call` or `plan`, and \
             anything else is rejected rather than partially understood"
        )]),
        None => Err(vec!["an action has no `type`".to_string()]),
    }
}

/// A required non-empty string field, or a problem naming what was found.
fn non_empty_string(action: &Value, key: &str, problems: &mut Vec<String>) -> Option<String> {
    match action.get(key).and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => Some(text.to_string()),
        other => {
            problems.push(format!(
                "`{key}` must be a non-empty string, found {}",
                other.map_or("(missing)", |_| "an empty or non-string value")
            ));
            None
        }
    }
}

/// An optional non-empty string field: absent is fine, present-and-empty is a problem.
fn present_string(action: &Value, key: &str, problems: &mut Vec<String>) -> Option<String> {
    if action.get(key).is_none_or(Value::is_null) {
        return None;
    }
    non_empty_string(action, key, problems)
}

/// An optional array of non-empty, unique strings — the shape the schema gives every file list.
fn string_list(action: &Value, key: &str, problems: &mut Vec<String>) -> Vec<String> {
    let Some(list) = action.get(key) else {
        return Vec::new();
    };
    let Some(items) = list.as_array() else {
        problems.push(format!("`{key}` must be an array of strings"));
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut values = Vec::new();
    for item in items {
        match item.as_str() {
            Some(text) if !text.trim().is_empty() => {
                if !seen.insert(text.to_string()) {
                    problems.push(format!("`{key}` lists `{text}` twice"));
                }
                values.push(text.to_string());
            }
            _ => problems.push(format!("`{key}` may hold only non-empty strings")),
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::{Parsed, parse};

    /// A proposal the schema accepts, in the shape a model would answer with.
    const VALID: &str = r#"{"kind":"action_proposal","version":1,"actions":[{"type":"plan","summary":"Stop the idle instance, then update TASKS.md","files_modify":[".ai/TASKS.md"],"files_create":[],"files_delete":[],"risks":["the instance may not be idle"]},{"type":"tool_call","tool":"aws.ec2.describe","arguments":{"instance_id":"i-0abc1234"},"reason":"Confirm the instance is idle before proposing a stop"}]}"#;

    #[test]
    fn a_bare_proposal_is_structured() {
        let Parsed::Structured(proposal) = parse(VALID) else {
            panic!("a schema-valid proposal must parse as structured");
        };
        assert_eq!(proposal.actions.len(), 2);
        assert!(matches!(
            &proposal.actions[0],
            super::Action::Plan { summary, files_modify, .. }
                if summary.starts_with("Stop the idle") && files_modify == &[".ai/TASKS.md".to_string()]
        ));
        assert!(matches!(
            &proposal.actions[1],
            super::Action::ToolCall { tool, reason, .. }
                if tool == "aws.ec2.describe"
                    && reason.as_deref().is_some_and(|r| r.starts_with("Confirm the instance"))
        ));
    }

    #[test]
    fn a_fenced_proposal_and_a_proposal_inside_prose_both_parse() {
        let fenced = format!("Here is the plan:\n```json\n{VALID}\n```");
        assert!(matches!(parse(&fenced), Parsed::Structured(_)));
        let chatty = format!("Sure! {VALID} Hope that helps.");
        assert!(matches!(parse(&chatty), Parsed::Structured(_)));
    }

    #[test]
    fn text_is_prose_and_stays_verbatim() {
        assert_eq!(
            parse("Update TASKS.md, then run the gates."),
            Parsed::Prose("Update TASKS.md, then run the gates.".to_string())
        );
    }

    #[test]
    fn json_that_is_not_a_claim_of_being_a_proposal_is_prose() {
        // A model echoing an unrelated object has not broken a contract it was never given.
        assert!(matches!(
            parse(r#"{"note":"I could not decide"}"#),
            Parsed::Prose(_)
        ));
    }

    #[test]
    fn an_unknown_action_type_is_rejected_rather_than_partially_understood() {
        let text = r#"{"kind":"action_proposal","version":1,"actions":[{"type":"dance"}]}"#;
        let Parsed::Invalid(problems) = parse(text) else {
            panic!("an unknown action type must be rejected");
        };
        assert!(problems[0].contains("unknown action type `dance`"));
    }

    #[test]
    fn every_problem_is_reported_not_just_the_first() {
        let text = r#"{"kind":"action_proposal","version":2,"actions":[{"type":"plan","summary":"","files_modify":["a","a"]},{"type":"tool_call","tool":"aws.ec2.stop"}]}"#;
        let Parsed::Invalid(problems) = parse(text) else {
            panic!("this proposal has three defects and must be rejected");
        };
        assert!(problems.iter().any(|p| p.contains("`version` must be 1")));
        assert!(problems
            .iter()
            .any(|p| p.contains("`summary` must be a non-empty string")));
        assert!(problems.iter().any(|p| p.contains("lists `a` twice")));
        assert!(problems
            .iter()
            .any(|p| p.contains("`arguments` must be an object")));
    }

    #[test]
    fn an_empty_action_list_is_rejected() {
        let text = r#"{"kind":"action_proposal","version":1,"actions":[]}"#;
        assert!(matches!(parse(text), Parsed::Invalid(_)));
    }

    #[test]
    fn the_schema_and_the_hand_rolled_rules_agree_on_every_fixture() {
        // The drift net: the committed schema and this module's validation are two definitions of
        // one contract, and this test is where they meet. If either side changes alone, one of the
        // two assertions below fails.
        let schema_source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../schemas/action-proposal.schema.json"),
        )
        .expect("the action-proposal schema is in the repository");
        let schema: serde_json::Value =
            serde_json::from_str(&schema_source).expect("the schema is valid JSON");
        let validator = jsonschema::validator_for(&schema).expect("the schema compiles");

        let cases = [
            (VALID, true),
            (
                r#"{"kind":"action_proposal","version":1,"actions":[{"type":"dance"}]}"#,
                false,
            ),
            (
                r#"{"kind":"action_proposal","version":2,"actions":[{"type":"plan","summary":"x"}]}"#,
                false,
            ),
            (
                r#"{"kind":"action_proposal","version":1,"actions":[]}"#,
                false,
            ),
            (
                r#"{"kind":"action_proposal","version":1,"actions":[{"type":"plan","summary":""}]}"#,
                false,
            ),
            (
                r#"{"kind":"action_proposal","version":1,"actions":[{"type":"tool_call","tool":"a.b","arguments":"nope"}]}"#,
                false,
            ),
            (
                r#"{"kind":"action_proposal","version":1,"actions":[{"type":"tool_call"}]}"#,
                false,
            ),
        ];
        for (text, schema_says_valid) in cases {
            let schema_result = validator.validate(&serde_json::from_str::<serde_json::Value>(text).unwrap()).is_ok();
            assert_eq!(
                schema_result, schema_says_valid,
                "the schema itself disagrees with the expected validity of {text}"
            );
            let ours = !matches!(parse(text), Parsed::Invalid(_));
            assert_eq!(
                ours, schema_says_valid,
                "hand-rolled validation disagrees with the schema on {text}"
            );
        }
    }
}
