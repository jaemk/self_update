//! End-to-end coverage for the permissions-UX preflight (#112), driven entirely through the public
//! API of the `custom` backend.
//!
//! The `custom` backend performs its release *listing* in-process via a [`ReleaseSource`] (no HTTP),
//! and routes only the crate-controlled *download* through the injected
//! [`HttpClient`](self_update::http_client::HttpClient). That split makes the central claim of the
//! feature cleanly observable: when the opt-in preflight
//! ([`check_install_path_writable(true)`](self_update::backends)) refuses because the install path
//! is definitely not writable, the recording download client must see **zero** requests -- nothing
//! is downloaded. When the preflight is off (the default) or indeterminate (a missing parent dir),
//! the flow must proceed *past* the probe and reach the download, so the recording client *does* get
//! a request.
//!
//! The unix-permission scenarios (a read-only `0555` install dir) are gated `#[cfg(unix)]`; the
//! indeterminate-parent scenario is cross-platform. No unix-only imports live at the top level, so
//! the file compiles on Windows.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use self_update::Error;
use self_update::backends::custom;
use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};
use self_update::update::{Release, ReleaseAsset, ReleaseSource};

/// A download transport that records every URL it is asked to GET and then fails the transfer. It is
/// never expected to produce a real body -- the tests assert on *whether* it was hit, and the
/// deliberate transport error stops the update right after the request so nothing touches the real
/// filesystem beyond the temp archive dir.
struct RecordingClient {
    hits: Arc<Mutex<Vec<String>>>,
}

impl HttpClient for RecordingClient {
    fn get(
        &self,
        url: &str,
        _headers: &HeaderMap,
        _timeout: Option<Duration>,
    ) -> self_update::Result<Box<dyn HttpResponse>> {
        self.hits.lock().unwrap().push(url.to_string());
        Err(Error::transport("recording client: no body served"))
    }
}

/// A canned in-process release source: one release, newer than the driving current version, carrying
/// a single asset. The listing therefore never hits the network, so the only requests a driving
/// updater can make are downloads through the injected client.
struct CannedSource {
    releases: Vec<Release>,
}

impl CannedSource {
    /// One release `9.9.9` (strictly newer than the `0.0.1` the tests configure) with a single,
    /// traversal-safe asset pointing at an unroutable loopback URL. The asset is never actually
    /// fetched in the refusal tests; in the proceed tests the recording client intercepts it.
    fn one_newer() -> Self {
        let asset = ReleaseAsset::new("app-9.9.9.tar.gz", "http://127.0.0.1:9/app-9.9.9.tar.gz");
        let release = Release::builder()
            .version("9.9.9")
            .asset(asset)
            .build()
            .unwrap();
        Self {
            releases: vec![release],
        }
    }
}

impl ReleaseSource for CannedSource {
    fn get_releases(&self) -> self_update::Result<Vec<Release>> {
        Ok(self.releases.clone())
    }
}

/// Whether `dir` genuinely rejects writes for this process. A `0555` directory is *not* write
/// protected when the tests run as root (the DAC check is bypassed), so the permission-refusal
/// assertions guard on this to avoid a spurious failure in a root CI container: if the dir is
/// writable anyway, there is nothing for the preflight to refuse and the test is skipped.
#[cfg(unix)]
fn is_write_protected(dir: &std::path::Path) -> bool {
    match tempfile::Builder::new()
        .prefix(".permcheck")
        .tempfile_in(dir)
    {
        Ok(_) => false,
        Err(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
    }
}

/// Create a `0555` (read + execute, no write) child directory under `parent` and return its path.
#[cfg(unix)]
fn make_readonly_dir(parent: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let ro = parent.join("ro");
    std::fs::create_dir(&ro).unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    ro
}

/// Restore write permission so the enclosing `TempDir` can be cleaned up.
#[cfg(unix)]
fn restore_writable(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
}

// (a) Preflight ON + a definitely-unwritable (0555) install dir: `update()` must fail with
// `InstallPathNotWritable` naming the configured install path, and the injected download client must
// have recorded ZERO requests. This is the load-bearing guarantee: on a definite refusal, NOTHING is
// downloaded.
#[cfg(unix)]
#[test]
fn preflight_on_unwritable_dir_refuses_before_any_download() {
    let tmp = tempfile::tempdir().unwrap();
    let ro_dir = make_readonly_dir(tmp.path());

    if !is_write_protected(&ro_dir) {
        // Running as root: a 0555 dir is still writable, so there is nothing to refuse. Skip.
        restore_writable(&ro_dir);
        eprintln!("skipping: install dir is writable (running as root?)");
        return;
    }

    let install_path = ro_dir.join("app");
    let hits = Arc::new(Mutex::new(Vec::new()));

    let updater = custom::Update::configure()
        .source(CannedSource::one_newer())
        .bin_name("app")
        .current_version("0.0.1")
        .bin_install_path(&install_path)
        .check_install_path_writable(true)
        .asset_matcher(|assets| assets.first().cloned())
        .no_confirm(true)
        .show_output(false)
        .http_client(Arc::new(RecordingClient { hits: hits.clone() }))
        .build()
        .unwrap();

    let result = updater.update();

    // Restore before asserting so a failed assertion still leaves the tempdir cleanable.
    restore_writable(&ro_dir);

    match result {
        Err(Error::InstallPathNotWritable { path, .. }) => {
            assert_eq!(
                path, install_path,
                "the preflight error must name the configured bin_install_path"
            );
        }
        other => panic!("expected Err(InstallPathNotWritable), got {other:?}"),
    }

    assert!(
        hits.lock().unwrap().is_empty(),
        "a refused preflight must download NOTHING, but the client saw requests: {:?}",
        hits.lock().unwrap()
    );
}

// (c) Preflight OFF (the default) + the same unwritable (0555) dir: the flow must NOT short-circuit
// on writability before the network. It proceeds past the (absent) probe and reaches the download,
// so the recording client gets a request and the surfaced error is the download's transport failure,
// never `InstallPathNotWritable` raised ahead of any request.
#[cfg(unix)]
#[test]
fn preflight_off_unwritable_dir_proceeds_to_download() {
    let tmp = tempfile::tempdir().unwrap();
    let ro_dir = make_readonly_dir(tmp.path());
    let install_path = ro_dir.join("app");
    let hits = Arc::new(Mutex::new(Vec::new()));

    let updater = custom::Update::configure()
        .source(CannedSource::one_newer())
        .bin_name("app")
        .current_version("0.0.1")
        .bin_install_path(&install_path)
        // check_install_path_writable left at its default (false).
        .asset_matcher(|assets| assets.first().cloned())
        .no_confirm(true)
        .show_output(false)
        .http_client(Arc::new(RecordingClient { hits: hits.clone() }))
        .build()
        .unwrap();

    let result = updater.update();

    restore_writable(&ro_dir);

    assert!(
        !matches!(result, Err(Error::InstallPathNotWritable { .. })),
        "with the preflight off, writability must not be raised before a download is attempted, \
         got {result:?}"
    );
    assert!(
        !hits.lock().unwrap().is_empty(),
        "with the preflight off, the update must proceed to the download and hit the client"
    );
}

// (d) Preflight ON but the case is indeterminate: the install path's parent directory does not
// exist. The probe cannot get a definite PermissionDenied (the temp-sibling create fails with
// NotFound), so it must return Ok and let the flow proceed to the download -- the recording client
// gets a request, and no `InstallPathNotWritable` is raised at the probe. Cross-platform (no unix
// permissions involved).
#[test]
fn preflight_on_indeterminate_missing_parent_proceeds_to_download() {
    let tmp = tempfile::tempdir().unwrap();
    // `no-such-dir` is never created, so the parent of the install path is missing.
    let install_path = tmp.path().join("no-such-dir").join("app");
    let hits = Arc::new(Mutex::new(Vec::new()));

    let updater = custom::Update::configure()
        .source(CannedSource::one_newer())
        .bin_name("app")
        .current_version("0.0.1")
        .bin_install_path(&install_path)
        .check_install_path_writable(true)
        .asset_matcher(|assets| assets.first().cloned())
        .no_confirm(true)
        .show_output(false)
        .http_client(Arc::new(RecordingClient { hits: hits.clone() }))
        .build()
        .unwrap();

    let result = updater.update();

    assert!(
        !matches!(result, Err(Error::InstallPathNotWritable { .. })),
        "an indeterminate (missing parent) preflight must proceed, not refuse, got {result:?}"
    );
    assert!(
        !hits.lock().unwrap().is_empty(),
        "an indeterminate preflight must proceed to the download and hit the client"
    );
}

// --- async parity --------------------------------------------------------------------------------

#[cfg(feature = "async")]
mod async_parity {
    use super::*;
    use self_update::futures_util::future::BoxFuture;
    use self_update::http_client::{AsyncHttpClient, AsyncHttpResponse};
    use self_update::update::AsyncReleaseSource;

    /// Async sibling of [`RecordingClient`]: records the download URL then fails the transfer.
    struct RecordingAsyncClient {
        hits: Arc<Mutex<Vec<String>>>,
    }

    impl AsyncHttpClient for RecordingAsyncClient {
        fn get<'a>(
            &'a self,
            url: &'a str,
            _headers: &'a HeaderMap,
            _timeout: Option<Duration>,
        ) -> BoxFuture<'a, self_update::Result<Box<dyn AsyncHttpResponse>>> {
            let hits = self.hits.clone();
            let url = url.to_string();
            Box::pin(async move {
                hits.lock().unwrap().push(url);
                Err(Error::transport("recording async client: no body served"))
            })
        }
    }

    /// Async sibling of [`CannedSource`].
    struct CannedAsyncSource {
        releases: Vec<Release>,
    }

    impl CannedAsyncSource {
        fn one_newer() -> Self {
            Self {
                releases: CannedSource::one_newer().releases,
            }
        }
    }

    impl AsyncReleaseSource for CannedAsyncSource {
        fn get_releases(
            &self,
        ) -> impl std::future::Future<Output = self_update::Result<Vec<Release>>> + Send + '_
        {
            let releases = self.releases.clone();
            async move { Ok(releases) }
        }
    }

    // (b) Async parity for (a): a refused preflight through `build_async()`/`update_async` downloads
    // NOTHING -- `InstallPathNotWritable` naming the path, zero requests to the async client.
    #[cfg(unix)]
    #[tokio::test]
    async fn preflight_on_unwritable_dir_refuses_before_any_download_async() {
        let tmp = tempfile::tempdir().unwrap();
        let ro_dir = make_readonly_dir(tmp.path());

        if !is_write_protected(&ro_dir) {
            restore_writable(&ro_dir);
            eprintln!("skipping: install dir is writable (running as root?)");
            return;
        }

        let install_path = ro_dir.join("app");
        let hits = Arc::new(Mutex::new(Vec::new()));

        let updater = custom::AsyncUpdate::<CannedAsyncSource>::configure()
            .source(CannedAsyncSource::one_newer())
            .bin_name("app")
            .current_version("0.0.1")
            .bin_install_path(&install_path)
            .check_install_path_writable(true)
            .asset_matcher(|assets| assets.first().cloned())
            .no_confirm(true)
            .show_output(false)
            .http_client_async(Arc::new(RecordingAsyncClient { hits: hits.clone() }))
            .build_async()
            .unwrap();

        let result = updater.update_async().await;

        restore_writable(&ro_dir);

        match result {
            Err(Error::InstallPathNotWritable { path, .. }) => {
                assert_eq!(
                    path, install_path,
                    "the async preflight error must name the configured bin_install_path"
                );
            }
            other => panic!("expected Err(InstallPathNotWritable), got {other:?}"),
        }

        assert!(
            hits.lock().unwrap().is_empty(),
            "a refused async preflight must download NOTHING, but the client saw: {:?}",
            hits.lock().unwrap()
        );
    }

    // Async parity for the indeterminate case: a missing parent dir proceeds to the download.
    #[tokio::test]
    async fn preflight_on_indeterminate_missing_parent_proceeds_to_download_async() {
        let tmp = tempfile::tempdir().unwrap();
        let install_path = tmp.path().join("no-such-dir").join("app");
        let hits = Arc::new(Mutex::new(Vec::new()));

        let updater = custom::AsyncUpdate::<CannedAsyncSource>::configure()
            .source(CannedAsyncSource::one_newer())
            .bin_name("app")
            .current_version("0.0.1")
            .bin_install_path(&install_path)
            .check_install_path_writable(true)
            .asset_matcher(|assets| assets.first().cloned())
            .no_confirm(true)
            .show_output(false)
            .http_client_async(Arc::new(RecordingAsyncClient { hits: hits.clone() }))
            .build_async()
            .unwrap();

        let result = updater.update_async().await;

        assert!(
            !matches!(result, Err(Error::InstallPathNotWritable { .. })),
            "an indeterminate async preflight must proceed, not refuse, got {result:?}"
        );
        assert!(
            !hits.lock().unwrap().is_empty(),
            "an indeterminate async preflight must proceed to the download and hit the client"
        );
    }
}
