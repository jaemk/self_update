/*!
Collection of modules supporting various release distribution backends
*/

use crate::errors::{Error, Result};
use crate::http_client;

pub(crate) mod common;
pub mod custom;
#[cfg(feature = "gitea")]
pub mod gitea;
#[cfg(feature = "github")]
pub mod github;
#[cfg(feature = "gitlab")]
pub mod gitlab;
#[cfg(feature = "s3")]
pub mod s3;

/// Search for the first "rel" link-header uri in a full link header string.
/// Seems like reqwest/hyper threw away their link-header parser implementation...
///
/// ex:
/// `Link: <https://api.github.com/resource?page=2>; rel="next"`
/// `Link: <https://gitlab.com/api/v4/projects/13083/releases?id=13083&page=2&per_page=20>; rel="next"`
///
/// https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Link
/// header values may contain multiple values separated by commas
/// `Link: <https://place.com>; rel="next", <https://wow.com>; rel="next"`
pub(crate) fn find_rel_next_link(link_str: &str) -> Option<&str> {
    for link in link_str.split(',') {
        let mut uri = None;
        let mut is_rel_next = false;
        for part in link.split(';') {
            let part = part.trim();
            if part.starts_with('<') && part.ends_with('>') {
                uri = Some(part.trim_start_matches('<').trim_end_matches('>'));
            } else if part.starts_with("rel=") {
                let part = part
                    .trim_start_matches("rel=")
                    .trim_end_matches('"')
                    .trim_start_matches('"');
                if part == "next" {
                    is_rel_next = true;
                }
            }

            if is_rel_next && uri.is_some() {
                return uri;
            }
        }
    }
    None
}

/// Maximum number of `Link: rel="next"` pages walked when listing releases — a safety bound
/// against pathological release histories.
pub(crate) const MAX_RELEASE_PAGES: usize = 100;

/// A sans-io description of a single page request: *what* to fetch (`url` + `headers`) and *how*
/// to parse the response body into items, the next page (if any), and an early-stop signal — with
/// no transport. The two drivers ([`run_paginated`] / [`run_paginated_async`]) perform the IO by
/// sending this via the shared `send`/`send_async` + `retry` machinery, then call `parse`.
///
/// `parse` is `+ Send` so [`run_paginated_async`]'s future stays `Send` (it is held across the
/// await in the async driver).
pub(crate) struct PageRequest<T> {
    pub url: String,
    pub headers: http_client::HeaderMap,
    /// Pure parser: `(body bytes, response headers) -> this page's items + next page + early-stop`.
    #[allow(clippy::type_complexity)]
    pub parse: Box<dyn FnOnce(&[u8], &http_client::HeaderMap) -> Result<Page<T>> + Send>,
}

/// The parsed result of one [`PageRequest`]: the page's `items`, the optional `next` page request,
/// and an early-`stop` flag. The driver appends `items`, then stops if `stop` is set, `next` is
/// `None`, or the [`MAX_RELEASE_PAGES`] bound is hit.
pub(crate) struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<PageRequest<T>>,
    pub stop: bool,
}

impl<T> Page<T> {
    /// A terminal single-page result: these `items`, no next page, no early stop.
    pub(crate) fn last(items: Vec<T>) -> Self {
        Self {
            items,
            next: None,
            stop: false,
        }
    }
}

/// Drive a sans-io [`PageRequest`] chain to completion over the sync transport.
///
/// Loops: send the request via [`send`] (reusing its retry/backoff machinery), read the body bytes
/// once, call `parse`, extend the accumulator, then stop if `page.stop`, `page.next` is `None`, or
/// the [`MAX_RELEASE_PAGES`] bound is reached (logging a warning if a further page was still
/// advertised at the bound).
pub(crate) fn run_paginated<T>(
    first: PageRequest<T>,
    config: &common::RequestConfig,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    let mut next = Some(first);
    let mut pages = 0usize;
    while let Some(request) = next {
        let PageRequest {
            url,
            headers,
            parse,
        } = request;
        let resp = send(&url, headers, config)?;
        let resp_headers = resp.headers().clone();
        let mut body = Vec::new();
        let mut reader = resp.body();
        std::io::Read::read_to_end(&mut reader, &mut body)?;
        let page = parse(&body, &resp_headers)?;
        out.extend(page.items);
        pages += 1;
        if page.stop {
            break;
        }
        if pages >= MAX_RELEASE_PAGES {
            if page.next.is_some() {
                log::warn!(
                    "self_update: stopped paginating releases after {MAX_RELEASE_PAGES} pages; \
                     older releases may be omitted"
                );
            }
            break;
        }
        next = page.next;
    }
    Ok(out)
}

/// Async sibling of [`run_paginated`]: drive a sans-io [`PageRequest`] chain over the async
/// transport. Reuses [`send_async`]'s retry/backoff machinery; reads the body bytes via the async
/// response trait, then calls the same `parse` closure.
#[cfg(feature = "async")]
pub(crate) async fn run_paginated_async<T>(
    first: PageRequest<T>,
    config: &common::RequestConfig,
) -> Result<Vec<T>> {
    use futures_util::StreamExt;

    let mut out = Vec::new();
    let mut next = Some(first);
    let mut pages = 0usize;
    while let Some(request) = next {
        let PageRequest {
            url,
            headers,
            parse,
        } = request;
        let resp = send_async(&url, headers, config).await?;
        let resp_headers = resp.headers().clone();
        // Drain the streamed body into a single buffer (one full read, honoring the I7 intent of
        // not double-buffering: the bytes stream feeds the buffer directly).
        let mut stream = resp.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk?);
        }
        let page = parse(&body, &resp_headers)?;
        out.extend(page.items);
        pages += 1;
        if page.stop {
            break;
        }
        if pages >= MAX_RELEASE_PAGES {
            if page.next.is_some() {
                log::warn!(
                    "self_update: stopped paginating releases after {MAX_RELEASE_PAGES} pages; \
                     older releases may be omitted"
                );
            }
            break;
        }
        next = page.next;
    }
    Ok(out)
}

/// Build the first-page request URL, defaulting the page size to 100 — unless the base URL
/// already carries query parameters (e.g. a `Link`-header "next" URL), in which case it is used
/// verbatim so an existing `page`/`per_page` is not clobbered.
pub(crate) fn first_page_url(base_url: &str) -> String {
    if base_url.contains('?') {
        base_url.to_owned()
    } else {
        format!("{base_url}?per_page=100")
    }
}

/// Extract the `rel="next"` URL from a response's `Link` header(s), if present.
pub(crate) fn next_link(headers: &http_client::HeaderMap) -> Option<String> {
    headers
        .get_all(http_client::header::LINK)
        .iter()
        .filter_map(|link| link.to_str().ok().and_then(find_rel_next_link))
        .next()
        .map(str::to_owned)
}

/// Exponential backoff in milliseconds before retry `attempt` (0-based): 100, 200, 400, … capped
/// at ~3.2s (attempt 5 and beyond).
pub(crate) fn retry_backoff_ms(attempt: u32) -> u64 {
    100u64 << attempt.min(5)
}

/// Run `attempt` until it succeeds or the retry budget is spent, invoking `on_retry(err, backoff)`
/// (which logs the failure and sleeps) between tries. With `retries == 0` the attempt runs exactly
/// once. The transport and the sleep are injected so the retry/backoff control flow can be
/// unit-tested without real requests or real delays.
pub(crate) fn retry<R>(
    retries: u32,
    mut attempt: impl FnMut() -> Result<R>,
    mut on_retry: impl FnMut(&Error, u64),
) -> Result<R> {
    let mut attempts = 0u32;
    loop {
        match attempt() {
            Ok(r) => return Ok(r),
            Err(e) => {
                if attempts >= retries {
                    return Err(e);
                }
                on_retry(&e, retry_backoff_ms(attempts));
                attempts += 1;
            }
        }
    }
}

/// Async sibling of [`retry`]: the same retry/backoff loop with an injected async transport and
/// async `sleep`. `log_retry` runs synchronously between tries (so the error is never held across
/// the await); `sleep` performs the backoff delay.
#[cfg(feature = "async")]
pub(crate) async fn retry_async<R, A, Fut, S, SFut>(
    retries: u32,
    mut attempt: A,
    mut log_retry: impl FnMut(&Error, u64),
    mut sleep: S,
) -> Result<R>
where
    A: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<R>>,
    S: FnMut(u64) -> SFut,
    SFut: std::future::Future<Output = ()>,
{
    let mut attempts = 0u32;
    loop {
        match attempt().await {
            Ok(r) => return Ok(r),
            Err(e) => {
                if attempts >= retries {
                    return Err(e);
                }
                let backoff = retry_backoff_ms(attempts);
                log_retry(&e, backoff);
                sleep(backoff).await;
                attempts += 1;
            }
        }
    }
}

/// Issue a GET request, merging the per-request transport `config` (extra headers + timeout)
/// on top of the supplied `base` headers, retrying a failed request up to `config.retries`
/// times with exponential backoff.
pub(crate) fn send(
    url: &str,
    mut base: http_client::HeaderMap,
    config: &common::RequestConfig,
) -> Result<Box<dyn http_client::HttpResponse>> {
    for (name, value) in &config.headers {
        base.insert(name.clone(), value.clone());
    }
    // Dispatch through the injected client if present, else the crate's default per-call client.
    let default;
    let client: &dyn http_client::HttpClient = match config.client.as_deref() {
        Some(c) => c,
        None => {
            default = http_client::default_client();
            &*default
        }
    };
    retry(
        config.retries,
        || client.get(url, &base, config.timeout),
        |e, backoff| {
            log::warn!("self_update: request to {url} failed ({e}); retrying in {backoff}ms");
            std::thread::sleep(std::time::Duration::from_millis(backoff));
        },
    )
}

/// Async sibling of [`send`]: issue a GET, merging the per-request transport `config` on top of
/// `base`, retrying up to `config.retries` times with `tokio::time::sleep` backoff.
#[cfg(feature = "async")]
pub(crate) async fn send_async(
    url: &str,
    mut base: http_client::HeaderMap,
    config: &common::RequestConfig,
) -> Result<Box<dyn http_client::AsyncHttpResponse>> {
    for (name, value) in &config.headers {
        base.insert(name.clone(), value.clone());
    }
    let default;
    let client: &dyn http_client::AsyncHttpClient = match config.async_client.as_deref() {
        Some(c) => c,
        None => {
            default = http_client::default_async_client();
            &*default
        }
    };
    retry_async(
        config.retries,
        || client.get(url, &base, config.timeout),
        |e, backoff| {
            log::warn!("self_update: request to {url} failed ({e}); retrying in {backoff}ms");
        },
        |backoff| tokio::time::sleep(std::time::Duration::from_millis(backoff)),
    )
    .await
}

#[cfg(test)]
mod test {
    use crate::backends::common::RequestConfig;
    use crate::backends::find_rel_next_link;
    use crate::backends::{Page, PageRequest};
    use crate::http_client::HeaderMap;

    #[test]
    fn test_find_rel_link() {
        let val = r##" <https://api.github.com/resource?page=2>; rel="next" "##;
        let link = find_rel_next_link(val);
        assert_eq!(link, Some("https://api.github.com/resource?page=2"));

        let val = r##" <https://gitlab.com/api/v4/projects/13083/releases?id=13083&page=2&per_page=20>; rel="next" "##;
        let link = find_rel_next_link(val);
        assert_eq!(
            link,
            Some("https://gitlab.com/api/v4/projects/13083/releases?id=13083&page=2&per_page=20")
        );

        // returns the first one
        let val = r##" <https://place.com>; rel="next", <https://wow.com>; rel="next" "##;
        let link = find_rel_next_link(val);
        assert_eq!(link, Some("https://place.com"));

        // bad format, returns the second one
        let val = r##" https://bad-format.com; rel="next", <https://wow.com>; rel="next" "##;
        let link = find_rel_next_link(val);
        assert_eq!(link, Some("https://wow.com"));

        // all bad format, returns none
        let val = r##" https://bad-format.com; rel="next", <https://also-bad.com; rel="next" , <https://good.com>; rel="preconnect" "##;
        let link = find_rel_next_link(val);
        assert!(link.is_none());
    }

    // -----------------------------------------------------------------------
    // run_paginated: sans-io page-chain driver (over a loopback TCP stub)
    // -----------------------------------------------------------------------

    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// A `PageRequest<i32>` that parses the body as a comma-separated list of ints and follows the
    /// `rel="next"` Link header (never setting the early-stop flag).
    fn int_page(url: String) -> PageRequest<i32> {
        PageRequest {
            url,
            headers: HeaderMap::new(),
            parse: Box::new(move |body, headers| {
                let text = std::str::from_utf8(body).unwrap_or("");
                let items: Vec<i32> = text
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse::<i32>().unwrap())
                    .collect();
                let next = crate::backends::next_link(headers).map(int_page);
                Ok(Page {
                    items,
                    next,
                    stop: false,
                })
            }),
        }
    }

    /// Build a stub serving pages whose `Link` next-URLs are wired to the stub's own base via the
    /// supplied paths. `specs` is `(next_path, body)` per page; a `None` next_path is the last page.
    fn linked_stub(
        specs: Vec<(Option<&str>, &str)>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        // Bind first so we know the base, then resolve the relative next-paths against it.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let pages: Vec<(Option<String>, String)> = specs
            .into_iter()
            .map(|(next, body)| (next.map(|p| format!("{base}{p}")), body.to_string()))
            .collect();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        std::thread::spawn(move || {
            for (link, body) in pages {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                sink.lock()
                    .unwrap()
                    .push(req.lines().next().unwrap_or("").to_string());
                let mut out = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n".to_string();
                if let Some(link) = link {
                    out.push_str(&format!("Link: <{link}>; rel=\"next\"\r\n"));
                }
                out.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                ));
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        (base, captured)
    }

    #[test]
    fn run_paginated_accumulates_pages() {
        // Three pages, the first two advertising a `rel="next"` link to the next.
        let (base, captured) = linked_stub(vec![
            (Some("/p2"), "1,2"),
            (Some("/p3"), "3"),
            (None, "4,5"),
        ]);
        let got = crate::backends::run_paginated(
            int_page(format!("{base}/p1")),
            &RequestConfig::default(),
        )
        .unwrap();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
        let paths = captured.lock().unwrap();
        assert_eq!(paths.len(), 3, "exactly three pages were requested");
        assert!(paths[0].contains("/p1"));
        assert!(paths[1].contains("/p2"));
        assert!(paths[2].contains("/p3"));
    }

    #[test]
    fn run_paginated_single_page() {
        let (base, captured) = linked_stub(vec![(None, "7,8,9")]);
        let got = crate::backends::run_paginated(
            int_page(format!("{base}/only")),
            &RequestConfig::default(),
        )
        .unwrap();
        assert_eq!(got, vec![7, 8, 9]);
        assert_eq!(captured.lock().unwrap().len(), 1, "one request only");
    }

    #[test]
    fn run_paginated_stops_early_on_page_stop_flag() {
        // Page 1 advertises a next page but sets `stop=true`; the driver must NOT request page 2.
        let (base, captured) = linked_stub(vec![
            (Some("/never"), "1,2"),
            (None, "should-not-be-served"),
        ]);
        let stopping = PageRequest {
            url: format!("{base}/p1"),
            headers: HeaderMap::new(),
            parse: Box::new(|body: &[u8], _headers: &HeaderMap| {
                let text = std::str::from_utf8(body).unwrap_or("");
                let items: Vec<i32> = text.split(',').map(|s| s.parse().unwrap()).collect();
                // Pretend the server still advertises a next page but we stop early.
                Ok(Page {
                    items,
                    next: Some(int_page("http://unused.invalid/never".into())),
                    stop: true,
                })
            }),
        };
        let got = crate::backends::run_paginated(stopping, &RequestConfig::default()).unwrap();
        assert_eq!(got, vec![1, 2]);
        assert_eq!(
            captured.lock().unwrap().len(),
            1,
            "stop=true must halt after page 1; page 2 must never be requested"
        );
    }

    #[test]
    fn run_paginated_is_bounded_by_max_pages() {
        use crate::backends::MAX_RELEASE_PAGES;
        // A server that always advertises a next page (pointing back at itself) must not loop
        // forever — the driver is bounded by MAX_RELEASE_PAGES.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let base_for_thread = base.clone();
        std::thread::spawn(move || {
            loop {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "0";
                let out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nLink: <{base_for_thread}/n>; rel=\"next\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        let got = crate::backends::run_paginated(
            int_page(format!("{base}/start")),
            &RequestConfig::default(),
        )
        .unwrap();
        assert_eq!(
            got.len(),
            MAX_RELEASE_PAGES,
            "the walk is bounded at MAX_RELEASE_PAGES even when next is always advertised"
        );
    }

    // -----------------------------------------------------------------------
    // run_paginated_async: direct coverage of the async page-chain driver
    // (body-drain via bytes_stream + async early-stop), mirroring the sync tests.
    // -----------------------------------------------------------------------

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn run_paginated_async_accumulates_pages() {
        let (base, captured) = linked_stub(vec![
            (Some("/p2"), "1,2"),
            (Some("/p3"), "3"),
            (None, "4,5"),
        ]);
        let got = crate::backends::run_paginated_async(
            int_page(format!("{base}/p1")),
            &RequestConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
        let paths = captured.lock().unwrap();
        assert_eq!(paths.len(), 3, "exactly three pages were requested");
        assert!(paths[0].contains("/p1"));
        assert!(paths[1].contains("/p2"));
        assert!(paths[2].contains("/p3"));
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn run_paginated_async_stops_early_on_page_stop_flag() {
        // Page 1 advertises a next page but sets `stop=true`; the async driver must NOT request
        // page 2 (same contract as the sync driver).
        let (base, captured) = linked_stub(vec![
            (Some("/never"), "1,2"),
            (None, "should-not-be-served"),
        ]);
        let stopping = PageRequest {
            url: format!("{base}/p1"),
            headers: HeaderMap::new(),
            parse: Box::new(|body: &[u8], _headers: &HeaderMap| {
                let text = std::str::from_utf8(body).unwrap_or("");
                let items: Vec<i32> = text.split(',').map(|s| s.parse().unwrap()).collect();
                Ok(Page {
                    items,
                    next: Some(int_page("http://unused.invalid/never".into())),
                    stop: true,
                })
            }),
        };
        let got = crate::backends::run_paginated_async(stopping, &RequestConfig::default())
            .await
            .unwrap();
        assert_eq!(got, vec![1, 2]);
        assert_eq!(
            captured.lock().unwrap().len(),
            1,
            "stop=true must halt after page 1; page 2 must never be requested (async)"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn run_paginated_async_is_bounded_by_max_pages() {
        use crate::backends::MAX_RELEASE_PAGES;
        // A server always advertising a next page (pointing back at itself) must not loop forever;
        // the async driver is bounded by MAX_RELEASE_PAGES just like the sync one.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let base_for_thread = base.clone();
        std::thread::spawn(move || {
            loop {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "0";
                let out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nLink: <{base_for_thread}/n>; rel=\"next\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        let got = crate::backends::run_paginated_async(
            int_page(format!("{base}/start")),
            &RequestConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            got.len(),
            MAX_RELEASE_PAGES,
            "the async walk is bounded at MAX_RELEASE_PAGES even when next is always advertised"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn run_paginated_async_propagates_fetch_error() {
        // A non-2xx status on the first page must propagate as the structured error over the async
        // transport, before any accumulation.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "boom";
                let out = format!(
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        let res = crate::backends::run_paginated_async(
            int_page(format!("{base}/p1")),
            &RequestConfig::default(),
        )
        .await;
        assert!(matches!(
            res,
            Err(crate::errors::Error::HttpStatus { status: 503, .. })
        ));
    }

    #[test]
    fn run_paginated_propagates_fetch_error() {
        // A non-2xx status on the first page must propagate as the structured error.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "boom";
                let out = format!(
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        let res = crate::backends::run_paginated(
            int_page(format!("{base}/p1")),
            &RequestConfig::default(),
        );
        assert!(matches!(
            res,
            Err(crate::errors::Error::HttpStatus { status: 503, .. })
        ));
    }

    #[test]
    fn retry_runs_once_on_immediate_success() {
        use crate::backends::retry;
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0u32);
        let backoffs = RefCell::new(Vec::<u64>::new());
        let res: crate::errors::Result<i32> = retry(
            3,
            || {
                calls.set(calls.get() + 1);
                Ok(7)
            },
            |_e, b| backoffs.borrow_mut().push(b),
        );
        assert_eq!(res.unwrap(), 7);
        assert_eq!(calls.get(), 1);
        assert!(backoffs.borrow().is_empty());
    }

    #[test]
    fn retry_with_zero_budget_attempts_once_then_errors() {
        use crate::backends::retry;
        use crate::errors::Error;
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0u32);
        let backoffs = RefCell::new(Vec::<u64>::new());
        let res: crate::errors::Result<i32> = retry(
            0,
            || {
                calls.set(calls.get() + 1);
                Err(Error::HttpStatus {
                    status: 503,
                    url: "u".into(),
                })
            },
            |_e, b| backoffs.borrow_mut().push(b),
        );
        assert!(matches!(res, Err(Error::HttpStatus { .. })));
        assert_eq!(calls.get(), 1);
        assert!(backoffs.borrow().is_empty());
    }

    #[test]
    fn retry_exhausts_budget_then_returns_last_error() {
        use crate::backends::retry;
        use crate::errors::Error;
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0u32);
        let backoffs = RefCell::new(Vec::<u64>::new());
        let res: crate::errors::Result<i32> = retry(
            2,
            || {
                calls.set(calls.get() + 1);
                Err(Error::HttpStatus {
                    status: 503,
                    url: "u".into(),
                })
            },
            |_e, b| backoffs.borrow_mut().push(b),
        );
        assert!(matches!(res, Err(Error::HttpStatus { .. })));
        // initial attempt + 2 retries
        assert_eq!(calls.get(), 3);
        assert_eq!(*backoffs.borrow(), vec![100, 200]);
    }

    #[test]
    fn retry_returns_ok_when_a_later_attempt_succeeds() {
        use crate::backends::retry;
        use crate::errors::Error;
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0u32);
        let backoffs = RefCell::new(Vec::<u64>::new());
        let res: crate::errors::Result<i32> = retry(
            5,
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    Err(Error::HttpStatus {
                        status: 503,
                        url: "u".into(),
                    })
                } else {
                    Ok(42)
                }
            },
            |_e, b| backoffs.borrow_mut().push(b),
        );
        assert_eq!(res.unwrap(), 42);
        assert_eq!(calls.get(), 3);
        assert_eq!(*backoffs.borrow(), vec![100, 200]);
    }

    #[test]
    fn retry_backoff_is_exponential_and_capped() {
        use crate::backends::retry_backoff_ms;
        assert_eq!(retry_backoff_ms(0), 100);
        assert_eq!(retry_backoff_ms(1), 200);
        assert_eq!(retry_backoff_ms(2), 400);
        assert_eq!(retry_backoff_ms(3), 800);
        assert_eq!(retry_backoff_ms(4), 1600);
        assert_eq!(retry_backoff_ms(5), 3200);
        // capped from attempt 5 onward
        assert_eq!(retry_backoff_ms(6), 3200);
        assert_eq!(retry_backoff_ms(100), 3200);
    }

    #[test]
    fn retry_with_a_single_retry_attempts_twice() {
        use crate::backends::retry;
        use crate::errors::Error;
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0u32);
        let backoffs = RefCell::new(Vec::<u64>::new());
        let res: crate::errors::Result<i32> = retry(
            1,
            || {
                calls.set(calls.get() + 1);
                Err(Error::HttpStatus {
                    status: 503,
                    url: "u".into(),
                })
            },
            |_e, b| backoffs.borrow_mut().push(b),
        );
        assert!(matches!(res, Err(Error::HttpStatus { .. })));
        // initial attempt + 1 retry; the `>` vs `>=` budget boundary
        assert_eq!(calls.get(), 2);
        assert_eq!(*backoffs.borrow(), vec![100]);
    }

    #[test]
    fn retry_backoff_sequence_through_the_loop_climbs_and_caps() {
        use crate::backends::retry;
        use crate::errors::Error;
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0u32);
        let backoffs = RefCell::new(Vec::<u64>::new());
        // Six retries drive the in-loop attempt index from 0 through 5, so the recorded backoff
        // sequence must climb 100 -> 3200 and hit the cap at the final step — proving the loop
        // feeds the rising attempt index into `retry_backoff_ms`, not just index 0/1.
        let res: crate::errors::Result<i32> = retry(
            6,
            || {
                calls.set(calls.get() + 1);
                Err(Error::HttpStatus {
                    status: 503,
                    url: "u".into(),
                })
            },
            |_e, b| backoffs.borrow_mut().push(b),
        );
        assert!(matches!(res, Err(Error::HttpStatus { .. })));
        assert_eq!(calls.get(), 7);
        assert_eq!(*backoffs.borrow(), vec![100, 200, 400, 800, 1600, 3200]);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn retry_async_exhausts_budget_then_returns_last_error() {
        use crate::backends::retry_async;
        use crate::errors::Error;
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0u32);
        let backoffs = RefCell::new(Vec::<u64>::new());
        let res: crate::errors::Result<i32> = retry_async(
            2,
            || {
                calls.set(calls.get() + 1);
                async {
                    Err(Error::HttpStatus {
                        status: 503,
                        url: "u".into(),
                    })
                }
            },
            |_e, b| backoffs.borrow_mut().push(b),
            |_b| async {},
        )
        .await;
        assert!(matches!(res, Err(Error::HttpStatus { .. })));
        assert_eq!(calls.get(), 3);
        assert_eq!(*backoffs.borrow(), vec![100, 200]);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn retry_async_returns_ok_when_a_later_attempt_succeeds() {
        use crate::backends::retry_async;
        use crate::errors::Error;
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0u32);
        let backoffs = RefCell::new(Vec::<u64>::new());
        let res: crate::errors::Result<i32> = retry_async(
            5,
            || {
                calls.set(calls.get() + 1);
                let done = calls.get() >= 3;
                async move {
                    if done {
                        Ok(42)
                    } else {
                        Err(Error::HttpStatus {
                            status: 503,
                            url: "u".into(),
                        })
                    }
                }
            },
            |_e, b| backoffs.borrow_mut().push(b),
            |_b| async {},
        )
        .await;
        assert_eq!(res.unwrap(), 42);
        assert_eq!(calls.get(), 3);
        assert_eq!(*backoffs.borrow(), vec![100, 200]);
    }

    #[test]
    fn first_page_url_appends_per_page_only_when_no_query() {
        use crate::backends::first_page_url;
        assert_eq!(
            first_page_url("https://api.github.com/repos/o/r/releases"),
            "https://api.github.com/repos/o/r/releases?per_page=100"
        );
        // A URL that already has query params (e.g. a `Link` next URL) is left untouched.
        assert_eq!(
            first_page_url("https://api.github.com/repos/o/r/releases?page=2&per_page=20"),
            "https://api.github.com/repos/o/r/releases?page=2&per_page=20"
        );
    }

    #[test]
    fn next_link_extracts_rel_next_from_link_header() {
        use crate::backends::next_link;
        use crate::http_client::header::{HeaderMap, LINK};

        // No Link header -> None.
        assert_eq!(next_link(&HeaderMap::new()), None);

        // A single `rel="next"` link is returned.
        let mut headers = HeaderMap::new();
        headers.insert(
            LINK,
            "<https://api.example.com/r?page=2>; rel=\"next\""
                .parse()
                .unwrap(),
        );
        assert_eq!(
            next_link(&headers),
            Some("https://api.example.com/r?page=2".to_string())
        );

        // The `next` link is picked out from among other relations.
        let mut headers = HeaderMap::new();
        headers.insert(
            LINK,
            "<https://api.example.com/r?page=5>; rel=\"last\", <https://api.example.com/r?page=2>; rel=\"next\""
                .parse()
                .unwrap(),
        );
        assert_eq!(
            next_link(&headers),
            Some("https://api.example.com/r?page=2".to_string())
        );
    }
}
