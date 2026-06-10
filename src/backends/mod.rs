/*!
Collection of modules supporting various release distribution backends
*/

use crate::errors::Result;
use crate::http_client;

pub(crate) mod common;
pub mod custom;
pub mod gitea;
pub mod github;
pub mod gitlab;
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

/// Accumulate items across `Link: rel="next"`-paginated pages, starting at `first_url`.
///
/// `fetch_page` performs one request and returns that page's items plus the next page's URL
/// (`None` when there is no `rel="next"` link). At most [`MAX_RELEASE_PAGES`] pages are walked;
/// if a further page is still advertised at that point, a warning is logged and the walk stops
/// (returning what was collected) rather than looping unbounded.
pub(crate) fn collect_paginated<T>(
    first_url: &str,
    mut fetch_page: impl FnMut(&str) -> Result<(Vec<T>, Option<String>)>,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    let mut next = Some(first_url.to_owned());
    let mut pages = 0usize;
    while let Some(url) = next {
        let (items, next_url) = fetch_page(&url)?;
        out.extend(items);
        pages += 1;
        if pages >= MAX_RELEASE_PAGES {
            if next_url.is_some() {
                log::warn!(
                    "self_update: stopped paginating releases after {MAX_RELEASE_PAGES} pages; \
                     older releases may be omitted"
                );
            }
            break;
        }
        next = next_url;
    }
    Ok(out)
}

/// Issue a GET request, merging the per-request transport `config` (extra headers + timeout)
/// on top of the supplied `base` headers, retrying a failed request up to `config.retries`
/// times with exponential backoff.
pub(crate) fn send(
    url: &str,
    mut base: http_client::HeaderMap,
    config: &common::RequestConfig,
) -> Result<impl http_client::HttpResponse> {
    for (name, value) in &config.headers {
        base.insert(name.clone(), value.clone());
    }
    let mut attempt = 0u32;
    loop {
        match http_client::get(url, base.clone(), config.timeout, &config.client) {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                if attempt >= config.retries {
                    return Err(e);
                }
                // Exponential backoff: 100ms, 200ms, 400ms, … capped at ~3.2s.
                let backoff = 100u64 << attempt.min(5);
                log::warn!("self_update: request to {url} failed ({e}); retrying in {backoff}ms");
                std::thread::sleep(std::time::Duration::from_millis(backoff));
                attempt += 1;
            }
        }
    }
}

/// Async sibling of [`send`]: issue a GET, merging the per-request transport `config` on top of
/// `base`, retrying up to `config.retries` times with `tokio::time::sleep` backoff.
#[cfg(feature = "async")]
pub(crate) async fn send_async(
    url: &str,
    mut base: http_client::HeaderMap,
    config: &common::RequestConfig,
) -> Result<http_client::AsyncResponse> {
    for (name, value) in &config.headers {
        base.insert(name.clone(), value.clone());
    }
    let mut attempt = 0u32;
    loop {
        match http_client::get_async(url, base.clone(), config.timeout, &config.client).await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                if attempt >= config.retries {
                    return Err(e);
                }
                // Exponential backoff: 100ms, 200ms, 400ms, … capped at ~3.2s.
                let backoff = 100u64 << attempt.min(5);
                log::warn!("self_update: request to {url} failed ({e}); retrying in {backoff}ms");
                tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                attempt += 1;
            }
        }
    }
}

/// Async sibling of [`collect_paginated`]: accumulate items across `Link: rel="next"` pages.
///
/// `fetch_page` takes an owned page URL (so it can be captured across the `await`) and returns that
/// page's items plus the next page URL. Bounded by [`MAX_RELEASE_PAGES`].
#[cfg(feature = "async")]
pub(crate) async fn collect_paginated_async<T, F, Fut>(
    first_url: &str,
    mut fetch_page: F,
) -> Result<Vec<T>>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<T>, Option<String>)>>,
{
    let mut out = Vec::new();
    let mut next = Some(first_url.to_owned());
    let mut pages = 0usize;
    while let Some(url) = next {
        let (items, next_url) = fetch_page(url).await?;
        out.extend(items);
        pages += 1;
        if pages >= MAX_RELEASE_PAGES {
            if next_url.is_some() {
                log::warn!(
                    "self_update: stopped paginating releases after {MAX_RELEASE_PAGES} pages; \
                     older releases may be omitted"
                );
            }
            break;
        }
        next = next_url;
    }
    Ok(out)
}

#[cfg(test)]
mod test {
    use crate::backends::find_rel_next_link;

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

    #[test]
    fn collect_paginated_accumulates_pages() {
        use crate::backends::collect_paginated;

        // Three pages of items, then no more `next` link.
        let mut pages = vec![
            (vec![1, 2], Some("page2".to_string())),
            (vec![3], Some("page3".to_string())),
            (vec![4, 5], None),
        ]
        .into_iter();
        let visited = std::cell::RefCell::new(Vec::new());
        let got = collect_paginated::<i32>("page1", |url| {
            visited.borrow_mut().push(url.to_string());
            Ok(pages.next().unwrap())
        })
        .unwrap();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
        assert_eq!(*visited.borrow(), vec!["page1", "page2", "page3"]);
    }

    #[test]
    fn collect_paginated_is_bounded_by_max_pages() {
        use crate::backends::{collect_paginated, MAX_RELEASE_PAGES};

        // A server that always advertises a next page must not loop forever.
        let mut calls = 0usize;
        let got = collect_paginated::<i32>("start", |_url| {
            calls += 1;
            Ok((vec![0], Some("next".to_string())))
        })
        .unwrap();
        assert_eq!(calls, MAX_RELEASE_PAGES);
        assert_eq!(got.len(), MAX_RELEASE_PAGES);
    }

    #[test]
    fn collect_paginated_single_page() {
        use crate::backends::collect_paginated;
        let mut calls = 0usize;
        let got = collect_paginated::<i32>("only", |_url| {
            calls += 1;
            Ok((vec![7, 8, 9], None))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(got, vec![7, 8, 9]);
    }

    #[test]
    fn collect_paginated_propagates_fetch_error() {
        use crate::backends::collect_paginated;
        use crate::errors::Error;
        let res: crate::errors::Result<Vec<i32>> =
            collect_paginated("u", |_url| Err(Error::Network("boom".to_string())));
        assert!(matches!(res, Err(Error::Network(_))));
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
