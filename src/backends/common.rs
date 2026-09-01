/*!
Configuration shared by every backend's `Update` builder.

Each backend (`github`, `gitlab`, `gitea`, `s3`, `custom`) layers a small amount of
backend-specific configuration (repo coordinates, host/url, bucket, credentials, a release
source) on top of an identical set of common update options (target, bin name, version,
progress style, …).

[`CommonBuilderConfig`] holds those common options while a backend builder is being
configured; [`CommonBuilderConfig::build`] validates them and produces a resolved
[`CommonConfig`] that each backend's `Update` embeds. The shared builder *setters* are
emitted into each backend builder by the `impl_common_builder_setters!` macro, and the
shared [`UpdateConfig`](crate::UpdateConfig) *accessors* are emitted as a full `impl` block for
each backend's `Update` by the `impl_update_config_accessors!` macro (both in `src/macros.rs`), so
the common surface lives in exactly one place.
*/

use std::path::PathBuf;
use std::time::Duration;

use crate::errors::*;
use crate::get_target;
use crate::http_client::HeaderMap;
use crate::http_client::header;

/// The HTTP authorization scheme a backend uses to present its auth token.
///
/// The token is rendered into the `Authorization` header as `"<scheme> <token>"`: `token <token>`
/// for [`Token`](AuthScheme::Token) (github/gitea) and `Bearer <token>` for
/// [`Bearer`](AuthScheme::Bearer) (gitlab). The scheme is a per-backend default carried in
/// [`RequestConfig`]; it is applied by the shared header-derivation
/// ([`RequestConfig::apply_auth`]) on both the listing and the download paths, and is overridden
/// when the user sets their own `Authorization` via `request_header`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AuthScheme {
    /// `Authorization: token <token>` (github, gitea).
    #[default]
    Token,
    /// `Authorization: Bearer <token>` (gitlab). Only constructed when the `gitlab` backend is
    /// enabled; the `allow(dead_code)` keeps it from warning in builds without that backend.
    #[cfg_attr(not(feature = "gitlab"), allow(dead_code))]
    Bearer,
}

impl AuthScheme {
    /// The header-value prefix this scheme renders before the token (`"token"` / `"Bearer"`).
    fn prefix(self) -> &'static str {
        match self {
            AuthScheme::Token => "token",
            AuthScheme::Bearer => "Bearer",
        }
    }
}

/// The boxed inner error of an [`Error::SemVer`] produced from a server-supplied release tag:
/// names the offending tag in its message and keeps the original `semver` parse failure
/// reachable via [`std::error::Error::source`].
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
#[derive(Debug)]
pub(crate) struct NonSemverTagError {
    tag: String,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl std::fmt::Display for NonSemverTagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "release tag `{}` is not a semver version: {}",
            self.tag, self.source
        )
    }
}

impl std::error::Error for NonSemverTagError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

/// Rewrap a semver-validation failure from `Release::builder().build()` so the error names the
/// offending release tag (`nightly`, `latest`, a date, ...) instead of surfacing a bare parse
/// failure with no context. The original parse error stays on the `source()` chain. Non-`SemVer`
/// errors pass through unchanged.
///
/// Only the forge backends (github/gitlab/gitea) funnel server-supplied tags through the builder;
/// the attribute keeps builds without any of them warning-free.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn name_tag_in_semver_error(tag: &str, err: Error) -> Error {
    match err {
        Error::SemVer(inner) => Error::SemVer(Box::new(NonSemverTagError {
            tag: tag.to_owned(),
            source: inner,
        })),
        other => other,
    }
}

/// Strip the configured tag prefix (or the conventional leading `v`) from a release tag to get the
/// bare version candidate.
///
/// With `prefix = None`, a single leading lowercase `v` is trimmed (the long-standing default) and
/// the result is always `Some`. With `prefix = Some(p)`, the tag must start with `p`; `p` is
/// stripped and a leading `v` after it is also trimmed (so `myapp-v1.2.3` and `myapp-1.2.3` both
/// yield `Some("1.2.3")`). A tag that does not start with `p` yields `None`, so the caller skips it:
/// with a prefix configured, only tags carrying it count as releases (a bare `1.0.0` tag is not
/// silently accepted just because its remainder parses as semver).
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn strip_tag_prefix(tag: &str, prefix: Option<&str>) -> Option<String> {
    match prefix {
        None => Some(tag.trim_start_matches('v').to_owned()),
        Some(p) => tag
            .strip_prefix(p)
            .map(|rest| rest.trim_start_matches('v').to_owned()),
    }
}

/// Build the skippable [`Error::SemVer`] returned when a tag does not carry the configured
/// `tag_prefix`. It uses the `SemVer` variant so the forge listing walk drops the release (its skip
/// arm keys on `Error::SemVer`), the same way it drops a non-semver tag.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn tag_prefix_mismatch_error(tag: &str, prefix: &str) -> Error {
    Error::SemVer(Box::new(crate::errors::MessageError(format!(
        "release tag `{tag}` does not start with the configured tag_prefix `{prefix}`"
    ))))
}

/// Pick the auth token out of a list of candidate `(env var name, value)` pairs: the first pair
/// whose value is present and non-empty after trimming surrounding whitespace wins.
///
/// Split out from [`token_from_env`] so the precedence rules are testable without mutating process
/// env, which is racy under the parallel test harness (and `unsafe` since the 2024 edition).
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn first_env_token(candidates: &[(&str, Option<String>)]) -> Option<String> {
    for (name, value) in candidates {
        let Some(value) = value else { continue };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        log::debug!("self_update: using the auth token from ${name}");
        return Some(value.to_owned());
    }
    None
}

/// Convert one raw environment value into a candidate token string.
///
/// A value that is not valid UTF-8 cannot be an HTTP header value, so it is reported and treated as
/// unset rather than silently ignored (which is what `std::env::var(..).ok()` would do, making a
/// mangled `GITHUB_TOKEN` indistinguishable from an absent one). Pure -- it takes the raw value, so
/// the non-UTF-8 path is testable without mutating process env.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn env_token_value(name: &str, raw: Option<std::ffi::OsString>) -> Option<String> {
    match raw?.into_string() {
        Ok(value) => Some(value),
        Err(_) => {
            log::warn!(
                "self_update: ignoring ${name}: its value is not valid UTF-8, so it cannot be used \
                 as an auth token. The request proceeds as if it were unset."
            );
            None
        }
    }
}

/// Read `names` from the environment in order and return the first present, non-empty value.
/// `None` when none is set, which leaves the builder's `auth_token` untouched.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn token_from_env(names: &[&str]) -> Option<String> {
    let candidates = names
        .iter()
        .map(|name| (*name, env_token_value(name, std::env::var_os(name))))
        .collect::<Vec<_>>();
    first_env_token(&candidates)
}

/// Whether an optional auth-token slot should be treated as unset: `None`, or `Some` holding only
/// whitespace.
///
/// A blank *explicit* token (`auth_token("")`, or the common
/// `auth_token(cfg.token.unwrap_or_default())` pattern applied to a missing config value) must not
/// block the environment fallback ([`fill_env_token_if_unset`]) and must not produce a literal
/// `Authorization: token ` header ([`RequestConfig::apply_auth`]); `has_auth_token()` answers
/// `false` for it too, matching "is a token configured?" rather than "is the slot `Some`?".
///
/// This is a distinct rule from [`first_env_token`]'s trimming: an *explicit* `auth_token(..)`
/// value is never trimmed or otherwise modified here, so a token merely surrounded by whitespace
/// still surfaces as [`Error::InvalidAuthToken`](crate::errors::Error::InvalidAuthToken) at request
/// time instead of being silently repaired (see the crate docs' trim-asymmetry note).
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn is_blank_token(token: Option<&str>) -> bool {
    token.is_none_or(|t| t.trim().is_empty())
}

/// Fill a builder's token slot from a lazily-resolved token, **only when the slot is blank**
/// (unset, or holding only whitespace -- see [`is_blank_token`]).
///
/// The environment is a *fallback*, never an override: an explicit `auth_token(..)` always wins,
/// whatever the call order, so the setter pair is order-independent like every other pair on these
/// builders, and an ambient developer/CI credential can never displace the credential the
/// application deliberately provisioned. Not finding one likewise leaves the slot alone, so
/// `auth_token_from_env()` can never silently drop a token (which would turn a missing env var into
/// a surprise 403).
///
/// `resolve` is only called when the slot is actually blank: with an explicit, non-blank token
/// already in place there is nothing to fall back to, so the environment is never even read. That
/// matters beyond an unnecessary syscall -- `token_from_env` logs which variable it used
/// (`log::debug!`) and warns about an unusable non-UTF-8 one, and neither diagnostic should fire for
/// a lookup whose result is discarded.
///
/// Returns `true` when the slot was filled from the environment, which is what the generated setter
/// records in its `auth_token_from_env` flag.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn fill_env_token_if_unset_with(
    slot: &mut Option<String>,
    resolve: impl FnOnce() -> Option<String>,
) -> bool {
    if !is_blank_token(slot.as_deref()) {
        return false;
    }
    match resolve() {
        Some(token) => {
            *slot = Some(token);
            true
        }
        None => false,
    }
}

/// Thin wrapper over [`fill_env_token_if_unset_with`] that takes an already-resolved value instead
/// of a closure, kept so the existing pure-value unit tests stand unchanged. The generated
/// `auth_token_from_env()` setter calls [`fill_env_token_if_unset_with`] directly (with a closure,
/// per A6), so outside of tests this wrapper is genuinely unused.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn fill_env_token_if_unset(slot: &mut Option<String>, resolved: Option<String>) -> bool {
    fill_env_token_if_unset_with(slot, || resolved)
}

/// The lowercased host of a URL, for auth-origin comparison. Parses with `http::Uri` (always
/// available, no `url` crate needed). Returns `None` when the URL has no host.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn host_of(url: &str) -> Option<String> {
    url.parse::<http::Uri>().ok()?.host().map(|h| {
        h.trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase()
    })
}

/// What [`env_token_host_decision`] decided to do with an env-sourced auth token bound to a host
/// the application did not necessarily type in as `auth_token(..)`.
///
/// Three states rather than a `bool` because the outcome for an unacknowledged host now differs by
/// backend (DECIDED, see `local/review/plan.md` "Cross-shard decisions 1"): github/gitlab/gitee
/// keep sending the token (with a warning); gitea, which has no canonical host to compare against,
/// withholds it instead. Returning an enum keeps the rule unit-testable without capturing logs, the
/// same way the old `-> bool` was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) enum EnvTokenDecision {
    /// The token is attached with no warning: not env-sourced, no host to compare against, or the
    /// host is acknowledged (the backend's canonical host, or an `allow_auth_host` entry).
    Sent,
    /// Env-sourced, the host is neither canonical nor acknowledged, and the backend HAS a canonical
    /// host to compare against (github/gitlab/gitee): the token is still attached, but a warning is
    /// logged. This is today's behavior, unchanged (DECIDED, A1).
    WarnedAndSent,
    /// Env-sourced, the host is not acknowledged, and the backend has NO canonical host (gitea): the
    /// token is withheld. The caller is responsible for actually clearing the request's auth token
    /// on this outcome; this function only decides and logs.
    Withheld,
}

/// Whether an env-sourced auth token bound to `host` is acknowledged, i.e. the application has, in
/// some form, said "yes, send it here": either `host` is the backend's `canonical_host`, or it is
/// one of the `auth_hosts` the application explicitly added via `allow_auth_host` (A2).
///
/// An `allow_auth_host` entry is itself the user's explicit "send the token to this host" -- the
/// same declaration an explicit `auth_token(..)` call makes about the token itself -- so once a host
/// is in that set it never triggers the environment-origin warning, exactly like the canonical host.
/// This is deliberately the *same* acknowledgement for both branches below: a canonical-host backend
/// stops warning about a host it was told to trust, and a canonical-host-less backend (gitea) starts
/// sending to one instead of withholding.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
fn host_is_acknowledged(host: &str, auth_hosts: &[String], canonical_host: Option<&str>) -> bool {
    canonical_host.is_some_and(|canonical| canonical.eq_ignore_ascii_case(host))
        || auth_hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
}

/// Decide what happens to a token taken from the environment when it is bound to a host that is not
/// the backend's canonical one, per [`EnvTokenDecision`]. This is the single acknowledgement rule
/// backing both A1 (gitea withholds) and A2 (an `allow_auth_host` entry silences the warning).
///
/// The env var list is tied to the backend *type* (`GITHUB_TOKEN` for github, ...), but the token is
/// sent to whatever host the application configured via `api_base_url` / `host`. The request-time
/// host gate cannot help here, because the configured host *is* `auth_base_host`. So an application
/// that exposes its update URL as configuration and runs in CI would hand `GITHUB_TOKEN` to an
/// attacker-chosen host with no signal at all. An explicitly-set token is the application's own
/// decision and is never flagged; only the env-sourced case is ([`EnvTokenDecision::Sent`] when
/// `env_sourced` is `false`).
///
/// `canonical_host` is `None` for a backend that has no canonical host of its own (gitea is always
/// self-hosted): with no host to compare against, an unacknowledged env-sourced token is withheld
/// rather than silently bound to whatever host happens to be configured (DECIDED, A1) -- a hard
/// `build()` failure would make the failure mode worse than the anonymous request it replaces, so
/// the token is dropped and `build()` still succeeds.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn env_token_host_decision(
    env_sourced: bool,
    auth_base_host: Option<&str>,
    auth_hosts: &[String],
    canonical_host: Option<&str>,
) -> EnvTokenDecision {
    if !env_sourced {
        return EnvTokenDecision::Sent;
    }
    // No parseable host means the request-time gate (`auth_allowed_for`) will not attach the token
    // to anything anyway -- fail-closed, so there is nothing to warn about or withhold.
    let Some(host) = auth_base_host else {
        return EnvTokenDecision::Sent;
    };
    if host_is_acknowledged(host, auth_hosts, canonical_host) {
        return EnvTokenDecision::Sent;
    }
    match canonical_host {
        Some(canonical) => {
            log::warn!(
                "self_update: the auth token resolved from the environment will be sent to `{host}`, \
                 which is not `{canonical}`. The environment variables are conventions of the \
                 backend's own service, so a token meant for `{canonical}` may be exposed to a \
                 different host. Set the token explicitly with auth_token(..), or acknowledge this \
                 host with allow_auth_host(..), if it is intended."
            );
            EnvTokenDecision::WarnedAndSent
        }
        None => {
            log::warn!(
                "self_update: withholding the auth token resolved from the environment: `{host}` was \
                 not explicitly acknowledged. This backend has no canonical host to compare an \
                 env-sourced token against, so -- rather than silently binding an ambient credential \
                 to whatever host the application happens to be pointed at -- the token is not \
                 attached and the request proceeds anonymously. Set the token explicitly with \
                 auth_token(..), or acknowledge this host with allow_auth_host(..), to send it."
            );
            EnvTokenDecision::Withheld
        }
    }
}

/// Set an explicit auth token, clearing the paired `auth_token_from_env` flag.
///
/// An explicit `auth_token(..)` is the application's own credential and always wins over the
/// environment, whatever the call order, so it is never treated as env-sourced. Shared by the
/// macro-generated `UpdateBuilder::auth_token` and all four hand-written `ReleaseListBuilder::auth_token`
/// setters, so a backend that forgets this reset cannot happen by omission: skipping it would not
/// break "explicit wins" (that is [`fill_env_token_if_unset`]'s job), but would produce a spurious
/// environment-origin warning -- or, after A1, a wrongly *withheld* token on a canonical-host-less
/// backend -- on every `build()`.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(dead_code)
)]
pub(crate) fn set_explicit_auth_token(
    slot: &mut Option<String>,
    env_sourced: &mut bool,
    value: impl Into<String>,
) {
    *slot = Some(value.into());
    *env_sourced = false;
}

#[cfg(feature = "progress-bar")]
use crate::{DEFAULT_PROGRESS_CHARS, DEFAULT_PROGRESS_TEMPLATE};

/// Per-request transport options shared by all of a backend's HTTP requests.
///
/// `headers` are extra headers merged into every request (on top of the backend's own auth /
/// user-agent headers); `timeout` bounds each request.
#[derive(Clone)]
pub(crate) struct RequestConfig {
    pub(crate) timeout: Option<Duration>,
    pub(crate) headers: HeaderMap,
    /// Number of times to retry a failed API request (with exponential backoff).
    pub(crate) retries: u32,
    /// Base delay (attempt 0) for the exponential retry backoff. The delay doubles each attempt up
    /// to [`retry_max_delay`](Self::retry_max_delay). Defaults to 100ms.
    pub(crate) retry_base_delay: Duration,
    /// Cap on the exponential retry backoff delay. Defaults to ~3.2s (100ms << 5).
    pub(crate) retry_max_delay: Duration,
    /// The backend's authorization scheme for rendering [`auth_token`](Self::auth_token) into the
    /// `Authorization` header. Per-backend default (github/gitea `Token`, gitlab `Bearer`).
    pub(crate) auth_scheme: AuthScheme,
    /// The backend auth token, if any, rendered via [`auth_scheme`](Self::auth_scheme). A user
    /// `request_header(AUTHORIZATION, ..)` override in [`headers`](Self::headers) takes precedence.
    pub(crate) auth_token: Option<String>,
    /// Optional user-supplied HTTP client to use through the [`HttpClient`](crate::http_client::HttpClient)
    /// trait instead of the per-call one the crate builds. `Arc`-backed so cloning a `RequestConfig`
    /// shares the client (and its connection pool).
    pub(crate) client: Option<std::sync::Arc<dyn crate::http_client::HttpClient>>,
    /// Optional user-supplied async HTTP client, mirroring [`client`](Self::client) for the async
    /// path. Async is reqwest-only.
    #[cfg(feature = "async")]
    pub(crate) async_client: Option<std::sync::Arc<dyn crate::http_client::AsyncHttpClient>>,
    /// First error produced converting a `request_header(name, value)` argument that wasn't a
    /// valid HTTP header. Stored here so the builder setter can stay infallible (`-> &mut Self`)
    /// and the failure is surfaced from `build()` as an `Error::InvalidHeader` instead of panicking.
    pub(crate) header_error: Option<String>,
    /// Custom TLS root CA certificates to bake into the HTTP client the crate builds when no client
    /// was injected. Materialized into [`client`](Self::client) (and [`async_client`](Self::async_client))
    /// by [`build_client`](Self::build_client) at `build()` time.
    pub(crate) root_certificates: Vec<crate::tls::Certificate>,
    /// First error produced materializing a client from [`root_certificates`](Self::root_certificates)
    /// (invalid cert bytes or a client-build failure). Deferred like [`header_error`](Self::header_error)
    /// and surfaced from [`check`](Self::check) as an `Error::InvalidCertificate`.
    pub(crate) cert_error: Option<String>,
    /// Programmatic proxy URL (`http://user:pass@host:port`) the crate-built HTTP client routes
    /// through, set via `proxy`. Baked into [`client`](Self::client) (and
    /// [`async_client`](Self::async_client)) by [`build_client`](Self::build_client) at `build()`
    /// time, alongside [`root_certificates`](Self::root_certificates). `None` leaves proxying to
    /// the client's own `HTTP(S)_PROXY` env support.
    pub(crate) proxy: Option<String>,
    /// First error produced materializing a client from [`proxy`](Self::proxy) (an unparseable
    /// proxy URL). Deferred like [`cert_error`](Self::cert_error) and surfaced from
    /// [`check`](Self::check) as an `Error::InvalidProxy`, with any embedded password redacted.
    pub(crate) proxy_error: Option<String>,
    /// The host of the backend's configured API base (e.g. `api.github.com`, `gitlab.com`, the
    /// gitea host). The derived [`auth_token`](Self::auth_token) is only attached to a request whose
    /// host matches this (or an [`auth_hosts`](Self::auth_hosts) entry), so a server-supplied asset
    /// `download_url` or `Link` next-page URL pointing at a different host does not receive the
    /// token. Set by each backend at `build()` time; `None` disables the token entirely.
    pub(crate) auth_base_host: Option<String>,
    /// Additional hosts the user has explicitly authorized to receive the auth token, via
    /// `allow_auth_host`. Checked alongside [`auth_base_host`](Self::auth_base_host).
    pub(crate) auth_hosts: Vec<String>,
    /// When `true`, the auth token may be attached over plain `http` (not just `https`) to a
    /// host-matched request. Off by default; set via `dangerously_allow_non_https_auth_forwarding`.
    pub(crate) allow_insecure_auth: bool,
}

/// Default base delay for the exponential retry backoff (attempt 0).
pub(crate) const DEFAULT_RETRY_BASE_DELAY: Duration = Duration::from_millis(100);
/// Default cap on the exponential retry backoff (100ms << 5 == 3200ms).
pub(crate) const DEFAULT_RETRY_MAX_DELAY: Duration = Duration::from_millis(3200);

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            timeout: None,
            headers: HeaderMap::new(),
            retries: 0,
            retry_base_delay: DEFAULT_RETRY_BASE_DELAY,
            retry_max_delay: DEFAULT_RETRY_MAX_DELAY,
            auth_scheme: AuthScheme::default(),
            auth_token: None,
            client: None,
            #[cfg(feature = "async")]
            async_client: None,
            header_error: None,
            root_certificates: Vec::new(),
            cert_error: None,
            proxy: None,
            proxy_error: None,
            auth_base_host: None,
            auth_hosts: Vec::new(),
            allow_insecure_auth: false,
        }
    }
}

impl std::fmt::Debug for RequestConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Exhaustive, no `..`: a field added to the struct and not listed here is a compile error,
        // rather than one that silently stops rendering (see `debug_renders_every_field`'s comment
        // for what the *runtime* test can and cannot catch).
        let Self {
            timeout,
            headers,
            retries,
            retry_base_delay,
            retry_max_delay,
            auth_scheme,
            auth_token,
            client,
            #[cfg(feature = "async")]
            async_client,
            header_error,
            root_certificates,
            cert_error,
            proxy,
            proxy_error,
            auth_base_host,
            auth_hosts,
            allow_insecure_auth,
        } = self;
        let mut s = f.debug_struct("RequestConfig");
        s.field("timeout", timeout)
            .field("headers", headers)
            .field("retries", retries)
            .field("retry_base_delay", retry_base_delay)
            .field("retry_max_delay", retry_max_delay)
            .field("auth_scheme", auth_scheme)
            .field("auth_token", &auth_token.as_ref().map(|_| "<token>"))
            .field("client", &client.as_ref().map(|_| "<http_client>"));
        #[cfg(feature = "async")]
        s.field(
            "async_client",
            &async_client.as_ref().map(|_| "<async_http_client>"),
        );
        s.field("header_error", header_error)
            .field(
                "root_certificates",
                &format_args!("<{} root_certificates>", root_certificates.len()),
            )
            .field("cert_error", cert_error)
            // A proxy URL may embed credentials (`http://user:pass@host`), so the password is
            // redacted here -- a `log::debug!("{builder:?}")` must not print it. The rest of the
            // URL is kept so the dump can still answer "which proxy is this going through?".
            .field(
                "proxy",
                &proxy.as_deref().map(crate::errors::redact_proxy_url),
            )
            .field("proxy_error", proxy_error)
            // The auth-host gate carries no secret (a hostname), and it decides whether the token is
            // attached at all -- including which host the env-token warning compares against -- so a
            // debug dump that hides it cannot answer "why was my token not sent?".
            .field("auth_base_host", auth_base_host)
            .field("auth_hosts", auth_hosts)
            .field("allow_insecure_auth", allow_insecure_auth)
            .finish()
    }
}

/// Whether an HTTP header name carries a credential and so must be redacted (`set_sensitive`)
/// wherever its value is inserted: the derived `Authorization` header
/// ([`RequestConfig::apply_auth`]) and any user-supplied header of the same shape
/// ([`RequestConfig::insert_header`]).
///
/// `name` is expected to already be lowercase, as [`header::HeaderName::as_str`] always returns it.
/// Covers `authorization` and gitlab's `private-token` (the header its docs use for a token outside
/// the `Authorization` scheme) by exact match, `cookie` by exact match, and any name ending in
/// `-token` so a custom gateway header (`X-Api-Token`, `X-Upstream-Token`, ...) is covered without
/// enumerating every vendor's spelling.
fn header_name_is_credential_bearing(name: &str) -> bool {
    matches!(name, "authorization" | "private-token" | "cookie") || name.ends_with("-token")
}

impl RequestConfig {
    /// Insert an extra request header from `TryInto<HeaderName>` / `TryInto<HeaderValue>` args. A
    /// conversion failure is recorded in [`header_error`](Self::header_error) (first one wins) and
    /// surfaced later by [`check`](Self::check); the header is simply not inserted.
    ///
    /// A credential-bearing header name (`authorization`, `private-token`, `cookie`, or any
    /// `*-token` name) is marked [`set_sensitive`](header::HeaderValue::set_sensitive), the same
    /// treatment [`apply_auth`](Self::apply_auth) gives the derived `Authorization` header: it keeps
    /// the value out of the transports' own header logging and renders it as `Sensitive` in any
    /// `Debug`, including the `Debug` this struct and every backend builder inherit. Without this, a
    /// user-supplied `request_header("Authorization", ..)` -- which [`apply_auth`] gives
    /// *precedence* over the backend's own token -- would print verbatim.
    pub(crate) fn insert_header<N, V>(&mut self, name: N, value: V)
    where
        N: ::core::convert::TryInto<crate::http_client::header::HeaderName>,
        V: ::core::convert::TryInto<crate::http_client::header::HeaderValue>,
    {
        let name = match name.try_into() {
            Ok(n) => n,
            Err(_) => {
                if self.header_error.is_none() {
                    self.header_error =
                        Some("invalid HTTP header name passed to `request_header`".to_string());
                }
                return;
            }
        };
        let mut value = match value.try_into() {
            Ok(v) => v,
            Err(_) => {
                if self.header_error.is_none() {
                    self.header_error =
                        Some("invalid HTTP header value passed to `request_header`".to_string());
                }
                return;
            }
        };
        if header_name_is_credential_bearing(name.as_str()) {
            value.set_sensitive(true);
        }
        self.headers.insert(name, value);
    }

    /// Apply this config's derived authorization to `headers`, honoring a user override.
    ///
    /// This is the single header-derivation used by **both** the listing path
    /// ([`send`](crate::backends::send) / `send_async`) and the download path
    /// ([`build_download`](crate::update)). Precedence:
    ///
    /// 1. If the user supplied their own `Authorization` via `request_header` (present in
    ///    [`headers`](Self::headers)), it wins and the backend scheme/token are not applied.
    /// 2. Otherwise, if an [`auth_token`](Self::auth_token) is set, it is rendered as
    ///    `"<scheme> <token>"` per [`auth_scheme`](Self::auth_scheme) and inserted.
    /// 3. Otherwise nothing is inserted.
    ///
    /// A token that does not encode as a header value surfaces as
    /// [`Error::InvalidAuthToken`](crate::errors::Error::InvalidAuthToken).
    ///
    /// The token is attached only when [`auth_allowed_for`](Self::auth_allowed_for) permits it for
    /// `url` (same host as the configured API base or an `allow_auth_host` entry, over https).
    /// A server-supplied asset `download_url` or `Link` next-page URL pointing at a different host
    /// gets no token, so a malicious release server cannot harvest the credential.
    pub(crate) fn apply_auth(&self, url: &str, headers: &mut HeaderMap) -> Result<()> {
        // A user-supplied Authorization header (via `request_header`) always wins.
        if self.headers.contains_key(header::AUTHORIZATION) {
            return Ok(());
        }
        let Some(token) = self.auth_token.as_deref() else {
            return Ok(());
        };
        // A blank (empty/whitespace-only) token is treated as unset, same as an absent one: it must
        // not block a would-be env fallback upstream and must not produce a literal
        // `Authorization: token ` header (see `is_blank_token`).
        if is_blank_token(Some(token)) {
            return Ok(());
        }
        if !self.auth_allowed_for(url) {
            log::warn!(
                "self_update: not attaching the auth token to {url}: its host is not the configured \
                 API host and is not in the allow_auth_host set (or the scheme is not https). The \
                 request proceeds without authorization."
            );
            return Ok(());
        }
        let mut value = format!("{} {}", self.auth_scheme.prefix(), token)
            .parse::<header::HeaderValue>()
            .map_err(|err| Error::InvalidAuthToken {
                source: Box::new(err),
            })?;
        // Mark the value sensitive so it renders as `Sensitive` in any `Debug` (e.g. a `Download`'s)
        // and is kept out of logs by the HTTP client.
        value.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, value);
        Ok(())
    }

    /// Whether the derived auth token may be attached to a request to `url`.
    ///
    /// The host must match the configured [`auth_base_host`](Self::auth_base_host) or an
    /// [`auth_hosts`](Self::auth_hosts) entry, and the scheme must be `https` -- except for loopback
    /// hosts (`localhost`, `127.0.0.1`, `::1`), which are allowed over plain http so a local mirror
    /// and the loopback test stubs keep working.
    pub(crate) fn auth_allowed_for(&self, url: &str) -> bool {
        let uri = match url.parse::<http::Uri>() {
            Ok(u) => u,
            Err(_) => return false,
        };
        let host = match uri.host() {
            Some(h) => h
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_ascii_lowercase(),
            None => return false,
        };
        let host_matches = self
            .auth_base_host
            .as_deref()
            .is_some_and(|b| b.eq_ignore_ascii_case(&host))
            || self
                .auth_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&host));
        if !host_matches {
            return false;
        }
        let is_loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false);
        uri.scheme_str() == Some("https") || is_loopback || self.allow_insecure_auth
    }

    /// Materialize a pre-configured HTTP client from `root_certificates` and/or `proxy` if either
    /// is set and no client was injected. On success, stores the client in `self.client` (and the
    /// async sibling). On failure, records the error in `self.proxy_error` or `self.cert_error`
    /// (first error wins, mirroring `header_error`).
    ///
    /// Each client slot is materialized independently: the sync client is built from the
    /// configuration only when the sync slot is empty, and the async client only when the async
    /// slot is empty. So injecting a client for one transport does not drop the custom roots or the
    /// proxy for the other (the injected client owns its own TLS and proxy config; the auto-built
    /// one still honors both). A build failure for a slot that will actually be built is recorded.
    pub(crate) fn build_client(&mut self) {
        let config = crate::http_client::ClientConfig {
            certs: &self.root_certificates,
            proxy: self.proxy.as_deref(),
        };
        if config.is_empty() {
            return;
        }
        // A failure is attributed to the setter that caused it: a proxy-parse failure to `proxy`,
        // anything else to `add_root_certificate` -- except when no certificates were supplied at
        // all, where a generic client-build failure can only have come from the proxy config, and
        // blaming a certificate the caller never set would be actively misleading.
        let mut record = |e: crate::http_client::ClientConfigError| {
            let (slot, message) = match e {
                crate::http_client::ClientConfigError::Proxy(e) => (&mut self.proxy_error, e),
                crate::http_client::ClientConfigError::Other(e)
                    if self.root_certificates.is_empty() =>
                {
                    (&mut self.proxy_error, e)
                }
                crate::http_client::ClientConfigError::Other(e) => (&mut self.cert_error, e),
            };
            if slot.is_none() {
                *slot = Some(message.to_string());
            }
        };
        if self.client.is_none() {
            match crate::http_client::build_configured_client(config) {
                Ok(c) => self.client = Some(c),
                Err(e) => record(e),
            }
        }
        #[cfg(feature = "async")]
        if self.async_client.is_none() {
            match crate::http_client::build_configured_async_client(config) {
                Ok(c) => self.async_client = Some(c),
                Err(e) => record(e),
            }
        }
    }

    /// Surface any deferred config error: a `request_header` conversion failure as
    /// `Error::InvalidHeader` (checked first, so it takes precedence), then a root-certificate /
    /// client-build failure as `Error::InvalidCertificate`, then a proxy failure as
    /// `Error::InvalidProxy`.
    pub(crate) fn check(&self) -> Result<()> {
        if let Some(msg) = &self.header_error {
            return Err(Error::InvalidHeader {
                source: Box::new(crate::errors::MessageError(msg.clone())),
            });
        }
        if let Some(msg) = &self.cert_error {
            return Err(Error::InvalidCertificate {
                source: Box::new(crate::errors::MessageError(msg.clone())),
            });
        }
        if let Some(msg) = &self.proxy_error {
            return Err(Error::InvalidProxy {
                source: Box::new(crate::errors::MessageError(msg.clone())),
            });
        }
        Ok(())
    }
}

/// The common, backend-independent options of an `Update` builder, before validation.
///
/// `Debug` is hand-written (not derived) so [`auth_token`](Self::auth_token) renders as `"<token>"`:
/// every backend's `UpdateBuilder` derives `Debug` over this struct, so a plain
/// `log::debug!("{builder:?}")` would otherwise print a live credential -- one the application
/// author may not even have typed, since `auth_token_from_env()` can put an ambient CI token there.
#[derive(Clone)]
pub(crate) struct CommonBuilderConfig {
    pub request: RequestConfig,
    pub target: Option<String>,
    pub asset_identifier: Option<String>,
    pub bin_name: Option<String>,
    pub bin_install_path: Option<PathBuf>,
    /// Opt-in preflight: probe `bin_install_path` writability before any download, failing early
    /// with [`Error::InstallPathNotWritable`] on a definite permission refusal. Default `false`;
    /// set via `check_install_path_writable(true)`.
    pub check_install_path_writable: bool,
    pub bin_path_in_archive: Option<String>,
    /// `true` when `bin_path_in_archive` was auto-derived from `bin_name` (not set explicitly by
    /// the user). Used by `bin_name` to re-derive when called again, while leaving an explicitly
    /// set value untouched.
    pub(crate) bin_path_in_archive_auto: bool,
    /// The bundle directory inside the archive, relative to the archive root (e.g. `MyApp.app`).
    /// `Some` selects bundle mode: the whole directory replaces `bundle_install_path` instead of
    /// one file replacing `bin_install_path`. Set via `bundle_path_in_archive`.
    pub bundle_path_in_archive: Option<String>,
    /// The installed bundle directory to replace in bundle mode. Defaults on macOS to the nearest
    /// `.app` ancestor of the running executable; required on every other platform. Set via
    /// `bundle_install_path`.
    pub bundle_install_path: Option<PathBuf>,
    pub show_download_progress: bool,
    pub show_output: bool,
    pub no_confirm: bool,
    pub show_release_notes: bool,
    pub update_strategy: crate::update::UpdateStrategy,
    /// Optional tag prefix used to derive the version from a release tag (e.g. `myapp-` for a
    /// monorepo tag `myapp-1.2.3`). `None` keeps the default of trimming a leading `v`. Only the
    /// forge backends (github/gitlab/gitea) consult it; set via their `tag_prefix` setter.
    pub tag_prefix: Option<String>,
    pub current_version: Option<String>,
    pub release_tag: Option<String>,
    #[cfg(feature = "progress-bar")]
    pub progress_template: String,
    #[cfg(feature = "progress-bar")]
    pub progress_chars: String,
    pub auth_token: Option<String>,
    /// `true` when [`auth_token`](Self::auth_token) was filled from the environment by the generated
    /// `auth_token_from_env()` setter. Cleared by every explicit `auth_token(..)` call (via
    /// [`set_explicit_auth_token`]). Read at `build()` time by [`env_token_host_decision`] so an
    /// ambient credential bound to an unacknowledged host is reported (or, for a backend with no
    /// canonical host, withheld).
    pub auth_token_from_env: bool,
    /// The backend's authorization scheme. Defaults to [`AuthScheme::Token`] (github/gitea); gitlab
    /// sets [`AuthScheme::Bearer`]. Threaded into the resolved [`RequestConfig::auth_scheme`].
    pub auth_scheme: AuthScheme,
    pub progress_callback: Option<crate::ProgressCallback>,
    pub verify: Option<crate::VerifyCallback>,
    /// Pre-extraction hook over the downloaded archive, set by `verify_archive(..)`. Distinct from
    /// [`verify`](Self::verify), which runs later and over the extracted binary.
    pub verify_archive: Option<crate::VerifyCallback>,
    pub asset_matcher: Option<crate::AssetMatcher>,
    #[cfg(feature = "checksums")]
    pub checksum: Option<crate::Checksum>,
    /// Name of a release asset carrying published digests (e.g. `SHA256SUMS`), set by
    /// `checksum_from_asset(..)`. Resolved into a `Checksum` during the update.
    #[cfg(feature = "checksums")]
    pub checksum_from_asset: Option<String>,
    /// Verify the download against the backend-published asset digest when one is present.
    /// On by default; `verify_release_digest(false)` opts out.
    #[cfg(feature = "checksums")]
    pub verify_release_digest: bool,
    #[cfg(feature = "signatures")]
    pub verifying_keys: Vec<[u8; zipsign_api::PUBLIC_KEY_LENGTH]>,
}

impl std::fmt::Debug for CommonBuilderConfig {
    /// Renders every field, with `auth_token` redacted to `"<token>"` exactly as
    /// [`RequestConfig::fmt`] does (`None` stays `None`, so "is a token set?" is still answerable
    /// from a debug dump without the value leaking).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Exhaustive, no `..`: a field added to the struct and not listed here is a compile error,
        // rather than one that silently stops rendering (see `debug_renders_every_field`'s comment
        // for what the *runtime* test can and cannot catch).
        let Self {
            request,
            target,
            asset_identifier,
            bin_name,
            bin_install_path,
            check_install_path_writable,
            bin_path_in_archive,
            bin_path_in_archive_auto,
            bundle_path_in_archive,
            bundle_install_path,
            show_download_progress,
            show_output,
            no_confirm,
            show_release_notes,
            update_strategy,
            tag_prefix,
            current_version,
            release_tag,
            #[cfg(feature = "progress-bar")]
            progress_template,
            #[cfg(feature = "progress-bar")]
            progress_chars,
            auth_token,
            auth_token_from_env,
            auth_scheme,
            progress_callback,
            verify,
            verify_archive,
            asset_matcher,
            #[cfg(feature = "checksums")]
            checksum,
            #[cfg(feature = "checksums")]
            checksum_from_asset,
            #[cfg(feature = "checksums")]
            verify_release_digest,
            #[cfg(feature = "signatures")]
            verifying_keys,
        } = self;
        let mut s = f.debug_struct("CommonBuilderConfig");
        s.field("request", request)
            .field("target", target)
            .field("asset_identifier", asset_identifier)
            .field("bin_name", bin_name)
            .field("bin_install_path", bin_install_path)
            .field("check_install_path_writable", check_install_path_writable)
            .field("bin_path_in_archive", bin_path_in_archive)
            .field("bin_path_in_archive_auto", bin_path_in_archive_auto)
            .field("bundle_path_in_archive", bundle_path_in_archive)
            .field("bundle_install_path", bundle_install_path)
            .field("show_download_progress", show_download_progress)
            .field("show_output", show_output)
            .field("no_confirm", no_confirm)
            .field("show_release_notes", show_release_notes)
            .field("update_strategy", update_strategy)
            .field("tag_prefix", tag_prefix)
            .field("current_version", current_version)
            .field("release_tag", release_tag);
        #[cfg(feature = "progress-bar")]
        s.field("progress_template", progress_template)
            .field("progress_chars", progress_chars);
        s.field("auth_token", &auth_token.as_ref().map(|_| "<token>"))
            .field("auth_token_from_env", auth_token_from_env)
            .field("auth_scheme", auth_scheme)
            .field("progress_callback", progress_callback)
            .field("verify", verify)
            .field("verify_archive", verify_archive)
            .field("asset_matcher", asset_matcher);
        #[cfg(feature = "checksums")]
        s.field("checksum", checksum)
            .field("checksum_from_asset", checksum_from_asset)
            .field("verify_release_digest", verify_release_digest);
        // Verifying keys are public by construction, so they render as they did under the derive.
        #[cfg(feature = "signatures")]
        s.field("verifying_keys", verifying_keys);
        s.finish()
    }
}

impl Default for CommonBuilderConfig {
    fn default() -> Self {
        Self {
            request: RequestConfig::default(),
            target: None,
            asset_identifier: None,
            bin_name: None,
            bin_install_path: None,
            check_install_path_writable: false,
            bin_path_in_archive: None,
            bin_path_in_archive_auto: false,
            bundle_path_in_archive: None,
            bundle_install_path: None,
            show_download_progress: false,
            show_output: true,
            no_confirm: false,
            show_release_notes: false,
            update_strategy: crate::update::UpdateStrategy::default(),
            tag_prefix: None,
            current_version: None,
            release_tag: None,
            #[cfg(feature = "progress-bar")]
            progress_template: DEFAULT_PROGRESS_TEMPLATE.to_string(),
            #[cfg(feature = "progress-bar")]
            progress_chars: DEFAULT_PROGRESS_CHARS.to_string(),
            auth_token: None,
            auth_token_from_env: false,
            auth_scheme: AuthScheme::default(),
            progress_callback: None,
            verify: None,
            verify_archive: None,
            asset_matcher: None,
            #[cfg(feature = "checksums")]
            checksum: None,
            #[cfg(feature = "checksums")]
            checksum_from_asset: None,
            #[cfg(feature = "checksums")]
            verify_release_digest: true,
            #[cfg(feature = "signatures")]
            verifying_keys: vec![],
        }
    }
}

impl CommonBuilderConfig {
    /// Validate the common options and resolve defaults, producing a [`CommonConfig`].
    ///
    /// `target` defaults to the crate's build target; `bin_install_path` defaults to the
    /// current executable. `current_version`, `bin_name`, and `bin_path_in_archive` are
    /// required (the last is set automatically by the `bin_name` setter).
    pub(crate) fn build(&self) -> Result<CommonConfig> {
        // Bundle mode: reject a conflicting single-file config and resolve the install path (which
        // may consult `current_exe()`), before any other work.
        let (bundle_path_in_archive, bundle_install_path) = self.resolve_bundle_mode()?;
        // Resolve the auth scheme/token into the request config so the shared header-derivation
        // (`apply_auth`) can apply it on both the listing and download paths.
        let mut request = self.request.clone();
        request.auth_scheme = self.auth_scheme;
        request.auth_token = self.auth_token.clone();
        // Materialize an HTTP client from any custom root CA certs (no-op if none / a client was
        // injected), then surface any deferred header/cert error as a config error.
        request.build_client();
        request.check()?;
        Ok(CommonConfig {
            request,
            target: self
                .target
                .clone()
                .unwrap_or_else(|| get_target().to_owned()),
            asset_identifier: self.asset_identifier.clone(),
            current_version: self.current_version.clone().ok_or(Error::MissingField {
                field: "current_version",
            })?,
            release_tag: self.release_tag.clone(),
            tag_prefix: self.tag_prefix.clone(),
            bin_name: self
                .bin_name
                .clone()
                .ok_or(Error::MissingField { field: "bin_name" })?,
            bin_install_path: match &self.bin_install_path {
                Some(p) => p.clone(),
                None => std::env::current_exe()?,
            },
            check_install_path_writable: self.check_install_path_writable,
            bin_path_in_archive: self
                .bin_path_in_archive
                .clone()
                .ok_or(Error::MissingField {
                    field: "bin_path_in_archive",
                })?,
            bundle_path_in_archive,
            bundle_install_path,
            show_download_progress: self.show_download_progress,
            show_output: self.show_output,
            no_confirm: self.no_confirm,
            show_release_notes: self.show_release_notes,
            update_strategy: self.update_strategy,
            #[cfg(feature = "progress-bar")]
            progress_template: self.progress_template.clone(),
            #[cfg(feature = "progress-bar")]
            progress_chars: self.progress_chars.clone(),
            progress_callback: self.progress_callback.clone(),
            verify: self.verify.clone(),
            verify_archive: self.verify_archive.clone(),
            asset_matcher: self.asset_matcher.clone(),
            #[cfg(feature = "checksums")]
            checksum: self.checksum.clone(),
            #[cfg(feature = "checksums")]
            checksum_from_asset: self.checksum_from_asset.clone(),
            #[cfg(feature = "checksums")]
            verify_release_digest: self.verify_release_digest,
            #[cfg(feature = "signatures")]
            verifying_keys: self.verifying_keys.clone(),
        })
    }

    /// Validate and resolve the bundle-mode options, returning the pair stored on the built
    /// [`CommonConfig`]: `(bundle_path_in_archive, bundle_install_path)`, both `None` when bundle
    /// mode is off.
    ///
    /// Bundle mode is selected by `bundle_path_in_archive`. It replaces a whole directory instead
    /// of one file, so combining it with an explicit `bin_install_path` or `bin_path_in_archive` is
    /// a config conflict rather than a silently-dropped setter. The value
    /// `bin_path_in_archive` auto-derives from `bin_name` does not count as explicit (it is simply
    /// unused in bundle mode).
    ///
    /// With no explicit `bundle_install_path`, macOS derives it from the running executable (the
    /// nearest `.app` ancestor); every other platform requires it.
    fn resolve_bundle_mode(&self) -> Result<(Option<String>, Option<PathBuf>)> {
        let Some(path_in_archive) = self.bundle_path_in_archive.clone() else {
            // `bundle_install_path` alone does not select bundle mode, and silently installing a
            // single file to the default path instead is the same footgun the conflict check below
            // exists to prevent, so say which setter is missing.
            if self.bundle_install_path.is_some() {
                return Err(Error::MissingField {
                    field: "bundle_path_in_archive",
                });
            }
            return Ok((None, None));
        };
        if self.bin_install_path.is_some() {
            return Err(Error::ConflictingConfig {
                field: "bundle_path_in_archive",
                conflict: "bin_install_path",
            });
        }
        if self.bin_path_in_archive.is_some() && !self.bin_path_in_archive_auto {
            return Err(Error::ConflictingConfig {
                field: "bundle_path_in_archive",
                conflict: "bin_path_in_archive",
            });
        }
        let install_path = match &self.bundle_install_path {
            Some(p) => p.clone(),
            None => crate::update::default_bundle_install_path()?,
        };
        Ok((Some(path_in_archive), Some(install_path)))
    }
}

/// The resolved common options of a built `Update`, embedded by every backend's `Update`.
#[derive(Debug)]
pub(crate) struct CommonConfig {
    pub request: RequestConfig,
    pub target: String,
    pub asset_identifier: Option<String>,
    pub current_version: String,
    pub release_tag: Option<String>,
    /// Only the forge backends (github/gitlab/gitea/gitee) read this; the attribute keeps a build
    /// without any of them (e.g. `--no-default-features --features "reqwest rustls s3"`)
    /// warning-free, like the helpers above.
    #[cfg_attr(
        not(any(
            feature = "github",
            feature = "gitlab",
            feature = "gitea",
            feature = "gitee"
        )),
        allow(dead_code)
    )]
    pub tag_prefix: Option<String>,
    pub bin_name: String,
    pub bin_install_path: PathBuf,
    /// Opt-in preflight writability probe of `bin_install_path` (default `false`).
    pub check_install_path_writable: bool,
    pub bin_path_in_archive: String,
    /// The bundle directory inside the archive; `Some` means bundle mode, in which case
    /// `bundle_install_path` is also `Some` and the single-file `bin_*` paths are unused.
    pub bundle_path_in_archive: Option<String>,
    /// The resolved installed bundle directory to replace, `Some` exactly when
    /// `bundle_path_in_archive` is.
    pub bundle_install_path: Option<PathBuf>,
    pub show_download_progress: bool,
    pub show_output: bool,
    pub no_confirm: bool,
    pub show_release_notes: bool,
    pub update_strategy: crate::update::UpdateStrategy,
    #[cfg(feature = "progress-bar")]
    pub progress_template: String,
    #[cfg(feature = "progress-bar")]
    pub progress_chars: String,
    pub progress_callback: Option<crate::ProgressCallback>,
    pub verify: Option<crate::VerifyCallback>,
    /// Pre-extraction hook over the downloaded archive; see
    /// [`CommonBuilderConfig::verify_archive`].
    pub verify_archive: Option<crate::VerifyCallback>,
    pub asset_matcher: Option<crate::AssetMatcher>,
    #[cfg(feature = "checksums")]
    pub checksum: Option<crate::Checksum>,
    /// Name of the release asset to resolve a digest from; see
    /// [`CommonBuilderConfig::checksum_from_asset`].
    #[cfg(feature = "checksums")]
    pub checksum_from_asset: Option<String>,
    #[cfg(feature = "checksums")]
    pub verify_release_digest: bool,
    #[cfg(feature = "signatures")]
    pub verifying_keys: Vec<[u8; zipsign_api::PUBLIC_KEY_LENGTH]>,
}

#[cfg(test)]
mod tests {
    use super::{CommonBuilderConfig, RequestConfig};
    use std::path::PathBuf;
    use std::time::Duration;

    /// A PEM-framed certificate whose body is not valid X.509 DER (base64 of "not a valid cert").
    /// reqwest accepts the PEM framing but rejects it at client-build time, so it reliably produces
    /// a cert-build error. Used by the per-slot cert tests, which are `async`-gated (async implies
    /// reqwest).
    #[cfg(feature = "async")]
    const BAD_PEM_CERT: &[u8] =
        b"-----BEGIN CERTIFICATE-----\nbm90IGEgdmFsaWQgY2VydA==\n-----END CERTIFICATE-----\n";

    /// Build the `(name, value)` candidate list `token_from_env` hands to `first_env_token`, from
    /// literal values, so the precedence rules are exercised without mutating process env (racy
    /// under the parallel harness, and `unsafe` since the 2024 edition).
    fn candidates<'a>(pairs: &[(&'a str, Option<&str>)]) -> Vec<(&'a str, Option<String>)> {
        pairs
            .iter()
            .map(|(name, value)| (*name, value.map(str::to_owned)))
            .collect()
    }

    // AUTH-1-1: the first candidate that is set and non-empty wins, even when a later one is also
    // set. This pins the documented precedence (e.g. GH_TOKEN over GITHUB_TOKEN, matching `gh`).
    #[test]
    fn first_env_token_takes_the_first_present_value() {
        let got = super::first_env_token(&candidates(&[
            ("GH_TOKEN", Some("first")),
            ("GITHUB_TOKEN", Some("second")),
        ]));
        assert_eq!(
            got.as_deref(),
            Some("first"),
            "the earlier variable must win over a later one"
        );
    }

    // A variable that is set but empty (or all-whitespace) is treated as unset, so an exported-but-
    // blank `GITHUB_TOKEN` (common in CI scaffolding) falls through to the next candidate instead of
    // producing an empty `Authorization` header.
    #[test]
    fn first_env_token_skips_empty_and_whitespace_values() {
        let got = super::first_env_token(&candidates(&[
            ("GITHUB_TOKEN", Some("")),
            ("GH_TOKEN", Some("   ")),
            ("OTHER_TOKEN", Some("real")),
        ]));
        assert_eq!(
            got.as_deref(),
            Some("real"),
            "empty and whitespace-only values must be skipped"
        );
    }

    // Surrounding whitespace is trimmed: a value pasted into a CI secret with a trailing newline
    // still yields a usable token (an untrimmed one would fail header encoding).
    #[test]
    fn first_env_token_trims_surrounding_whitespace() {
        let got = super::first_env_token(&candidates(&[("GITHUB_TOKEN", Some("  ghp_abc\n"))]));
        assert_eq!(got.as_deref(), Some("ghp_abc"));
    }

    // AUTH-1-2: nothing set (or everything empty) resolves to `None`, which leaves the builder's
    // token untouched and sends the request unauthenticated.
    #[test]
    fn first_env_token_returns_none_when_nothing_is_set() {
        assert_eq!(
            super::first_env_token(&candidates(&[
                ("GITHUB_TOKEN", None),
                ("GH_TOKEN", Some("  ")),
            ])),
            None,
            "no present, non-empty candidate must resolve to None"
        );
    }

    // The environment is a FALLBACK, never an override: a resolved token must NOT displace a token
    // the application set explicitly, so `auth_token("x").auth_token_from_env()` keeps `x`. The old
    // behavior (overwrite) made the setter pair order-sensitive and let an ambient developer/CI
    // credential -- or one an attacker who can influence the environment plants -- silently replace
    // a deliberately provisioned one.
    #[test]
    fn fill_env_token_if_unset_keeps_an_explicit_token() {
        let mut slot = Some("explicit".to_string());
        let filled = super::fill_env_token_if_unset(&mut slot, Some("from-env".to_string()));
        assert_eq!(
            slot.as_deref(),
            Some("explicit"),
            "an explicitly-set token must survive a resolved env token"
        );
        assert!(
            !filled,
            "nothing was taken from the environment, so the token is not env-sourced"
        );
    }

    // An empty slot is what the environment is for: the resolved token lands, and the call reports
    // that the token is env-sourced (which is what drives the non-canonical-host warning).
    #[test]
    fn fill_env_token_if_unset_fills_an_empty_slot() {
        let mut slot = None;
        let filled = super::fill_env_token_if_unset(&mut slot, Some("from-env".to_string()));
        assert_eq!(slot.as_deref(), Some("from-env"));
        assert!(
            filled,
            "filling an empty slot must report an env-sourced token"
        );
    }

    // The other half of that rule: an unresolved lookup must NOT clear an explicitly-set token.
    // Clearing it would turn an unset env var into a silent loss of authorization (a surprise 403
    // against a private repo), so `auth_token_from_env()` is additive when the environment is empty.
    #[test]
    fn fill_env_token_if_unset_keeps_an_existing_token_when_env_resolves_to_none() {
        let mut slot = Some("explicit".to_string());
        let filled = super::fill_env_token_if_unset(&mut slot, None);
        assert_eq!(
            slot.as_deref(),
            Some("explicit"),
            "an empty environment must leave an explicitly-set token in place"
        );
        assert!(!filled);
    }

    // With no token set and nothing in the environment, the slot stays empty: the request goes out
    // unauthenticated exactly as it would without the call.
    #[test]
    fn fill_env_token_if_unset_leaves_an_empty_slot_empty() {
        let mut slot = None;
        let filled = super::fill_env_token_if_unset(&mut slot, None);
        assert_eq!(slot, None);
        assert!(!filled);
    }

    /// An `OsString` that is not valid UTF-8, built without touching process env. Both platform
    /// families can hold such a value (a raw byte on unix, an unpaired surrogate on windows).
    #[cfg(any(unix, windows))]
    fn non_utf8_os_string() -> std::ffi::OsString {
        #[cfg(unix)]
        {
            std::os::unix::ffi::OsStringExt::from_vec(vec![b'g', b'h', b'p', 0x80])
        }
        #[cfg(windows)]
        {
            std::os::windows::ffi::OsStringExt::from_wide(&[0x0067, 0x0068, 0xD800])
        }
    }

    // A variable whose value is not valid UTF-8 cannot become an HTTP header value. It is reported
    // (via `log::warn!`) and treated as unset, rather than silently swallowed the way the old
    // `std::env::var(name).ok()` did -- which made a mangled token indistinguishable from an absent
    // one. Exercised through the pure helper, so no process env is mutated.
    #[cfg(any(unix, windows))]
    #[test]
    fn env_token_value_treats_a_non_utf8_value_as_unset() {
        assert_eq!(
            super::env_token_value("GH_TOKEN", Some(non_utf8_os_string())),
            None,
            "a non-UTF-8 value must resolve to None"
        );
        // ... and a valid value still passes through untouched (trimming happens later).
        assert_eq!(
            super::env_token_value("GH_TOKEN", Some(std::ffi::OsString::from(" ghp_abc "))),
            Some(" ghp_abc ".to_string())
        );
        assert_eq!(super::env_token_value("GH_TOKEN", None), None);
    }

    // H: a non-UTF-8 value falls through to the NEXT candidate rather than aborting the lookup, so
    // a mangled `GH_TOKEN` still lets `GITHUB_TOKEN` supply the token.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_non_utf8_candidate_falls_through_to_the_next_variable() {
        let candidates = vec![
            (
                "GH_TOKEN",
                super::env_token_value("GH_TOKEN", Some(non_utf8_os_string())),
            ),
            (
                "GITHUB_TOKEN",
                super::env_token_value("GITHUB_TOKEN", Some(std::ffi::OsString::from("real"))),
            ),
        ];
        assert_eq!(super::first_env_token(&candidates).as_deref(), Some("real"));
    }

    // --- H/A1/A2: deciding what to do with an env-sourced token bound to an unacknowledged host ---

    use super::EnvTokenDecision;

    // No `auth_hosts` entries in play for most of these; a short alias keeps the calls readable.
    const NO_EXTRA_HOSTS: &[String] = &[];

    // The canonical host is the expected case and must stay silent: the env var list is a
    // convention of that very service.
    #[test]
    fn sends_silently_when_an_env_token_targets_the_canonical_host() {
        assert_eq!(
            super::env_token_host_decision(
                true,
                Some("api.github.com"),
                NO_EXTRA_HOSTS,
                Some("api.github.com")
            ),
            EnvTokenDecision::Sent,
            "the canonical host must not warn"
        );
        // Host comparison is case-insensitive, like the request-time host gate.
        assert_eq!(
            super::env_token_host_decision(
                true,
                Some("API.GitHub.com"),
                NO_EXTRA_HOSTS,
                Some("api.github.com")
            ),
            EnvTokenDecision::Sent
        );
    }

    // The case this guard exists for: a custom `api_base_url`/`host` plus a token the application
    // never typed. The request-time host gate cannot catch it (the configured host *is*
    // `auth_base_host`), so an app that exposes its update URL as config and runs in CI would hand
    // `GITHUB_TOKEN` to an attacker-chosen host with no signal at all. A backend WITH a canonical
    // host still sends it, just with a warning (DECIDED, A1).
    #[test]
    fn warns_and_sends_when_an_env_token_targets_an_unacknowledged_custom_host() {
        assert_eq!(
            super::env_token_host_decision(
                true,
                Some("evil.example.com"),
                NO_EXTRA_HOSTS,
                Some("api.github.com")
            ),
            EnvTokenDecision::WarnedAndSent,
            "an env-sourced token bound to an unacknowledged host must warn but still be sent"
        );
    }

    // A2: an `allow_auth_host` entry is itself the user's explicit "send it here" -- once the
    // configured host is in that set, the warning falls silent even though it is not the canonical
    // host, exactly as if it were. Case-insensitive, like every other host comparison here.
    #[test]
    fn sends_silently_when_the_host_is_acknowledged_via_allow_auth_host() {
        let auth_hosts = ["Evil.Example.com".to_string()];
        assert_eq!(
            super::env_token_host_decision(
                true,
                Some("evil.example.com"),
                &auth_hosts,
                Some("api.github.com")
            ),
            EnvTokenDecision::Sent,
            "a host present in allow_auth_host must not warn, even though it is not canonical"
        );
        // An UNLISTED custom host on the same builder still warns -- acknowledging one host does not
        // silence the check for every other one.
        assert_eq!(
            super::env_token_host_decision(
                true,
                Some("other.example.com"),
                &auth_hosts,
                Some("api.github.com")
            ),
            EnvTokenDecision::WarnedAndSent
        );
    }

    // An explicitly-set token is the application's own decision about which host to trust, so the
    // same custom host must stay silent -- otherwise every enterprise/self-hosted user with an
    // explicit token would be nagged. Not env-sourced short-circuits before any host comparison.
    #[test]
    fn no_action_for_an_explicitly_set_token_on_a_custom_host() {
        assert_eq!(
            super::env_token_host_decision(
                false,
                Some("github.mycorp.com"),
                NO_EXTRA_HOSTS,
                Some("api.github.com")
            ),
            EnvTokenDecision::Sent,
            "an explicitly-set token must never warn or be withheld"
        );
    }

    // A1 (DECIDED): a backend with NO canonical host (gitea is always self-hosted) has nothing to
    // compare against, so an unacknowledged env-sourced token is WITHHELD rather than silently sent
    // to whatever host happens to be configured -- the opposite of the canonical-host backends,
    // which still send it.
    #[test]
    fn withholds_for_a_backend_without_a_canonical_host_and_no_acknowledgement() {
        assert_eq!(
            super::env_token_host_decision(true, Some("gitea.example.com"), NO_EXTRA_HOSTS, None),
            EnvTokenDecision::Withheld
        );
    }

    // A1's remedy: `allow_auth_host(configured_host)` re-affirms the host and the token is sent,
    // even though the backend still has no canonical host of its own. This is the "explicit
    // re-affirmation" half of A1's contract (the other half, an explicit `auth_token(..)`, never
    // reaches this function at all -- it clears `env_sourced`).
    #[test]
    fn sends_for_a_backend_without_a_canonical_host_once_the_host_is_acknowledged() {
        let auth_hosts = ["gitea.example.com".to_string()];
        assert_eq!(
            super::env_token_host_decision(true, Some("gitea.example.com"), &auth_hosts, None),
            EnvTokenDecision::Sent
        );
    }

    // No parseable host at all: the request-time gate (`auth_allowed_for`) will not attach the
    // token to anything regardless, so there is nothing to warn about or withhold -- fail-closed in
    // both directions (canonical-host and canonical-host-less backends alike).
    #[test]
    fn sends_silently_when_there_is_no_host_at_all() {
        assert_eq!(
            super::env_token_host_decision(true, None, NO_EXTRA_HOSTS, Some("api.github.com")),
            EnvTokenDecision::Sent
        );
        assert_eq!(
            super::env_token_host_decision(true, None, NO_EXTRA_HOSTS, None),
            EnvTokenDecision::Sent
        );
    }

    // --- A5/B6: a blank auth token is treated as unset -----------------------------------------

    // Neither an absent nor a whitespace-only token counts as "configured"; only real content does.
    #[test]
    fn is_blank_token_treats_none_and_whitespace_as_blank() {
        assert!(super::is_blank_token(None));
        assert!(super::is_blank_token(Some("")));
        assert!(super::is_blank_token(Some("   \n\t")));
        assert!(!super::is_blank_token(Some("ghp_abc")));
        // Surrounding whitespace around real content does not make it blank (it is not trimmed away
        // here -- only the blank-vs-not judgment ignores it; see the `first_env_token` trimming
        // asymmetry documented on `auth_token`).
        assert!(!super::is_blank_token(Some("  ghp_abc  ")));
    }

    // A5: `auth_token("").auth_token_from_env()` must not leave the blank explicit value blocking
    // the fallback -- the resolved env token must land in the slot.
    #[test]
    fn fill_env_token_if_unset_fills_over_a_blank_explicit_token() {
        let mut slot = Some("".to_string());
        let filled = super::fill_env_token_if_unset(&mut slot, Some("from-env".to_string()));
        assert!(
            filled,
            "a blank explicit token must not block the env fallback"
        );
        assert_eq!(slot.as_deref(), Some("from-env"));

        let mut slot = Some("   ".to_string());
        let filled = super::fill_env_token_if_unset(&mut slot, Some("from-env".to_string()));
        assert!(
            filled,
            "an all-whitespace explicit token must not block the env fallback"
        );
        assert_eq!(slot.as_deref(), Some("from-env"));
    }

    // A5: with nothing in the environment either, a blank explicit token is left exactly as it was
    // (still blank) -- `is_blank_token` is what makes it inert downstream (`apply_auth`,
    // `has_auth_token`), not a mutation here.
    #[test]
    fn fill_env_token_if_unset_leaves_a_blank_token_blank_when_the_env_resolves_to_none() {
        let mut slot = Some("".to_string());
        let filled = super::fill_env_token_if_unset(&mut slot, None);
        assert!(!filled);
        assert_eq!(slot.as_deref(), Some(""));
    }

    // A5: `apply_auth` must not send a literal `Authorization: token ` for a blank token -- the
    // request goes out exactly as an anonymous one would.
    #[test]
    fn apply_auth_treats_a_blank_token_as_unset() {
        for blank in ["", "   ", "\n\t"] {
            let req = RequestConfig {
                auth_token: Some(blank.to_string()),
                auth_base_host: Some("api.github.com".to_string()),
                ..Default::default()
            };
            let mut headers = crate::http_client::HeaderMap::new();
            req.apply_auth("https://api.github.com/repos/o/r/releases", &mut headers)
                .expect("a blank token must not fail encoding");
            assert!(
                headers
                    .get(crate::http_client::header::AUTHORIZATION)
                    .is_none(),
                "a blank token ({blank:?}) must not produce an Authorization header"
            );
        }
        // Sanity check: the same config with a real token DOES attach one, so the assertions above
        // are exercising the blank-token path and not some other reason nothing was sent.
        let req = RequestConfig {
            auth_token: Some("real".to_string()),
            auth_base_host: Some("api.github.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.github.com/repos/o/r/releases", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_some()
        );
    }

    // --- A6: the env lookup must not run when its result would be discarded ---------------------

    // With an explicit, non-blank token already in the slot, the resolver closure must never be
    // invoked -- proving the "using the auth token from $X" log (and the non-UTF-8 warning) cannot
    // fire for a lookup whose result is thrown away.
    #[test]
    fn fill_env_token_if_unset_with_does_not_call_the_resolver_when_the_slot_is_filled() {
        let mut slot = Some("explicit".to_string());
        let mut called = false;
        let filled = super::fill_env_token_if_unset_with(&mut slot, || {
            called = true;
            Some("from-env".to_string())
        });
        assert!(!filled);
        assert!(
            !called,
            "the resolver must not run when the slot already holds a real token"
        );
        assert_eq!(slot.as_deref(), Some("explicit"));
    }

    // The resolver DOES run for an empty slot -- confirms the guard above is "already filled", not
    // "never call it".
    #[test]
    fn fill_env_token_if_unset_with_calls_the_resolver_when_the_slot_is_empty() {
        let mut slot = None;
        let mut called = false;
        let filled = super::fill_env_token_if_unset_with(&mut slot, || {
            called = true;
            Some("from-env".to_string())
        });
        assert!(filled);
        assert!(called);
        assert_eq!(slot.as_deref(), Some("from-env"));
    }

    // A5+A6 together: a BLANK explicit token is not "already filled" either, so the resolver still
    // runs and its result lands in the slot.
    #[test]
    fn fill_env_token_if_unset_with_calls_the_resolver_when_the_slot_is_blank() {
        let mut slot = Some("   ".to_string());
        let mut called = false;
        let filled = super::fill_env_token_if_unset_with(&mut slot, || {
            called = true;
            Some("from-env".to_string())
        });
        assert!(filled);
        assert!(called);
        assert_eq!(slot.as_deref(), Some("from-env"));
    }

    // --- E4: the shared explicit-token setter -----------------------------------------------

    #[test]
    fn set_explicit_auth_token_sets_the_value_and_clears_env_sourced() {
        let mut slot = None;
        let mut env_sourced = true;
        super::set_explicit_auth_token(&mut slot, &mut env_sourced, "explicit");
        assert_eq!(slot.as_deref(), Some("explicit"));
        assert!(
            !env_sourced,
            "an explicit token must clear the env-sourced flag"
        );
    }

    // The blank-token rule (A5) applies to the *slot*, not to this setter: `auth_token(..)`
    // unconditionally overwrites and unconditionally clears the flag, even with a blank value. So the
    // two orders around a BLANK explicit token are NOT symmetric, unlike a real one:
    //
    //   auth_token("").auth_token_from_env()  -> env token wins (the slot was blank, so it filled)
    //   auth_token_from_env().auth_token("")  -> anonymous (the blank overwrote the env token)
    //
    // The second is the "last writer wins with an unset value" reading, and it is what
    // `has_auth_token()` then reports (`false`). Pinned here because nothing else states it: a future
    // "don't clobber a live token with a blank" change would be a real behavior change and must not
    // slip in silently. (See the certification note on `macros.rs`'s `auth_token` rustdoc, which
    // claims this case works "in either call order".)
    #[test]
    fn set_explicit_auth_token_with_a_blank_value_overwrites_an_env_sourced_token() {
        let mut slot = Some("from-env".to_string());
        let mut env_sourced = true;
        super::set_explicit_auth_token(&mut slot, &mut env_sourced, "   ");
        assert_eq!(
            slot.as_deref(),
            Some("   "),
            "an explicit blank value overwrites the slot rather than being ignored"
        );
        assert!(
            !env_sourced,
            "the token is no longer env-sourced once an explicit setter ran, blank or not"
        );
        assert!(
            super::is_blank_token(slot.as_deref()),
            "and the resulting slot is blank, i.e. no token is configured at all"
        );
    }

    // --- A: the builder config's Debug must not leak the token ---------------------------------

    // `CommonBuilderConfig` is embedded (and `Debug`-derived over) by every backend's
    // `UpdateBuilder`, so a plain `log::debug!("{builder:?}")` used to print a live credential --
    // one the application author may not even have typed, since `auth_token_from_env()` puts an
    // ambient CI token there. It renders as `"<token>"`, exactly like `RequestConfig`'s Debug, and
    // the other fields still show.
    #[test]
    fn debug_redacts_the_auth_token_but_keeps_other_fields() {
        let cfg = CommonBuilderConfig {
            auth_token: Some("ghp_supersecret".to_string()),
            bin_name: Some("app".to_string()),
            ..Default::default()
        };
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("ghp_supersecret"),
            "the token value must never appear in Debug output, got: {rendered}"
        );
        assert!(
            rendered.contains("<token>"),
            "the token must render as the redaction marker, got: {rendered}"
        );
        assert!(
            rendered.contains("\"app\""),
            "non-secret fields must still be rendered, got: {rendered}"
        );
        // An unset token still renders as `None`, so "is a token set?" stays answerable.
        assert!(format!("{:?}", CommonBuilderConfig::default()).contains("auth_token: None"));
    }

    // The other failure mode of a hand-written `Debug`: not leaking a secret but silently *losing*
    // a field. Every backend's `UpdateBuilder` derives its `Debug` over this struct, so this is the
    // dump an application pastes into a bug report; an existing line dropped while adding the
    // redaction makes the dump quietly misleading. This RUNTIME test only catches that direction --
    // a field removed from `fmt` while still on the struct -- because it asserts a hardcoded literal
    // list, so a field added to neither the struct nor the list would pass silently. It does NOT
    // catch a field added to the struct and never added to `fmt`: that direction is instead a
    // COMPILE error, from the exhaustive `let Self { .. } = self;` destructure at the top of `fmt`
    // (no `..`), which fails to build the moment a new field exists on the struct but not in the
    // pattern. The assertion above -- "does not contain the secret" -- passes for a `Debug` that
    // prints nothing at all.
    #[test]
    fn debug_renders_every_field() {
        let rendered = format!("{:?}", CommonBuilderConfig::default());
        let mut fields = vec![
            "request",
            "target",
            "asset_identifier",
            "bin_name",
            "bin_install_path",
            "check_install_path_writable",
            "bin_path_in_archive",
            "bin_path_in_archive_auto",
            "bundle_path_in_archive",
            "bundle_install_path",
            "show_download_progress",
            "show_output",
            "no_confirm",
            "show_release_notes",
            "update_strategy",
            "tag_prefix",
            "current_version",
            "release_tag",
            "auth_token",
            "auth_token_from_env",
            "auth_scheme",
            "progress_callback",
            "verify",
            "verify_archive",
            "asset_matcher",
        ];
        #[cfg(feature = "progress-bar")]
        fields.extend(["progress_template", "progress_chars"]);
        #[cfg(feature = "checksums")]
        fields.extend(["checksum", "checksum_from_asset", "verify_release_digest"]);
        #[cfg(feature = "signatures")]
        fields.push("verifying_keys");
        for field in fields {
            assert!(
                rendered.contains(&format!("{field}:")),
                "the hand-written Debug dropped `{field}`, got: {rendered}"
            );
        }
    }

    // `RequestConfig`'s `Debug` is hand-written too (to redact `auth_token`) and is reached from
    // every backend's dump through the embedded `request` field, so it has the same "silently lost a
    // field" failure mode. It had actually lost three: `auth_base_host`, `auth_hosts` and
    // `allow_insecure_auth` -- the fields that decide whether the token is attached to a given
    // request at all, and the host the env-token warning compares against. A dump missing them
    // cannot answer "why was my token not sent?" or "which host did the warning mean?", and the
    // "does not contain the secret" assertion below passes for a `Debug` that renders nothing at all.
    #[test]
    fn request_config_debug_renders_every_field() {
        let req = RequestConfig {
            auth_token: Some("ghp_supersecret".to_string()),
            auth_base_host: Some("api.github.com".to_string()),
            auth_hosts: vec!["cdn.example.com".to_string()],
            allow_insecure_auth: true,
            proxy: Some("http://corpuser:hunter2@proxy.corp:8080".to_string()),
            ..Default::default()
        };
        let rendered = format!("{req:?}");
        let mut fields = vec![
            "timeout",
            "headers",
            "retries",
            "retry_base_delay",
            "retry_max_delay",
            "auth_scheme",
            "auth_token",
            "client",
            "header_error",
            "root_certificates",
            "cert_error",
            "proxy",
            "proxy_error",
            "auth_base_host",
            "auth_hosts",
            "allow_insecure_auth",
        ];
        // `cfg!` rather than a `#[cfg]` attribute so the `mut` above is used on every lane.
        if cfg!(feature = "async") {
            fields.push("async_client");
        }
        for field in fields {
            // Matched with the leading separator space so `client` cannot be satisfied by
            // `async_client`'s rendering.
            assert!(
                rendered.contains(&format!(" {field}:")),
                "the hand-written Debug dropped `{field}`, got: {rendered}"
            );
        }
        // The three host-gate fields must render their *values*, not just their names: they are
        // hostnames and a flag, carrying nothing secret.
        assert!(
            rendered.contains("api.github.com")
                && rendered.contains("cdn.example.com")
                && rendered.contains("allow_insecure_auth: true"),
            "the auth-host gate must be readable from the dump, got: {rendered}"
        );
        // A proxy URL's password is a credential too: the dump must name the proxy (so "which
        // proxy?" stays answerable) while redacting the password.
        assert!(
            !rendered.contains("hunter2"),
            "the proxy password must never appear in Debug output, got: {rendered}"
        );
        assert!(
            rendered.contains("http://corpuser:REDACTED@proxy.corp:8080"),
            "the proxy must render with only its password redacted, got: {rendered}"
        );
        // And the token redaction is untouched by the addition.
        assert!(
            !rendered.contains("ghp_supersecret"),
            "the token value must never appear in Debug output, got: {rendered}"
        );
        assert!(
            rendered.contains("auth_token: Some(\"<token>\")"),
            "the token must still render as the redaction marker, got: {rendered}"
        );
        // An unset token still renders as `None`, so "is a token set?" stays answerable.
        assert!(
            format!("{:?}", RequestConfig::default()).contains("auth_token: None"),
            "an unset token must render as None"
        );
    }

    // --- A3: a user-supplied `request_header("Authorization", ..)` must not leak in `Debug` -------

    // `apply_auth` gives a user-supplied `Authorization` header PRECEDENCE over the backend's own
    // token (see `apply_auth`'s doc, point 1), so it is exactly as much a live credential as
    // `auth_token` -- and before this fix it rendered verbatim in `RequestConfig`'s `Debug` (and so
    // in every backend builder's `Debug`, which embeds it). `insert_header` must mark it sensitive
    // the same way `apply_auth` marks the derived header.
    #[test]
    fn request_config_debug_redacts_a_user_supplied_authorization_header() {
        let mut req = RequestConfig::default();
        req.insert_header("Authorization", "Bearer user-supplied-secret");
        let rendered = format!("{req:?}");
        assert!(
            !rendered.contains("user-supplied-secret"),
            "a user-supplied Authorization header must never appear in Debug output, got: {rendered}"
        );
        assert!(
            rendered.contains("Sensitive"),
            "a redacted header renders as `Sensitive` in http's HeaderValue Debug, got: {rendered}"
        );
    }

    // The other credential-shaped header names `insert_header` must redact: gitlab's `PRIVATE-TOKEN`
    // (case-insensitive; `HeaderName` always lowercases), `Cookie`, and the generic `*-token` shape a
    // custom gateway header might use. A header that does NOT match must render in the clear, so the
    // rule is not "redact everything".
    #[test]
    fn insert_header_marks_every_credential_shaped_header_name_sensitive() {
        for name in [
            "PRIVATE-TOKEN",
            "Cookie",
            "X-Upstream-Token",
            "authorization",
        ] {
            let mut req = RequestConfig::default();
            req.insert_header(name, "super-secret-value");
            let rendered = format!("{req:?}");
            assert!(
                !rendered.contains("super-secret-value"),
                "`{name}` must be redacted in Debug output, got: {rendered}"
            );
        }
        // A non-credential header is unaffected: it must still render in the clear.
        let mut req = RequestConfig::default();
        req.insert_header("X-Request-Id", "not-a-secret");
        assert!(
            format!("{req:?}").contains("not-a-secret"),
            "a non-credential header must not be redacted"
        );
    }

    // A3, the other half: `set_sensitive` must be a *marking*, not a mutation. The redaction is only
    // acceptable because the header still goes out byte-for-byte as the application wrote it -- a
    // "fix" that stored a placeholder, trimmed, or dropped the value would silence the Debug leak and
    // simultaneously break every gateway that needs the header. Asserted through the stored
    // `HeaderValue` (which is what the transports send) rather than the Debug rendering.
    #[test]
    fn insert_header_marks_a_credential_sensitive_without_altering_its_value() {
        let mut req = RequestConfig::default();
        req.insert_header("Authorization", "Bearer user-supplied-secret");
        req.insert_header("X-Request-Id", "not-a-secret");
        let auth = req
            .headers
            .get(crate::http_client::header::AUTHORIZATION)
            .expect("the user-supplied header must still be stored");
        assert_eq!(
            auth.to_str().unwrap(),
            "Bearer user-supplied-secret",
            "the value sent on the wire must be exactly what the application passed"
        );
        assert!(
            auth.is_sensitive(),
            "a credential-bearing header must be flagged sensitive, which is what keeps it out of \
             Debug output and the transports' own header logging"
        );
        // The flag is per-value, not per-map: an ordinary header stays unflagged, so `is_sensitive`
        // above is discriminating and not simply true for everything in the map.
        assert!(
            !req.headers
                .get("x-request-id")
                .expect("the ordinary header must be stored")
                .is_sensitive(),
            "a non-credential header must not be marked sensitive"
        );
    }

    // --- E6: the destructured `Debug` impls must pair each name with its OWN value ---------------

    // The exhaustive `let Self { .. } = self;` destructure (E6) makes a *missing* field a compile
    // error, and `request_config_debug_renders_every_field` catches a *dropped* line -- but neither
    // catches the failure mode the destructure itself introduces: every field is now a bare local, so
    // rendering `.field("auth_base_host", auth_hosts)` (or swapping the two `Duration`s, or the two
    // `Option<String>` error slots) compiles and keeps every field name present. The dump would then
    // be confidently wrong about exactly the fields an application reads to answer "why was my token
    // not sent?". Distinct values per field, asserted as name/value PAIRS.
    #[test]
    fn request_config_debug_pairs_each_field_with_its_own_value() {
        let req = RequestConfig {
            timeout: Some(Duration::from_secs(11)),
            retries: 7,
            retry_base_delay: Duration::from_millis(13),
            retry_max_delay: Duration::from_millis(17),
            header_error: Some("header-error-marker".to_string()),
            cert_error: Some("cert-error-marker".to_string()),
            proxy: Some("http://proxy-marker.example.test:8080".to_string()),
            proxy_error: Some("proxy-error-marker".to_string()),
            auth_base_host: Some("base.example.test".to_string()),
            auth_hosts: vec!["extra.example.test".to_string()],
            allow_insecure_auth: true,
            ..Default::default()
        };
        let rendered = format!("{req:?}");
        for (field, value) in [
            ("timeout", "Some(11s)"),
            ("retries", "7"),
            ("retry_base_delay", "13ms"),
            ("retry_max_delay", "17ms"),
            ("header_error", "Some(\"header-error-marker\")"),
            ("cert_error", "Some(\"cert-error-marker\")"),
            ("proxy", "Some(\"http://proxy-marker.example.test:8080\")"),
            ("proxy_error", "Some(\"proxy-error-marker\")"),
            ("auth_base_host", "Some(\"base.example.test\")"),
            ("auth_hosts", "[\"extra.example.test\"]"),
            ("allow_insecure_auth", "true"),
        ] {
            assert!(
                rendered.contains(&format!("{field}: {value}")),
                "`{field}` must render its own value (`{value}`), got: {rendered}"
            );
        }
    }

    // Same guard for `CommonBuilderConfig`, which has far more same-typed neighbours to confuse: five
    // `Option<String>`s, two `Option<PathBuf>`s and six `bool`s, all rendered by hand from locals of
    // identical type. This is the dump an application pastes into a bug report.
    #[test]
    fn common_builder_config_debug_pairs_each_field_with_its_own_value() {
        let cfg = CommonBuilderConfig {
            target: Some("target-marker".to_string()),
            asset_identifier: Some("asset-identifier-marker".to_string()),
            bin_name: Some("bin-name-marker".to_string()),
            bin_install_path: Some(PathBuf::from("/bin-install-path-marker")),
            check_install_path_writable: true,
            bin_path_in_archive: Some("bin-path-in-archive-marker".to_string()),
            bin_path_in_archive_auto: true,
            bundle_path_in_archive: Some("bundle-path-in-archive-marker".to_string()),
            bundle_install_path: Some(PathBuf::from("/bundle-install-path-marker")),
            show_download_progress: true,
            show_output: false,
            no_confirm: true,
            show_release_notes: false,
            tag_prefix: Some("tag-prefix-marker".to_string()),
            current_version: Some("current-version-marker".to_string()),
            release_tag: Some("release-tag-marker".to_string()),
            auth_token_from_env: true,
            ..Default::default()
        };
        let rendered = format!("{cfg:?}");
        for (field, value) in [
            ("target", "Some(\"target-marker\")"),
            ("asset_identifier", "Some(\"asset-identifier-marker\")"),
            ("bin_name", "Some(\"bin-name-marker\")"),
            ("check_install_path_writable", "true"),
            (
                "bin_path_in_archive",
                "Some(\"bin-path-in-archive-marker\")",
            ),
            ("bin_path_in_archive_auto", "true"),
            (
                "bundle_path_in_archive",
                "Some(\"bundle-path-in-archive-marker\")",
            ),
            ("show_download_progress", "true"),
            ("show_output", "false"),
            ("no_confirm", "true"),
            ("show_release_notes", "false"),
            ("tag_prefix", "Some(\"tag-prefix-marker\")"),
            ("current_version", "Some(\"current-version-marker\")"),
            ("release_tag", "Some(\"release-tag-marker\")"),
            ("auth_token_from_env", "true"),
        ] {
            assert!(
                rendered.contains(&format!("{field}: {value}")),
                "`{field}` must render its own value (`{value}`), got: {rendered}"
            );
        }
        // The two `Option<PathBuf>`s render platform-dependently (`Some("/x")` vs `Some("\\x")`), so
        // pair them by marker substring rather than by an exact literal.
        for (field, marker) in [
            ("bin_install_path", "bin-install-path-marker"),
            ("bundle_install_path", "bundle-install-path-marker"),
        ] {
            let at = rendered
                .find(&format!("{field}: "))
                .unwrap_or_else(|| panic!("`{field}` must be rendered, got: {rendered}"));
            let tail = &rendered[at..];
            let end = tail.find(", ").unwrap_or(tail.len());
            assert!(
                tail[..end].contains(marker),
                "`{field}` must render its own value (`{marker}`), got: {}",
                &tail[..end]
            );
        }
    }

    // --- `host_of`: the input to BOTH the request-time auth gate and the new host warning --------

    // `host_of` decides which host may receive the token (`auth_base_host`) and which host the
    // env-token warning compares against, so its edge cases are security-relevant rather than
    // cosmetic. It had no direct test: the case folding, the port, and the IPv6 brackets were only
    // exercised incidentally through backends configured with a plain `https://host` URL.
    #[test]
    fn host_of_extracts_a_comparable_host() {
        assert_eq!(
            super::host_of("https://api.github.com").as_deref(),
            Some("api.github.com")
        );
        // A path (github enterprise's `/api/v3`) and a port are not part of the host.
        assert_eq!(
            super::host_of("https://github.mycorp.com:8443/api/v3").as_deref(),
            Some("github.mycorp.com"),
            "the port and path must not become part of the host"
        );
        // Lowercased, so the comparisons against a canonical host and against a request URL are
        // both case-insensitive (DNS is).
        assert_eq!(
            super::host_of("https://API.GitHub.COM").as_deref(),
            Some("api.github.com")
        );
        // IPv6 literals lose their brackets, matching what the request-time gate compares.
        assert_eq!(
            super::host_of("https://[::1]:8080/x").as_deref(),
            Some("::1")
        );
        // Userinfo does not leak into the host (it would otherwise never match, silently dropping
        // the token).
        assert_eq!(
            super::host_of("https://user:pw@gitlab.com/x").as_deref(),
            Some("gitlab.com")
        );
    }

    // A scheme-less host (`host("gitlab.mycorp.com")`, an easy mistake given the setter takes an
    // "instance host") still yields a host, because `http::Uri` accepts authority form. That
    // matters for the env-token warning: it still fires for such a host instead of being silently
    // skipped. (The token itself is separately withheld at request time, since the resulting
    // request URL has no `https` scheme.)
    #[test]
    fn host_of_accepts_a_scheme_less_authority() {
        assert_eq!(
            super::host_of("gitlab.mycorp.com").as_deref(),
            Some("gitlab.mycorp.com")
        );
        assert_eq!(
            super::env_token_host_decision(
                true,
                super::host_of("gitlab.mycorp.com").as_deref(),
                NO_EXTRA_HOSTS,
                Some("gitlab.com")
            ),
            EnvTokenDecision::WarnedAndSent,
            "a scheme-less custom host must still be reported for an env-sourced token"
        );
    }

    // A URL with no host at all yields `None`, which disables the token entirely
    // (`auth_allowed_for` requires a host match) *and* silences the canonical-host warning -- a
    // fail-closed pairing: nothing is sent, so there is nothing to warn about.
    #[test]
    fn host_of_is_none_without_a_host() {
        assert_eq!(super::host_of(""), None);
        assert_eq!(super::host_of("/just/a/path"), None);
        assert_eq!(
            super::env_token_host_decision(true, None, NO_EXTRA_HOSTS, Some("gitlab.com")),
            EnvTokenDecision::Sent,
            "no parseable host means no token is sent, so there is nothing to warn about"
        );
    }

    // `name_tag_in_semver_error` names the tag in the message and keeps the original
    // `semver::Error` reachable through the `source()` chain (SemVer -> NonSemverTagError ->
    // semver::Error), so callers walking the chain still find the parse failure.
    #[test]
    fn name_tag_in_semver_error_names_tag_and_keeps_source_chain() {
        let parse_err = "nightly".parse::<semver::Version>().unwrap_err();
        let parse_msg = parse_err.to_string();
        let wrapped =
            super::name_tag_in_semver_error("nightly", crate::errors::Error::from(parse_err));
        let crate::errors::Error::SemVer(inner) = &wrapped else {
            panic!("expected Error::SemVer, got {wrapped:?}");
        };
        assert!(
            inner.to_string().contains("`nightly`"),
            "the message must name the tag, got: {inner}"
        );
        let chained = inner
            .source()
            .expect("the original semver parse error must stay on the chain");
        assert_eq!(chained.to_string(), parse_msg);
    }

    // Non-SemVer errors pass through `name_tag_in_semver_error` unchanged.
    #[test]
    fn name_tag_in_semver_error_passes_other_errors_through() {
        let err = crate::errors::Error::MissingField { field: "version" };
        let out = super::name_tag_in_semver_error("nightly", err);
        assert!(
            matches!(out, crate::errors::Error::MissingField { field: "version" }),
            "non-SemVer errors must pass through unchanged, got {out:?}"
        );
    }

    #[test]
    fn insert_header_records_invalid_value_error() {
        // The setter is infallible; an invalid *value* (control char) is deferred to `check()`
        // as an `Error::InvalidHeader` and the header is not inserted. (Only the invalid-*name* path
        // was tested at the backend level before; this covers the value branch directly.)
        let mut req = RequestConfig::default();
        req.insert_header("x-ok", "bad\nvalue");
        assert!(
            req.headers.get("x-ok").is_none(),
            "an invalid value must not be inserted"
        );
        let err = req
            .check()
            .expect_err("invalid value must surface from check()");
        match err {
            crate::errors::Error::InvalidHeader { source } => {
                assert!(
                    source.to_string().contains("value"),
                    "value-conversion error should mention the value, got: {}",
                    source
                );
            }
            other => panic!("expected Error::InvalidHeader, got {:?}", other),
        }
    }

    #[test]
    fn insert_header_records_invalid_name_error() {
        let mut req = RequestConfig::default();
        req.insert_header("inva lid", "ok");
        assert!(req.headers.get("inva lid").is_none());
        match req
            .check()
            .expect_err("invalid name must surface from check()")
        {
            crate::errors::Error::InvalidHeader { source } => {
                assert!(source.to_string().contains("name"))
            }
            other => panic!("expected Error::InvalidHeader, got {:?}", other),
        }
    }

    #[test]
    fn insert_header_first_error_wins() {
        // First a bad *name*, then a bad *value*. The recorded error must be the first one (name),
        // proving the `header_error.is_none()` guard keeps the earliest failure.
        let mut req = RequestConfig::default();
        req.insert_header("bad name", "ok"); // invalid name -> records "name" error
        req.insert_header("x-ok", "bad\nvalue"); // invalid value -> must NOT overwrite
        match req.check().expect_err("an error is recorded") {
            crate::errors::Error::InvalidHeader { source } => assert!(
                source.to_string().contains("name"),
                "the first (name) error must win, got: {}",
                source
            ),
            other => panic!("expected Error::InvalidHeader, got {:?}", other),
        }
    }

    #[test]
    fn insert_header_valid_then_invalid_still_keeps_valid_header() {
        // A valid header is inserted; a later invalid one is recorded as an error but does not
        // remove the already-inserted valid header.
        let mut req = RequestConfig::default();
        req.insert_header("x-good", "value");
        req.insert_header("x-bad", "bad\nvalue");
        assert_eq!(req.headers.get("x-good").unwrap(), "value");
        assert!(req.check().is_err());
    }

    #[test]
    fn check_is_ok_when_no_error_recorded() {
        let mut req = RequestConfig::default();
        req.insert_header("x-fine", "ok");
        assert!(req.check().is_ok());
        assert_eq!(req.headers.get("x-fine").unwrap(), "ok");
    }

    #[test]
    fn build_requires_current_version_bin_name_and_archive_path() {
        // Nothing set -> `current_version` missing.
        assert!(CommonBuilderConfig::default().build().is_err());

        // `current_version` set, but `bin_name` / `bin_path_in_archive` still missing.
        let cfg = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            ..Default::default()
        };
        assert!(cfg.build().is_err());

        // All required fields present.
        let cfg = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            bin_path_in_archive: Some("app".to_string()),
            ..Default::default()
        };
        let built = cfg.build().expect("all required fields present");
        assert_eq!(built.current_version, "0.1.0");
        assert_eq!(built.bin_name, "app");
    }

    #[test]
    fn build_defaults_and_propagates_update_strategy() {
        // Default is `Compatible`; an explicit `Latest` is carried into the resolved config.
        let base = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            bin_path_in_archive: Some("app".to_string()),
            ..Default::default()
        };
        assert_eq!(
            base.clone().build().unwrap().update_strategy,
            crate::update::UpdateStrategy::Compatible,
            "the default update strategy must be Compatible"
        );

        let latest = CommonBuilderConfig {
            update_strategy: crate::update::UpdateStrategy::Latest,
            ..base
        };
        assert_eq!(
            latest.build().unwrap().update_strategy,
            crate::update::UpdateStrategy::Latest,
            "an explicit Latest strategy must be carried into the resolved config"
        );
    }

    #[test]
    fn build_resolves_target_and_install_path_defaults() {
        let base = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            bin_path_in_archive: Some("app".to_string()),
            ..Default::default()
        };

        // `target` unset -> defaults to the crate build target; install path -> current exe.
        let built = base.clone().build().unwrap();
        assert_eq!(built.target.as_str(), crate::get_target());
        assert!(!built.bin_install_path.as_os_str().is_empty());

        // `target` set -> used verbatim.
        let with_target = CommonBuilderConfig {
            target: Some("custom-target".to_string()),
            ..base
        };
        assert_eq!(with_target.build().unwrap().target, "custom-target");
    }

    // --- bundle mode (BNDL-1) ----------------------------------------------------------------

    // A builder config with the required single-file fields set, as a base for the bundle tests.
    fn bundle_base() -> CommonBuilderConfig {
        CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            // As the `bin_name` setter derives it: auto, not explicit.
            bin_path_in_archive: Some("app".to_string()),
            bin_path_in_archive_auto: true,
            ..Default::default()
        }
    }

    // BNDL-1-2/BNDL-1-3: with an explicit `bundle_install_path`, bundle mode resolves to that path
    // on every platform and carries the archive-side path through to the built config.
    #[test]
    fn build_resolves_bundle_mode_with_an_explicit_install_path() {
        let cfg = CommonBuilderConfig {
            bundle_path_in_archive: Some("MyApp.app".to_string()),
            bundle_install_path: Some(PathBuf::from("/Applications/MyApp.app")),
            ..bundle_base()
        };
        let built = cfg
            .build()
            .expect("an explicit bundle install path must build");
        assert_eq!(built.bundle_path_in_archive.as_deref(), Some("MyApp.app"));
        assert_eq!(
            built.bundle_install_path.as_deref(),
            Some(std::path::Path::new("/Applications/MyApp.app"))
        );
    }

    // Bundle mode is off by default: both resolved fields stay `None`, so the pipeline takes the
    // single-file path.
    #[test]
    fn build_leaves_bundle_fields_none_without_the_setter() {
        let built = bundle_base().build().unwrap();
        assert!(built.bundle_path_in_archive.is_none());
        assert!(built.bundle_install_path.is_none());
    }

    // BNDL-1-4: bundle mode plus an explicit `bin_install_path` is a config conflict, named in the
    // error rather than silently dropping one of the two.
    #[test]
    fn build_rejects_bundle_mode_with_an_explicit_bin_install_path() {
        let cfg = CommonBuilderConfig {
            bundle_path_in_archive: Some("MyApp.app".to_string()),
            bundle_install_path: Some(PathBuf::from("/Applications/MyApp.app")),
            bin_install_path: Some(PathBuf::from("/usr/local/bin/app")),
            ..bundle_base()
        };
        match cfg.build() {
            Err(crate::errors::Error::ConflictingConfig { field, conflict }) => {
                assert_eq!(field, "bundle_path_in_archive");
                assert_eq!(conflict, "bin_install_path");
            }
            other => panic!("expected ConflictingConfig, got {other:?}"),
        }
    }

    // BNDL-1-4: likewise for an explicit `bin_path_in_archive` -- but NOT for the value the
    // `bin_name` setter auto-derives, which is simply unused in bundle mode.
    #[test]
    fn build_rejects_bundle_mode_only_with_an_explicit_bin_path_in_archive() {
        let explicit = CommonBuilderConfig {
            bundle_path_in_archive: Some("MyApp.app".to_string()),
            bundle_install_path: Some(PathBuf::from("/Applications/MyApp.app")),
            bin_path_in_archive: Some("dist/app".to_string()),
            bin_path_in_archive_auto: false,
            ..bundle_base()
        };
        match explicit.build() {
            Err(crate::errors::Error::ConflictingConfig { field, conflict }) => {
                assert_eq!(field, "bundle_path_in_archive");
                assert_eq!(conflict, "bin_path_in_archive");
            }
            other => panic!("expected ConflictingConfig, got {other:?}"),
        }

        let auto = CommonBuilderConfig {
            bundle_path_in_archive: Some("MyApp.app".to_string()),
            bundle_install_path: Some(PathBuf::from("/Applications/MyApp.app")),
            ..bundle_base()
        };
        assert!(
            auto.build().is_ok(),
            "the auto-derived bin_path_in_archive must not count as a conflict"
        );
    }

    // BNDL-1-3: off macOS there is no default bundle install path, so bundle mode without the
    // setter is a missing-field error naming it.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn build_requires_bundle_install_path_off_macos() {
        let cfg = CommonBuilderConfig {
            bundle_path_in_archive: Some("MyApp.app".to_string()),
            ..bundle_base()
        };
        match cfg.build() {
            Err(crate::errors::Error::MissingField { field }) => {
                assert_eq!(field, "bundle_install_path");
            }
            other => panic!("expected MissingField, got {other:?}"),
        }
    }

    // BNDL-1-3: `bundle_path_in_archive` is what selects bundle mode, so `bundle_install_path` set
    // on its own is a missing-field error naming the setter that is absent. Installing a single file
    // to the default path instead would silently discard the caller's install path, the same footgun
    // the bin/bundle conflict check prevents from the other direction.
    #[test]
    fn build_rejects_a_bundle_install_path_without_the_archive_path() {
        let cfg = CommonBuilderConfig {
            bundle_install_path: Some(PathBuf::from("/Applications/MyApp.app")),
            ..bundle_base()
        };
        match cfg.build() {
            Err(crate::errors::Error::MissingField { field }) => {
                assert_eq!(field, "bundle_path_in_archive");
            }
            other => panic!("expected MissingField, got {other:?}"),
        }

        // Neither bundle setter: plain single-file mode, both bundle fields unset.
        let built = bundle_base().build().expect("single-file mode must build");
        assert!(built.bundle_path_in_archive.is_none());
        assert!(built.bundle_install_path.is_none());
    }

    // BNDL-1-5: bundle mode does not relax the shared required fields -- `current_version` and
    // `bin_name` (which names the asset and feeds `{{ bin }}`) are still required.
    #[test]
    fn build_still_requires_current_version_and_bin_name_in_bundle_mode() {
        let no_version = CommonBuilderConfig {
            bundle_path_in_archive: Some("MyApp.app".to_string()),
            bundle_install_path: Some(PathBuf::from("/Applications/MyApp.app")),
            current_version: None,
            ..bundle_base()
        };
        match no_version.build() {
            Err(crate::errors::Error::MissingField { field }) => {
                assert_eq!(field, "current_version");
            }
            other => panic!("expected MissingField(current_version), got {other:?}"),
        }

        let no_bin_name = CommonBuilderConfig {
            bundle_path_in_archive: Some("MyApp.app".to_string()),
            bundle_install_path: Some(PathBuf::from("/Applications/MyApp.app")),
            bin_name: None,
            // No `bin_name` setter call means no auto-derived archive path either.
            bin_path_in_archive: None,
            bin_path_in_archive_auto: false,
            ..bundle_base()
        };
        match no_bin_name.build() {
            Err(crate::errors::Error::MissingField { field }) => {
                assert_eq!(field, "bin_name");
            }
            other => panic!("expected MissingField(bin_name), got {other:?}"),
        }
    }

    // BNDL-1-4: the conflict is reported before the install path is resolved, so a caller who set
    // both gets the actionable "these two setters conflict" error rather than a
    // missing/undetectable-`bundle_install_path` error from the default resolution (which on macOS
    // would even consult `current_exe()` first).
    #[test]
    fn build_reports_the_conflict_before_resolving_the_install_path() {
        let cfg = CommonBuilderConfig {
            bundle_path_in_archive: Some("MyApp.app".to_string()),
            bin_install_path: Some(PathBuf::from("/usr/local/bin/app")),
            ..bundle_base()
        };
        match cfg.build() {
            Err(crate::errors::Error::ConflictingConfig { field, conflict }) => {
                assert_eq!(field, "bundle_path_in_archive");
                assert_eq!(conflict, "bin_install_path");
            }
            other => panic!("expected ConflictingConfig, got {other:?}"),
        }
    }

    // --- Item 5: self-fixing error messages --------------------------------------------------

    #[test]
    fn build_error_message_names_the_setter_for_current_version() {
        let err = CommonBuilderConfig::default().build().unwrap_err();
        match err {
            crate::errors::Error::MissingField { field } => {
                assert_eq!(
                    field, "current_version",
                    "the missing-field error must name `current_version`, got: {}",
                    field
                );
            }
            other => panic!("expected Error::MissingField, got {:?}", other),
        }
    }

    // --- CORP-1: custom root CA certificates ------------------------------------------------

    #[test]
    fn build_client_with_no_certs_leaves_client_none() {
        // With no `root_certificates`, `build_client` is a no-op: it must not attempt any client
        // construction and must leave `client` as `None` (the crate default path stays in effect).
        let mut req = RequestConfig::default();
        req.build_client();
        assert!(
            req.client.is_none(),
            "no certs => build_client must not materialize a client"
        );
        assert!(req.cert_error.is_none(), "no certs => no cert_error");
    }

    #[test]
    fn build_client_with_injected_client_skips_cert_build() {
        // An injected client wins: even with a (garbage) cert present, `build_client` must NOT try to
        // build a client over it, so no `cert_error` is recorded and the injected client is kept.
        //
        // Cert materialization is per slot, so with `async` enabled the async slot must be injected
        // too. Filling only the sync slot leaves the async one to build from the garbage certs and
        // record the error, which is what
        // `build_client_injected_sync_still_builds_async_from_certs` pins.
        struct DummyClient;
        impl crate::http_client::HttpClient for DummyClient {
            fn get(
                &self,
                _url: &str,
                _headers: &crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> crate::Result<Box<dyn crate::http_client::HttpResponse>> {
                unreachable!("not called in this test")
            }
        }
        #[cfg(feature = "async")]
        struct DummyAsyncClient;
        #[cfg(feature = "async")]
        impl crate::http_client::AsyncHttpClient for DummyAsyncClient {
            fn get<'a>(
                &'a self,
                _url: &'a str,
                _headers: &'a crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> futures_util::future::BoxFuture<
                'a,
                crate::Result<Box<dyn crate::http_client::AsyncHttpResponse>>,
            > {
                unreachable!("not called in this test")
            }
        }
        let mut req = RequestConfig {
            client: Some(std::sync::Arc::new(DummyClient)),
            #[cfg(feature = "async")]
            async_client: Some(std::sync::Arc::new(DummyAsyncClient)),
            ..Default::default()
        };
        req.root_certificates
            .push(crate::tls::Certificate::from_pem(b"garbage".to_vec()));
        req.build_client();
        assert!(
            req.cert_error.is_none(),
            "an injected client must short-circuit the sync cert build (no cert_error)"
        );
        assert!(req.client.is_some(), "the injected client must be kept");
    }

    // Cert materialization is per slot: when a sync client is injected but the async slot is empty,
    // the async cert-build still runs, so garbage cert bytes surface as a cert_error even though the
    // injected sync client is valid. Injecting one slot does not suppress the other.
    #[cfg(feature = "async")]
    #[test]
    fn build_client_injected_sync_still_builds_async_from_certs() {
        // Per-slot cert materialization: injecting a sync client does NOT skip the async slot's
        // cert-build. The injected sync client is kept as-is, but the async client is still built
        // from the custom roots (so async listing trusts the CA). With garbage cert bytes the async
        // build fails, which is recorded in cert_error -- proving the async slot ran.
        struct DummyClient;
        impl crate::http_client::HttpClient for DummyClient {
            fn get(
                &self,
                _url: &str,
                _headers: &crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> crate::Result<Box<dyn crate::http_client::HttpResponse>> {
                unreachable!("not called in this test")
            }
        }
        let mut req = RequestConfig {
            client: Some(std::sync::Arc::new(DummyClient)),
            ..Default::default()
        };
        req.root_certificates
            .push(crate::tls::Certificate::from_pem(BAD_PEM_CERT.to_vec()));
        req.build_client();
        assert!(
            req.cert_error.is_some(),
            "the async slot must attempt the cert-build even when a sync client is injected"
        );
        assert!(
            req.client.is_some(),
            "the injected sync client must be kept as-is"
        );
    }

    // The bad-cert path only records an error when a real client backend can attempt (and reject)
    // the parse. With neither client feature, `client_with_root_certs` returns the
    // "no HTTP client feature enabled" error instead, which still populates `cert_error`.
    #[cfg(any(feature = "reqwest", feature = "ureq"))]
    #[test]
    fn build_client_bad_cert_records_cert_error() {
        // A malformed cert with no injected client: `build_client` asks the active backend to build
        // a client, the parse fails, and the error is recorded in `cert_error`. The two backends
        // reject different malformed inputs, so the bad bytes are selected to match the same backend
        // `client_with_root_certs` dispatches to (reqwest preferred when both features are on):
        //   - reqwest validates at client-build time, accepting PEM framing but rejecting a body that
        //     decodes to non-X.509-DER bytes (base64 of "not a valid cert").
        //   - ureq validates the PEM framing in `from_pem` (deferring DER), so it rejects bytes that
        //     contain no PEM certificate at all.
        #[cfg(feature = "reqwest")]
        let bad_cert = crate::tls::Certificate::from_pem(
            b"-----BEGIN CERTIFICATE-----\nbm90IGEgdmFsaWQgY2VydA==\n-----END CERTIFICATE-----\n"
                .to_vec(),
        );
        #[cfg(all(feature = "ureq", not(feature = "reqwest")))]
        let bad_cert = crate::tls::Certificate::from_pem(b"not a pem certificate".to_vec());

        let mut req = RequestConfig::default();
        req.root_certificates.push(bad_cert);
        req.build_client();
        assert!(
            req.cert_error.is_some(),
            "a malformed cert must record a cert_error"
        );
        assert!(
            req.client.is_none(),
            "a failed cert build must not leave a client"
        );
    }

    #[test]
    fn check_surfaces_cert_error_as_invalid_certificate() {
        // A recorded `cert_error` (and no header error) surfaces from `check()` as
        // `Error::InvalidCertificate` carrying the stored message via `source()`.
        let req = RequestConfig {
            cert_error: Some("boom".to_string()),
            ..Default::default()
        };
        match req
            .check()
            .expect_err("cert_error must surface from check()")
        {
            crate::errors::Error::InvalidCertificate { source } => {
                assert_eq!(source.to_string(), "boom")
            }
            other => panic!("expected Error::InvalidCertificate, got {:?}", other),
        }
    }

    #[test]
    fn check_surfaces_header_error_before_cert_error() {
        // When BOTH a header error and a cert error are present, `check()` must report the header
        // error (`Error::InvalidHeader`) first: header validation takes precedence.
        let req = RequestConfig {
            header_error: Some("bad header".to_string()),
            cert_error: Some("bad cert".to_string()),
            ..Default::default()
        };
        match req.check().expect_err("an error must surface") {
            crate::errors::Error::InvalidHeader { .. } => {}
            other => panic!("expected Error::InvalidHeader to win, got {:?}", other),
        }
    }

    // --- CORP-3: programmatic proxy ----------------------------------------------------------

    // spec: CORP-3-2, CORP-3-4
    #[test]
    fn build_client_with_a_proxy_materializes_a_client() {
        // A proxy alone (no certificates) must be enough to make `build()` bake a client: before
        // CORP-3 the only trigger was `root_certificates`, so a proxy-only config would silently
        // fall through to the per-call default client and never proxy anything.
        let mut req = RequestConfig {
            proxy: Some("http://corpuser:hunter2@proxy.corp:8080".to_string()),
            ..Default::default()
        };
        req.build_client();
        assert!(
            req.client.is_some(),
            "a configured proxy must materialize a client"
        );
        assert!(
            req.proxy_error.is_none() && req.cert_error.is_none(),
            "a valid proxy must not record an error, got proxy_error={:?} cert_error={:?}",
            req.proxy_error,
            req.cert_error
        );
    }

    // spec: CORP-3-3
    #[cfg(any(feature = "reqwest", feature = "ureq"))]
    #[test]
    fn build_client_bad_proxy_records_proxy_error_not_cert_error() {
        // An unparseable proxy URL must be recorded in `proxy_error`, NOT in `cert_error`: the
        // caller supplied no certificate at all, and telling them their certificate is invalid
        // sends them to the wrong setter. The recorded message must not carry the password.
        let mut req = RequestConfig {
            proxy: Some("http://corpuser:hunter2@ not a proxy url".to_string()),
            ..Default::default()
        };
        req.build_client();
        let recorded = req
            .proxy_error
            .as_deref()
            .expect("an unparseable proxy URL must record a proxy_error");
        assert!(
            !recorded.contains("hunter2") && recorded.contains("REDACTED"),
            "the recorded proxy error must be redacted, got: {recorded}"
        );
        assert!(
            req.cert_error.is_none(),
            "a proxy failure must not be misreported as a certificate failure"
        );
        assert!(
            req.client.is_none(),
            "a failed proxy build must not leave a client"
        );
    }

    // spec: CORP-3-3
    #[test]
    fn check_surfaces_proxy_error_as_invalid_proxy() {
        // A recorded `proxy_error` (and no header/cert error) surfaces from `check()` as
        // `Error::InvalidProxy` carrying the stored message via `source()`.
        let req = RequestConfig {
            proxy_error: Some("boom".to_string()),
            ..Default::default()
        };
        match req
            .check()
            .expect_err("proxy_error must surface from check()")
        {
            crate::errors::Error::InvalidProxy { source } => {
                assert_eq!(source.to_string(), "boom")
            }
            other => panic!("expected Error::InvalidProxy, got {:?}", other),
        }
    }

    // spec: CORP-3-6
    #[test]
    fn build_client_with_injected_client_skips_the_proxy_build() {
        // An injected client owns its own proxy config, so a configured proxy must not cause a
        // replacement client to be built over it -- even an unparseable one, which would otherwise
        // record an error for a knob that has no effect on this transport.
        struct DummyClient;
        impl crate::http_client::HttpClient for DummyClient {
            fn get(
                &self,
                _url: &str,
                _headers: &crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> crate::Result<Box<dyn crate::http_client::HttpResponse>> {
                unreachable!("not called in this test")
            }
        }
        #[cfg(feature = "async")]
        struct DummyAsyncClient;
        #[cfg(feature = "async")]
        impl crate::http_client::AsyncHttpClient for DummyAsyncClient {
            fn get<'a>(
                &'a self,
                _url: &'a str,
                _headers: &'a crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> futures_util::future::BoxFuture<
                'a,
                crate::Result<Box<dyn crate::http_client::AsyncHttpResponse>>,
            > {
                unreachable!("not called in this test")
            }
        }
        let mut req = RequestConfig {
            client: Some(std::sync::Arc::new(DummyClient)),
            #[cfg(feature = "async")]
            async_client: Some(std::sync::Arc::new(DummyAsyncClient)),
            proxy: Some("http://corpuser:hunter2@ not a proxy url".to_string()),
            ..Default::default()
        };
        req.build_client();
        assert!(
            req.proxy_error.is_none(),
            "an injected client must short-circuit the proxy build"
        );
        assert!(req.client.is_some(), "the injected client must be kept");
    }

    #[test]
    fn build_error_message_names_the_setter_for_bin_name() {
        let err = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            ..Default::default()
        }
        .build()
        .unwrap_err();
        match err {
            crate::errors::Error::MissingField { field } => {
                assert_eq!(
                    field, "bin_name",
                    "the missing-field error must name `bin_name`, got: {}",
                    field
                );
            }
            other => panic!("expected Error::MissingField, got {:?}", other),
        }
    }

    #[test]
    fn build_error_message_names_the_setter_for_bin_path_in_archive() {
        // With current_version and bin_name both set, the only remaining required field is
        // bin_path_in_archive. Verify the error names that field specifically.
        let err = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            ..Default::default()
        }
        .build()
        .unwrap_err();
        match err {
            crate::errors::Error::MissingField { field } => {
                assert_eq!(
                    field, "bin_path_in_archive",
                    "the missing-field error must name `bin_path_in_archive`, got: {}",
                    field
                );
            }
            other => panic!("expected Error::MissingField, got {:?}", other),
        }
    }

    // --- apply_auth: auth-header derivation --------------------------------------------------

    #[test]
    fn apply_auth_no_token_is_noop() {
        // With no auth_token set, apply_auth must not insert any Authorization header.
        let req = RequestConfig::default();
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "apply_auth with no token must not insert an Authorization header"
        );
    }

    #[test]
    fn apply_auth_token_scheme_inserts_authorization_header() {
        // With auth_token set and the default Token scheme, apply_auth must insert
        // "Authorization: token <token>" for a request to the configured API host.
        let req = RequestConfig {
            auth_token: Some("mytoken".to_string()),
            auth_base_host: Some("api.example.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.example.com/x", &mut headers)
            .unwrap();
        let auth = headers
            .get(crate::http_client::header::AUTHORIZATION)
            .expect("apply_auth must insert an Authorization header when a token is set");
        assert_eq!(
            auth, "token mytoken",
            "Token scheme must render as 'token <token>'"
        );
    }

    #[test]
    fn apply_auth_user_supplied_authorization_header_wins() {
        // When the user sets their own Authorization header via `request_header`, apply_auth
        // must see it in self.headers and return early without inserting the crate's token into
        // the passed-in headers map. The crate must never overwrite the user's own auth.
        let mut req = RequestConfig {
            auth_token: Some("should-not-appear".to_string()),
            ..Default::default()
        };
        req.insert_header(
            crate::http_client::header::AUTHORIZATION,
            "custom my-custom-token",
        );
        let mut out_headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.example.com/x", &mut out_headers)
            .unwrap();
        assert!(
            out_headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "apply_auth must not insert its token when the user supplied their own Authorization"
        );
    }

    #[test]
    fn apply_auth_bearer_scheme_renders_bearer_prefix() {
        // The Bearer auth scheme (gitlab) must render as "Bearer <token>".
        // AuthScheme::Bearer is always compiled in (the allow(dead_code) attr only suppresses
        // the lint warning in non-gitlab builds), so this test is valid across all feature sets.
        let req = RequestConfig {
            auth_token: Some("mytoken".to_string()),
            auth_scheme: super::AuthScheme::Bearer,
            auth_base_host: Some("api.example.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.example.com/x", &mut headers)
            .unwrap();
        let auth = headers
            .get(crate::http_client::header::AUTHORIZATION)
            .expect("apply_auth must insert an Authorization header");
        assert_eq!(
            auth, "Bearer mytoken",
            "Bearer scheme must render as 'Bearer <token>'"
        );
    }

    #[test]
    fn apply_auth_invalid_token_surfaces_invalid_auth_token_error() {
        // A token that contains a control character (newline) cannot be encoded as an HTTP
        // header value. apply_auth must surface this as Error::InvalidAuthToken, not panic.
        let req = RequestConfig {
            auth_token: Some("bad\ntoken".to_string()),
            auth_base_host: Some("api.example.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        match req.apply_auth("https://api.example.com/x", &mut headers) {
            Err(crate::errors::Error::InvalidAuthToken { .. }) => {}
            other => panic!(
                "expected Error::InvalidAuthToken for a token with a newline, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn apply_auth_not_attached_to_cross_origin_url() {
        // The token must NOT be attached to a request whose host differs from the configured API
        // host. A malicious release server that sets the asset download_url (or a Link next-page
        // URL) to its own host must not receive the credential.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("api.github.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://evil.example.com/x.tar.gz", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "the token must not be attached to a cross-origin URL"
        );
    }

    #[test]
    fn apply_auth_not_attached_over_plaintext_http() {
        // The token must NOT be sent over plaintext http to a non-loopback host, even when the host
        // matches the configured API host (guards against a downgraded/misconfigured URL).
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("api.example.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("http://api.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "the token must not be sent over plaintext http to a non-loopback host"
        );
    }

    #[test]
    fn apply_auth_attached_to_allow_auth_host() {
        // A host the user explicitly authorized via allow_auth_host receives the token even though
        // it differs from the API base host.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("api.example.com".to_string()),
            auth_hosts: vec!["cdn.example.com".to_string()],
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://cdn.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_some(),
            "an allow_auth_host entry must receive the token"
        );
    }

    #[test]
    fn apply_auth_over_http_when_insecure_forwarding_allowed() {
        // With the escape hatch set, the token is attached over plain http to a host-matched
        // (non-loopback) request.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("internal.example.com".to_string()),
            allow_insecure_auth: true,
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("http://internal.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_some(),
            "the escape hatch must allow the token over http to a host-matched request"
        );
    }

    #[test]
    fn apply_auth_insecure_flag_still_requires_host_match() {
        // The escape hatch only lifts the https requirement; a cross-origin host still gets no token.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("internal.example.com".to_string()),
            allow_insecure_auth: true,
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("http://evil.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "the escape hatch must not attach the token to a cross-origin host"
        );
    }

    #[test]
    fn apply_auth_attached_to_loopback_over_http() {
        // Loopback hosts may use plain http (local mirrors and the loopback test stubs), provided
        // the host matches the configured base.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("127.0.0.1".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("http://127.0.0.1:8080/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_some(),
            "a loopback host matching the base must receive the token over http"
        );
    }

    // --- build() auth propagation ------------------------------------------------------------

    #[test]
    fn build_propagates_auth_token_and_scheme_to_request_config() {
        // CommonBuilderConfig::build() copies auth_token and auth_scheme from the builder into
        // the resolved RequestConfig so the shared apply_auth path can use them on both the
        // listing and download paths.
        let cfg = CommonBuilderConfig {
            current_version: Some("1.0.0".to_string()),
            bin_name: Some("mybin".to_string()),
            bin_path_in_archive: Some("mybin".to_string()),
            auth_token: Some("secrettoken".to_string()),
            auth_scheme: super::AuthScheme::Bearer,
            ..Default::default()
        };
        let built = cfg.build().expect("valid config must build");
        assert_eq!(
            built.request.auth_token.as_deref(),
            Some("secrettoken"),
            "build() must copy auth_token into request.auth_token"
        );
        assert_eq!(
            built.request.auth_scheme,
            super::AuthScheme::Bearer,
            "build() must copy auth_scheme into request.auth_scheme"
        );
    }

    // --- Symmetric async-only injected client ------------------------------------------------

    // Per-slot: injecting only an async client leaves the sync slot empty, so build_client still
    // builds the sync client from the custom roots (garbage cert -> cert_error set). The injected
    // async client is kept as-is.
    #[cfg(feature = "async")]
    #[test]
    fn build_client_injected_async_only_still_builds_sync_from_certs() {
        struct DummyAsyncClient;
        impl crate::http_client::AsyncHttpClient for DummyAsyncClient {
            fn get<'a>(
                &'a self,
                _url: &'a str,
                _headers: &'a crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> futures_util::future::BoxFuture<
                'a,
                crate::Result<Box<dyn crate::http_client::AsyncHttpResponse>>,
            > {
                unreachable!("not called in this test")
            }
        }
        let mut req = RequestConfig {
            async_client: Some(std::sync::Arc::new(DummyAsyncClient)),
            ..Default::default()
        };
        req.root_certificates
            .push(crate::tls::Certificate::from_pem(BAD_PEM_CERT.to_vec()));
        req.build_client();
        assert!(
            req.cert_error.is_some(),
            "the sync slot must attempt the cert-build even when an async client is injected"
        );
        assert!(
            req.async_client.is_some(),
            "the injected async client must be kept as-is"
        );
    }

    // --- End-to-end: CommonBuilderConfig::build with all slots injected + garbage cert ----------

    #[test]
    fn common_builder_config_build_with_injected_clients_skips_cert_error() {
        // CommonBuilderConfig::build() calls build_client() internally. When every compiled client
        // slot is injected, no slot needs building, so a garbage cert produces no cert_error and
        // build() succeeds.
        struct DummyClient;
        impl crate::http_client::HttpClient for DummyClient {
            fn get(
                &self,
                _url: &str,
                _headers: &crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> crate::Result<Box<dyn crate::http_client::HttpResponse>> {
                unreachable!("not called in this test")
            }
        }
        #[cfg(feature = "async")]
        struct DummyAsyncClient;
        #[cfg(feature = "async")]
        impl crate::http_client::AsyncHttpClient for DummyAsyncClient {
            fn get<'a>(
                &'a self,
                _url: &'a str,
                _headers: &'a crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> futures_util::future::BoxFuture<
                'a,
                crate::Result<Box<dyn crate::http_client::AsyncHttpResponse>>,
            > {
                unreachable!("not called in this test")
            }
        }
        let mut builder = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            bin_path_in_archive: Some("app".to_string()),
            ..Default::default()
        };
        builder.request.client = Some(std::sync::Arc::new(DummyClient));
        #[cfg(feature = "async")]
        {
            builder.request.async_client = Some(std::sync::Arc::new(DummyAsyncClient));
        }
        builder
            .request
            .root_certificates
            .push(crate::tls::Certificate::from_pem(b"garbage".to_vec()));
        // build() must succeed because every compiled client slot is injected.
        let config = builder
            .build()
            .expect("injected clients must prevent cert_error from blocking build");
        assert!(
            config.request.cert_error.is_none(),
            "cert_error must be None when all client slots were injected"
        );
        assert!(
            config.request.client.is_some(),
            "the injected client must be present in the resolved config"
        );
    }
}
