#![cfg(feature = "reqwest")]

use reqwest::blocking::Response;

use super::{ClientOverride, HeaderMap, HttpResponse};
use crate::Result;

pub fn get(
    url: &str,
    headers: HeaderMap,
    timeout: Option<std::time::Duration>,
    client: &ClientOverride,
) -> Result<impl HttpResponse> {
    let resp = if let Some(client) = &client.blocking {
        // Injected client: reuse it; layer the per-request timeout + headers onto the request.
        let mut req = client.get(url).headers(headers);
        if let Some(timeout) = timeout {
            req = req.timeout(timeout);
        }
        req.send()?
    } else {
        let mut client_builder = reqwest::blocking::ClientBuilder::new();

        if let Some(timeout) = timeout {
            client_builder = client_builder.timeout(timeout);
        }

        #[cfg(feature = "rustls")]
        {
            client_builder = client_builder.use_rustls_tls();
        }

        let client = client_builder.http2_adaptive_window(true).build()?;
        client.get(url).headers(headers).send()?
    };

    if !resp.status().is_success() {
        return Err(crate::errors::status_to_error(resp.status().as_u16(), url));
    }

    Ok(resp)
}

/// Async sibling of [`get`], used by the `async` update API. Returns the raw `reqwest::Response`
/// (async = reqwest-only, so there's a single concrete type — no trait abstraction needed). Like
/// `get`, it errors on a non-success status.
#[cfg(feature = "async")]
pub async fn get_async(
    url: &str,
    headers: HeaderMap,
    timeout: Option<std::time::Duration>,
    client: &ClientOverride,
) -> Result<::reqwest::Response> {
    let resp = if let Some(client) = &client.r#async {
        // Injected client: reuse it; layer the per-request timeout + headers onto the request.
        let mut req = client.get(url).headers(headers);
        if let Some(timeout) = timeout {
            req = req.timeout(timeout);
        }
        req.send().await?
    } else {
        let mut client_builder = ::reqwest::ClientBuilder::new();

        if let Some(timeout) = timeout {
            client_builder = client_builder.timeout(timeout);
        }

        #[cfg(feature = "rustls")]
        {
            client_builder = client_builder.use_rustls_tls();
        }

        let client = client_builder.http2_adaptive_window(true).build()?;
        client.get(url).headers(headers).send().await?
    };

    if !resp.status().is_success() {
        return Err(crate::errors::status_to_error(resp.status().as_u16(), url));
    }

    Ok(resp)
}

impl HttpResponse for Response {
    fn json<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        Ok(Response::json(self)?)
    }

    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        Response::headers(self)
    }

    fn body(self) -> impl std::io::Read {
        self
    }

    fn text(self) -> Result<String> {
        Ok(Response::text(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::http_client::ClientOverride;
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

    /// Sync `get` against the loopback stub serving `status`; returns the mapped error.
    fn get_status(status: &'static str) -> Error {
        let client = ClientOverride::default();
        let base = stub(status);
        super::get(&base, HeaderMap::new(), None, &client)
            .err()
            .expect("non-2xx must be an Err")
    }

    #[test]
    fn sync_get_maps_each_status_to_its_structured_variant() {
        // The sync `get` runs `status_to_error` on any non-2xx. Pin the full mapping table so a
        // regression in the per-call client path (not just `status_to_error` in isolation) is
        // caught: 404 -> NotFound, 401/403 -> Unauthorized, 400/500/503 -> HttpStatus.
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
        let client = ClientOverride::default();
        let err = super::get(&url, HeaderMap::new(), None, &client)
            .err()
            .expect("connection refused must be an Err");
        assert!(
            matches!(err, Error::Transport(_)),
            "uncompleted request must map to Error::Transport, got {:?}",
            err
        );
        assert_eq!(err.http_status(), None);
    }

    /// Async `get_async` against the loopback stub serving `status`; returns the mapped error.
    #[cfg(feature = "async")]
    async fn get_async_status(status: &'static str) -> Error {
        let client = ClientOverride::default();
        let base = stub(status);
        super::get_async(&base, HeaderMap::new(), None, &client)
            .await
            .expect_err("non-2xx must be an Err")
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_get_maps_each_status_to_its_structured_variant() {
        // The async `get_async` shares the same `status_to_error` mapping as the sync path. Pin it
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{}/", addr);
        let client = ClientOverride::default();
        let err = super::get_async(&url, HeaderMap::new(), None, &client)
            .await
            .expect_err("connection refused must be an Err");
        assert!(
            matches!(err, Error::Transport(_)),
            "uncompleted async request must map to Error::Transport, got {:?}",
            err
        );
        assert_eq!(err.http_status(), None);
    }
}
