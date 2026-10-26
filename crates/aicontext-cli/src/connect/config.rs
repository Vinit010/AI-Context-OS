//! `.ai/ai.yaml`: the committed provider list, read and written without losing what this version
//! does not know about.
//!
//! The file is the committed half of provider configuration (`docs/AI_PROVIDER_SPEC.md` §4, §5). It
//! may name a credential *source* — an environment variable's name, a keychain entry's name — and
//! must never carry a value; `schemas/ai-config.schema.json` deliberately has no vocabulary for a
//! token. `connect` and `disconnect` (`TASK-053`) are the only writers.
//!
//! The document is held as a `yaml_serde` value rather than deserialised into a struct. A struct
//! would round-trip only the keys this build knows about, so a key an older or newer build wrote —
//! `default_model`, a budget block, a field a future adapter needs — would be deleted by the mere act
//! of adding one provider. Walking the mapping keeps every unknown key exactly as it was.
//!
//! This is the second module in the workspace to see a YAML type, after `aicontext-context`'s
//! `YamlCodec` (`ADR-007`). `ARCHITECTURE.md` §3.2 rule 7 confines the *front-matter* codec to
//! `aicontext-context`; this file is a different document with a different schema, and it stays
//! private to the two commands that write it.

use std::path::{Path, PathBuf};

use aicontext_providers::{Cost, CredentialSource, KeychainReference, Price, Pricing};
use yaml_serde::{Mapping, Value};

use super::error::ConnectError;
use crate::project;

/// The repository-relative path of the provider configuration.
pub(crate) const CONFIG_PATH: &str = ".ai/ai.yaml";

/// The keychain service every entry `aicontext` manages is stored under.
///
/// Together with the account (`keychain_entry`) it forms the service/account pair
/// `aicontext_providers::KeychainReference` names. Fixed rather than configurable, so the name
/// `connect` writes through is the name resolution reads through
/// (`docs/AI_PROVIDER_SPEC.md` §4).
pub(crate) const KEYCHAIN_SERVICE: &str = "aicontext";

/// Which credential source a provider records, mirroring `auth.type` in the schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthKind {
    /// The provider needs no credential.
    None,
    /// The credential lives only in the OS keychain.
    Keychain,
    /// An environment variable wins, then the keychain.
    EnvironmentOrKeychain,
}

impl AuthKind {
    /// The spelling `schemas/ai-config.schema.json` fixes.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Keychain => "keychain",
            Self::EnvironmentOrKeychain => "env_or_keychain",
        }
    }

    /// The kind the `type` field names, or `None` for a spelling this version does not know.
    fn parse(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "keychain" => Some(Self::Keychain),
            "env_or_keychain" => Some(Self::EnvironmentOrKeychain),
            _ => None,
        }
    }
}

/// The `auth` block of one provider: names only, never a value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthEntry {
    /// Which source the provider records.
    pub(crate) kind: AuthKind,
    /// The *name* of the environment variable, when one is part of the source.
    pub(crate) env: Option<String>,
    /// The keychain account, when the keychain is part of the source.
    pub(crate) keychain_entry: Option<String>,
}

impl AuthEntry {
    /// The credential source this entry records, in the provider crate's vocabulary.
    ///
    /// The keychain account defaults to the provider id, which is exactly the name `connect`
    /// writes when it stores a credential without `--keychain-entry` — the same default on both
    /// sides, or the entry could be written under one name and looked up under another
    /// (`docs/AI_PROVIDER_SPEC.md` §4).
    pub(crate) fn source(&self, provider_id: &str) -> CredentialSource {
        let account = self
            .keychain_entry
            .clone()
            .unwrap_or_else(|| provider_id.to_string());
        let reference = KeychainReference::new(KEYCHAIN_SERVICE, account);
        match self.kind {
            AuthKind::None => CredentialSource::None,
            AuthKind::Keychain => CredentialSource::Keychain { reference },
            AuthKind::EnvironmentOrKeychain => CredentialSource::EnvironmentOrKeychain {
                variable: self.env.clone().unwrap_or_default(),
                reference,
            },
        }
    }
}

/// One entry of the `providers` list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderEntry {
    /// The id the provider is registered under.
    pub(crate) id: String,
    /// The adapter that speaks the provider's wire format.
    pub(crate) adapter: String,
    /// The server root.
    pub(crate) base_url: Option<String>,
    /// The model `connect` verified with.
    pub(crate) model: Option<String>,
    /// Where the credential comes from.
    pub(crate) auth: AuthEntry,
}

impl ProviderEntry {
    /// The entry as the YAML mapping the schema describes.
    ///
    /// `auth` is always written, because the schema requires it, and the order of keys is the order
    /// the documentation shows, so a hand-edited file and a generated one agree.
    fn to_value(&self) -> Value {
        let mut auth = Mapping::new();
        put(
            &mut auth,
            "type",
            Value::String(self.auth.kind.as_str().to_string()),
        );
        if let Some(variable) = &self.auth.env {
            put(&mut auth, "env", Value::String(variable.clone()));
        }
        if let Some(entry) = &self.auth.keychain_entry {
            put(&mut auth, "keychain_entry", Value::String(entry.clone()));
        }

        let mut provider = Mapping::new();
        put(&mut provider, "id", Value::String(self.id.clone()));
        put(
            &mut provider,
            "adapter",
            Value::String(self.adapter.clone()),
        );
        if let Some(base_url) = &self.base_url {
            put(&mut provider, "base_url", Value::String(base_url.clone()));
        }
        if let Some(model) = &self.model {
            put(&mut provider, "model", Value::String(model.clone()));
        }
        put(&mut provider, "auth", Value::Mapping(auth));
        Value::Mapping(provider)
    }

    /// Reads an entry back, taking only the fields these two commands act on.
    ///
    /// A field this version does not read is left in the document; only `id` is required, because a
    /// provider with no id cannot be looked up or removed, and that is a shape the schema forbids.
    fn from_value(value: &Value, path: &str) -> Result<Self, ConnectError> {
        let id = string_field(value, "id").ok_or_else(|| ConnectError::Shape {
            path: path.to_string(),
            detail: "a provider has no `id`".to_string(),
        })?;
        let auth = value.get("auth");
        Ok(Self {
            id,
            adapter: string_field(value, "adapter").unwrap_or_default(),
            base_url: string_field(value, "base_url"),
            model: string_field(value, "model"),
            auth: match auth {
                Some(auth) => AuthEntry {
                    kind: string_field(auth, "type")
                        .as_deref()
                        .and_then(AuthKind::parse)
                        .unwrap_or(AuthKind::None),
                    env: string_field(auth, "env"),
                    keychain_entry: string_field(auth, "keychain_entry"),
                },
                None => AuthEntry {
                    kind: AuthKind::None,
                    env: None,
                    keychain_entry: None,
                },
            },
        })
    }
}

/// The parsed `.ai/ai.yaml`, or an empty document when the file does not exist yet.
#[derive(Debug)]
pub(crate) struct AiConfig {
    path: PathBuf,
    document: Value,
}

impl AiConfig {
    /// Loads `.ai/ai.yaml` under `root`, or starts an empty document when it is absent.
    ///
    /// Absent is not an error: the first `connect` in a fresh project creates the file. A file that
    /// exists but cannot be read or parsed *is* an error, because silently replacing it would discard
    /// a provider list the developer wrote.
    pub(crate) fn load(root: &Path) -> Result<Self, ConnectError> {
        let path = root.join(CONFIG_PATH);
        let document = match std::fs::read_to_string(&path) {
            Ok(text) => parse(&path, &text)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => empty(),
            Err(error) => {
                return Err(ConnectError::Read {
                    path: project::display(&path),
                    reason: error.to_string(),
                });
            }
        };
        Ok(Self { path, document })
    }

    /// The path as a report spells it.
    pub(crate) fn display(&self) -> String {
        project::display(&self.path)
    }

    /// Adds `entry`, replacing any provider that already has its id.
    ///
    /// The replacement updates the fields `connect` writes and leaves every other key alone. A
    /// `pricing` block, a `default_model`, or a field a future adapter needs is configuration the
    /// developer wrote; reconnecting must not be the act that deletes it. A key that is optional in
    /// [`ProviderEntry`] and absent from the replacement (`base_url`, `model`) is removed, because
    /// its absence is the new state.
    pub(crate) fn upsert(&mut self, entry: &ProviderEntry) -> Result<(), ConnectError> {
        let replacement = entry.to_value();
        let providers = self.providers_mut()?;
        match providers
            .iter_mut()
            .find(|provider| provider.get("id").and_then(Value::as_str) == Some(entry.id.as_str()))
        {
            Some(existing) => merge_provider(existing, &replacement),
            None => providers.push(replacement),
        }
        Ok(())
    }

    /// Removes the provider `id`, returning the entry it removed.
    ///
    /// `None` means there was no such provider; the caller turns that into an error rather than
    /// treating "already absent" as success, so a mistyped id is not silently ignored
    /// (`RULES.md` §4.10).
    pub(crate) fn remove(&mut self, id: &str) -> Result<Option<ProviderEntry>, ConnectError> {
        let path = self.display();
        let providers = self.providers_mut()?;
        let Some(index) = providers
            .iter()
            .position(|provider| provider.get("id").and_then(Value::as_str) == Some(id))
        else {
            return Ok(None);
        };
        let removed = providers.remove(index);
        ProviderEntry::from_value(&removed, &path).map(Some)
    }

    /// Records `id` as `default_provider` when the file names no default yet.
    ///
    /// Only when it is absent: a default the developer chose is never overwritten by connecting a
    /// second provider.
    pub(crate) fn ensure_default(&mut self, id: &str) -> Result<(), ConnectError> {
        let mapping = self.root()?;
        if mapping.get("default_provider").is_none() {
            mapping.insert(
                Value::String("default_provider".to_string()),
                Value::String(id.to_string()),
            );
        }
        Ok(())
    }

    /// Clears `default_provider` when it names `id`.
    ///
    /// Only when it names the provider being removed. A default pointing at a provider the file no
    /// longer lists is a dangling reference, and `schemas/ai-config.schema.json` calls it a finding,
    /// so taking a provider away has to take its default with it.
    pub(crate) fn clear_default(&mut self, id: &str) -> Result<(), ConnectError> {
        let mapping = self.root()?;
        if mapping.get("default_provider").and_then(Value::as_str) == Some(id) {
            mapping.remove("default_provider");
        }
        Ok(())
    }

    /// The top-level mapping every mutation walks, with the path spelled for an error.
    fn root(&mut self) -> Result<&mut Mapping, ConnectError> {
        let path = self.display();
        self.document
            .as_mapping_mut()
            .ok_or_else(|| ConnectError::Shape {
                path,
                detail: "the top level is not a mapping of keys to values".to_string(),
            })
    }

    /// Every provider id the file lists, in written order.
    pub(crate) fn provider_ids(&self) -> Result<Vec<String>, ConnectError> {
        let path = self.display();
        self.providers()?
            .iter()
            .map(|provider| {
                provider
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| ConnectError::Shape {
                        path: path.clone(),
                        detail: "a provider has no `id`".to_string(),
                    })
            })
            .collect()
    }

    /// The id `default_provider` names, when the file names one.
    ///
    /// `agent run` reads through this rather than guessing which provider to call: a project with
    /// three providers and no default must be told which one to use, not have one picked for it
    /// (`RULES.md` §4.10).
    pub(crate) fn default_provider(&self) -> Option<String> {
        self.document
            .as_mapping()
            .and_then(|mapping| mapping.get("default_provider"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// The entry for `id`, when the file lists it.
    ///
    /// `None` means the id is not in the list — for `default_provider`, a dangling reference the
    /// caller reports rather than works around.
    pub(crate) fn provider(&self, id: &str) -> Result<Option<ProviderEntry>, ConnectError> {
        let path = self.display();
        let found = self
            .providers()?
            .iter()
            .find(|provider| provider.get("id").and_then(Value::as_str) == Some(id))
            .cloned();
        found
            .map(|value| ProviderEntry::from_value(&value, &path))
            .transpose()
    }

    /// The per-run spend ceiling `budget.max_run_usd` records, in micro-USD.
    ///
    /// `None` means no ceiling is configured, which is different from a ceiling of zero: zero
    /// refuses every non-free call, `None` refuses none. A ceiling is read exactly — no `f64` touches
    /// it — and a value finer than a micro-dollar is rounded **up**, so the ceiling can only be
    /// stricter than the number written, never looser.
    pub(crate) fn budget_ceiling(&self) -> Result<Option<Cost>, ConnectError> {
        let path = self.display();
        let Some(budget) = self
            .document
            .as_mapping()
            .and_then(|mapping| mapping.get("budget"))
        else {
            return Ok(None);
        };
        let budget = budget.as_mapping().ok_or_else(|| ConnectError::Shape {
            path: path.clone(),
            detail: "`budget` is not a mapping of keys to values".to_string(),
        })?;
        let Some(value) = budget.get("max_run_usd") else {
            return Ok(None);
        };
        let micro_usd = usd_micros(value, "budget.max_run_usd", &path)?;
        Ok(Some(Cost::from_micro_usd(micro_usd)))
    }

    /// The list prices `id`'s `pricing` block records, when it has one.
    ///
    /// `None` means the provider is priced by nothing this tool knows: its cost is *unavailable*,
    /// never zero (`docs/AI_PROVIDER_SPEC.md` §7). A block that exists but cannot be read is a shape
    /// error rather than a silent omission, because a price that is half understood is a budget
    /// check that cannot be trusted.
    pub(crate) fn pricing(&self, id: &str) -> Result<Option<Pricing>, ConnectError> {
        let path = self.display();
        let Some(provider) = self
            .providers()?
            .iter()
            .find(|provider| provider.get("id").and_then(Value::as_str) == Some(id))
        else {
            return Ok(None);
        };
        let Some(pricing) = provider.get("pricing") else {
            return Ok(None);
        };
        let pricing = pricing.as_mapping().ok_or_else(|| ConnectError::Shape {
            path: path.clone(),
            detail: format!("provider `{id}`: `pricing` is not a mapping of keys to values"),
        })?;
        let input = price_field(pricing, "input_per_mtok", id, &path)?;
        let output = price_field(pricing, "output_per_mtok", id, &path)?;
        let cached_input = match pricing.get("cached_input_per_mtok") {
            Some(value) => Some(Price::from_micro_usd(usd_micros(
                value,
                "pricing.cached_input_per_mtok",
                &path,
            )?)),
            None => None,
        };
        Ok(Some(Pricing::new(input, output, cached_input)))
    }

    /// Writes the document back, verbatim apart from the change that was made.
    pub(crate) fn save(&self) -> Result<(), ConnectError> {
        let text = yaml_serde::to_string(&self.document).map_err(|error| ConnectError::Write {
            path: self.display(),
            reason: error.to_string(),
        })?;
        std::fs::write(&self.path, text).map_err(|error| ConnectError::Write {
            path: self.display(),
            reason: error.to_string(),
        })
    }

    /// Removes the file, for the one state the schema refuses to describe.
    ///
    /// `schemas/ai-config.schema.json` requires `providers` to hold at least one item, and `init`
    /// never creates this file, so a project with no providers has no `.ai/ai.yaml`. Writing an empty
    /// list would leave a document that fails its own schema, and a document other tools trust
    /// (`CONTEXT_SPEC.md` §1). Returning the tree to the state it was in before the first `connect`
    /// is the only version of "the last provider is gone" that stays true to both.
    pub(crate) fn discard(&self) -> Result<(), ConnectError> {
        std::fs::remove_file(&self.path).map_err(|error| ConnectError::Discard {
            path: self.display(),
            reason: error.to_string(),
        })
    }

    /// The `providers` list, inserting an empty one when the key is absent.
    fn providers_mut(&mut self) -> Result<&mut Vec<Value>, ConnectError> {
        let path = self.display();
        let mapping = self.root()?;
        if mapping.get("providers").is_none() {
            mapping.insert(
                Value::String("providers".to_string()),
                Value::Sequence(Vec::new()),
            );
        }
        mapping
            .get_mut("providers")
            .and_then(Value::as_sequence_mut)
            .ok_or_else(|| ConnectError::Shape {
                path,
                detail: "`providers` is not a list".to_string(),
            })
    }

    /// The `providers` list, without inserting anything.
    fn providers(&self) -> Result<&Vec<Value>, ConnectError> {
        let path = self.display();
        self.document
            .as_mapping()
            .and_then(|mapping| mapping.get("providers"))
            .and_then(Value::as_sequence)
            .ok_or_else(|| ConnectError::Shape {
                path,
                detail: if self.document.is_mapping() {
                    "`providers` is missing or is not a list".to_string()
                } else {
                    "the top level is not a mapping of keys to values".to_string()
                },
            })
    }
}

/// Parses `text` into a mapping, or reports why it is not one.
///
/// An empty file parses to `null`, which is treated as an empty document so a zero-byte
/// `.ai/ai.yaml` is not a permanent error.
fn parse(path: &Path, text: &str) -> Result<Value, ConnectError> {
    let value: Value = yaml_serde::from_str(text).map_err(|error| ConnectError::Malformed {
        path: project::display(path),
        detail: error.to_string(),
    })?;
    if value.is_null() {
        return Ok(empty());
    }
    if !value.is_mapping() {
        return Err(ConnectError::Shape {
            path: project::display(path),
            detail: format!("the top level is {}, not a mapping", describe(&value)),
        });
    }
    Ok(value)
}

/// The document a project with no `.ai/ai.yaml` starts from.
fn empty() -> Value {
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String("version".to_string()),
        Value::Number(1_i64.into()),
    );
    mapping.insert(
        Value::String("providers".to_string()),
        Value::Sequence(Vec::new()),
    );
    Value::Mapping(mapping)
}

/// Reads a string field, treating any other type as absent.
fn string_field(value: &Value, name: &str) -> Option<String> {
    value.get(name).and_then(Value::as_str).map(str::to_string)
}

/// Writes one field under its schema name.
///
/// A function rather than an inline `insert` so the string key is spelled once per document build:
/// the schema's field names are the contract, and a typo in one is a file nothing else can read.
fn put(mapping: &mut Mapping, name: &str, value: Value) {
    mapping.insert(Value::String(name.to_string()), value);
}

/// Updates a provider's written fields without deleting keys this build does not own.
fn merge_provider(existing: &mut Value, replacement: &Value) {
    let Some(replacement) = replacement.as_mapping() else {
        return;
    };
    let Value::Mapping(current) = existing else {
        *existing = Value::Mapping(replacement.clone());
        return;
    };
    // These are optional in the entry, so an absent one in the replacement is the new state. Every
    // other key — `pricing`, a future adapter's fields — is configuration this writer must not own.
    for optional in ["base_url", "model"] {
        if !replacement.contains_key(optional) {
            current.remove(optional);
        }
    }
    for (key, value) in replacement {
        current.insert(key.clone(), value.clone());
    }
}

/// Reads one required price from a provider's `pricing` block.
fn price_field(pricing: &Mapping, key: &str, id: &str, path: &str) -> Result<Price, ConnectError> {
    let Some(value) = pricing.get(key) else {
        return Err(ConnectError::Shape {
            path: path.to_string(),
            detail: format!("provider `{id}`: `pricing` has no `{key}`"),
        });
    };
    Ok(Price::from_micro_usd(usd_micros(
        value,
        &format!("pricing.{key}"),
        path,
    )?))
}

/// Reads a non-negative USD amount as micro-USD, rounding **up**.
///
/// Rounding up is the conservative direction both for a price and for a ceiling: a price can never
/// undercharge a run, and a ceiling can never be looser than the number the developer wrote.
fn usd_micros(value: &Value, field: &str, path: &str) -> Result<u64, ConnectError> {
    let Value::Number(number) = value else {
        return Err(ConnectError::Shape {
            path: path.to_string(),
            detail: format!(
                "`{field}` must be a non-negative USD amount, found {}",
                describe(value)
            ),
        });
    };
    decimal_micros(&number.to_string()).ok_or_else(|| ConnectError::Shape {
        path: path.to_string(),
        detail: format!(
            "`{field}` must be a non-negative USD amount with at most 18 integer digits"
        ),
    })
}

/// Converts a decimal USD string to micro-USD exactly, rounding any remainder up.
///
/// The value comes from [`yaml_serde::Number`]'s shortest round-trip rendering, so `2.50` arrives as
/// `2.5`; that is the same number. The parse is textual rather than `f64` multiplication so a price
/// or a ceiling is the number that was written, not the nearest binary float to it.
fn decimal_micros(text: &str) -> Option<u64> {
    let text = text.strip_prefix('+').unwrap_or(text);
    if text.starts_with('-') {
        return None;
    }
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(index) => (&text[..index], text[index + 1..].parse::<i32>().ok()?),
        None => (text, 0_i32),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let whole = if whole.is_empty() { "0" } else { whole };
    if !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some(0);
    }
    let scale = exponent
        .checked_sub(i32::try_from(fraction.len()).ok()?)?
        .checked_add(6)?;
    let value = digits.parse::<u128>().ok()?;
    let micros = if scale >= 0 {
        value.checked_mul(10u128.checked_pow(u32::try_from(scale).ok()?)?)?
    } else {
        let divisor = 10u128.checked_pow(u32::try_from(scale.checked_neg()?).ok()?)?;
        value.checked_add(divisor.checked_sub(1)?)? / divisor
    };
    u64::try_from(micros).ok()
}

/// One word for what a value is, for an error message.
fn describe(value: &Value) -> String {
    match value {
        Value::Null => "empty".to_string(),
        Value::Bool(_) => "a boolean".to_string(),
        Value::Number(_) => "a number".to_string(),
        Value::String(_) => "a string".to_string(),
        Value::Sequence(_) => "a list".to_string(),
        Value::Mapping(_) => "a mapping".to_string(),
        Value::Tagged(_) => "a tagged value".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use aicontext_providers::{Cost, Price};

    use super::{AiConfig, AuthEntry, AuthKind, KEYCHAIN_SERVICE, ProviderEntry, decimal_micros};
    use crate::connect::ConnectError;

    fn entry(id: &str) -> ProviderEntry {
        ProviderEntry {
            id: id.to_string(),
            adapter: "openai-compatible".to_string(),
            base_url: Some("http://localhost:11434/v1".to_string()),
            model: Some("qwen2.5-coder".to_string()),
            auth: AuthEntry {
                kind: AuthKind::None,
                env: None,
                keychain_entry: None,
            },
        }
    }

    fn write(root: &std::path::Path, text: &str) {
        std::fs::create_dir_all(root.join(".ai")).expect("create .ai");
        std::fs::write(root.join(".ai/ai.yaml"), text).expect("write ai.yaml");
    }

    /// A project root with an `.ai/` tree.
    ///
    /// `save` deliberately does not create it: only `init` writes the skeleton
    /// (`ARCHITECTURE.md` §3.1), so every test of a file *inside* `.ai/` starts from one that exists.
    fn tree() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ai")).expect("create .ai");
        temp
    }

    #[test]
    fn a_missing_file_starts_an_empty_document_that_round_trips() {
        let temp = tree();
        let mut config = AiConfig::load(temp.path()).expect("loads empty");
        config.upsert(&entry("local")).expect("upsert");
        config.ensure_default("local").expect("default");
        config.save().expect("save");

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(text.contains("version: 1"), "{text}");
        assert!(text.contains("default_provider: local"), "{text}");
        assert!(text.contains("id: local"), "{text}");
        assert!(text.contains("type: none"), "{text}");
        assert_eq!(KEYCHAIN_SERVICE, "aicontext");
    }

    #[test]
    fn an_empty_file_is_an_empty_document_not_a_syntax_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        write(temp.path(), "");
        let mut config = AiConfig::load(temp.path()).expect("an empty file loads");
        config.upsert(&entry("local")).expect("upsert");
        assert_eq!(
            config.provider_ids().expect("ids"),
            vec!["local".to_string()]
        );
    }

    #[test]
    fn unknown_keys_survive_an_upsert() {
        // A key this build does not know about is exactly what the value-not-struct choice protects:
        // connecting one provider must not delete another's configuration.
        let temp = tempfile::tempdir().expect("tempdir");
        write(
            temp.path(),
            "version: 1\ndefault_model: qwen\nbudget:\n  max_run_usd: 1.0\nproviders:\n  - id: hosted\n    adapter: openai-compatible\n    auth:\n      type: none\n      future_field: keep-me\n",
        );
        let mut config = AiConfig::load(temp.path()).expect("loads");
        config.upsert(&entry("local")).expect("upsert");
        config.save().expect("save");

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(text.contains("default_model: qwen"), "{text}");
        assert!(text.contains("max_run_usd: 1.0"), "{text}");
        assert!(text.contains("future_field: keep-me"), "{text}");
        assert!(text.contains("id: hosted"), "{text}");
        assert!(text.contains("id: local"), "{text}");
    }

    #[test]
    fn upserting_the_same_id_replaces_rather_than_duplicates() {
        let temp = tree();
        let mut config = AiConfig::load(temp.path()).expect("loads");
        config.upsert(&entry("local")).expect("first");
        config
            .upsert(&ProviderEntry {
                base_url: Some("http://example.test/v1".to_string()),
                ..entry("local")
            })
            .expect("second");
        assert_eq!(
            config.provider_ids().expect("ids"),
            vec!["local".to_string()]
        );
        config.save().expect("save");

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert_eq!(text.matches("id: local").count(), 1, "{text}");
        assert!(text.contains("http://example.test/v1"), "{text}");
    }

    #[test]
    fn remove_returns_the_entry_and_leaves_the_others_alone() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = AiConfig::load(temp.path()).expect("loads");
        config.upsert(&entry("local")).expect("local");
        config.upsert(&entry("hosted")).expect("hosted");

        let removed = config.remove("local").expect("remove").expect("was there");
        assert_eq!(removed.id, "local");
        assert_eq!(
            config.provider_ids().expect("ids"),
            vec!["hosted".to_string()]
        );
        assert!(config.remove("local").expect("remove again").is_none());
    }

    #[test]
    fn a_default_already_set_is_not_overwritten() {
        let temp = tree();
        write(
            temp.path(),
            "default_provider: hosted\nproviders:\n  - id: hosted\n    auth:\n      type: none\n",
        );
        let mut config = AiConfig::load(temp.path()).expect("loads");
        config.ensure_default("local").expect("default");
        config.save().expect("save");

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(
            text.contains("default_provider: hosted"),
            "a default the developer chose is theirs: {text}"
        );
        assert!(!text.contains("default_provider: local"), "{text}");
    }

    #[test]
    fn a_file_that_is_not_a_mapping_is_a_shape_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        write(temp.path(), "- one\n- two\n");
        let error = AiConfig::load(temp.path()).expect_err("a list is not a document");
        assert!(matches!(error, ConnectError::Shape { .. }), "{error:?}");
        assert_eq!(error.code(), "CONNECT-003");
    }

    #[test]
    fn a_file_that_is_not_yaml_is_a_malformed_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        write(temp.path(), "providers: [\n");
        let error = AiConfig::load(temp.path()).expect_err("unbalanced flow sequence");
        assert!(matches!(error, ConnectError::Malformed { .. }), "{error:?}");
        assert_eq!(error.code(), "CONNECT-002");
    }

    #[test]
    fn providers_that_is_not_a_list_is_a_shape_error() {
        let temp = tree();
        write(temp.path(), "providers: a-string\n");
        let config = AiConfig::load(temp.path()).expect("the file itself parses");
        let error = config.provider_ids().expect_err("not a list");
        assert!(matches!(error, ConnectError::Shape { .. }), "{error:?}");
        assert_eq!(error.code(), "CONNECT-003");
    }

    #[test]
    fn a_provider_with_no_id_is_a_shape_error_rather_than_an_unfindable_entry() {
        let temp = tree();
        write(temp.path(), "providers:\n  - adapter: openai-compatible\n");
        let config = AiConfig::load(temp.path()).expect("the file itself parses");
        let error = config
            .provider_ids()
            .expect_err("no id to look anything up by");
        assert!(matches!(error, ConnectError::Shape { .. }), "{error:?}");
        assert_eq!(error.code(), "CONNECT-003");
    }

    #[test]
    fn a_budget_ceiling_is_read_exactly_and_rounds_up() {
        let temp = tree();
        write(
            temp.path(),
            "budget:\n  max_run_usd: 1.25\nproviders:\n  - id: local\n    auth:\n      type: none\n",
        );
        let config = AiConfig::load(temp.path()).expect("loads");
        assert_eq!(
            config.budget_ceiling().expect("a ceiling"),
            Some(Cost::from_micro_usd(1_250_000))
        );

        write(
            temp.path(),
            "budget:\n  max_run_usd: 0.0000001\nproviders:\n  - id: local\n    auth:\n      type: none\n",
        );
        let config = AiConfig::load(temp.path()).expect("loads");
        assert_eq!(
            config.budget_ceiling().expect("a ceiling"),
            Some(Cost::from_micro_usd(1)),
            "a ceiling below one micro-dollar is rounded up, never down"
        );
    }

    #[test]
    fn no_budget_block_means_no_ceiling_and_a_malformed_one_is_a_shape_error() {
        let temp = tree();
        write(
            temp.path(),
            "providers:\n  - id: local\n    auth:\n      type: none\n",
        );
        let config = AiConfig::load(temp.path()).expect("loads");
        assert_eq!(config.budget_ceiling().expect("no ceiling"), None);

        write(
            temp.path(),
            "budget:\n  max_run_usd: -1\nproviders:\n  - id: local\n    auth:\n      type: none\n",
        );
        let config = AiConfig::load(temp.path()).expect("loads");
        let error = config.budget_ceiling().expect_err("a negative ceiling");
        assert!(matches!(error, ConnectError::Shape { .. }), "{error:?}");
        assert_eq!(error.code(), "CONNECT-003");
        assert!(error.to_string().contains("max_run_usd"));
    }

    #[test]
    fn a_pricing_block_is_read_into_provider_prices() {
        let temp = tree();
        write(
            temp.path(),
            "providers:\n  - id: hosted\n    adapter: anthropic\n    auth:\n      type: none\n    \
             pricing:\n      input_per_mtok: 2.5\n      output_per_mtok: 10\n      \
             cached_input_per_mtok: 1.25\n",
        );
        let config = AiConfig::load(temp.path()).expect("loads");
        let pricing = config
            .pricing("hosted")
            .expect("a readable block")
            .expect("a configured block");
        assert_eq!(pricing.input().micro_usd(), 2_500_000);
        assert_eq!(pricing.output().micro_usd(), 10_000_000);
        assert_eq!(
            pricing.cached_input().map(Price::micro_usd),
            Some(1_250_000)
        );
    }

    #[test]
    fn a_provider_without_pricing_has_no_cost_not_a_free_one() {
        let temp = tree();
        write(
            temp.path(),
            "providers:\n  - id: local\n    auth:\n      type: none\n",
        );
        let config = AiConfig::load(temp.path()).expect("loads");
        assert_eq!(config.pricing("local").expect("readable"), None);
        assert_eq!(
            config.pricing("absent").expect("not listed"),
            None,
            "an unlisted provider has no prices either"
        );
    }

    #[test]
    fn an_incomplete_or_malformed_pricing_block_is_a_shape_error() {
        let temp = tree();
        write(
            temp.path(),
            "providers:\n  - id: hosted\n    auth:\n      type: none\n    pricing:\n      \
             input_per_mtok: 2.5\n",
        );
        let config = AiConfig::load(temp.path()).expect("loads");
        let error = config.pricing("hosted").expect_err("no output price");
        assert!(matches!(error, ConnectError::Shape { .. }), "{error:?}");
        assert!(error.to_string().contains("output_per_mtok"));

        write(
            temp.path(),
            "providers:\n  - id: hosted\n    auth:\n      type: none\n    pricing:\n      \
             input_per_mtok: '2.50'\n      output_per_mtok: 10\n",
        );
        let config = AiConfig::load(temp.path()).expect("loads");
        let error = config
            .pricing("hosted")
            .expect_err("a string is not a price");
        assert!(matches!(error, ConnectError::Shape { .. }), "{error:?}");
        assert_eq!(error.code(), "CONNECT-003");
    }

    #[test]
    fn reconnecting_preserves_a_manually_added_pricing_block_and_unknown_keys() {
        let temp = tree();
        write(
            temp.path(),
            "providers:\n  - id: local\n    adapter: openai-compatible\n    model: old-model\n    \
             auth:\n      type: none\n    pricing:\n      input_per_mtok: 2.5\n      \
             output_per_mtok: 10\n    future_field: keep-me\n",
        );
        let mut config = AiConfig::load(temp.path()).expect("loads");
        config.upsert(&entry("local")).expect("upsert");
        config.save().expect("save");

        let text = std::fs::read_to_string(temp.path().join(".ai/ai.yaml")).expect("read");
        assert!(text.contains("input_per_mtok: 2.5"), "{text}");
        assert!(text.contains("output_per_mtok: 10"), "{text}");
        assert!(text.contains("future_field: keep-me"), "{text}");
        assert!(text.contains("model: qwen2.5-coder"), "{text}");
        assert!(
            !text.contains("model: old-model"),
            "the verified model is the new state: {text}"
        );
    }

    #[test]
    fn a_usd_amount_is_converted_to_micro_usd_without_binary_floating_point() {
        assert_eq!(decimal_micros("2.5"), Some(2_500_000));
        assert_eq!(decimal_micros("0.075"), Some(75_000));
        assert_eq!(decimal_micros("0.000001"), Some(1));
        assert_eq!(
            decimal_micros("0.0000001"),
            Some(1),
            "anything finer than a micro-dollar rounds up"
        );
        assert_eq!(decimal_micros("1e-6"), Some(1));
        assert_eq!(decimal_micros("0"), Some(0));
        assert_eq!(decimal_micros("-1"), None);
        assert_eq!(decimal_micros("not-a-number"), None);
    }
}
