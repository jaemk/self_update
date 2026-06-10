#![cfg(feature = "reqwest")]

use reqwest::blocking::Response;

use super::{ClientOverride, HeaderMap, HttpResponse};
use crate::{Error, Result};

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
        bail!(
            Error::Network,
            "api request failed with status: {:?} - for: {:?}",
            resp.status(),
            url
        )
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
        bail!(
            Error::Network,
            "api request failed with status: {:?} - for: {:?}",
            resp.status(),
            url
        )
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

    fn status(&self) -> http::StatusCode {
        Response::status(self)
    }

    fn body(self) -> impl std::io::Read {
        self
    }

    fn text(self) -> Result<String> {
        Ok(Response::text(self)?)
    }
}
