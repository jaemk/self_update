#![cfg(feature = "ureq")]

use ureq::tls::TlsProvider;
use ureq::{http::Response, Agent, Body};

use super::{ClientOverride, HeaderMap, HttpResponse};
use crate::{Error, Result};

pub fn get(
    url: &str,
    headers: HeaderMap,
    timeout: Option<std::time::Duration>,
    client: &ClientOverride,
) -> Result<impl HttpResponse> {
    // Use the injected agent if present (by reference — `Agent::get` takes `&self`), otherwise
    // build one per call. An injected agent owns its own timeout/TLS/proxy config, so the
    // per-request `timeout` is only applied to the per-call agent (see the `### Custom HTTP client`
    // docs).
    let built_agent;
    let agent: &Agent = match &client.agent {
        Some(agent) => agent,
        None => {
            #[cfg(feature = "rustls")]
            let provider = TlsProvider::Rustls;
            #[cfg(not(feature = "rustls"))]
            let provider = TlsProvider::NativeTls;

            let config = Agent::config_builder()
                .tls_config(ureq::tls::TlsConfig::builder().provider(provider).build())
                .timeout_global(timeout)
                // Honor HTTP(S)_PROXY / NO_PROXY env vars (reqwest does this automatically).
                .proxy(ureq::Proxy::try_from_env())
                .build();
            built_agent = Agent::new_with_config(config);
            &built_agent
        }
    };
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

    Ok(res)
}

impl HttpResponse for Response<Body> {
    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        Response::headers(self)
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
