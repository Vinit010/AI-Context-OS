//! The plugin's typed failures.
//!
//! Every failure a tool can report has a stable `github.*` code, a message that names the concrete
//! subject, and a remediation. `docs/PLUGIN_SPEC.md` §4 makes a tool failure a result with
//! `is_error: true`, not a JSON-RPC error, so the model sees the code and remediation as data and
//! the host keeps the whole envelope untrusted.
//!
//! The codes are deliberately separate from the host's `PLG-*` family: `PLG-*` describes the
//! plugin protocol, while these describe GitHub's answer. A caller can therefore distinguish "the
//! plugin could not start" from "GitHub rate-limited the read" without parsing prose.

use serde_json::{Value, json};
use thiserror::Error;

/// Something a GitHub read could not do.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GitHubError {
    /// The request never produced an HTTP response.
    #[error("the request to GitHub could not be sent: {detail}")]
    Transport {
        /// The transport's own diagnosis, without credentials.
        detail: String,
    },

    /// GitHub answered 401.
    #[error("GitHub rejected the credentials: {detail}")]
    Unauthorized {
        /// GitHub's message, bounded and sanitised by the caller.
        detail: String,
    },

    /// GitHub answered 403 for a reason other than rate limiting.
    #[error("GitHub refused the request: {detail}")]
    Forbidden {
        /// GitHub's message.
        detail: String,
    },

    /// GitHub answered 404.
    #[error("GitHub did not find {resource}")]
    NotFound {
        /// The owner/repository/number or path that was requested.
        resource: String,
    },

    /// GitHub answered 410.
    #[error("GitHub reports that {resource} is gone")]
    Gone {
        /// The resource that was removed.
        resource: String,
    },

    /// GitHub answered 422.
    #[error("GitHub rejected the request parameters: {detail}")]
    Validation {
        /// GitHub's message.
        detail: String,
    },

    /// GitHub answered with a status this plugin does not classify.
    #[error("GitHub answered HTTP {status}: {detail}")]
    HttpStatus {
        /// The numeric status.
        status: u16,
        /// GitHub's message.
        detail: String,
    },

    /// GitHub's primary or secondary rate limit is exhausted.
    #[error("GitHub's rate limit is exhausted: {detail}")]
    RateLimited {
        /// A human-readable description of which limit was hit.
        detail: String,
        /// The `Retry-After` delay, in seconds, when GitHub sent one.
        retry_after_seconds: Option<u64>,
        /// The `X-RateLimit-Reset` Unix timestamp, when GitHub sent one.
        reset_unix: Option<u64>,
        /// The `X-RateLimit-Limit` value, when GitHub sent one.
        limit: Option<u64>,
        /// The `X-RateLimit-Remaining` value, when GitHub sent one.
        remaining: Option<u64>,
    },

    /// A `Link` header could not be used, or a requested page does not exist.
    #[error("the pagination response is not usable: {detail}")]
    Pagination {
        /// What was wrong and which page the caller should use instead.
        detail: String,
    },

    /// An argument is semantically invalid even though it matched the input schema.
    #[error("the argument `{field}` is invalid: {detail}")]
    InvalidArgument {
        /// The argument name.
        field: &'static str,
        /// What is wrong with it.
        detail: String,
    },

    /// The plugin was asked for a tool it does not implement.
    #[error("the tool `{tool}` is not implemented by this plugin")]
    UnknownTool {
        /// The requested tool name.
        tool: String,
    },

    /// A number names a different kind of GitHub resource.
    #[error("`#{number}` is a {kind}, not the requested resource")]
    WrongResource {
        /// The issue or pull request number.
        number: u64,
        /// What the number actually names.
        kind: &'static str,
    },

    /// A contents response named something other than a regular file.
    #[error("`{path}` is a {kind}, not a file")]
    NotAFile {
        /// The requested repository path.
        path: String,
        /// GitHub's `type` value: `dir`, `symlink`, `submodule`, or another kind.
        kind: String,
    },

    /// The contents API did not return the file body.
    #[error("`{path}` is too large to return as text: {detail}")]
    FileTooLarge {
        /// The requested repository path.
        path: String,
        /// GitHub's size or encoding explanation.
        detail: String,
    },

    /// The file body is not UTF-8 text.
    #[error("the file `{path}` is not valid UTF-8 text")]
    NotText {
        /// The requested repository path.
        path: String,
    },

    /// The HTTP response exceeded the plugin's own body cap.
    #[error("the GitHub response exceeded the {limit}-byte plugin limit")]
    ResponseTooLarge {
        /// The cap, in bytes.
        limit: usize,
    },

    /// GitHub answered with a document that does not have the documented shape.
    #[error("GitHub returned an unexpected response: {detail}")]
    Unexpected {
        /// The missing or malformed field.
        detail: String,
    },

    /// A JSON or base64 body could not be decoded.
    #[error("the GitHub response could not be decoded: {detail}")]
    Decode {
        /// The decoder's diagnosis.
        detail: String,
    },

    /// The configured API base is not a URL this plugin will use.
    #[error("the API base URL is not usable: {detail}")]
    ApiBase {
        /// The rejected shape.
        detail: String,
    },
}

impl GitHubError {
    /// The stable code a caller can branch on.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Transport { .. } => "github.transport",
            Self::Unauthorized { .. } => "github.unauthorized",
            Self::Forbidden { .. } => "github.forbidden",
            Self::NotFound { .. } => "github.not_found",
            Self::Gone { .. } => "github.gone",
            Self::Validation { .. } => "github.validation",
            Self::HttpStatus { .. } => "github.http_status",
            Self::RateLimited { .. } => "github.rate_limited",
            Self::Pagination { .. } => "github.pagination",
            Self::InvalidArgument { .. } => "github.invalid_argument",
            Self::UnknownTool { .. } => "github.unknown_tool",
            Self::WrongResource { .. } => "github.wrong_resource",
            Self::NotAFile { .. } => "github.not_a_file",
            Self::FileTooLarge { .. } => "github.file_too_large",
            Self::NotText { .. } => "github.not_text",
            Self::ResponseTooLarge { .. } => "github.response_too_large",
            Self::Unexpected { .. } => "github.unexpected_response",
            Self::Decode { .. } => "github.decode",
            Self::ApiBase { .. } => "github.invalid_api_base",
        }
    }

    /// What the caller can do about it.
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        match self {
            Self::Transport { .. } => {
                "Check the network connection and retry; the plugin does not retry automatically."
            }
            Self::Unauthorized { .. } => {
                "Set GITHUB_TOKEN to a valid token with read-only repository access, or unset it to \
                 read public repositories only."
            }
            Self::Forbidden { .. } => {
                "Check that the token has the required read scope and that SSO is authorized for \
                 the organization."
            }
            Self::NotFound { .. } => {
                "Check the owner, repository, and number or path; a private resource also requires \
                 a token with access."
            }
            Self::Gone { .. } => {
                "The resource was removed or moved; list the repository again to find its current \
                 location."
            }
            Self::Validation { .. } => {
                "Check the argument values against the tool's input schema and retry."
            }
            Self::HttpStatus { .. } => {
                "Retry; if it persists, check GitHub's status page. The status is outside the \
                 responses this plugin classifies."
            }
            Self::RateLimited { .. } => {
                "Wait for the reset (or the Retry-After delay), then retry; set GITHUB_TOKEN to \
                 raise the limit."
            }
            Self::Pagination { .. } => {
                "Use a page number the response reports, or omit page to start at 1; do not follow \
                 a link whose page cannot be parsed."
            }
            Self::InvalidArgument { .. } => {
                "Correct the argument and retry; the tool description lists the accepted shape."
            }
            Self::UnknownTool { .. } => "Call tools/list and use one of the declared tool names.",
            Self::WrongResource { .. } => {
                "Use the matching read tool: github.pr.read for a pull request, github.issue.read \
                 for an issue."
            }
            Self::NotAFile { .. } => {
                "Use github.file.read with a file path; GitHub's contents API returns a directory \
                 listing for a directory."
            }
            Self::FileTooLarge { .. } => {
                "Read a smaller file, or use git to fetch the repository; the contents API returns \
                 at most 1 MiB."
            }
            Self::NotText { .. } => {
                "The file is binary; use a binary-aware tool instead of github.file.read."
            }
            Self::ResponseTooLarge { .. } => {
                "Request a smaller page or a smaller file; the plugin caps a response below the \
                 host's max_output_bytes."
            }
            Self::Unexpected { .. } => {
                "Retry; if it persists, check GitHub's status page. The response did not have the \
                 documented shape."
            }
            Self::Decode { .. } => {
                "Retry; if it persists, the response was not valid JSON or base64."
            }
            Self::ApiBase { .. } => {
                "Use an HTTPS URL with a host and no query or fragment; plain HTTP is accepted only \
                 for loopback tests."
            }
        }
    }

    /// The structured payload a failed tool returns.
    ///
    /// The top-level `trust` marker is redundant inside the host's envelope, which already marks
    /// every result untrusted (`docs/CONTEXT_SPEC.md` §7.6), but it keeps the same guarantee when a
    /// developer drives the plugin directly during development.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut error = serde_json::Map::new();
        error.insert("code".to_owned(), json!(self.code()));
        error.insert("message".to_owned(), json!(self.to_string()));
        error.insert("remediation".to_owned(), json!(self.remediation()));

        if let Self::RateLimited {
            retry_after_seconds,
            reset_unix,
            limit,
            remaining,
            ..
        } = self
        {
            if let Some(value) = *retry_after_seconds {
                error.insert("retry_after_seconds".to_owned(), json!(value));
            }
            if let Some(value) = *reset_unix {
                error.insert("reset_unix".to_owned(), json!(value));
            }
            if let Some(value) = *limit {
                error.insert("limit".to_owned(), json!(value));
            }
            if let Some(value) = *remaining {
                error.insert("remaining".to_owned(), json!(value));
            }
        }

        json!({
            "trust": "untrusted",
            "error": Value::Object(error),
        })
    }
}
