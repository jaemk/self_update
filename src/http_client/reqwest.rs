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
    ) -> std::result::Result<std::sync::Arc<dyn crate::http_client::HttpClient>, String> {
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
        for cert in certs {
            let c = if cert.is_pem() {
                reqwest::Certificate::from_pem(cert.bytes())
                    .map_err(|e| format!("invalid PEM certificate: {e}"))?
            } else {
                reqwest::Certificate::from_der(cert.bytes())
                    .map_err(|e| format!("invalid DER certificate: {e}"))?
            };
            builder = builder.tls_certs_merge([c]);
        }
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
            return Err(crate::errors::status_to_error(resp.status().as_u16(), url));
        }
        Ok(Box::new(resp))
    }
}

impl HttpResponse for Response {
    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        Response::headers(self)
    }

    fn json_value(&mut self) -> Result<serde_json::Value> {
        // `Response::json` consumes `self`; replace this response with a placeholder we can take by
        // value. (The body has not been read yet, so this is the first and only consumer.)
        let resp = std::mem::replace(self, dummy_response());
        Ok(resp.json::<serde_json::Value>()?)
    }

    fn text(&mut self) -> Result<String> {
        let resp = std::mem::replace(self, dummy_response());
        Ok(resp.text()?)
    }

    fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
        self
    }
}

/// A throwaway `reqwest::blocking::Response` used only to satisfy `std::mem::replace` when
/// consuming the real response from behind `&mut self` in the object-safe `json_value`/`text`. It
/// is never read.
fn dummy_response() -> Response {
    Response::from(http::Response::new(Vec::new()))
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
    ) -> std::result::Result<std::sync::Arc<dyn crate::http_client::AsyncHttpClient>, String> {
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
        for cert in certs {
            let c = if cert.is_pem() {
                reqwest::Certificate::from_pem(cert.bytes())
                    .map_err(|e| format!("invalid PEM certificate: {e}"))?
            } else {
                reqwest::Certificate::from_der(cert.bytes())
                    .map_err(|e| format!("invalid DER certificate: {e}"))?
            };
            builder = builder.tls_certs_merge([c]);
        }
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
                return Err(crate::errors::status_to_error(resp.status().as_u16(), url));
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
    fn stub(status: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "err";
                let out = format!(
                    "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        base
    }

    /// Serve a single `200 OK` response with the given `body` (a known content type), then close.
    /// Returns the base URL. Used by the body-consumption tests below.
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

    /// Fetch a `200 OK` body through the trait and return the boxed response handle.
    fn ok_response(body: &'static str, content_type: &'static str) -> Box<dyn HttpResponse> {
        let client = ReqwestClient::default();
        let base = stub_ok(body, content_type);
        client
            .get(&base, &HeaderMap::new(), None)
            .expect("200 must be Ok")
    }

    #[test]
    fn json_value_then_text_after_body_taken_does_not_panic_or_return_real_data() {
        // `json_value`/`text` `std::mem::replace` the live response with `dummy_response()` to move
        // the body out from behind `&mut self`. The author flagged that the placeholder must never
        // surface as observable data. After the body has been consumed once, a *second* consuming
        // call sees only the empty placeholder: it must return cleanly (Ok empty / parse-Err), never
        // panic and never hand back the original body or bogus content.
        let mut resp = ok_response("{\"k\":\"v\"}", "application/json");
        let first = resp
            .json_value()
            .expect("first json_value parses the real body");
        assert_eq!(first["k"], "v", "first consumer sees the real body");

        // Second consumer (text) now reads the dummy. The placeholder is an empty-body response, so
        // `text` must yield the empty string (NOT the original JSON, NOT a panic).
        let second = resp
            .text()
            .expect("text on the drained response must not error/panic");
        assert_eq!(
            second, "",
            "after the body was taken, the dummy placeholder yields an empty body, not stale data"
        );
    }

    #[test]
    fn text_then_json_value_after_body_taken_is_defined() {
        // Mirror of the above with the order swapped: `text` consumes the real body, then
        // `json_value` reads the empty placeholder. Parsing empty bytes as JSON is a clean `Err`
        // (`Error::Transport` from reqwest's json layer or `Error::Json`), never a panic and never
        // the original object.
        let mut resp = ok_response("hello world", "text/plain");
        let body = resp.text().expect("first text reads the real body");
        assert_eq!(body, "hello world");

        // Empty placeholder body is not valid JSON; the result must be an Err, not a panic.
        let res = resp.json_value();
        assert!(
            res.is_err(),
            "json_value on the drained (empty) placeholder must be a clean Err, got {:?}",
            res
        );
    }

    #[test]
    fn body_after_json_value_streams_the_empty_placeholder_not_stale_data() {
        // After `json_value` takes the real body, consuming `body()` (which returns `self`, now the
        // placeholder) must stream the placeholder's empty body, proving the swapped-in dummy is
        // what remains in `self` and carries no leftover bytes.
        let mut resp = ok_response("{\"a\":1}", "application/json");
        let _ = resp.json_value().expect("json parses");
        let mut sink = String::new();
        resp.body()
            .read_to_string(&mut sink)
            .expect("reading the placeholder body must not error");
        assert_eq!(
            sink, "",
            "the placeholder left in self after json_value carries no data"
        );
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
        // `HttpClient::get` runs `status_to_error` on any non-2xx before returning. Pin the full
        // mapping table so a regression in the per-call client path (not just `status_to_error` in
        // isolation) is caught: 404 -> NotFound, 401/403 -> Unauthorized, 400/500/503 -> HttpStatus.
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
        // The async client shares the same `status_to_error` mapping as the sync path. Pin it
        // independently so the async lane cannot drift from the sync lane.
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
