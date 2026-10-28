//! The GitHub REST client: one GET, one body cap, typed statuses and pagination.
//!
//! The plugin is read-only, so this module has no method that can send anything but a GET. That is
//! a structural property, not a convention: there is no generic `request` method for a future tool
//! to reach for, and the write tools in `TASK-081` will need a separate, reviewable path.
//!
//! # Why the client is synchronous
//!
//! The plugin protocol is synchronous: the supervisor writes one request frame and waits for one
//! response frame under the manifest's deadline. A blocking client keeps the plugin's one-call-at-a-
//! time shape obvious, and avoids a second runtime whose cancellation behaviour would need its own
//! tests. The HTTP timeout is shorter than the manifest's `call_timeout_ms`, so a slow GitHub
//! response becomes a typed `github.transport` error before the host kills the process.
//!
//! # Token handling
//!
//! `GITHUB_TOKEN` is read once, only by [`GitHubClient::from_environment`], and is never logged or
//! included in an error. [`GitHubClient`]'s `Debug` implementation prints only whether a token is
//! present. The host's sandbox forwards the variable only because the manifest declares it
//! (`docs/PLUGIN_SPEC.md` §7).

use std::fmt;
use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, HeaderMap, LINK};
use serde_json::Value;

use crate::error::GitHubError;

/// The public GitHub API, used when `--api-base` is not supplied.
pub const DEFAULT_API_BASE: &str = "https://api.github.com";

/// The largest response body the plugin reads.
///
/// The manifest's `max_output_bytes` is 1 MiB and the host measures the whole envelope, so the
/// plugin leaves headroom for the JSON wrapper and the envelope's own fields. A response above this
/// cap is a typed `github.response_too_large`, not a silent truncation.
pub const MAX_RESPONSE_BYTES: usize = 768 * 1024;

/// The HTTP deadline. The manifest's `call_timeout_ms` is 30 seconds.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// The API version sent on every request, as GitHub documents.
const API_VERSION: &str = "2022-11-28";

/// The user agent GitHub asks every client to send.
const USER_AGENT: &str = concat!("aicontext-plugin-github/", env!("CARGO_PKG_VERSION"));

/// The pagination state GitHub reported for one list call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageInfo {
    /// The page that was requested.
    pub page: u32,
    /// The page size that was requested.
    pub per_page: u32,
    /// The page the `Link` header names as `next`, if any.
    pub next: Option<u32>,
    /// The page the `Link` header names as `last`, if any.
    pub last: Option<u32>,
}

impl PageInfo {
    /// Whether GitHub reported another page.
    #[must_use]
    pub const fn has_more(self) -> bool {
        self.next.is_some()
    }

    /// The JSON object a list tool returns.
    #[must_use]
    pub fn to_json(self) -> Value {
        serde_json::json!({
            "page": self.page,
            "per_page": self.per_page,
            "has_more": self.has_more(),
            "next": self.next,
            "last": self.last,
        })
    }
}

/// A read-only GitHub client.
pub struct GitHubClient {
    http: Client,
    api_base: Url,
    token: Option<String>,
}

impl fmt::Debug for GitHubClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitHubClient")
            .field("api_base", &self.api_base)
            .field("authenticated", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

impl GitHubClient {
    /// A client for `api_base`, or the public API when it is `None`.
    ///
    /// `token` is the credential value itself. The caller is responsible for obtaining it from the
    /// declared source; [`GitHubClient::from_environment`] is the production path.
    ///
    /// # Errors
    ///
    /// Returns [`GitHubError::ApiBase`] for a URL without a host, with a query or fragment, with
    /// embedded credentials, or with plain HTTP for a non-loopback host. Returns
    /// [`GitHubError::Transport`] if the HTTP client cannot be built.
    pub fn new(api_base: Option<&str>, token: Option<String>) -> Result<Self, GitHubError> {
        let raw = api_base.unwrap_or(DEFAULT_API_BASE);
        let url = Url::parse(raw).map_err(|error| GitHubError::ApiBase {
            detail: format!("`{raw}` is not a URL: {error}"),
        })?;

        if !url.username().is_empty() || url.password().is_some() {
            return Err(GitHubError::ApiBase {
                detail: "the URL must not contain a username or password; use GITHUB_TOKEN instead"
                    .to_owned(),
            });
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(GitHubError::ApiBase {
                detail: "the URL must not contain a query string or fragment".to_owned(),
            });
        }
        if url.host_str().is_none() {
            return Err(GitHubError::ApiBase {
                detail: "the URL has no host".to_owned(),
            });
        }
        match url.scheme() {
            "https" => {}
            "http" if is_loopback(&url) => {}
            "http" => {
                return Err(GitHubError::ApiBase {
                    detail: "plain HTTP is accepted only for loopback hosts; use HTTPS so the \
                             token is not sent in clear text"
                        .to_owned(),
                });
            }
            other => {
                return Err(GitHubError::ApiBase {
                    detail: format!("the scheme `{other}` is not http or https"),
                });
            }
        }

        let http = Client::builder()
            // GitHub's API does not redirect tool calls in normal operation, and following a
            // redirect would let a response move the request (and its Authorization header) to a
            // host the manifest did not declare. A renamed repository is reported as a status
            // error the caller can act on instead.
            .redirect(reqwest::redirect::Policy::none())
            // The sandbox clears the environment and the manifest names the only egress host. A
            // system proxy would move the request somewhere the manifest did not declare, and a
            // loopback test must not be routed through a developer's proxy either.
            .no_proxy()
            .timeout(HTTP_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| GitHubError::Transport {
                detail: format!("the HTTP client could not be built: {error}"),
            })?;

        Ok(Self {
            http,
            api_base: url,
            token: token.filter(|value| !value.trim().is_empty()),
        })
    }

    /// A client that reads `GITHUB_TOKEN` from the environment.
    ///
    /// An absent or blank variable means an unauthenticated client: public repositories still work,
    /// with GitHub's lower anonymous rate limit. The value is never logged.
    ///
    /// # Errors
    ///
    /// As [`GitHubClient::new`].
    pub fn from_environment(api_base: Option<&str>) -> Result<Self, GitHubError> {
        let token = std::env::var("GITHUB_TOKEN")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        Self::new(api_base, token)
    }

    /// The API base, for diagnostics. It never contains a credential.
    #[must_use]
    pub fn api_base(&self) -> &Url {
        &self.api_base
    }

    /// Whether the client will send a token.
    #[must_use]
    pub const fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    /// GET one path, returning the decoded JSON and the pagination state when requested.
    ///
    /// `pagination` is `Some((page, per_page))` for a list endpoint. The caller supplies the values
    /// it asked for; the `Link` header supplies `next` and `last`.
    ///
    /// # Errors
    ///
    /// Every failure in [`GitHubError`]: a transport failure, a classified HTTP status, an exhausted
    /// rate limit, an unusable `Link` header, an oversized body, or a body that is not JSON.
    pub fn get_json<S: AsRef<str>>(
        &self,
        path: &[S],
        query: &[(&str, String)],
        pagination: Option<(u32, u32)>,
    ) -> Result<(Value, Option<PageInfo>), GitHubError> {
        let mut url = self.api_base.clone();
        {
            let mut segments = url.path_segments_mut().map_err(|()| GitHubError::ApiBase {
                detail: "the base URL cannot have path segments appended".to_owned(),
            })?;
            segments.pop_if_empty();
            for segment in path {
                segments.push(segment.as_ref());
            }
        }
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }

        let mut request = self
            .http
            .get(url)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }

        let response = request.send().map_err(|error| GitHubError::Transport {
            detail: error.to_string(),
        })?;
        let status = response.status();
        let headers = response.headers().clone();

        let mut body = Vec::new();
        response
            .take((MAX_RESPONSE_BYTES as u64) + 1)
            .read_to_end(&mut body)
            .map_err(|error| GitHubError::Transport {
                detail: format!("the response body could not be read: {error}"),
            })?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(GitHubError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }

        let value: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).map_err(|error| GitHubError::Decode {
                detail: error.to_string(),
            })?
        };

        if !status.is_success() {
            let resource = path.iter().map(AsRef::as_ref).collect::<Vec<_>>().join("/");
            return Err(status_error(status.as_u16(), &headers, &value, &resource));
        }

        let page = match pagination {
            Some((page, per_page)) => Some(parse_page_info(&headers, page, per_page)?),
            None => None,
        };
        Ok((value, page))
    }
}

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host == "127.0.0.1"
            || host == "::1"
            || host == "[::1]"
    })
}

/// Classify a non-success status into the typed error a caller can act on.
fn status_error(status: u16, headers: &HeaderMap, body: &Value, resource: &str) -> GitHubError {
    let detail = body.get("message").and_then(Value::as_str).map_or_else(
        || format!("GitHub answered HTTP {status} without a message"),
        |message| bounded(message, 500),
    );

    let limit = header_u64(headers, "x-ratelimit-limit");
    let remaining = header_u64(headers, "x-ratelimit-remaining");
    let reset_unix = header_u64(headers, "x-ratelimit-reset");
    let retry_after_seconds = header_u64(headers, "retry-after")
        .or_else(|| reset_unix.map(|reset| reset.saturating_sub(now_unix())));
    let lower = detail.to_ascii_lowercase();
    let rate_limited = status == 429
        || remaining == Some(0)
        || header_u64(headers, "retry-after").is_some()
        || lower.contains("rate limit");

    if rate_limited {
        let mut parts = vec!["GitHub returned a rate-limit response".to_owned()];
        if let Some(value) = limit {
            parts.push(format!("the limit is {value} requests"));
        }
        if let Some(value) = remaining {
            parts.push(format!("{value} remain"));
        }
        if let Some(value) = reset_unix {
            parts.push(format!("the reset is at Unix time {value}"));
        }
        if let Some(value) = retry_after_seconds {
            parts.push(format!("retry after {value} seconds"));
        }
        return GitHubError::RateLimited {
            detail: parts.join("; "),
            retry_after_seconds,
            reset_unix,
            limit,
            remaining,
        };
    }

    match status {
        401 => GitHubError::Unauthorized { detail },
        403 => GitHubError::Forbidden { detail },
        404 => GitHubError::NotFound {
            resource: resource.to_owned(),
        },
        410 => GitHubError::Gone {
            resource: resource.to_owned(),
        },
        422 => GitHubError::Validation { detail },
        other => GitHubError::HttpStatus {
            status: other,
            detail,
        },
    }
}

/// Parse GitHub's `Link` header into the page numbers a caller can use.
///
/// A missing header means the response is the only page. A header that is present but malformed is
/// an error rather than "no next page": silently ending pagination would hide data, and guessing a
/// page number would hide the guess.
fn parse_page_info(headers: &HeaderMap, page: u32, per_page: u32) -> Result<PageInfo, GitHubError> {
    let mut next = None;
    let mut last = None;
    let mut saw_link = false;

    for value in headers.get_all(LINK) {
        saw_link = true;
        let text = value.to_str().map_err(|_| GitHubError::Pagination {
            detail: "the Link header is not ASCII".to_owned(),
        })?;
        for part in text.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((target, params)) = part.split_once(';') else {
                return Err(GitHubError::Pagination {
                    detail: format!("the Link header segment `{part}` has no parameters"),
                });
            };
            let target = target.trim();
            let Some(target) = target
                .strip_prefix('<')
                .and_then(|target| target.strip_suffix('>'))
            else {
                return Err(GitHubError::Pagination {
                    detail: format!(
                        "the Link header target `{target}` is not enclosed in angle brackets"
                    ),
                });
            };
            let rel = params.split(';').find_map(|param| {
                let (name, value) = param.trim().split_once('=')?;
                if name.trim().eq_ignore_ascii_case("rel") {
                    Some(value.trim().trim_matches('"').to_owned())
                } else {
                    None
                }
            });
            match rel.as_deref() {
                Some("next") => next = Some(link_page(target)?),
                Some("last") => last = Some(link_page(target)?),
                _ => {}
            }
        }
    }

    if !saw_link {
        return Ok(PageInfo {
            page,
            per_page,
            next: None,
            last: None,
        });
    }

    if let Some(last) = last {
        if page > last {
            return Err(GitHubError::Pagination {
                detail: format!(
                    "page {page} was requested but GitHub reports the last page is {last}; use \
                     page <= {last}"
                ),
            });
        }
    }
    if let Some(next) = next {
        if next <= page {
            return Err(GitHubError::Pagination {
                detail: format!(
                    "the next link points to page {next}, which does not advance past page {page}"
                ),
            });
        }
    }

    Ok(PageInfo {
        page,
        per_page,
        next,
        last,
    })
}

fn link_page(target: &str) -> Result<u32, GitHubError> {
    let url = Url::parse(target).map_err(|error| GitHubError::Pagination {
        detail: format!("the Link target `{target}` is not a URL: {error}"),
    })?;
    let mut page = None;
    for (name, value) in url.query_pairs() {
        if name == "page" {
            let parsed = value.parse::<u32>().map_err(|_| GitHubError::Pagination {
                detail: format!("the Link target's page `{value}` is not a positive integer"),
            })?;
            page = Some(parsed);
        }
    }
    match page {
        Some(0) | None => Err(GitHubError::Pagination {
            detail: format!("the Link target `{target}` has no usable page parameter"),
        }),
        Some(page) => Ok(page),
    }
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Keep a remote message readable without letting it carry control characters into a prompt.
fn bounded(text: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_bytes));
    for character in text.chars() {
        if out.len() >= max_bytes {
            break;
        }
        out.push(if character.is_control() {
            ' '
        } else {
            character
        });
    }
    out
}
