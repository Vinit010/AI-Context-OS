//! The [`AIProvider`] trait: the one interface every adapter implements.
//!
//! Its signature is the whole abstraction. It names no provider, no URL, no header, and no wire
//! type — only [`CompletionRequest`], [`CompletionResponse`], [`StreamEvent`], [`Capabilities`], and
//! [`ProviderError`] (`docs/AI_PROVIDER_SPEC.md` §1, §2). An adapter's entire job is the translation
//! on either side of it, and if adding one ever required changing this file, the abstraction would
//! be a lie (`docs/AI_PROVIDER_SPEC.md` §12).

use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;

use crate::error::ProviderError;
use crate::types::{Capabilities, CompletionRequest, CompletionResponse, ProviderId, StreamEvent};

/// A stream of [`StreamEvent`]s, `'static` and `Send` so it can cross an `.await` in an async host.
///
/// Generic so a test or a future adapter can box a stream of something else, but the one the trait
/// uses is [`BoxStream<StreamEvent>`] (`docs/AI_PROVIDER_SPEC.md` §2).
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send + 'static>>;

/// One configured model provider.
///
/// The trait is `Send + Sync` and dyn-compatible, because the registry holds providers of different
/// concrete types behind `Box<dyn AIProvider>` and the host may call from any task. That is why the
/// methods are `async` in a trait via `async-trait` rather than native `async fn`, which cannot be
/// dyn-compatible.
///
/// # What an implementation may and may not do
///
/// It may tokenise, open a transport, and translate its own wire format. It may **not** touch the
/// filesystem, spawn a process, or make a tool call (`docs/AI_PROVIDER_SPEC.md` §10): a
/// [`crate::ToolCall`] is a proposal the gateway acts on, never something the provider executes.
/// It must not place [`crate::DataBlock`] content into a [`crate::PolicyRegion`] (§11 T9), and it
/// must not put a credential into any error, log, or usage record (§11 T10).
#[async_trait]
pub trait AIProvider: Send + Sync {
    /// The id this provider is registered under, which is the value
    /// [`crate::ProviderRegistry::route`] returns and the key an error names.
    fn id(&self) -> &ProviderId;

    /// Answers `request` in one shot (`docs/AI_PROVIDER_SPEC.md` §2).
    ///
    /// # Errors
    ///
    /// Any [`ProviderError`]. A model *refusal* is not an error: it is a successful
    /// [`CompletionResponse`] whose `finish_reason` is [`crate::FinishReason::Refusal`]
    /// (`docs/AI_PROVIDER_SPEC.md` §6).
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError>;

    /// Answers `request` as a stream of [`StreamEvent`]s (`docs/AI_PROVIDER_SPEC.md` §2).
    ///
    /// # Errors
    ///
    /// [`ProviderError::Unsupported`] when [`AIProvider::capabilities`] reports
    /// [`Capabilities::streaming`] as `false`, or any other [`ProviderError`] once an attempt is
    /// made.
    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<StreamEvent>, ProviderError>;

    /// What this provider can do, without making a call (`docs/AI_PROVIDER_SPEC.md` §9).
    fn capabilities(&self) -> Capabilities;
}
