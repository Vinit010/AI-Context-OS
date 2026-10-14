//! The [`ProviderRegistry`]: the providers this run knows about, and the seam that picks one.

use std::collections::BTreeMap;
use std::fmt;

use crate::provider::AIProvider;
use crate::types::{ProviderId, RoutingRequest};

/// The providers configured for this run, keyed by id.
///
/// It owns its providers as `Box<dyn AIProvider>`, so two providers of different concrete types live
/// in one map, and an adapter can be added without this file changing.
///
/// # Routing is a seam, not logic
///
/// [`ProviderRegistry::route`] exists because the context engine, the CLI contract, and the gateway
/// must never have to change when routing becomes real. In the MVP it answers with the configured
/// default and ignores the request entirely — no classification, no budget handling, no fallback
/// (`docs/AI_PROVIDER_SPEC.md` §8, `PRD.md` §49).
pub struct ProviderRegistry {
    default: ProviderId,
    providers: BTreeMap<ProviderId, Box<dyn AIProvider>>,
}

impl ProviderRegistry {
    /// A registry whose default provider is `default`, holding no providers yet.
    ///
    /// The default may be registered later; [`ProviderRegistry::get`] on an unregistered id is what
    /// reports the gap, not this constructor.
    #[must_use]
    pub fn new(default: ProviderId) -> Self {
        Self {
            default,
            providers: BTreeMap::new(),
        }
    }

    /// Adds `provider` under its own id, returning the provider it replaced, if any.
    ///
    /// Returning the replaced provider rather than dropping it keeps a double registration visible:
    /// the caller decides whether last-wins is right (`RULES.md` §7).
    pub fn register(&mut self, provider: Box<dyn AIProvider>) -> Option<Box<dyn AIProvider>> {
        self.providers.insert(provider.id().clone(), provider)
    }

    /// The provider registered under `id`, if this run has one.
    #[must_use]
    pub fn get(&self, id: &ProviderId) -> Option<&dyn AIProvider> {
        self.providers.get(id).map(Box::as_ref)
    }

    /// The id routing answers with when no routing logic applies: the configured default.
    #[must_use]
    pub fn default_id(&self) -> &ProviderId {
        &self.default
    }

    /// The provider to use for `request`.
    ///
    /// In the MVP this is the configured default and `request` is unused; the parameter is in the
    /// signature now so that adding routing later does not change any caller
    /// (`docs/AI_PROVIDER_SPEC.md` §8).
    #[must_use]
    pub fn route(&self, _request: &RoutingRequest) -> ProviderId {
        self.default.clone()
    }

    /// How many providers are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Whether no provider is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

impl fmt::Debug for ProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `dyn AIProvider` is not `Debug`, and printing a provider could print a credential, so the
        // registry prints ids only (`docs/AI_PROVIDER_SPEC.md` §11 T10).
        formatter
            .debug_struct("ProviderRegistry")
            .field("default", &self.default)
            .field(
                "providers",
                &self.providers.keys().collect::<Vec<&ProviderId>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::ProviderRegistry;
    use crate::error::ProviderError;
    use crate::provider::{AIProvider, BoxStream};
    use crate::types::{
        Capabilities, CompletionRequest, CompletionResponse, FinishReason, ProviderId,
        RoutingRequest, StreamEvent, Usage,
    };

    /// A provider that answers with a fixed text, so the registry can be exercised without a
    /// transport. Test-only, and deliberately named so it cannot be mistaken for a shipped adapter.
    struct FakeProvider {
        id: ProviderId,
    }

    impl FakeProvider {
        fn new(id: &str) -> Self {
            Self {
                id: ProviderId::new(id).expect("a test provider id is a valid slug"),
            }
        }
    }

    #[async_trait]
    impl AIProvider for FakeProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                text: "ok".to_owned(),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: Usage::reported(1, 1),
                provider_meta: serde_json::Value::Null,
            })
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<BoxStream<StreamEvent>, ProviderError> {
            Err(ProviderError::Unsupported {
                provider: self.id.clone(),
                capability: "streaming",
            })
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
    }

    fn request() -> CompletionRequest {
        // Only the fields the trait reads matter here; the rest are the assembler's business.
        CompletionRequest {
            policy: crate::types::PolicyRegion::default(),
            messages: Vec::new(),
            data: Vec::new(),
            tools: Vec::new(),
            budget: crate::types::TokenBudget {
                max_output_tokens: 16,
            },
            options: crate::types::ModelOptions::default(),
            idempotency_key: "test".to_owned(),
        }
    }

    #[test]
    fn a_new_registry_is_empty_and_answers_with_its_default() {
        let default = ProviderId::new("local").expect("valid");
        let registry = ProviderRegistry::new(default.clone());
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert_eq!(registry.route(&RoutingRequest::default()), default);
        assert_eq!(registry.default_id(), &default);
        assert!(registry.get(&default).is_none());
    }

    #[test]
    fn register_reaches_the_provider_by_its_id() {
        let registry_id = ProviderId::new("local").expect("valid");
        let mut registry = ProviderRegistry::new(registry_id.clone());
        let replaced = registry.register(Box::new(FakeProvider::new("local")));
        assert!(replaced.is_none(), "a first registration replaces nothing");
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
        assert!(registry.get(&registry_id).is_some());
        let other = ProviderId::new("hosted").expect("valid");
        assert!(registry.get(&other).is_none());
    }

    #[test]
    fn register_returns_the_provider_it_replaced() {
        // Last-wins is the caller's decision, so the displaced provider comes back rather than
        // disappearing.
        let mut registry = ProviderRegistry::new(ProviderId::new("local").expect("valid"));
        registry.register(Box::new(FakeProvider::new("local")));
        let replaced = registry.register(Box::new(FakeProvider::new("local")));
        assert!(replaced.is_some(), "the second registration displaces the first");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn route_ignores_the_request_and_returns_the_default() {
        // Even a request carrying another provider's name is not consulted in the MVP
        // (`docs/AI_PROVIDER_SPEC.md` 8).
        let registry = ProviderRegistry::new(ProviderId::new("local").expect("valid"));
        assert_eq!(
            registry.route(&RoutingRequest::default()).as_str(),
            "local"
        );
    }

    #[test]
    fn debug_names_ids_and_never_a_provider_body() {
        let mut registry = ProviderRegistry::new(ProviderId::new("local").expect("valid"));
        registry.register(Box::new(FakeProvider::new("local")));
        let rendered = format!("{registry:?}");
        assert!(rendered.contains("ProviderRegistry"));
        assert!(rendered.contains("local"));
    }

    #[test]
    fn a_registered_provider_answers_through_the_trait_object() {
        // The point of the registry: a caller works with `dyn AIProvider` and never the concrete
        // type, which is what lets an adapter be added without touching callers.
        let id = ProviderId::new("local").expect("valid");
        let mut registry = ProviderRegistry::new(id.clone());
        registry.register(Box::new(FakeProvider::new("local")));
        let provider = registry.get(&id).expect("registered");
        let response = futures_executor::block_on(provider.complete(request()))
            .expect("the fake provider succeeds");
        assert_eq!(response.text, "ok");
    }
}
