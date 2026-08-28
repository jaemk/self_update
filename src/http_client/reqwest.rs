#![cfg(feature = "reqwest")]

use std::time::Duration;

use reqwest::blocking::Response;

use super::{HeaderMap, HttpClient, HttpResponse};
use crate::Result;

/// Sync [`HttpClient`] backed by a `reqwest::blocking::Client`.
///
/// The default (`ReqwestClient(None)`) builds a fresh per-call client honoring the per-request
/// timeout, the TLS feature, proxy-env, and http2 adaptive window. A `ReqwestClient(Some(client))`
/// (built via `From<reqwest::blocking::Client>`, used by the `reqwest_client` convenience setter)
/// reuses the injected client; the per-request timeout/headers are still layered on, but proxy-env
/// and TLS defer to the injected client.
#[derive(Default)]
pub struct ReqwestClient(Option<reqwest::blocking::Client>);

impl From<reqwest::blocking::Client> for ReqwestClient {
    fn from(client: reqwest::blocking::Client) -> Self {
        Self(Some(client))
    }
}

impl ReqwestClient {
    /// Build a ReqwestClient with custom root CA certificates baked in.
    /// Uses the same TLS backend selection (rustls wins over native-tls) as the per-call path.
    pub(crate) fn build_with_certs(
        certs: &[crate::tls::Certificate],
    ) -> std::result::Result<
        std::sync::Arc<dyn crate::http_client::HttpClient>,
        crate::http_client::ClientBuildError,
    > {
        let mut builder = reqwest::blocking::ClientBuilder::new();
        #[cfg(feature = "rustls")]
        {
            builder = builder.use_rustls_tls();
        }
        #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
        {
            builder = builder.use_native_tls();
        }
        builder = builder.http2_adaptive_window(true);
        // Collect all certs and merge in a single call: `tls_certs_merge` accumulates the passed
        // certs onto the trust store, so one call with the whole set is equivalent to (and clearer
        // than) one call per cert.
        let mut collected = Vec::with_capacity(certs.len());
        for cert in certs {
            let c = if cert.is_pem() {
                reqwest::Certificate::from_pem(cert.bytes())
                    .map_err(|e| format!("invalid PEM certificate: {e}"))?
            } else {
                reqwest::Certificate::from_der(cert.bytes())
                    .map_err(|e| format!("invalid DER certificate: {e}"))?
            };
            collected.push(c);
        }
        builder = builder.tls_certs_merge(collected);
        let client = builder
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(std::sync::Arc::new(ReqwestClient::from(client)))
    }
}

impl HttpClient for ReqwestClient {
    fn get(
        &self,
        url: &str,
        headers: &HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<Box<dyn HttpResponse>> {
        let resp = match &self.0 {
            Some(client) => {
                // Injected client: reuse it; layer the per-request timeout + headers on.
                let mut req = client.get(url).headers(headers.clone());
                if let Some(timeout) = timeout {
                    req = req.timeout(timeout);
                }
                req.send()?
            }
            None => {
                let mut client_builder = reqwest::blocking::ClientBuilder::new();
                if let Some(timeout) = timeout {
                    client_builder = client_builder.timeout(timeout);
                }
                // When both TLS features are enabled, rustls wins (it is the crate default).
                #[cfg(feature = "rustls")]
                {
                    client_builder = client_builder.use_rustls_tls();
                }
                #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
                {
                    client_builder = client_builder.use_native_tls();
                }
                let client = client_builder.http2_adaptive_window(true).build()?;
                client.get(url).headers(headers.clone()).send()?
            }
        };

        if !resp.status().is_success() {
            return Err(crate::errors::status_to_error_with_headers(
                resp.status().as_u16(),
                url,
                resp.headers(),
            ));
        }
        Ok(Box::new(resp))
    }
}

impl HttpResponse for Response {
    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        Response::headers(self)
    }

    fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
        self
    }
}

/// Async [`super::AsyncHttpClient`] backed by a `reqwest::Client`. Mirrors [`ReqwestClient`]:
/// `None` builds a fresh per-call client, `Some` reuses an injected one.
#[cfg(feature = "async")]
#[derive(Default)]
pub struct ReqwestAsyncClient(Option<reqwest::Client>);

#[cfg(feature = "async")]
impl From<reqwest::Client> for ReqwestAsyncClient {
    fn from(client: reqwest::Client) -> Self {
        Self(Some(client))
    }
}

#[cfg(feature = "async")]
impl ReqwestAsyncClient {
    /// Async sibling of [`ReqwestClient::build_with_certs`]: build a `ReqwestAsyncClient` with
    /// custom root CA certificates baked in, using `reqwest::ClientBuilder` (async) and the same
    /// TLS backend selection.
    pub(crate) fn build_async_with_certs(
        certs: &[crate::tls::Certificate],
    ) -> std::result::Result<
        std::sync::Arc<dyn crate::http_client::AsyncHttpClient>,
        crate::http_client::ClientBuildError,
    > {
        let mut builder = reqwest::ClientBuilder::new();
        #[cfg(feature = "rustls")]
        {
            builder = builder.use_rustls_tls();
        }
        #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
        {
            builder = builder.use_native_tls();
        }
        builder = builder.http2_adaptive_window(true);
        let mut collected = Vec::with_capacity(certs.len());
        for cert in certs {
            let c = if cert.is_pem() {
                reqwest::Certificate::from_pem(cert.bytes())
                    .map_err(|e| format!("invalid PEM certificate: {e}"))?
            } else {
                reqwest::Certificate::from_der(cert.bytes())
                    .map_err(|e| format!("invalid DER certificate: {e}"))?
            };
            collected.push(c);
        }
        builder = builder.tls_certs_merge(collected);
        let client = builder
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(std::sync::Arc::new(ReqwestAsyncClient::from(client)))
    }
}

#[cfg(feature = "async")]
impl super::AsyncHttpClient for ReqwestAsyncClient {
    fn get<'a>(
        &'a self,
        url: &'a str,
        headers: &'a HeaderMap,
        timeout: Option<Duration>,
    ) -> futures_util::future::BoxFuture<'a, Result<Box<dyn super::AsyncHttpResponse>>> {
        Box::pin(async move {
            let resp = match &self.0 {
                Some(client) => {
                    let mut req = client.get(url).headers(headers.clone());
                    if let Some(timeout) = timeout {
                        req = req.timeout(timeout);
                    }
                    req.send().await?
                }
                None => {
                    let mut client_builder = reqwest::ClientBuilder::new();
                    if let Some(timeout) = timeout {
                        client_builder = client_builder.timeout(timeout);
                    }
                    #[cfg(feature = "rustls")]
                    {
                        client_builder = client_builder.use_rustls_tls();
                    }
                    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
                    {
                        client_builder = client_builder.use_native_tls();
                    }
                    let client = client_builder.http2_adaptive_window(true).build()?;
                    client.get(url).headers(headers.clone()).send().await?
                }
            };
            if !resp.status().is_success() {
                return Err(crate::errors::status_to_error_with_headers(
                    resp.status().as_u16(),
                    url,
                    resp.headers(),
                ));
            }
            Ok(Box::new(resp) as Box<dyn super::AsyncHttpResponse>)
        })
    }
}

#[cfg(feature = "async")]
impl super::AsyncHttpResponse for reqwest::Response {
    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        reqwest::Response::headers(self)
    }

    fn text(self: Box<Self>) -> futures_util::future::BoxFuture<'static, Result<String>> {
        Box::pin(async move { Ok((*self).text().await?) })
    }

    fn bytes_stream(
        self: Box<Self>,
    ) -> futures_util::stream::BoxStream<'static, Result<bytes::Bytes>> {
        use futures_util::StreamExt;
        Box::pin((*self).bytes_stream().map(|chunk| Ok(chunk?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// Serve a single HTTP response (the given status line + a short body) over a fresh loopback
    /// listener, then close. Returns the base URL (`http://127.0.0.1:<port>/`). No external network.
    /// Thin wrapper over [`stub_with_headers`] with no extra headers, so there is one stub server.
    fn stub(status: &'static str) -> String {
        stub_with_headers(status, "")
    }

    /// Like [`stub`], but injects `extra_headers` (a pre-formatted `Name: value\r\n` block) into the
    /// response. Used to serve the rate-limit headers a spent GitHub quota carries. Takes a plain
    /// `&str` (copied into the serving thread) so a header block can carry a value derived at
    /// runtime, e.g. a reset epoch relative to *now*.
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

    /// Serve a single `200 OK` response with the given `body` (a known content type), then close.
    /// Returns the base URL. Used by the async JSON-mapping test below.
    #[cfg(feature = "async")]
    fn stub_ok(body: &'static str, content_type: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    content_type,
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        base
    }

    /// A PEM block carrying a CERTIFICATE marker but a body that decodes to bytes which are not a
    /// valid X.509 DER certificate. reqwest's TLS backend accepts the PEM framing but rejects this
    /// at client-build time, exercising the deferred-validation path (the `from_*` constructors are
    /// infallible). `bm90IGEgdmFsaWQgY2VydA==` is base64 for "not a valid cert".
    const BAD_PEM: &[u8] =
        b"-----BEGIN CERTIFICATE-----\nbm90IGEgdmFsaWQgY2VydA==\n-----END CERTIFICATE-----\n";

    #[test]
    fn build_with_certs_rejects_garbage_pem() {
        // A PEM-framed but non-certificate body must surface a config-time `Err` from
        // `build_with_certs` (the parse is deferred to here from the infallible
        // `Certificate::from_pem` constructor) rather than panicking or building a usable client.
        let res =
            ReqwestClient::build_with_certs(&[crate::tls::Certificate::from_pem(BAD_PEM.to_vec())]);
        assert!(
            res.is_err(),
            "garbage PEM must be rejected at build time, got Ok"
        );
    }

    #[test]
    fn build_with_certs_rejects_garbage_der() {
        // Same as the PEM case for the DER decoder: invalid DER bytes must produce an `Err`.
        let res = ReqwestClient::build_with_certs(&[crate::tls::Certificate::from_der(
            b"not der".to_vec(),
        )]);
        assert!(
            res.is_err(),
            "garbage DER must be rejected at build time, got Ok"
        );
    }

    /// Sync `get` (through the trait) against the loopback stub serving `status`; returns the mapped
    /// error.
    fn get_status(status: &'static str) -> Error {
        let client = ReqwestClient::default();
        let base = stub(status);
        client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err")
    }

    #[test]
    fn sync_get_maps_each_status_to_its_structured_variant() {
        // `HttpClient::get` runs `status_to_error_with_headers` on any non-2xx before returning. Pin
        // the full mapping table for a response carrying no quota headers, so a regression in the
        // per-call client path (not just the classifier in isolation) is caught: 404 -> NotFound,
        // 401/403 -> Unauthorized, 400/500/503 -> HttpStatus. The quota-header cases (RateLimited)
        // are pinned separately below.
        let err = get_status("404 Not Found");
        assert!(
            matches!(err, Error::NotFound { .. }),
            "404 -> NotFound, got {:?}",
            err
        );
        assert_eq!(err.http_status(), Some(404));

        assert!(matches!(
            get_status("401 Unauthorized"),
            Error::Unauthorized { status: 401, .. }
        ));
        assert!(matches!(
            get_status("403 Forbidden"),
            Error::Unauthorized { status: 403, .. }
        ));
        assert!(matches!(
            get_status("400 Bad Request"),
            Error::HttpStatus { status: 400, .. }
        ));
        assert!(matches!(
            get_status("500 Internal Server Error"),
            Error::HttpStatus { status: 500, .. }
        ));
        assert!(matches!(
            get_status("503 Service Unavailable"),
            Error::HttpStatus { status: 503, .. }
        ));
    }

    #[test]
    fn sync_get_maps_a_spent_quota_403_to_rate_limited() {
        // The classification needs the *response headers*, so this pins that the client actually
        // hands them to `status_to_error_with_headers`: the same 403 that maps to `Unauthorized`
        // above must map to `RateLimited` once it carries `x-ratelimit-remaining: 0`, with the
        // reset instant recovered from `x-ratelimit-reset`. The served epoch is derived from now, and
        // the wait is asserted positive, so the test really exercises a future window (a hardcoded
        // epoch ages into the past, where `reset_at: Some(_)` still holds but means nothing).
        let client = ReqwestClient::default();
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
            "a 403 with a spent quota must map to RateLimited, got {:?}",
            err
        );
        assert_wait_within_window(&err, "sync 403 spent quota");
        assert_eq!(err.http_status(), Some(403));
    }

    #[test]
    fn sync_get_maps_a_429_with_gitlab_header_spelling_to_rate_limited() {
        // The 429 status branch and gitlab's un-prefixed `RateLimit-Remaining` spelling, end to end
        // through the real client: both the header names and the status travel over the wire and
        // through reqwest's own `HeaderMap`, so a typo in either the status branch or the header
        // lookup fails here (the classifier unit tests build the signals struct by hand). The reset
        // epoch is derived from now and the resulting wait asserted, so the gitlab spelling is proven
        // to feed a *usable* future window rather than a stale instant.
        let client = ReqwestClient::default();
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
        assert_wait_within_window(&err, "sync 429 gitlab spelling");
        assert_eq!(err.http_status(), Some(429));
    }

    #[test]
    fn sync_get_parses_retry_after_from_the_response_headers() {
        // `Retry-After` is looked up by name on the live response headers; every other test builds
        // the signals struct directly, so this is the only coverage that the `retry-after` lookup
        // finds a really-served header. Assert the parsed delta-seconds `Duration`, not the variant.
        let client = ReqwestClient::default();
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
    fn sync_get_maps_a_403_with_only_retry_after_to_rate_limited() {
        // GitHub's *secondary* rate limit answers 403 + `Retry-After` while the remaining-quota
        // header is still NONZERO. Nothing else drives that shape through a real response: the
        // spent-quota tests all serve `remaining: 0`, so a client that only forwarded the
        // remaining/reset headers (or a classifier that keyed solely on a spent quota) would still
        // pass them while misfiling this as `Unauthorized` -- "your token is bad" for what is a
        // back-off. Pin the variant *and* the carried delay.
        let client = ReqwestClient::default();
        let base = stub_with_headers(
            "403 Forbidden",
            "x-ratelimit-remaining: 57\r\nRetry-After: 60\r\n",
        );
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err");
        let Error::RateLimited {
            status,
            retry_after,
            ..
        } = err
        else {
            panic!("a 403 carrying Retry-After must be RateLimited, got {err:?}");
        };
        assert_eq!(status, 403);
        assert_eq!(
            retry_after,
            Some(Duration::from_secs(60)),
            "the served `Retry-After: 60` must be carried as a 60s Duration"
        );
    }

    #[test]
    fn sync_get_keeps_a_403_unauthorized_without_a_rate_limit_signal() {
        // The negative half of the broadened 403 rule, end to end: a 403 is only `RateLimited` when
        // the response actually reports a spent quota or asks for a `Retry-After`. A 403 whose
        // headers say quota *remains* is a genuine authorization failure, and must not be softened
        // into "just wait" by the mere presence of rate-limit headers on the response. The reset
        // epoch is a live future one, so the 403 is not kept `Unauthorized` merely because the served
        // window had already elapsed.
        let client = ReqwestClient::default();
        let base = stub_with_headers(
            "403 Forbidden",
            &format!(
                "x-ratelimit-remaining: 57\r\nx-ratelimit-reset: {}\r\n",
                reset_epoch(RESET_WINDOW)
            ),
        );
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err");
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "a 403 with quota remaining must stay Unauthorized, got {:?}",
            err
        );
    }

    #[test]
    fn sync_get_keeps_a_403_unauthorized_when_retry_after_is_over_the_ceiling() {
        // The 24h ceiling is what stops a hostile response from parking an update channel, and it
        // is applied *before* the 403 decision: a `Retry-After` past the ceiling is no signal at
        // all, so the 403 stays `Unauthorized` instead of becoming a `RateLimited` carrying no
        // usable wait. Served over the wire so the ceiling is observable on the client path, not
        // only in the classifier's unit tests. 30 days.
        let client = ReqwestClient::default();
        let base = stub_with_headers("403 Forbidden", "Retry-After: 2592000\r\n");
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err");
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "an over-ceiling Retry-After must not promote a 403 to RateLimited, got {:?}",
            err
        );
    }

    #[test]
    fn sync_get_transport_failure_maps_to_transport() {
        // A connection refused (no listener) cannot complete, so `From<reqwest::Error>` routes the
        // failure to `Error::Transport` (via the `?` on `send()`), never a status variant.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{}/", addr);
        let client = ReqwestClient::default();
        let err = client
            .get(&url, &HeaderMap::new(), None)
            .err()
            .expect("connection refused must be an Err");
        assert!(
            matches!(err, Error::Transport(_)),
            "uncompleted request must map to Error::Transport, got {:?}",
            err
        );
        assert_eq!(err.http_status(), None);
    }

    /// Async `get` (through the trait) against the loopback stub serving `status`; returns the
    /// mapped error.
    #[cfg(feature = "async")]
    async fn get_async_status(status: &'static str) -> Error {
        use super::super::AsyncHttpClient;
        let client = ReqwestAsyncClient::default();
        let base = stub(status);
        client
            .get(&base, &HeaderMap::new(), None)
            .await
            .err()
            .expect("non-2xx must be an Err")
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_get_maps_each_status_to_its_structured_variant() {
        // The async client shares the same `status_to_error_with_headers` mapping as the sync path
        // (here with no quota headers served). Pin it independently so the async lane cannot drift
        // from the sync lane.
        let err = get_async_status("404 Not Found").await;
        assert!(
            matches!(err, Error::NotFound { .. }),
            "404 -> NotFound (async), got {:?}",
            err
        );
        assert_eq!(err.http_status(), Some(404));

        assert!(matches!(
            get_async_status("401 Unauthorized").await,
            Error::Unauthorized { status: 401, .. }
        ));
        assert!(matches!(
            get_async_status("403 Forbidden").await,
            Error::Unauthorized { status: 403, .. }
        ));
        assert!(matches!(
            get_async_status("500 Internal Server Error").await,
            Error::HttpStatus { status: 500, .. }
        ));
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_get_maps_a_spent_quota_403_to_rate_limited() {
        // Async sibling of `sync_get_maps_a_spent_quota_403_to_rate_limited`, deliberately
        // symmetric with it: the same headers are served (a reset epoch derived from now) and the
        // same fields asserted, including the positive wait, so the async lane cannot silently drop
        // the reset instant (or the status) that the sync lane recovers.
        use super::super::AsyncHttpClient;
        let client = ReqwestAsyncClient::default();
        let base = stub_with_headers("403 Forbidden", &spent_quota_headers());
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .await
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
            "a 403 with a spent quota must map to RateLimited (async), got {:?}",
            err
        );
        assert_wait_within_window(&err, "async 403 spent quota");
        assert_eq!(err.http_status(), Some(403));
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_get_maps_a_429_with_gitlab_header_spelling_to_rate_limited() {
        // Async sibling of `sync_get_maps_a_429_with_gitlab_header_spelling_to_rate_limited`: the
        // async lane has its own status check and its own `resp.headers()` call, so the 429 branch
        // and gitlab's un-prefixed `RateLimit-*` spelling need their own end-to-end proof here.
        // Same served headers and same asserted fields as the sync test, so the lanes cannot drift.
        use super::super::AsyncHttpClient;
        let client = ReqwestAsyncClient::default();
        let base = stub_with_headers(
            "429 Too Many Requests",
            &format!(
                "RateLimit-Remaining: 0\r\nRateLimit-Reset: {}\r\n",
                reset_epoch(RESET_WINDOW)
            ),
        );
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .await
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
            "a 429 with gitlab's un-prefixed spent-quota header must map to RateLimited (async), \
             got {:?}",
            err
        );
        assert_wait_within_window(&err, "async 429 gitlab spelling");
        assert_eq!(err.http_status(), Some(429));
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_get_parses_retry_after_from_the_response_headers() {
        // Async sibling of `sync_get_parses_retry_after_from_the_response_headers`: assert the
        // parsed delta-seconds `Duration`, not just the variant, so the async lane cannot drop the
        // served `Retry-After` (which is the only wait GitHub's secondary limit supplies) while
        // still returning a plausible-looking `RateLimited`.
        use super::super::AsyncHttpClient;
        let client = ReqwestAsyncClient::default();
        let base = stub_with_headers(
            "429 Too Many Requests",
            "RateLimit-Remaining: 0\r\nRetry-After: 120\r\n",
        );
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .await
            .err()
            .expect("non-2xx must be an Err");
        let Error::RateLimited { retry_after, .. } = err else {
            panic!("a 429 with a spent quota must be RateLimited (async), got {err:?}");
        };
        assert_eq!(
            retry_after,
            Some(Duration::from_secs(120)),
            "the served `Retry-After: 120` must be parsed into a 120s Duration (async)"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_get_maps_a_403_with_only_retry_after_to_rate_limited() {
        // Async sibling of `sync_get_maps_a_403_with_only_retry_after_to_rate_limited`: GitHub's
        // secondary rate limit (403 + `Retry-After`, remaining still NONZERO) must classify as
        // `RateLimited` on the async lane too, carrying the delay rather than reporting a bad
        // credential.
        use super::super::AsyncHttpClient;
        let client = ReqwestAsyncClient::default();
        let base = stub_with_headers(
            "403 Forbidden",
            "x-ratelimit-remaining: 57\r\nRetry-After: 60\r\n",
        );
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .await
            .err()
            .expect("non-2xx must be an Err");
        let Error::RateLimited {
            status,
            retry_after,
            ..
        } = err
        else {
            panic!("a 403 carrying Retry-After must be RateLimited (async), got {err:?}");
        };
        assert_eq!(status, 403);
        assert_eq!(retry_after, Some(Duration::from_secs(60)));
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_get_keeps_a_403_unauthorized_without_a_rate_limit_signal() {
        // Async sibling of `sync_get_keeps_a_403_unauthorized_without_a_rate_limit_signal`: a 403
        // whose headers report quota *remaining* (and no `Retry-After`) is a real authorization
        // failure and must stay `Unauthorized` on the async lane, i.e. the broadened rule did not
        // become "any 403 with rate-limit headers is a rate limit". The served window is a live
        // future one (derived from now), not a stale epoch that would make the case vacuous.
        use super::super::AsyncHttpClient;
        let client = ReqwestAsyncClient::default();
        let base = stub_with_headers(
            "403 Forbidden",
            &format!(
                "x-ratelimit-remaining: 57\r\nx-ratelimit-reset: {}\r\n",
                reset_epoch(RESET_WINDOW)
            ),
        );
        let err = client
            .get(&base, &HeaderMap::new(), None)
            .await
            .err()
            .expect("non-2xx must be an Err");
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "a 403 with quota remaining must stay Unauthorized (async), got {:?}",
            err
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_get_transport_failure_maps_to_transport() {
        use super::super::AsyncHttpClient;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{}/", addr);
        let client = ReqwestAsyncClient::default();
        let err = client
            .get(&url, &HeaderMap::new(), None)
            .await
            .err()
            .expect("connection refused must be an Err");
        assert!(
            matches!(err, Error::Transport(_)),
            "uncompleted async request must map to Error::Transport, got {:?}",
            err
        );
        assert_eq!(err.http_status(), None);
    }

    /// The async response trait dropped reqwest's `.json()` in favor of `text().await? ->
    /// serde_json::from_str`. A malformed body must therefore surface as `Error::Json` (via the
    /// `From<serde_json::Error>` conversion the backends rely on), not as a transport error or a
    /// panic. This pins the async JSON error mapping end-to-end: drive a real `ReqwestAsyncClient`
    /// against a 200 serving invalid JSON, read it through the trait's `text()`, and parse exactly
    /// as the async backends do.
    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_text_then_from_str_maps_malformed_json_to_error_json() {
        use super::super::AsyncHttpClient;
        let client = ReqwestAsyncClient::default();
        let base = stub_ok("{not valid json", "application/json");
        let resp = client
            .get(&base, &HeaderMap::new(), None)
            .await
            .expect("200 must be Ok");
        // Exactly the async backend pattern: `text().await?` then `serde_json::from_str`.
        let body = resp.text().await.expect("text() reads the body");
        let parsed: Result<serde_json::Value> =
            serde_json::from_str::<serde_json::Value>(&body).map_err(Into::into);
        let err = parsed.expect_err("malformed JSON must be an Err");
        assert!(
            matches!(err, Error::Json(_)),
            "malformed async JSON must map to Error::Json, got {:?}",
            err
        );
    }

    /// The async seam (`AsyncHttpClient`/`AsyncHttpResponse`) must stay object-safe just like the
    /// sync seam — an injected client is carried as `Arc<dyn AsyncHttpClient>`, so any leaked generic
    /// method would break these `Box<dyn ...>` coercions at compile time.
    #[cfg(feature = "async")]
    #[test]
    fn async_traits_are_object_safe() {
        let _client: Box<dyn super::super::AsyncHttpClient> =
            Box::new(ReqwestAsyncClient::default());
        let _arc: std::sync::Arc<dyn super::super::AsyncHttpClient> =
            std::sync::Arc::new(ReqwestAsyncClient::default());
    }
}
