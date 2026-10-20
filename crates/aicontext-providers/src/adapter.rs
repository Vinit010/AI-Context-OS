//! The adapter names `.ai/ai.yaml` carries, and the one place that builds an adapter from one.
//!
//! `docs/AI_PROVIDER_SPEC.md` §3 fixes the names — `openai-compatible`, `anthropic`, `gemini` — and
//! §4 puts one of them in a provider entry. A command that reads a config must turn that string
//! into a provider without itself knowing that Anthropic wants `x-api-key` and Gemini wants the
//! model in the URL: that knowledge belongs to the adapter, so it lives here in the crate that
//! owns the adapters rather than in the CLI that calls them.
//!
//! [`Adapter::build`] takes a plain credential because the *mapping* of that credential onto a
//! header is the adapter's decision. [`Adapter::from_name`] is separate so a command can reject an
//! unknown name as a usage error — naming what *is* built — before any config is written.

use crate::anthropic::{Anthropic, AnthropicConfig};
use crate::error::ProviderError;
use crate::gemini::{Gemini, GeminiConfig};
use crate::openai::{OpenAiCompatible, OpenAiCompatibleConfig};
use crate::provider::AIProvider;
use crate::secret::Secret;
use crate::transport::Auth;
use crate::types::ProviderId;

/// An adapter this build knows how to run, named as `.ai/ai.yaml` names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Adapter {
    /// The chat-completions shape: OpenAI, Ollama, vLLM, and compatible servers.
    OpenAiCompatible,
    /// The Anthropic Messages API.
    Anthropic,
    /// The Gemini `generateContent` API.
    Gemini,
}

impl Adapter {
    /// Every adapter name this build knows, in the order `docs/AI_PROVIDER_SPEC.md` §3 lists them.
    pub const ALL: [Adapter; 3] = [Self::OpenAiCompatible, Self::Anthropic, Self::Gemini];

    /// The name this adapter answers to in `.ai/ai.yaml`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }

    /// The adapter a config named, or `None` for a name no build implements.
    ///
    /// `None` is a usage error, not a provider failure: a command should report the names that
    /// *are* built rather than write a config it cannot run.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "openai-compatible" => Some(Self::OpenAiCompatible),
            "anthropic" => Some(Self::Anthropic),
            "gemini" => Some(Self::Gemini),
            _ => None,
        }
    }

    /// Builds this adapter for one server and one model.
    ///
    /// `credential` is the resolved secret, or `None` for a server that wants none. Which header
    /// it goes into is this adapter's decision and not the caller's: an API key becomes
    /// `Authorization` for a compatible server, `x-api-key` for Anthropic, and `x-goog-api-key`
    /// for Gemini.
    ///
    /// The timeouts and retry policy are the defaults; a host that needs to tune them builds the
    /// adapter's config type directly, which is the same crate and fully public.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Transport`] when the adapter's HTTP client cannot be built.
    pub fn build(
        self,
        id: ProviderId,
        base_url: &str,
        model: &str,
        credential: Option<Secret>,
    ) -> Result<Box<dyn AIProvider>, ProviderError> {
        let auth = credential.map_or(Auth::None, Auth::ApiKey);
        match self {
            Self::OpenAiCompatible => Ok(Box::new(OpenAiCompatible::new(
                OpenAiCompatibleConfig::new(id, base_url, model).with_auth(auth),
            )?)),
            Self::Anthropic => Ok(Box::new(Anthropic::new(
                AnthropicConfig::new(id, base_url, model).with_auth(auth),
            )?)),
            Self::Gemini => Ok(Box::new(Gemini::new(
                GeminiConfig::new(id, base_url, model).with_auth(auth),
            )?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Adapter;
    use crate::ProviderId;

    #[test]
    fn every_adapter_names_and_parses_itself() {
        for adapter in Adapter::ALL {
            assert_eq!(Adapter::from_name(adapter.name()), Some(adapter));
        }
    }

    #[test]
    fn the_names_are_the_ones_the_spec_fixes() {
        assert_eq!(
            Adapter::ALL.map(Adapter::name),
            ["openai-compatible", "anthropic", "gemini"]
        );
    }

    #[test]
    fn an_unknown_name_is_none_rather_than_a_guess() {
        assert_eq!(Adapter::from_name("opena1"), None);
        assert_eq!(Adapter::from_name(""), None);
        assert_eq!(Adapter::from_name("OpenAI-Compatible"), None);
    }

    #[test]
    fn every_adapter_builds_and_reports_the_id_it_was_given() {
        let id = ProviderId::new("hosted").expect("valid");
        for adapter in Adapter::ALL {
            let provider = adapter
                .build(id.clone(), "http://127.0.0.1:9", "test-model", None)
                .expect("the adapter builds");
            assert_eq!(provider.id(), &id);
        }
    }
}
