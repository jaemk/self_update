#![cfg(feature = "reqwest")]

use reqwest::blocking::Response;

use super::{HeaderMap, HttpResponse};
use crate::{Error, Result};

pub fn get(url: &str, headers: HeaderMap) -> Result<impl HttpResponse> {
    let client_builder = reqwest::blocking::ClientBuilder::new();

    #[cfg(feature = "rustls")]
    let client_builder = client_builder.use_rustls_tls();

    let client = client_builder.http2_adaptive_window(true).build()?;
    let resp = client.get(url).headers(headers).send()?;

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
