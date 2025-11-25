#![cfg(feature = "ureq")]

use ureq::tls::TlsProvider;
use ureq::{http::Response, Agent, Body};

use super::{HeaderMap, HttpResponse};
use crate::{Error, Result};

pub fn get(url: &str, headers: HeaderMap) -> Result<impl HttpResponse> {
    #[allow(unused_mut)]
    let mut provider = TlsProvider::NativeTls;

    #[cfg(feature = "rustls")]
    {
        provider = TlsProvider::Rustls;
    }

    let config = Agent::config_builder()
        .tls_config(ureq::tls::TlsConfig::builder().provider(provider).build())
        .build();
    let agent = Agent::new_with_config(config);
    let mut req = agent.get(url);

    for (key, value) in headers.into_iter() {
        if let Some(key) = key {
            req = req.header(key, value);
        }
    }

    let res = req.call()?;

    if !res.status().is_success() {
        bail!(
            Error::Network,
            "api request failed with status: {:?} - for: {:?}",
            res.status(),
            url
        )
    }

    res.headers();

    Ok(res)
}

impl HttpResponse for Response<Body> {
    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        Response::headers(&self)
    }

    fn status(&self) -> http::StatusCode {
        Response::status(&self)
    }

    fn body(self) -> impl std::io::Read {
        self.into_body().into_reader()
    }

    fn json<T: serde::de::DeserializeOwned>(mut self) -> Result<T> {
        Ok(self.body_mut().read_json::<T>()?)
    }

    fn text(mut self) -> Result<String> {
        Ok(self.body_mut().read_to_string()?)
    }
}
