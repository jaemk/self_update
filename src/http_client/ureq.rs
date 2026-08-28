#![cfg(feature = "ureq")]

use std::time::Duration;

use ureq::tls::TlsProvider;
use ureq::{Agent, Body, http::Response};

use super::{HeaderMap, HttpClient, HttpResponse};
use crate::errors::{status_to_error, status_to_error_with_headers};
use crate::{Error, Result};

/// The certificate set a crate-built ureq agent trusts. `Vec<Certificate<'static>>` so the roots
/// outlive the per-call agent built from them.
#[cfg(any(not(feature = "reqwest"), test))]
type UreqRootCerts = std::sync::Arc<Vec<ureq::tls::Certificate<'static>>>;

/// How a [`UreqClient`] obtains the agent for each request.
enum UreqInner {
    /// Build a fresh per-call agent honoring the per-request timeout, the TLS feature, and proxy-env.
    Default,
    /// A user-injected agent (via `From<ureq::Agent>` / the `ureq_agent` setter) that owns its own
    /// timeout/TLS/proxy config, so the per-request timeout is *not* applied to it.
    Injected(Agent),
    /// Build a fresh per-call agent (like [`Default`](UreqInner::Default), so it still honors the
    /// per-request timeout and proxy-env) that trusts these custom root certificates.
    #[cfg(any(not(feature = "reqwest"), test))]
    Certs(UreqRootCerts),
}

/// Sync [`HttpClient`] backed by a `ureq::Agent`.
pub struct UreqClient(UreqInner);

impl Default for UreqClient {
    fn default() -> Self {
        Self(UreqInner::Default)
    }
}

impl From<Agent> for UreqClient {
    fn from(agent: Agent) -> Self {
        Self(UreqInner::Injected(agent))
    }
}

/// Build a per-call ureq agent honoring the per-request `timeout`, the TLS feature, and proxy-env.
/// `root_certs`, when `Some`, replaces the default trust store with the supplied certificates.
fn build_call_agent(
    timeout: Option<Duration>,
    #[cfg(any(not(feature = "reqwest"), test))] root_certs: Option<UreqRootCerts>,
) -> Agent {
    use ureq::tls::TlsConfig;
    // When both TLS features are enabled, rustls wins (it is the crate default); otherwise fall
    // back to native-tls (also the case when no TLS feature is set).
    #[cfg(feature = "rustls")]
    let provider = TlsProvider::Rustls;
    #[cfg(not(feature = "rustls"))]
    let provider = TlsProvider::NativeTls;

    #[cfg(any(not(feature = "reqwest"), test))]
    let mut tls = TlsConfig::builder().provider(provider);
    #[cfg(all(feature = "reqwest", not(test)))]
    let tls = TlsConfig::builder().provider(provider);
    #[cfg(any(not(feature = "reqwest"), test))]
    if let Some(certs) = root_certs {
        tls = tls.root_certs(ureq::tls::RootCerts::Specific(certs));
    }
    let config = Agent::config_builder()
        .tls_config(tls.build())
        .timeout_global(timeout)
        // Honor HTTP(S)_PROXY / NO_PROXY env vars (reqwest does this automatically).
        .proxy(ureq::Proxy::try_from_env())
        // Disable ureq's built-in status-error so we reach our own is_success() check, which has
        // the response headers in hand and maps the status through `status_to_error_with_headers`
        // (NotFound / Unauthorized / RateLimited / HttpStatus).
        .http_status_as_error(false)
        .build();
    Agent::new_with_config(config)
}

// `client_with_root_certs` only dispatches to the ureq builder when reqwest is NOT also enabled
// (reqwest wins, exactly like `default_client`), so this is dead in a both-features lib build. Gate
// it to the lanes that actually reach it (and to `test`, where the ureq cert test exercises it).
#[cfg(any(not(feature = "reqwest"), test))]
impl UreqClient {
    /// Build a UreqClient that trusts the supplied custom root CA certificates.
    ///
    /// The certificates are parsed and validated here (a malformed PEM certificate returns `Err`);
    /// the agent itself is built per request in [`get`](HttpClient::get), so it still honors the
    /// per-request timeout and proxy-env. `RootCerts::Specific` replaces the default trust store, so
    /// only the supplied certificates are trusted (see the `add_root_certificate` docs).
    pub(crate) fn build_with_certs(
        certs: &[crate::tls::Certificate],
    ) -> std::result::Result<
        std::sync::Arc<dyn crate::http_client::HttpClient>,
        crate::http_client::ClientBuildError,
    > {
        let mut ureq_certs = Vec::with_capacity(certs.len());
        for cert in certs {
            let c = if cert.is_pem() {
                ureq::tls::Certificate::from_pem(cert.bytes())
                    .map_err(|e| format!("invalid PEM certificate: {e}"))?
            } else {
                // `from_der` is infallible in ureq; invalid DER bytes are surfaced at connection
                // time, not here (documented on `Certificate::from_der` / `add_root_certificate`).
                ureq::tls::Certificate::from_der(cert.bytes())
            };
            ureq_certs.push(c.to_owned());
        }
        Ok(std::sync::Arc::new(UreqClient(UreqInner::Certs(
            std::sync::Arc::new(ureq_certs),
        ))))
    }
}

/// Whether the per-request `http_status_as_error(false)` override in [`get`](HttpClient::get) is
/// needed for this call. Only an injected agent whose OWN config still has
/// `http_status_as_error(true)` (ureq's default) needs it. A caller who already disabled it at the
/// agent level needs no override -- skipping it there avoids the request-level TLS-config-cache
/// cost documented at the call site, and it is the only lane that keeps that agent's own TLS cache
/// warm across requests. A crate-built (non-injected) agent never takes this path at all, since it
/// is always built with `http_status_as_error(false)` (see `build_call_agent`).
fn needs_status_override(is_injected: bool, agent: &Agent) -> bool {
    is_injected && agent.config().http_status_as_error()
}

impl HttpClient for UreqClient {
    fn get(
        &self,
        url: &str,
        headers: &HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<Box<dyn HttpResponse>> {
        // An injected agent owns its own timeout/TLS/proxy config, so the per-request `timeout` is
        // only applied to the crate-built (Default / Certs) agents.
        let built_agent;
        let (agent, is_injected): (&Agent, bool) = match &self.0 {
            UreqInner::Injected(agent) => (agent, true),
            UreqInner::Default => {
                built_agent = build_call_agent(
                    timeout,
                    #[cfg(any(not(feature = "reqwest"), test))]
                    None,
                );
                (&built_agent, false)
            }
            #[cfg(any(not(feature = "reqwest"), test))]
            UreqInner::Certs(certs) => {
                built_agent = build_call_agent(timeout, Some(certs.clone()));
                (&built_agent, false)
            }
        };
        let mut req = agent.get(url);
        // Skip the override entirely when the injected agent's own config already disables
        // ureq's status-error (see the cost this avoids, described below) -- a free win for that
        // class of callers, and it makes this a genuinely distinct code path from the one below.
        if needs_status_override(is_injected, agent) {
            // An injected agent that still has ureq's default `http_status_as_error(true)` would
            // turn a non-2xx into a headerless `ureq::Error::StatusCode`. ureq 3 supports overriding
            // that per request, so apply the override here: the response then reaches the
            // header-aware check at the bottom of `get` and every client lane classifies a non-2xx
            // identically. Nothing about the *values* of the injected agent's own timeout/TLS/proxy
            // config is touched by this -- but the override is not free. Verified against the locked
            // ureq 3.3.0 (`Cargo.lock`): `RequestBuilder::config()` inserts a `RequestLevelConfig`
            // extension into the request, which `run()` (`run.rs:37-41`) reads back as
            // `ConnectionDetails::request_level = true` for that connection. Both TLS backends'
            // connectors (`tls/rustls.rs:97-114`, `tls/native_tls.rs:86-103`) refuse to *populate*
            // the agent's `OnceLock` TLS-config cache on a request-level connection -- they may only
            // reuse a value an agent-level connection already cached. An agent injected solely for
            // this crate never makes an agent-level connection, so this branch rebuilds the
            // `ClientConfig` / `TlsConnector` (root store included) on every new HTTPS connection.
            // ureq 3 exposes no way to derive a modified `Config` from an injected agent other than
            // this per-request override, so the cost is unavoidable for a caller who wants
            // `http_status_as_error(true)` from an injected agent; the `&&` above is the only lane
            // that avoids it (and, as a side effect, lets that agent's own TLS cache stay warm).
            req = req.config().http_status_as_error(false).build();
        }

        for (key, value) in headers.iter() {
            req = req.header(key, value);
        }

        let res = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) if is_injected => {
                // Deliberately-kept dead code, in both lanes above. When the override in `get` runs
                // (agent had `http_status_as_error(true)`), it forces the *effective* config for this
                // connection to `false` (`run.rs:128-129` only raises `StatusCode` when
                // `config.http_status_as_error()` is true), so this arm is not expected to fire; if a
                // future ureq ignores the request-level override, map the bare code rather than
                // surfacing it as an opaque transport error. When the override is *skipped* (agent
                // already had `http_status_as_error(false)`), `run()` falls back to the agent-level
                // config (`run.rs:37-41`), which is that same `false` -- so this arm stays unreachable
                // there too, for the same reason, not because the skip reopened it. `StatusCode`
                // carries no headers, so this path cannot see a spent quota on a 403 (it stays
                // `Unauthorized`) and a 429 classifies as `RateLimited` carrying no wait -- the
                // status alone is the rate-limit signal (RFC 6585).
                return Err(status_to_error(code, url));
            }
            Err(e) => return Err(Error::Transport(Box::new(e))),
        };

        if !res.status().is_success() {
            return Err(status_to_error_with_headers(
                res.status().as_u16(),
                url,
                res.headers(),
            ));
        }

        Ok(Box::new(res))
    }
}

impl HttpResponse for Response<Body> {
    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        Response::headers(self)
    }

    fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
        Box::new((*self).into_body().into_reader())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// Serve a single HTTP response (the given status line + a short body) over a fresh loopback
    /// listener, then close. Returns the base URL (`http://127.0.0.1:<port>/`). No external network.
    /// Thin wrapper over [`stub_with_headers`] with no extra headers, so there is one stub server.
    fn stub(status: &'static str) -> String {
        stub_with_headers(status, "")
    }

    /// Default-agent path (`is_injected == false`): the per-call agent is built with
    /// `http_status_as_error(false)`, so a non-2xx response is returned to our own `is_success()`
    /// check at the bottom of `get`, which routes it through `status_to_error_with_headers`.
    fn get_default(status: &'static str) -> Error {
        let client = UreqClient::default();
        let base = stub(status);
        client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err")
    }

    /// Injected-agent path (`is_injected == true`): a user-supplied default `ureq::Agent` has
    /// `http_status_as_error == true` (the ureq default), but `get` overrides that *per request*, so
    /// a non-2xx is returned as a response and routed through the same header-aware
    /// `status_to_error_with_headers` check as the crate-built agents.
    fn get_injected(status: &'static str) -> Error {
        let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
        let client = UreqClient::from(agent);
        let base = stub(status);
        client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err")
    }

    /// Injected agent built with `http_status_as_error(false)` (the OTHER injected case): the user
    /// disabled ureq's status-error at the agent level too, so the per-request override in `get` is
    /// a no-op and `call()` returns `Ok(res)` on a non-2xx. Control reaches the same bottom-of-`get`
    /// `!res.status().is_success()` check, routing the status through `status_to_error_with_headers`.
    fn get_injected_no_status_error(status: &'static str) -> Error {
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let client = UreqClient::from(agent);
        let base = stub(status);
        client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err")
    }

    /// Like [`stub`], but injects `extra_headers` (a pre-formatted `Name: value\r\n` block), so a
    /// response can carry the rate-limit headers a spent quota comes with. Takes a plain `&str`
    /// (copied into the serving thread) so a header block can carry a value derived at runtime, e.g.
    /// a reset epoch relative to *now*.
    fn stub_with_headers(status: &'static str, extra_headers: &str) -> String {
        let extra_headers = extra_headers.to_string();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "err";
                let out = format!(
                    "HTTP/1.1 {}\r\nContent-Type: text/plain\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    extra_headers,
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        base
    }

    /// The `x-ratelimit-reset` / `RateLimit-Reset` header value for a window resetting
    /// `offset_secs` from *now*, as the unix timestamp those headers carry.
    ///
    /// Derived rather than hardcoded: a fixed epoch silently ages into the past, and the classifier
    /// keeps a past instant (it just renders no wait), so tests pinned to one stop exercising the
    /// future-window path without ever failing. `offset_secs` must stay under the 24h ceiling
    /// (`MAX_RATE_LIMIT_WAIT`), past which a reset instant is rejected as `None`.
    fn reset_epoch(offset_secs: u64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after the unix epoch")
            .as_secs();
        (now + offset_secs).to_string()
    }

    /// The header block for a spent primary quota (`remaining: 0`) whose window resets in
    /// [`RESET_WINDOW`] seconds, in the `x-ratelimit-*` (github/gitea/gitee) spelling.
    fn spent_quota_headers() -> String {
        format!(
            "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: {}\r\n",
            reset_epoch(RESET_WINDOW)
        )
    }

    /// The reset window every derived-epoch test serves: comfortably inside the 24h ceiling, and
    /// long enough that the remaining wait cannot round down to zero on a loaded CI box.
    const RESET_WINDOW: u64 = 600;

    /// Assert that `err` is a `RateLimited` whose `reset_at` resolves to a *positive* wait bounded by
    /// the window that was served -- the assertion `reset_at: Some(_)` cannot make, since a reset
    /// instant in the past is kept as `Some` and yields no wait at all.
    fn assert_wait_within_window(err: &Error, context: &str) {
        let wait = err
            .rate_limit_delay()
            .unwrap_or_else(|| panic!("{context}: a future reset instant must yield a wait"));
        assert!(
            wait > Duration::from_secs(RESET_WINDOW / 2)
                && wait <= Duration::from_secs(RESET_WINDOW),
            "{context}: the wait must be derived from the served future window (~{RESET_WINDOW}s), \
             got {wait:?}"
        );
    }

    #[test]
    fn default_agent_maps_a_spent_quota_403_to_rate_limited() {
        // The default per-call agent reaches the bottom-of-`get` check with the response in hand, so
        // it must classify a 403 carrying `x-ratelimit-remaining: 0` as RateLimited rather than
        // Unauthorized -- matching the reqwest lane. The reset epoch is derived from now and the
        // wait asserted positive, so a future window is really exercised (a hardcoded epoch ages into
        // the past, where `reset_at: Some(_)` still holds but carries no wait).
        let client = UreqClient::default();
        let base = stub_with_headers("403 Forbidden", &spent_quota_headers());
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err");
        assert!(
            matches!(
                err,
                Error::RateLimited {
                    status: 403,
                    reset_at: Some(_),
                    ..
                }
            ),
            "default-agent 403 with a spent quota must map to RateLimited, got {:?}",
            err
        );
        assert_wait_within_window(&err, "ureq default agent, 403 spent quota");
    }

    #[test]
    fn injected_agent_sees_rate_limit_headers() {
        // An injected agent keeps ureq's default `http_status_as_error(true)`, which used to make a
        // non-2xx surface as a headerless `ureq::Error::StatusCode` and left a spent-quota 403
        // classified as `Unauthorized` -- disagreeing with both reqwest lanes. `get` now overrides
        // that per request, so the injected lane must see the quota headers and classify the same
        // `RateLimited` (with the reset instant) the default agent does.
        let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
        let client = UreqClient::from(agent);
        let base = stub_with_headers("403 Forbidden", &spent_quota_headers());
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err");
        assert!(
            matches!(
                err,
                Error::RateLimited {
                    status: 403,
                    reset_at: Some(_),
                    ..
                }
            ),
            "an injected agent must reach the header-aware check and classify a spent quota, got {:?}",
            err
        );
        assert_wait_within_window(&err, "ureq injected agent, 403 spent quota");
        assert_eq!(err.http_status(), Some(403));
    }

    #[test]
    fn injected_agent_with_status_error_disabled_sees_rate_limit_headers() {
        // The OTHER injected case: the user already built the agent with
        // `http_status_as_error(false)`, so `get` skips the per-request override entirely (see
        // `needs_status_override`) rather than applying a redundant no-op one. This lane must still
        // classify the spent quota identically -- pinned separately from the default-injected agent
        // above so a failure names which injected path broke.
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let client = UreqClient::from(agent);
        let base = stub_with_headers("403 Forbidden", "x-ratelimit-remaining: 0\r\n");
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err");
        assert!(
            matches!(err, Error::RateLimited { status: 403, .. }),
            "an injected agent that defers status handling must reach the header-aware check, got {:?}",
            err
        );
    }

    #[test]
    fn needs_status_override_skips_agents_that_already_disabled_status_error() {
        // Pins the decision `get` makes before ever touching the network (A8): an injected agent
        // whose own config still has ureq's default `http_status_as_error(true)` must be overridden
        // per request (this is the only lane that pays the request-level TLS-config-cache cost
        // documented at the call site in `get`); an injected agent that already disabled it must be
        // skipped rather than overridden again (the free win: no request-level marking at all, so
        // that agent's own TLS-config cache stays eligible to warm up). Without the `&&` on the
        // agent's own setting, the first two assertions below would both see `true` and this test
        // would fail on the second one.
        let default_status_error_agent =
            ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
        assert!(
            needs_status_override(true, &default_status_error_agent),
            "an injected agent that still has ureq's default http_status_as_error(true) must be \
             overridden"
        );

        let status_error_disabled_agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        assert!(
            !needs_status_override(true, &status_error_disabled_agent),
            "an injected agent that already disabled http_status_as_error must be skipped, not \
             overridden again"
        );

        // is_injected gates the whole branch independently of the agent's own setting: a crate-built
        // (non-injected) agent never takes this path, even if (hypothetically) its config still had
        // http_status_as_error(true).
        assert!(
            !needs_status_override(false, &default_status_error_agent),
            "a non-injected agent must never take the injected-only override path"
        );
    }

    #[test]
    fn get_maps_a_429_with_gitlab_header_spelling_to_rate_limited() {
        // The 429 status branch and gitlab's un-prefixed `RateLimit-Remaining` spelling, end to end
        // through the real client: the header names travel over the wire and through ureq's own
        // `HeaderMap`, so a typo in either the status branch or the header lookup fails here. The
        // reset epoch is derived from now and the resulting wait asserted, so the gitlab spelling is
        // proven to feed a *usable* future window rather than a stale instant.
        let client = UreqClient::default();
        let base = stub_with_headers(
            "429 Too Many Requests",
            &format!(
                "RateLimit-Remaining: 0\r\nRateLimit-Reset: {}\r\n",
                reset_epoch(RESET_WINDOW)
            ),
        );
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err");
        assert!(
            matches!(
                err,
                Error::RateLimited {
                    status: 429,
                    reset_at: Some(_),
                    ..
                }
            ),
            "a 429 with gitlab's un-prefixed spent-quota header must map to RateLimited, got {:?}",
            err
        );
        assert_wait_within_window(&err, "ureq default agent, 429 gitlab spelling");
        assert_eq!(err.http_status(), Some(429));
    }

    #[test]
    fn get_parses_retry_after_from_the_response_headers() {
        // `Retry-After` is read off the live response headers by name; every other test builds the
        // signals struct directly, so this is the only coverage that the `retry-after` lookup finds
        // a real served header. Assert the parsed delta-seconds `Duration`, not just the variant.
        let client = UreqClient::default();
        let base = stub_with_headers(
            "429 Too Many Requests",
            "RateLimit-Remaining: 0\r\nRetry-After: 120\r\n",
        );
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err");
        let Error::RateLimited { retry_after, .. } = err else {
            panic!("a 429 with a spent quota must be RateLimited, got {err:?}");
        };
        assert_eq!(
            retry_after,
            Some(Duration::from_secs(120)),
            "the served `Retry-After: 120` must be parsed into a 120s Duration"
        );
    }

    #[test]
    fn build_with_certs_rejects_non_pem() {
        // ureq's `tls::Certificate::from_pem` validates the PEM framing eagerly and errors when the
        // bytes contain no PEM certificate, so `build_with_certs` must surface a config-time `Err`
        // (the parse is deferred to here from the infallible `Certificate::from_pem` constructor)
        // rather than panicking or building an agent over garbage.
        let res = UreqClient::build_with_certs(&[crate::tls::Certificate::from_pem(
            b"not a pem certificate".to_vec(),
        )]);
        assert!(
            res.is_err(),
            "bytes with no PEM certificate must be rejected at build time, got Ok"
        );
    }

    #[test]
    fn build_with_certs_accepts_garbage_der_deferring_validation() {
        // ureq's `tls::Certificate::from_der` is infallible, so invalid DER bytes are accepted
        // here and only surface at connection time. This is intentional and documented (on
        // `add_root_certificate` and in the reference specs); the reqwest client rejects the same
        // bytes at build time (`build_with_certs_rejects_garbage_der` in reqwest.rs). This test
        // pins the asymmetry: if ureq ever gains eager DER validation, this fails and the
        // deferred-validation caveat in the docs should be removed.
        let res =
            UreqClient::build_with_certs(&[crate::tls::Certificate::from_der(b"not der".to_vec())]);
        assert!(
            res.is_ok(),
            "ureq DER validation is deferred to connection time; build must accept the bytes"
        );
    }

    #[test]
    fn injected_agent_no_status_error_falls_through_to_is_success_check() {
        // 404 must still map to NotFound via the bottom-of-`get` is_success() path (the
        // defensive `StatusCode` arm never fires when http_status_as_error(false)).
        let err = get_injected_no_status_error("404 Not Found");
        assert!(
            matches!(err, Error::NotFound { .. }),
            "injected no-status-error 404 must map to Error::NotFound via is_success(), got {:?}",
            err
        );
        assert_eq!(err.http_status(), Some(404));

        // 500 maps to HttpStatus carrying its exact code through the same fall-through path.
        let err = get_injected_no_status_error("500 Internal Server Error");
        assert!(
            matches!(err, Error::HttpStatus { status: 500, .. }),
            "injected no-status-error 500 must map to Error::HttpStatus(500), got {:?}",
            err
        );
        assert_eq!(err.http_status(), Some(500));
    }

    #[test]
    fn injected_agent_maps_404_to_not_found() {
        let err = get_injected("404 Not Found");
        assert!(
            matches!(err, Error::NotFound { .. }),
            "injected-agent 404 must map to Error::NotFound, got {:?}",
            err
        );
        assert_eq!(err.http_status(), Some(404));
    }

    #[test]
    fn injected_agent_maps_401_and_403_to_unauthorized() {
        let err = get_injected("401 Unauthorized");
        assert!(
            matches!(err, Error::Unauthorized { status: 401, .. }),
            "injected-agent 401 must map to Error::Unauthorized(401), got {:?}",
            err
        );
        let err = get_injected("403 Forbidden");
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "injected-agent 403 must map to Error::Unauthorized(403), got {:?}",
            err
        );
    }

    #[test]
    fn injected_agent_maps_500_and_400_to_http_status() {
        let err = get_injected("500 Internal Server Error");
        assert!(
            matches!(err, Error::HttpStatus { status: 500, .. }),
            "injected-agent 500 must map to Error::HttpStatus(500), got {:?}",
            err
        );
        let err = get_injected("400 Bad Request");
        assert!(
            matches!(err, Error::HttpStatus { status: 400, .. }),
            "injected-agent 400 must map to Error::HttpStatus(400), got {:?}",
            err
        );
    }

    #[test]
    fn default_agent_path_maps_statuses_identically_to_injected() {
        // The default per-call agent (`http_status_as_error(false)`) reaches the bottom-of-`get`
        // `is_success()` check and must produce the SAME structured variants as an injected agent
        // (which gets the same setting via the per-request override), so both ureq lanes agree.
        assert!(matches!(
            get_default("404 Not Found"),
            Error::NotFound { .. }
        ));
        assert!(matches!(
            get_default("401 Unauthorized"),
            Error::Unauthorized { status: 401, .. }
        ));
        assert!(matches!(
            get_default("403 Forbidden"),
            Error::Unauthorized { status: 403, .. }
        ));
        assert!(matches!(
            get_default("503 Service Unavailable"),
            Error::HttpStatus { status: 503, .. }
        ));
    }

    #[test]
    fn transport_failure_maps_to_transport_variant() {
        // A connection refused to a closed port (no listener) cannot complete the request, so the
        // catch-all `Err(e) => Error::Transport` arm fires (default agent) -- NOT a status variant.
        // Bind+drop to obtain a port nothing is listening on.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{}/", addr);
        let client = UreqClient::default();
        let err = client
            .get(&url, &HeaderMap::new(), None)
            .err()
            .expect("connection refused must be an Err");
        assert!(
            matches!(err, Error::Transport(_)),
            "a failed (uncompleted) request must map to Error::Transport, got {:?}",
            err
        );
        assert_eq!(err.http_status(), None, "Transport has no HTTP status code");
    }
}
