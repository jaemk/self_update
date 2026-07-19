/*!

[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

Supported backends: **GitHub**, **GitLab**, **Gitea**, **Gitee**, **S3** (Amazon S3, Google GCS,
DigitalOcean Spaces, or any S3-compatible endpoint), and **Manifest** (any static file server).
The forge and S3 backends each expose a `ReleaseList` builder alongside the `Update`
(configure -> build -> update) API; the manifest backend exposes `Update` only.

## Quick start

```rust
use self_update::cargo_crate_version;

fn update() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}
```

> **Upgrading from 0.x?** 1.0 makes a focused set of breaking changes to clean up the public
> API. See the [1.0 migration guide](https://github.com/jaemk/self_update/blob/master/docs/migrations/0.x-to-1.0-human.md)
> for a step-by-step walkthrough, or the
> [agent-oriented guide](https://github.com/jaemk/self_update/blob/master/docs/migrations/0.x-to-1.0.md)
> for automated migration tooling.

> **Running unattended (daemon / CI / service)?** The defaults are interactive: `show_output`
> is `true` and `no_confirm` is `false`, so `update()` prints a release-status block to stdout
> and then **blocks on an interactive `yes/no` prompt** waiting on stdin. With no terminal
> attached this stalls (or aborts). For any non-interactive caller set `.no_confirm(true)` to
> skip the prompt, and usually `.show_output(false)` to silence the status block. These are
> settings only -- the defaults are unchanged. Note the status block is printed *before* the
> confirmation prompt, so suppressing one does not suppress the other.

## Usage

### Features

At least one HTTP client must be selected. A build with **no** client -- for example
`default-features = false` with only a TLS feature such as `features = ["rustls"]` -- fails to
compile with `no HTTP client selected - enable at least one of the reqwest (default) or ureq
features`. Add a client explicitly, e.g. `default-features = false, features = ["ureq", "rustls",
"github"]`. Multiple clients and multiple TLS backends may coexist (reqwest is preferred when both
are present):

* `reqwest` (default): use the [`reqwest`](https://docs.rs/reqwest) HTTP client;
* `ureq`: use the [`ureq`](https://docs.rs/ureq) HTTP client, either alongside reqwest or as a drop-in replacement (set `default-features = false` to drop reqwest);
* `rustls` (default): [pure-Rust TLS](https://github.com/rustls/rustls); does _not_ support 32-bit macOS;
* `native-tls`: opt-in native/OpenSSL TLS for the selected client;
* `native-tls-vendored`: build OpenSSL from source and link it statically (for targets where a usable system OpenSSL is awkward, e.g. musl or some cross-compiles); implies `native-tls`, applies to the reqwest client;

Note that enabling a client with neither TLS feature compiles (plain-`http` release hosts remain
reachable) but any `https` URL then fails at request time with a transport error; enable `rustls`
or `native-tls` for `https`.

The following [cargo features](https://doc.rust-lang.org/cargo/reference/manifest.html#the-features-section)
are enabled by default:

* `github`: the GitHub Releases backend;
* `progress-bar`: terminal download progress bar;

The following are opt-in; activate the one(s) your release files need:

* `gitlab`: the GitLab Releases backend;
* `gitea`: the Gitea Releases backend;
* `gitee`: the Gitee Releases backend;
* `s3`: the S3-compatible backend (Amazon S3, GCS, DigitalOcean Spaces, etc.);
* `s3-auth`: sign S3 requests (AWS SigV4) for private buckets; implies `s3`;
* `manifest`: the static-file manifest backend; fetches releases from a `manifest.json` served by any HTTP endpoint; no new dependencies;
* `archive-tar`: support for _tar_ archive format;
* `archive-zip`: support for _zip_ archive format;
* `compression-tar-gz`: support for _gzip_ compression (`.tar.gz`, `.tgz`, plain `.gz`);
* `compression-tar-xz`: support for _xz_ compression (`.tar.xz`, `.txz`, plain `.xz`); pure-Rust, no C `liblzma` dependency;
* `compression-zip-deflate`: support for _zip_'s _deflate_ compression format;
* `compression-zip-bzip2`: support for _zip_'s _bzip2_ compression format;
* `signatures`: use [zipsign](https://github.com/Kijewski/zipsign) to verify `.zip` and `.tar.gz` artifacts. Artifacts are assumed to have been signed using zipsign;
* `checksums`: verify a downloaded artifact against a SHA-256/SHA-512 checksum before installing it -- automatically against the digest github publishes per release asset, and/or against a known checksum you pass in (e.g. from a `SHA256SUMS` file); see [Checksum verification](#checksum-verification) below;
* `async`: add async (`*_async`) update methods alongside the unchanged blocking API; tokio-only, requires `reqwest` (ureq and reqwest can coexist -- reqwest serves the async path, and the sync API prefers reqwest when both are present); see [Async](#async) below.

`github` is the only backend in the default feature set. The S3 backend requires the `s3` feature; `s3-auth` implies `s3`. `gitlab`, `gitea`, `gitee`, and `manifest` each require their own feature.

### Example

Run the following example to see `self_update` in action:

`cargo run --example github --features "signatures archive-tar compression-tar-gz"`.

There are equivalent examples for the other backends (`gitlab`, `gitea`, `gitee`, `s3`), e.g.:

`cargo run --example gitlab --features "gitlab archive-tar compression-tar-gz"`.

Amazon S3, Google GCS, and DigitalOcean Spaces, as well as any S3 compatible server are also supported
through the `S3` backend to check for new releases.  Provided a `bucket_name`
and `asset_prefix` string, `self_update` will look up all matching files using the following format
as a convention for the filenames: `[directory/]<asset name>-<semver>-<platform/target>.<extension>`.
Leading directories will be stripped from the file name allowing the use of subdirectories in the S3 bucket,
and any file not matching the format, or not matching the provided prefix string, will be ignored.

```rust
# #[cfg(feature = "s3")]
# mod s3_example {
use self_update::cargo_crate_version;

fn update() -> Result<(), Box<dyn ::std::error::Error>> {
    let status = self_update::backends::s3::Update::configure()
        // .endpoint(self_update::backends::s3::Endpoint::GCS)
        // .endpoint("https://s3.example.com")
        .bucket_name("self_update_releases")
        .asset_prefix("something/self_update")
        .region("eu-west-2")
        .bin_name("self_update_example")
        // To authenticate (requires the `s3-auth` feature), read the credentials at
        // runtime rather than baking them into the binary with `env!`:
        // .access_key((std::env::var("AWS_ACCESS_KEY_ID")?, std::env::var("AWS_SECRET_ACCESS_KEY")?))
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("S3 Update status: `{}`!", status.version());
    Ok(())
}
# }
```

The `manifest` backend (`manifest` feature) serves releases from a `manifest.json` file hosted
on any static file server. The tool author publishes the manifest at a stable URL; assets may be
absolute URLs or relative paths resolved against that URL. Asset `digest` fields (`sha256:<hex>`)
plug into the existing checksum verification path when the `checksums` feature is on. See
`specs/ref-manifest-backend.md` for the full schema.

```rust
# #[cfg(feature = "manifest")]
# mod manifest_example {
use self_update::cargo_crate_version;

fn update() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::manifest::Update::configure()
        .manifest_url("https://example.net/releases/manifest.json")
        .bin_name("app")
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Manifest update status: `{}`!", status.version());
    Ok(())
}
# }
```

Separate utilities are also exposed (**NOTE**: the following example extracts a `.tar.gz`, which
_requires_ both the `archive-tar` and `compression-tar-gz` features -- `archive-tar` reads the tar
archive and `compression-tar-gz` decodes the gzip layer; see the [features](#features) section
above). It downloads, extracts, and replaces the running binary
by hand; the staging directory and the in-place replacement use the [`tempfile`](https://crates.io/crates/tempfile)
and [`self_replace`](https://crates.io/crates/self-replace) crates, which you add as your own dependencies
(they are no longer re-exported from `self_update`):

```rust
# #[cfg(feature = "archive-tar")]
fn update() -> Result<(), Box<dyn std::error::Error>> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    // get the first available release (`fetch` returns a `Releases`; `latest()` is the first entry)
    let latest = releases.latest().unwrap();
    let asset = latest
        .asset_for(&self_update::get_target(), None)
        .unwrap();

    let tmp_dir = tempfile::Builder::new()
            .prefix("self_update")
            .tempdir_in(::std::env::current_dir()?)?;
    let tmp_tarball_path = tmp_dir.path().join(asset.name());
    let tmp_tarball = ::std::fs::File::create(&tmp_tarball_path)?;

    self_update::Download::from_url(asset.download_url())
        .request_header(self_update::http::header::ACCEPT, "application/octet-stream")
        .download_to(&tmp_tarball)?;

    let bin_name = std::path::PathBuf::from("self_update_bin");
    self_update::Extract::from_source(&tmp_tarball_path)
        .archive(self_update::ArchiveKind::Tar(Some(self_update::Compression::Gz)))
        .extract_file(&tmp_dir.path(), &bin_name)?;

    let new_exe = tmp_dir.path().join(bin_name);
    self_replace::self_replace(new_exe)?;

    Ok(())
}
```

### Multi-file / non-executable install

The high-level `update()` flow replaces a single executable. To update a tool that ships **more
than one file** (a binary plus sidecar libraries/resources), or to install files that aren't the
running executable, download and extract the whole archive yourself and then install the files
with `MoveAll`, which applies a set of `(source -> dest)` moves **transactionally**: either every
move succeeds, or — on the first failure — all already-applied moves are rolled back, so a failed
update can't leave a half-installed tool. Because it uses `rename` (which can't cross
filesystems), the source files, every destination, and the temp dir must all be on the same
filesystem.

**NOTE**: this example extracts a `.tar.gz`, which requires both the `archive-tar` and
`compression-tar-gz` features.

```rust
# #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
fn update() -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir = tempfile::TempDir::new()?;
    let tarball_path = tmp_dir.path().join("release.tar.gz");
    // ... download the archive to `tarball_path` (see the example above) ...

    // The extracted files are renamed into place, so the staging dir (the move sources) and the
    // stash dir must be on the same filesystem as the destinations — create both next to them
    // rather than in $TMPDIR. The `/usr/local` paths below are illustrative; use destinations
    // and temp dirs you have write access to (these may require elevated privileges).
    let staging = tempfile::TempDir::new_in("/usr/local")?;
    self_update::Extract::from_source(&tarball_path)
        .archive(self_update::ArchiveKind::Tar(Some(self_update::Compression::Gz)))
        .extract_into(staging.path())?;

    // Install several files atomically (all-or-nothing).
    let stash = tempfile::TempDir::new_in("/usr/local")?;
    self_update::MoveAll::from_temp(stash.path())
        .add(staging.path().join("app"), "/usr/local/bin/app")
        .add(staging.path().join("libapp.so"), "/usr/local/lib/libapp.so")
        .commit()?;
    Ok(())
}
```

### Checksum verification

With the `checksums` feature, the crate verifies the downloaded artifact against a digest
**before** installing — a mismatch aborts the update. Two sources of digests, independently
applied (when both apply, both must pass):

- **Release-published digests, automatic.** GitHub publishes a `sha256:<hex>` digest per release
  asset; the updater verifies the download against it whenever the selected asset carries one.
  This is on by default with the `checksums` feature — no configuration needed — and can be
  disabled with `verify_release_digest(false)`. The other backends' APIs publish no digest, so
  the check is a no-op there (a custom `ReleaseSource` can supply one via
  `ReleaseAsset::with_digest`). Note this is an *integrity* check only — the forge recomputes
  the digest if an asset is replaced — so it is not a substitute for the `signatures` feature.
- **A known digest you pass explicitly** (e.g. one published in a `SHA256SUMS` file alongside
  the release) via `verify_checksum`. The algorithm is chosen by the `Checksum` variant
  (`Sha256` / `Sha512`).

Both complement the `signatures` feature (zipsign), which verifies authenticity rather than a
published digest.

```rust
# #[cfg(feature = "checksums")]
fn update() -> Result<(), Box<dyn std::error::Error>> {
    self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        // hex digest, obtained out of band (e.g. parsed from the release's SHA256SUMS)
        .verify_checksum(self_update::Checksum::Sha256("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".into()))
        .build()?
        .update()?;
    Ok(())
}
```

### Checking for an update without installing

To check whether a newer release exists without downloading or installing anything, call
`is_update_available()` on the built updater. It fetches the release listing and returns the newest
strictly-newer `Release` (or `None` when up to date):

```rust
fn check() -> Result<(), Box<dyn std::error::Error>> {
    let update = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        .build()?;

    match update.is_update_available()? {
        Some(release) => println!("update available: {}", release.version()),
        None => println!("already up to date"),
    }
    Ok(())
}
```

### Restarting after an update

After `update()` returns [`VersionStatus::Updated`](crate::VersionStatus::Updated) the on-disk
executable has been replaced, but the running process keeps executing the old code until it exits.
To relaunch into the new binary immediately, use the [`restart`](crate::restart) module:
`restart::restart()` re-runs with the current arguments, and `restart::restart_with(args)` re-runs
with a fresh argument list (e.g. to drop an `--upgrade` flag so the new process does not update
again). On unix the process image is replaced with `exec` (the PID is preserved); on windows the new
binary is spawned and the current process exits. See the module docs for the platform details.

### Permissions

The crate never escalates privileges. There is no sudo re-exec, no polkit interaction, and no UAC
prompt. Privilege escalation is always the caller's choice.

An install into an unwritable location fails with
[`Error::InstallPathNotWritable`](crate::errors::Error::InstallPathNotWritable) naming the path
(the configured `bin_install_path`). Any other IO failure at the install step surfaces as
[`Error::Io`](crate::errors::Error::Io) with a message naming the install path, so the path is
visible in the error regardless of the kind.

Setting `check_install_path_writable(true)` on the builder opts into a preflight probe that runs
immediately before the download. Only a definite `PermissionDenied` refusal errors early;
indeterminate results (a missing parent directory, an unusual filesystem) are treated as "proceed"
and let the real install step surface the outcome. The default is `false`.

```rust,no_run
fn update() -> Result<(), Box<dyn std::error::Error>> {
    match self_update::backends::github::Update::configure()
        .repo_owner("owner")
        .repo_name("repo")
        .bin_name("app")
        .current_version(self_update::cargo_crate_version!())
        .check_install_path_writable(true)
        .build()?
        .update()
    {
        Ok(status) => println!("updated: {}", status.version()),
        Err(self_update::Error::InstallPathNotWritable { .. }) => {
            // The install path is not writable by this process. Elevation is the
            // application's choice: re-run under sudo, spawn a UAC-elevated child, etc.
            // Use the `restart` module for the exec/spawn mechanics when relaunching
            // with a modified argument list.
            eprintln!("install path not writable; re-run with elevated privileges");
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
```

### Periodic update checks

Every `update()` / `is_update_available()` call makes a network request. To avoid checking on every
run, gate the check behind [`UpdateCheckGuard`](crate::check_interval::UpdateCheckGuard), a small
stamp-file guard: `should_check()` reports whether the configured interval has elapsed since the
last recorded check, and `record_check()` stamps the current time. The caller owns the stamp-file
path. It is a guard, not a scheduler -- no threads or timers, and no extra dependencies. See the
[`check_interval`](crate::check_interval) module for the semantics.

### GitHub rate limits

Requests to the GitHub REST API are rate limited by GitHub itself, not by this crate:

- **Unauthenticated** requests are limited to **60 per hour per source IP**; **authenticated**
  requests (set any personal access token via `auth_token`) get **5000 per hour**. A token needs no
  scopes to raise the limit for a public repository.
- An update check costs **one** API request (the latest-release lookup, or one request per page of a
  paginated listing). The asset **download** itself is a CDN redirect and does not count against the
  core API limit.
- When you are rate limited, GitHub responds with **HTTP 403** (and an `x-ratelimit-remaining: 0`
  header), which this crate surfaces as `Error::Unauthorized { status: 403, .. }` -- the same
  variant as a genuine auth failure, so recognize it by the symptom (a 403 that appears only under
  frequent checking).
- To avoid it: set an `auth_token` (5000/hour), and check less often -- the
  [`UpdateCheckGuard`](crate::check_interval::UpdateCheckGuard) above throttles how often you check.
  The retry/backoff setters do **not** help here; retrying a rate-limited request only consumes more
  quota.

### Listing releases (`ReleaseList`)

Each built-in backend exposes a `ReleaseList` builder for fetching the list of available releases
without performing an update. There is **no single unifying `self_update::ReleaseList` type**:
every backend has its own, distinct `ReleaseList` (the fields and request shape differ per host),
so they are reached through their backend modules rather than re-exported at the crate root:

* `backends::github::ReleaseList`
* `backends::gitlab::ReleaseList`
* `backends::gitea::ReleaseList`
* `backends::gitee::ReleaseList`
* `backends::s3::ReleaseList`

The `manifest` backend has no separate `ReleaseList` struct. Its `ManifestSource` is a
`ReleaseSource` implementation that can be used directly, or listing can be driven through the
inherent verbs (`get_latest_release`, `get_newer_releases`, `is_update_available`) on a built
`manifest::Update`.

The custom backend has no `ReleaseList` by design: listing is performed entirely by your
`ReleaseSource` (or `AsyncReleaseSource`) implementation, which already returns
`Release` values directly.

### Custom backends

To update from a host the built-in backends (`github`, `gitlab`, `gitea`, `gitee`, `s3`, `manifest`) don't cover —
another forge, a private artifact registry, a plain HTTP directory — implement the
`ReleaseSource` trait and drive a full update through the `backends::custom` backend, which reuses
the crate's compare → select-asset → download → verify → extract → install flow. Only
`get_releases` (the fetch that says *where releases come from*) is required;
`get_latest_release` / `get_release_version` are derived from it by default and can be overridden
when the host has cheaper dedicated endpoints. You build `Release`s with `Release::builder` and
`ReleaseAsset::new`; the `ReleaseUpdate` trait stays sealed.

`ReleaseSource` is **synchronous**. For a natively-async source, implement `AsyncReleaseSource`
(the same fetches as `async fn`) and drive it through
`backends::custom::AsyncUpdate` + `build_async()`; to reuse a
`Clone` sync source from the async API, wrap it in
`backends::custom::Blocking`.

```rust
use self_update::{Release, ReleaseAsset, ReleaseSource, cargo_crate_version};

struct MyHost;
impl ReleaseSource for MyHost {
    fn get_releases(&self) -> self_update::Result<Vec<Release>> {
        Ok(vec![Release::builder()
            .version("1.2.3")
            .asset(ReleaseAsset::new("app-x86_64-unknown-linux-gnu.tar.gz", "https://host/app.tar.gz"))
            .build()?])
    }
}

fn update() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::custom::Update::configure()
        .source(MyHost)
        .bin_name("app")
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("custom backend update status: `{}`!", status.version());
    Ok(())
}
```

### Async

With the `async` feature, every built-in backend's `Update` builder gains a `build_async()` that
returns a distinct `AsyncUpdate` wrapper (one per backend). Its async (`*_async`) verbs —
`update_async()`, `update_extended_async()`, `get_latest_release_async()`,
`get_newer_releases_async()`, `get_release_version_async()`, and `is_update_available_async()` — are
**inherent methods** on that wrapper, so a `tokio` application can update without wrapping the
blocking calls in `spawn_blocking` and without importing any trait. Crucially, the `AsyncUpdate`
wrapper does **not** expose the blocking verbs: calling `.update()` on an async-built updater is a
compile error, so the old footgun of accidentally running a blocking update from an async context
is gone. The blocking API is unchanged; the async path is purely additive. It is **tokio-only and
requires `reqwest`** -- ureq and reqwest can coexist (reqwest serves the async path, and the sync
API prefers reqwest when both are present); the only invalid configuration is `async` without
`reqwest`. Network IO becomes async, and the extract/replace tail runs on
`tokio::task::spawn_blocking` so it does not block the executor.

```rust
# #[cfg(feature = "async")]
async fn update() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        .build_async()?
        .update_async()
        .await?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}
```

The `AsyncUpdate` wrapper exposes only the `*_async` verbs; the blocking `update()` is not a method
on it, so accidentally calling it from async code does not compile. The following block is
`compile_fail` for exactly that reason — `update` is not a method on the async wrapper (this block
is intentionally not feature-gated: gating it behind `cfg(feature = "async")` would make it an empty,
successfully-compiling doctest in the crate's no-`async` test lanes, which a `compile_fail` block
must never do):

```rust,compile_fail
fn wont_compile() -> Result<(), Box<dyn std::error::Error>> {
    let updater = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        .build_async()?;
    // `update()` is the BLOCKING verb; it is not exposed on the async `AsyncUpdate` wrapper.
    updater.update()?;
    Ok(())
}
```

### Custom HTTP client

The `.timeout()` / `.request_header()` / `.retries()` builder knobs cover most transport needs, but
for full control — custom TLS roots / mTLS, connection pooling, redirect policy, proxy-with-auth, or
simply reusing your application's existing client — you can hand the crate a **pre-built client**.
It is used for both the release listing and the download. The client-specific convenience setters
are `reqwest_client` (a blocking `reqwest::blocking::Client`, used by the blocking API),
`reqwest_async_client` (an async `reqwest::Client`, used by the `*_async` verbs), and `ureq_agent`
(a `ureq::Agent`); each wraps your client behind the crate's object-safe HTTP transport trait. The
compiled client crate(s) are re-exported (`self_update::reqwest` / `self_update::ureq`) so you don't
need a separate dependency to name the type. (Since the transport is a runtime trait seam, `reqwest`
and `ureq` are no longer mutually exclusive — both can be enabled, and the sync API prefers reqwest
when both are present.) For test doubles or fully custom transport, inject any type that implements
the object-safe trait directly via `.http_client(Arc<dyn HttpClient>)` (sync) or
`.http_client_async(Arc<dyn AsyncHttpClient>)` (async); see the [`http_client`](crate::http_client)
module for the trait definitions.

When you inject a client, `.request_header()` still applies, and `.retries()` still applies to the
release-listing requests and to the download's request-establishment phase (a mid-stream failure
is not retried, as that would corrupt the partially-written destination), and for `reqwest` the per-request
`.timeout()` is layered on too; but `HTTP(S)_PROXY` env and the crate's TLS feature are left entirely
to your client (and a `ureq::Agent` owns its own timeout, so `.timeout()` does not apply to an
injected agent — configure it on the agent). `reqwest_client` feeds the sync verbs and
`reqwest_async_client` the async ones — injecting only one and calling the other half just uses the
crate's per-call client for that half.

```rust
# #[cfg(feature = "reqwest")]
fn update() -> Result<(), Box<dyn std::error::Error>> {
    let client = self_update::reqwest::blocking::Client::builder()
        // .add_root_certificate(...) / .proxy(...) / .danger_accept_invalid_certs(...) etc.
        .build()?;
    self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        .reqwest_client(client)
        .build()?
        .update()?;
    Ok(())
}
```

### Troubleshooting

**Cross-compilation (`cross` / `cargo-cross`).** `rustls` is the default TLS backend, so
no additional configuration is needed for cross-compilation: a build on default features
already uses rustls. If you have explicitly switched to `native-tls` and want to revert,
remove the `native-tls` feature; `rustls` is active by default.

**TLS certificate errors on Linux (`native-tls` / OpenSSL).** With the native-TLS backend,
OpenSSL finds the system CA bundle on its own on most distributions. In a minimal environment where
it can't (some containers, `musl` static builds, or a non-standard cert layout) a request may fail
with a certificate-verification error. Point OpenSSL at the bundle by exporting `SSL_CERT_FILE`
(and, if needed, `SSL_CERT_DIR`) before running your program — the paths vary by distribution, e.g.
on a Debian/Ubuntu base:

```bash
export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
export SSL_CERT_DIR=/etc/ssl/certs
```

Alternatively build with the `rustls` feature, which uses a bundled root store and does not depend
on the system OpenSSL cert layout.

*/

// Enable the `doc_cfg` feature on docs.rs (nightly-only, guarded by the `docsrs` cfg set via
// `rustdoc-args = ["--cfg", "docsrs"]` in Cargo.toml). Stable builds are unaffected because
// the cfg is never set outside of the docs.rs environment.
#![cfg_attr(docsrs, feature(doc_cfg))]
// Keep the crate's rustdoc intra-doc links honest: an unresolved `[link]` in any doc comment is a
// hard error, not a silent warning. The full-crate check is the final barrier during a doc build.
#![deny(rustdoc::broken_intra_doc_links)]

// The HTTP transport is now an object-safe trait seam (`http_client::HttpClient`), so `reqwest` and
// `ureq` are no longer mutually exclusive — both client impls can be compiled and one is selected at
// runtime via `default_client()` (reqwest preferred when both are on). The genuine no-client case is
// a `compile_error!` in `http_client/mod.rs`. TLS features can also coexist: when both `native-tls`
// and `rustls` are enabled the per-call builders prefer rustls.

// The async API is reqwest-only — ureq has no async story. With the trait seam the two clients are
// no longer mutually exclusive, so `async` + `ureq` together is fine (async uses reqwest for the
// async path, ureq serves the sync path). The genuine bad case is `async` without the `reqwest`
// client at all; the `async` feature already implies `reqwest` (see Cargo.toml), so this guard only
// fires if that implication is ever broken.
#[cfg(all(feature = "async", not(feature = "reqwest")))]
compile_error!("feature `async` requires the `reqwest` client - `ureq` has no async API");

pub use http;
// Re-export the crates whose types appear in the async transport-trait signatures
// (`AsyncHttpClient` / `AsyncHttpResponse` name `BoxFuture`, `BoxStream`, and `Bytes`), so a
// custom async transport can be implemented without adding `futures-util`/`bytes` as direct
// dependencies (and without a version-skew risk against the ones this crate links).
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use bytes;
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use futures_util;
// Re-export the selected HTTP client so callers can name the types accepted by the client-injection
// setters (`reqwest_client` / `reqwest_async_client` / `ureq_agent`) without a separate dependency.
#[cfg(feature = "reqwest")]
#[cfg_attr(docsrs, doc(cfg(feature = "reqwest")))]
pub use reqwest;
#[cfg(feature = "signatures")]
#[cfg_attr(docsrs, doc(cfg(feature = "signatures")))]
pub use update::verify_signature;
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use update::{AsyncReleaseSource, AsyncReleaseUpdate};
pub use update::{
    Release, ReleaseAsset, ReleaseBuilder, ReleaseSource, ReleaseStatus, ReleaseUpdate, Releases,
    UpdateConfig, UpdateStrategy,
};
#[cfg(feature = "ureq")]
#[cfg_attr(docsrs, doc(cfg(feature = "ureq")))]
pub use ureq;

/// Re-export of the [`zipsign_api`] crate, whose [`PUBLIC_KEY_LENGTH`] constant defines the
/// size of the ed25519 verifying keys accepted by the `verifying_keys` builder methods.
///
/// [`PUBLIC_KEY_LENGTH`]: zipsign_api::PUBLIC_KEY_LENGTH
#[cfg(feature = "signatures")]
#[cfg_attr(docsrs, doc(cfg(feature = "signatures")))]
pub use zipsign_api;

/// An ed25519ph verifying key used to validate a signed download (see the `signatures` feature).
///
/// This is a convenience alias for the fixed-size key array accepted by the `verifying_keys`
/// builder methods, so consumers don't need to depend on `zipsign-api` directly.
///
/// # Compile-time embedding
///
/// The typical way to supply a key is to embed it at compile time:
///
/// ```rust,ignore
/// const VERIFYING_KEY: self_update::VerifyingKey =
///     *include_bytes!("path/to/key.pub");
/// ```
///
/// The file must be exactly 32 raw bytes (the ed25519 public key in wire format).
/// zipsign key files are raw 32-byte ed25519 public keys, not PEM.
/// If the file length does not match, Rust will emit a compile error because
/// the array size is fixed at `PUBLIC_KEY_LENGTH` (32).
///
/// # Key rotation
///
/// When rotating signing keys, sign new releases with both the old key and the
/// new key.  Old binaries, which embed only the old key, can still verify and
/// update because the archive carries both signatures.  zipsign uses any-of
/// semantics: verification passes as soon as any (key, signature) pair matches.
/// New binaries embed only the new key.  Once the transition window has passed
/// and no old binaries remain in the field, releases only need the new key's
/// signature.
#[cfg(feature = "signatures")]
#[cfg_attr(docsrs, doc(cfg(feature = "signatures")))]
pub type VerifyingKey = [u8; zipsign_api::PUBLIC_KEY_LENGTH];

#[cfg(feature = "progress-bar")]
use indicatif::{ProgressBar, ProgressStyle as IndicatifProgressStyle};
use log::debug;
#[cfg(feature = "progress-bar")]
use std::cmp::min;
use std::fs;
use std::io;
use std::path;

#[macro_use]
mod macros;
pub mod backends;
pub mod check_interval;
#[cfg(feature = "checksums")]
mod checksum;
pub mod errors;
pub mod http_client;
pub mod restart;
mod tls;
pub mod update;
pub mod version;

/// An opaque TLS root CA certificate, supplied to a backend builder or a [`Download`] via the
/// `add_root_certificate` setter so the crate-built HTTP client trusts a private/internal CA.
/// Construct with [`Certificate::from_pem`](crate::Certificate::from_pem) or
/// [`Certificate::from_der`](crate::Certificate::from_der); the bytes are validated when the client
/// is built, not at construction.
pub use tls::Certificate;

/// Re-export the crate's [`Error`] and [`Result`] at the crate root,
/// so consumers (and `ReleaseSource` implementors) can write `self_update::Result<T>` /
/// `self_update::Error` without naming the `errors` module.
pub use errors::{Error, Result};

/// A checksum variant (`Sha256` / `Sha512`) used with `verify_checksum` to validate a downloaded
/// artifact against a known digest before installation. Requires the `checksums` feature.
#[cfg(feature = "checksums")]
#[cfg_attr(docsrs, doc(cfg(feature = "checksums")))]
pub use checksum::Checksum;

use http_client::header;

/// The User-Agent sent on the crate's own requests (API listings and downloads) when the caller
/// has not set one via `request_header`. One shared value so every backend and the standalone
/// [`Download`] identify themselves the same way regardless of the compiled HTTP client.
pub(crate) const DEFAULT_USER_AGENT: &str = concat!("self-update/", env!("CARGO_PKG_VERSION"));

#[cfg(feature = "progress-bar")]
pub(crate) const DEFAULT_PROGRESS_TEMPLATE: &str =
    "[{elapsed_precise}] [{bar:40}] {bytes}/{total_bytes} ({eta}) {msg}";
#[cfg(feature = "progress-bar")]
pub(crate) const DEFAULT_PROGRESS_CHARS: &str = "=>-";

/// The download progress-bar style: an `indicatif` `template` plus the `chars` it renders the bar
/// with. Requires the `progress-bar` feature.
///
/// This is a typed pair so the two strings can't be transposed at a call site (the previous setter
/// took two `impl Into<String>` args in template-then-chars order, which were easy to swap). Build
/// one with [`ProgressStyle::new`] and pass it to the `Update` builder's `progress_style` or
/// [`Download::progress_style`].
///
/// ```
/// # #[cfg(feature = "progress-bar")] {
/// let style = self_update::ProgressStyle::new(
///     "[{bar:40}] {bytes}/{total_bytes}",
///     "=>-",
/// );
/// # let _ = style;
/// # }
/// ```
#[cfg(feature = "progress-bar")]
#[cfg_attr(docsrs, doc(cfg(feature = "progress-bar")))]
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ProgressStyle {
    /// The `indicatif` progress-bar template (see `indicatif::ProgressStyle::template`).
    pub template: String,
    /// The characters used to render the bar (see `indicatif::ProgressStyle::progress_chars`).
    pub chars: String,
}

#[cfg(feature = "progress-bar")]
impl ProgressStyle {
    /// Construct a `ProgressStyle` from a `template` and its progress `chars`.
    pub fn new(template: impl Into<String>, chars: impl Into<String>) -> Self {
        Self {
            template: template.into(),
            chars: chars.into(),
        }
    }
}

/// Get the current target triple.
///
/// Returns a target triple (e.g. `x86_64-unknown-linux-gnu` or `i686-pc-windows-msvc`)
pub fn get_target() -> &'static str {
    env!("TARGET")
}

/// Flush a message to stdout and check if they respond `yes`.
/// Interprets a blank response as yes.
///
/// * Errors:
///     * Io flushing
///     * User entered anything other than enter/Y/y
fn confirm(msg: &str) -> Result<()> {
    print_flush!("{}", msg);

    let mut s = String::new();
    // EOF (closed stdin: a daemon, `</dev/null`, CI) reads zero bytes. Treat it as a decline, not a
    // blank-line "yes", so an unattended caller that forgot `no_confirm` aborts rather than silently
    // proceeding with a self-replacement.
    if io::stdin().read_line(&mut s)? == 0 {
        return Err(Error::Aborted);
    }
    let s = s.trim().to_lowercase();
    if !s.is_empty() && s != "y" {
        return Err(Error::Aborted);
    }
    Ok(())
}

/// The lightweight result of [`update`](update::ReleaseUpdate::update) (and its async sibling
/// `update_async`): it carries only the version tag of the latest release.
///
/// Wrapped `String`s are version tags.
///
/// This is the lightweight counterpart of [`ReleaseStatus`], the richer
/// result of [`update_extended`](update::ReleaseUpdate::update_extended) which carries the full
/// [`Release`] (name, date, body, assets). Reach for `VersionStatus` when the
/// version string is all you need; reach for `ReleaseStatus` when you need the installed release's
/// details.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum VersionStatus {
    UpToDate(String),
    Updated(String),
}
impl VersionStatus {
    /// Return the version tag
    pub fn version(&self) -> &str {
        use VersionStatus::*;
        match *self {
            UpToDate(ref s) => s,
            Updated(ref s) => s,
        }
    }

    /// Returns `true` if `VersionStatus::UpToDate`
    pub fn is_up_to_date(&self) -> bool {
        matches!(*self, VersionStatus::UpToDate(_))
    }

    /// Returns `true` if `VersionStatus::Updated`
    pub fn is_updated(&self) -> bool {
        matches!(*self, VersionStatus::Updated(_))
    }
}

impl std::fmt::Display for VersionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use VersionStatus::*;
        match *self {
            UpToDate(ref s) => write!(f, "UpToDate({})", s),
            Updated(ref s) => write!(f, "Updated({})", s),
        }
    }
}

/// The archive format of a release asset, as detected from its file extension.
///
/// `#[non_exhaustive]`, and the `Tar`/`Zip` variants are gated on the `archive-tar` / `archive-zip`
/// features: if the matching feature is off the variant does not exist and `detect_archive` for
/// that extension returns [`Error::ArchiveNotEnabled`] instead.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ArchiveKind {
    /// A tarball, optionally compressed (e.g. `.tar`, `.tar.gz`, `.tar.xz`). Requires `archive-tar`.
    #[cfg(feature = "archive-tar")]
    #[cfg_attr(docsrs, doc(cfg(feature = "archive-tar")))]
    Tar(Option<Compression>),
    /// A bare file, optionally compressed (e.g. a plain binary, or a `.gz` / `.xz` of one).
    Plain(Option<Compression>),
    /// A zip archive (`.zip`). Requires `archive-zip`.
    #[cfg(feature = "archive-zip")]
    #[cfg_attr(docsrs, doc(cfg(feature = "archive-zip")))]
    Zip,
}

impl std::fmt::Display for ArchiveKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            #[cfg(feature = "archive-tar")]
            ArchiveKind::Tar(Some(Compression::Gz)) => write!(f, "tar.gz"),
            #[cfg(feature = "archive-tar")]
            ArchiveKind::Tar(Some(Compression::Xz)) => write!(f, "tar.xz"),
            #[cfg(feature = "archive-tar")]
            ArchiveKind::Tar(None) => write!(f, "tar"),
            ArchiveKind::Plain(Some(Compression::Gz)) => write!(f, "gz"),
            ArchiveKind::Plain(Some(Compression::Xz)) => write!(f, "xz"),
            ArchiveKind::Plain(None) => write!(f, "plain"),
            #[cfg(feature = "archive-zip")]
            ArchiveKind::Zip => write!(f, "zip"),
        }
    }
}

/// A compression codec applied to an [`ArchiveKind`]. `#[non_exhaustive]`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Compression {
    /// gzip (`.gz`); decoding the stream requires the `compression-tar-gz` feature.
    Gz,
    /// xz / LZMA2 (`.xz`); decoding the stream requires the `compression-tar-xz` feature.
    Xz,
}

fn detect_archive(path: &path::Path) -> Result<ArchiveKind> {
    let ext = path.extension();

    debug!("Detecting archive type using extension: {:?}", ext);

    let res = match ext {
        Some(extension) if extension == std::ffi::OsStr::new("zip") => {
            #[cfg(feature = "archive-zip")]
            {
                debug!("Detected .zip archive");
                Ok(ArchiveKind::Zip)
            }
            #[cfg(not(feature = "archive-zip"))]
            {
                Err(Error::ArchiveNotEnabled("zip".to_string()))
            }
        }
        Some(extension) if extension == std::ffi::OsStr::new("tar") => {
            #[cfg(feature = "archive-tar")]
            {
                debug!("Detected .tar archive");
                Ok(ArchiveKind::Tar(None))
            }
            #[cfg(not(feature = "archive-tar"))]
            {
                Err(Error::ArchiveNotEnabled("tar".to_string()))
            }
        }
        Some(extension) if extension == std::ffi::OsStr::new("tgz") => {
            #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
            {
                debug!("Detected .tgz archive");
                Ok(ArchiveKind::Tar(Some(Compression::Gz)))
            }
            #[cfg(all(feature = "archive-tar", not(feature = "compression-tar-gz")))]
            {
                Err(Error::CompressionNotEnabled("gz".to_string()))
            }
            #[cfg(not(feature = "archive-tar"))]
            {
                Err(Error::ArchiveNotEnabled("tar".to_string()))
            }
        }
        Some(extension) if extension == std::ffi::OsStr::new("gz") => match path
            .file_stem()
            .map(path::Path::new)
            .and_then(|f| f.extension())
        {
            Some(extension) if extension == std::ffi::OsStr::new("tar") => {
                #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
                {
                    debug!("Detected .tar.gz archive");
                    Ok(ArchiveKind::Tar(Some(Compression::Gz)))
                }
                #[cfg(all(feature = "archive-tar", not(feature = "compression-tar-gz")))]
                {
                    Err(Error::CompressionNotEnabled("gz".to_string()))
                }
                #[cfg(not(feature = "archive-tar"))]
                {
                    Err(Error::ArchiveNotEnabled("tar".to_string()))
                }
            }
            // A plain `.gz` single-file asset: decoding the gzip layer requires the
            // `compression-tar-gz` feature. Without it, refuse rather than installing the still
            // compressed bytes as the binary.
            _ => {
                #[cfg(feature = "compression-tar-gz")]
                {
                    Ok(ArchiveKind::Plain(Some(Compression::Gz)))
                }
                #[cfg(not(feature = "compression-tar-gz"))]
                {
                    Err(Error::CompressionNotEnabled("gz".to_string()))
                }
            }
        },
        Some(extension) if extension == std::ffi::OsStr::new("txz") => {
            #[cfg(all(feature = "archive-tar", feature = "compression-tar-xz"))]
            {
                debug!("Detected .txz archive");
                Ok(ArchiveKind::Tar(Some(Compression::Xz)))
            }
            #[cfg(all(feature = "archive-tar", not(feature = "compression-tar-xz")))]
            {
                Err(Error::CompressionNotEnabled("xz".to_string()))
            }
            #[cfg(not(feature = "archive-tar"))]
            {
                Err(Error::ArchiveNotEnabled("tar".to_string()))
            }
        }
        Some(extension) if extension == std::ffi::OsStr::new("xz") => match path
            .file_stem()
            .map(path::Path::new)
            .and_then(|f| f.extension())
        {
            Some(extension) if extension == std::ffi::OsStr::new("tar") => {
                #[cfg(all(feature = "archive-tar", feature = "compression-tar-xz"))]
                {
                    debug!("Detected .tar.xz archive");
                    Ok(ArchiveKind::Tar(Some(Compression::Xz)))
                }
                #[cfg(all(feature = "archive-tar", not(feature = "compression-tar-xz")))]
                {
                    Err(Error::CompressionNotEnabled("xz".to_string()))
                }
                #[cfg(not(feature = "archive-tar"))]
                {
                    Err(Error::ArchiveNotEnabled("tar".to_string()))
                }
            }
            // A plain `.xz` single-file asset: decoding the xz layer requires the
            // `compression-tar-xz` feature. Without it, refuse rather than installing the still
            // compressed bytes as the binary.
            _ => {
                #[cfg(feature = "compression-tar-xz")]
                {
                    Ok(ArchiveKind::Plain(Some(Compression::Xz)))
                }
                #[cfg(not(feature = "compression-tar-xz"))]
                {
                    Err(Error::CompressionNotEnabled("xz".to_string()))
                }
            }
        },
        _ => Ok(ArchiveKind::Plain(None)),
    };

    debug!("Detected archive type: {:?}", res);

    res
}

/// Extract contents of an encoded archive (e.g. tar.gz) file to a specified directory
///
/// * Errors:
///     * Io - opening files
///     * Io - gzip decoding
///     * Io - archive unpacking
#[derive(Debug)]
#[non_exhaustive]
pub struct Extract {
    source: path::PathBuf,
    archive: Option<ArchiveKind>,
}
/// A [`Read`](io::Read) over an archive's bytes with any single compression layer (`.gz`, `.xz`)
/// transparently decoded, so the tar/plain readers above it see the decompressed stream. `Plain`
/// is the undecoded passthrough. Each compressed variant exists only when its `compression-tar-*`
/// feature is enabled; [`detect_archive`] rejects a compression whose feature is off before this
/// is ever built. The gzip layer decodes as a stream; the xz layer is decoded up front into memory
/// (the `lzma-rs` decoder is one-shot), which is fine for the modestly sized release artifacts this
/// crate downloads to a temp file.
enum ArchiveReader {
    Plain(fs::File),
    // Boxed: a `GzDecoder` is far larger than the other variants, so an unboxed variant would bloat
    // every `ArchiveReader` to its size (clippy::large_enum_variant).
    #[cfg(feature = "compression-tar-gz")]
    Gz(Box<flate2::read::GzDecoder<fs::File>>),
    #[cfg(feature = "compression-tar-xz")]
    Xz(io::Cursor<Vec<u8>>),
}

impl io::Read for ArchiveReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            ArchiveReader::Plain(r) => r.read(buf),
            #[cfg(feature = "compression-tar-gz")]
            ArchiveReader::Gz(r) => r.read(buf),
            #[cfg(feature = "compression-tar-xz")]
            ArchiveReader::Xz(r) => r.read(buf),
        }
    }
}

impl Extract {
    /// Create an `Extract`or from a source path. Accepts anything path-like (`&Path`, `PathBuf`,
    /// `&str`, …), storing an owned [`PathBuf`](std::path::PathBuf).
    pub fn from_source(source: impl AsRef<path::Path>) -> Extract {
        Self {
            source: source.as_ref().to_path_buf(),
            archive: None,
        }
    }

    /// Specify an archive format of the source being extracted. If not specified, the
    /// archive format will determined from the file extension.
    pub fn archive(&mut self, kind: ArchiveKind) -> &mut Self {
        self.archive = Some(kind);
        self
    }

    #[allow(unused_variables)]
    fn get_archive_reader(
        source: fs::File,
        compression: Option<Compression>,
    ) -> Result<ArchiveReader> {
        match compression {
            None => Ok(ArchiveReader::Plain(source)),
            #[cfg(feature = "compression-tar-gz")]
            Some(Compression::Gz) => Ok(ArchiveReader::Gz(Box::new(flate2::read::GzDecoder::new(
                source,
            )))),
            #[cfg(feature = "compression-tar-xz")]
            Some(Compression::Xz) => {
                // `lzma-rs` is a one-shot decoder (no streaming `Read` adapter), so decode the
                // whole `.xz` stream into memory and hand the tar/plain layer a cursor over it.
                let mut input = io::BufReader::new(source);
                let mut decoded = Vec::new();
                lzma_rs::xz_decompress(&mut input, &mut decoded).map_err(|e| Error::Internal {
                    message: format!("failed to decode xz stream: {e}"),
                    source: None,
                })?;
                Ok(ArchiveReader::Xz(io::Cursor::new(decoded)))
            }
            // A compression whose decoder feature is disabled is rejected by `detect_archive`
            // before extraction, so this is unreachable in practice.
            #[allow(unreachable_patterns)]
            Some(_) => Err(Error::CompressionNotEnabled("unsupported".to_string())),
        }
    }

    /// Extract an entire source archive into a specified path. If the source is a single compressed
    /// file and not an archive, it will be extracted into a file with the same name inside of
    /// `into_dir`.
    ///
    /// # Symlink handling
    ///
    /// Zip entries that are symbolic links (their unix mode carries `S_IFLNK`) are restored as
    /// real symlinks on unix, with the link target read from the entry contents. This preserves
    /// directory trees that rely on symlinks (for example a macOS `.app` bundle whose
    /// `Frameworks/*/Versions/Current` links are load-bearing for code signatures) instead of
    /// materializing the target string as a regular file. A symlink target that would escape the
    /// extraction root -- an absolute target, or a relative target whose `..` components resolve
    /// above `into_dir` -- is rejected with an error, mirroring the zip-slip defense applied to
    /// entry names.
    ///
    /// That per-entry target check is purely lexical, so it cannot see a symlinked intermediate
    /// directory that aliases an entry's parent to a shallower physical path (the classic
    /// symlinked-parent traversal: an entry `d/sl -> ..` followed by `d/sl/evil -> ../../x`, where
    /// the second link is lexically in-bounds yet physically lands above the root). As a backstop,
    /// for every zip entry -- symlink or regular file -- after its parent directories are
    /// materialized the physical parent is canonicalized and must equal the canonical extraction
    /// root joined with the entry's lexical parent; any descent through a symlinked ancestor (or a
    /// canonicalize failure) is rejected with an `Error::Internal`, while descent through real
    /// directories is unaffected.
    ///
    /// On non-unix platforms (creating symlinks requires elevated privileges on Windows) symlink
    /// entries are written as regular files containing the target path. Tar archives restore
    /// symlinks via `tar`'s own unpack logic.
    pub fn extract_into(&self, into_dir: impl AsRef<path::Path>) -> Result<()> {
        let into_dir = into_dir.as_ref();
        let source = fs::File::open(&self.source)?;
        let archive = match self.archive {
            Some(archive) => archive,
            None => detect_archive(&self.source)?,
        };

        // We cannot use a feature flag in a match arm. To bypass this the code block is
        // isolated in a closure and called accordingly.
        let extract_into_plain_or_tar = |source: fs::File, compression: Option<Compression>| {
            let mut reader = Self::get_archive_reader(source, compression)?;

            match archive {
                ArchiveKind::Plain(_) => {
                    match fs::create_dir_all(into_dir) {
                        Ok(_) => (),
                        Err(e) => {
                            if e.kind() != io::ErrorKind::AlreadyExists {
                                return Err(Error::Io(e));
                            }
                        }
                    }
                    let file_name = self.source.file_name().ok_or_else(|| Error::Internal {
                        message: "Extractor source has no file-name".to_string(),
                        source: None,
                    })?;
                    let mut out_path = into_dir.join(file_name);
                    out_path.set_extension("");
                    let mut out_file = fs::File::create(&out_path)?;
                    io::copy(&mut reader, &mut out_file)?;
                }
                #[cfg(feature = "archive-tar")]
                ArchiveKind::Tar(_) => {
                    let mut archive = tar::Archive::new(reader);
                    archive.unpack(into_dir)?;
                }
                #[allow(unreachable_patterns)]
                _ => unreachable!(
                    "detect_archive() returns in case the proper feature flag is not enabled"
                ),
            };

            Ok(())
        };

        match archive {
            #[cfg(feature = "archive-tar")]
            ArchiveKind::Plain(compression) | ArchiveKind::Tar(compression) => {
                extract_into_plain_or_tar(source, compression)?;
            }
            #[cfg(not(feature = "archive-tar"))]
            ArchiveKind::Plain(compression) => {
                extract_into_plain_or_tar(source, compression)?;
            }
            #[cfg(feature = "archive-zip")]
            ArchiveKind::Zip => {
                let mut archive = zip::ZipArchive::new(source)?;

                // The destination must exist so its canonical (symlink-free) form can be
                // captured once up front. Each entry's physical parent is later checked against
                // this root to reject any descent through a symlinked directory (the
                // symlinked-parent traversal that a per-entry lexical check cannot catch).
                fs::create_dir_all(into_dir)?;
                let canonical_root = fs::canonicalize(into_dir)?;

                for i in 0..archive.len() {
                    let mut file = archive.by_index(i)?;

                    // Reject entries whose name would escape `into_dir` (zip-slip). `enclosed_name`
                    // returns `None` for an absolute path or one containing `..`.
                    let Some(rel_path) = file.enclosed_name() else {
                        return Err(Error::Internal {
                            message: format!("zip entry has an unsafe path: {:?}", file.name()),
                            source: None,
                        });
                    };
                    let output_path = into_dir.join(&rel_path);

                    if file.is_dir() {
                        fs::create_dir_all(&output_path)?;
                        continue;
                    }
                    if let Some(parent_dir) = output_path.parent() {
                        match fs::create_dir_all(parent_dir) {
                            Ok(()) => {}
                            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                            Err(e) => return Err(Error::Io(e)),
                        }

                        // Physical-parent verification (symlinked-parent traversal defense).
                        // The per-entry lexical target check cannot catch an intermediate
                        // symlinked directory (e.g. an earlier `d/sl -> ..` entry) that aliases
                        // this entry's parent to a shallower real path, which would let a later
                        // entry be created outside the root through the alias. After the parents
                        // are materialized, require the parent to canonicalize to exactly its
                        // expected location: the canonical root joined with the entry's lexical
                        // (only-`Normal`) parent. A prefix/`start_with` check alone is
                        // insufficient -- `d/sl` canonicalizes to the root, which trivially lies
                        // within the root -- so compare for equality: real directories match
                        // their lexical path, while any symlinked ancestor resolves elsewhere and
                        // is rejected. A canonicalize failure is treated as a rejection too,
                        // matching the zip-slip/escape error style. This guards both the
                        // symlink-creation path and the regular-file path below.
                        let lexical_parent =
                            rel_path.parent().unwrap_or_else(|| path::Path::new(""));
                        let expected_parent = canonical_root.join(lexical_parent);
                        let physical_parent =
                            fs::canonicalize(parent_dir).map_err(|e| Error::Internal {
                                message: format!(
                                    "could not resolve the parent directory of zip entry {:?}: {}",
                                    file.name(),
                                    e
                                ),
                                source: Some(Box::new(e)),
                            })?;
                        if physical_parent != expected_parent {
                            return Err(Error::Internal {
                                message: format!(
                                    "zip entry {:?} descends through a symlinked directory that escapes the extraction dir",
                                    file.name()
                                ),
                                source: None,
                            });
                        }
                    }

                    // On unix, restore a symlink entry as a real symlink instead of writing its
                    // target string out as a regular file. The escaping-target check mirrors the
                    // `enclosed_name` zip-slip defense above. On non-unix targets this block is
                    // compiled out and the entry falls through to the regular-file path.
                    #[cfg(unix)]
                    if file.is_symlink() {
                        use std::ffi::OsStr;
                        use std::io::Read;
                        use std::os::unix::ffi::OsStrExt;

                        let entry_name = file.name().to_string();
                        let mut target_bytes = Vec::new();
                        file.read_to_end(&mut target_bytes)?;
                        let target = path::Path::new(OsStr::from_bytes(&target_bytes));

                        // The link lives at `into_dir/rel_path`; a relative target resolves against
                        // the link's parent directory. Reject any target (absolute, or `..`
                        // climbing above `into_dir`) whose lexical resolution escapes the root.
                        let link_parent = rel_path.parent().unwrap_or_else(|| path::Path::new(""));
                        if symlink_target_escapes(link_parent, target) {
                            return Err(Error::Internal {
                                message: format!(
                                    "zip symlink entry {:?} points outside the extraction dir: {:?}",
                                    entry_name, target
                                ),
                                source: None,
                            });
                        }

                        // A duplicate entry may already have created a file/link here; `symlink`
                        // fails on an existing path, so remove it first (regular-file entries
                        // truncate via `File::create`, so match that "last entry wins" behavior).
                        match fs::remove_file(&output_path) {
                            Ok(()) => {}
                            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                            Err(e) => return Err(Error::Io(e)),
                        }
                        std::os::unix::fs::symlink(target, &output_path)?;
                        // A symlink carries no meaningful permission bits of its own; do not call
                        // `set_permissions` (it would follow the link and alter the target).
                        continue;
                    }

                    let mut output = fs::File::create(&output_path)?;
                    io::copy(&mut file, &mut output)?;
                    // Preserve the archived unix permission mode (notably the executable bit) so a
                    // binary extracted from a zip is runnable when installed to a custom path.
                    // Mask off the setuid/setgid/sticky bits (`0o7000`): a crafted archive must not
                    // be able to install a setuid binary, so only the standard `rwx` permission
                    // bits (`0o777`) are honored.
                    #[cfg(unix)]
                    if let Some(mode) = file.unix_mode() {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(
                            &output_path,
                            fs::Permissions::from_mode(mode & 0o777),
                        )?;
                    }
                }
            }
        };
        Ok(())
    }

    /// Extract a single file from a source and save to a file of the same name in `into_dir`.
    /// If the source is a single compressed file, it will be saved with the name `file_to_extract`
    /// in the specified `into_dir`.
    ///
    /// If the named zip entry is a symbolic link (its unix mode carries `S_IFLNK`), extraction
    /// fails with an error rather than writing the link's target string out as the requested file:
    /// this API returns a single concrete file, and silently substituting the target path text for
    /// the payload would be surprising. Callers who need symlinks preserved should use
    /// [`extract_into`](Self::extract_into), which restores them as real links on unix.
    pub fn extract_file<T: AsRef<path::Path>>(
        &self,
        into_dir: impl AsRef<path::Path>,
        file_to_extract: T,
    ) -> Result<()> {
        let into_dir = into_dir.as_ref();
        let file_to_extract = file_to_extract.as_ref();
        let source = fs::File::open(&self.source)?;
        let archive = match self.archive {
            Some(archive) => archive,
            None => detect_archive(&self.source)?,
        };

        debug!(
            "Attempting to extract {:?} file from {:?}",
            file_to_extract, self.source
        );

        // We cannot use a feature flag in a match arm. To bypass this the code block is
        // isolated in a closure and called accordingly.
        let extract_file_plain_or_tar = |source: fs::File, compression: Option<Compression>| {
            let mut reader = Self::get_archive_reader(source, compression)?;

            match archive {
                ArchiveKind::Plain(_) => {
                    debug!("Copying file directly");
                    match fs::create_dir_all(into_dir) {
                        Ok(_) => (),
                        Err(e) => {
                            if e.kind() != io::ErrorKind::AlreadyExists {
                                return Err(Error::Io(e));
                            }
                        }
                    }
                    let file_name = file_to_extract.file_name().ok_or_else(|| Error::Internal {
                        message: "Extractor source has no file-name".to_string(),
                        source: None,
                    })?;
                    let out_path = into_dir.join(file_name);
                    let mut out_file = fs::File::create(out_path)?;
                    io::copy(&mut reader, &mut out_file)?;
                }
                #[cfg(feature = "archive-tar")]
                ArchiveKind::Tar(_) => {
                    debug!("Extracting from tar");

                    let mut archive = tar::Archive::new(reader);
                    let mut entry = archive
                        .entries()?
                        .filter_map(|e| e.ok())
                        .find(|e| {
                            let p = e.path();
                            debug!("Archive path: {:?}", p);
                            p.ok().filter(|p| p == file_to_extract).is_some()
                        })
                        .ok_or_else(|| Error::Internal {
                            message: format!(
                                "Could not find the required path in the archive: {:?}",
                                file_to_extract
                            ),
                            source: None,
                        })?;
                    entry.unpack_in(into_dir)?;
                }
                #[allow(unreachable_patterns)]
                _ => unreachable!(
                    "detect_archive() returns in case the proper feature flag is not enabled"
                ),
            };

            Ok(())
        };

        match archive {
            #[cfg(feature = "archive-tar")]
            ArchiveKind::Plain(compression) | ArchiveKind::Tar(compression) => {
                extract_file_plain_or_tar(source, compression)?;
            }
            #[cfg(not(feature = "archive-tar"))]
            ArchiveKind::Plain(compression) => {
                extract_file_plain_or_tar(source, compression)?;
            }
            #[cfg(feature = "archive-zip")]
            ArchiveKind::Zip => {
                let mut archive = zip::ZipArchive::new(source)?;
                let file_name = file_to_extract.to_str().ok_or_else(|| Error::Internal {
                    message: format!(
                        "cannot extract file with a non-UTF-8 path: {:?}",
                        file_to_extract
                    ),
                    source: None,
                })?;
                let mut file = archive.by_name(file_name)?;

                let Some(rel_path) = file.enclosed_name() else {
                    return Err(Error::Internal {
                        message: format!("zip entry has an unsafe path: {:?}", file.name()),
                        source: None,
                    });
                };
                // A symlink entry has no regular-file payload; its "contents" are the link target
                // path. Rather than write that target string out as `file_to_extract`, reject it
                // (see the rustdoc): use `extract_into` to restore symlinks. Rejecting on every
                // platform keeps the single-file API's behavior uniform.
                if file.is_symlink() {
                    return Err(Error::Internal {
                        message: format!(
                            "zip entry {:?} is a symlink; use extract_into to restore symlinks",
                            file.name()
                        ),
                        source: None,
                    });
                }

                let output_path = into_dir.join(rel_path);
                if let Some(parent_dir) = output_path.parent()
                    && let Err(e) = fs::create_dir_all(parent_dir)
                    && e.kind() != io::ErrorKind::AlreadyExists
                {
                    return Err(Error::Io(e));
                }

                let mut output = fs::File::create(&output_path)?;
                io::copy(&mut file, &mut output)?;
                // Preserve the archived unix permission mode so the extracted binary is runnable,
                // but mask off the setuid/setgid/sticky bits (`0o7000`) so a crafted archive cannot
                // install a setuid binary; only the standard `rwx` bits (`0o777`) are honored.
                #[cfg(unix)]
                if let Some(mode) = file.unix_mode() {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&output_path, fs::Permissions::from_mode(mode & 0o777))?;
                }
            }
        };
        Ok(())
    }
}

/// Lexically decide whether a zip symlink target escapes the extraction root.
///
/// `link_parent` is the link entry's parent directory expressed relative to the extraction root
/// (derived from the already zip-slip-checked entry name, so it contains only normal components).
/// `target` is the raw link target read from the entry contents. Returns `true` if the target is
/// absolute or if resolving its `..`/`.` components against `link_parent` climbs above the root.
/// This is a purely lexical check (no filesystem access), matching the intent of the
/// `enclosed_name` defense used for entry names.
#[cfg(all(feature = "archive-zip", unix))]
fn symlink_target_escapes(link_parent: &path::Path, target: &path::Path) -> bool {
    use std::path::Component;

    // Seed the virtual stack with the link's parent directory (only normal components).
    let mut depth: usize = 0;
    for comp in link_parent.components() {
        match comp {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            // `link_parent` comes from an enclosed (relative, `..`-free) name, so other
            // components should not occur; treat any as unsafe to be conservative.
            _ => return true,
        }
    }

    for comp in target.components() {
        match comp {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                // Climbing above the root escapes the extraction dir.
                let Some(next) = depth.checked_sub(1) else {
                    return true;
                };
                depth = next;
            }
            // An absolute target (root dir or a drive prefix) always escapes.
            Component::RootDir | Component::Prefix(_) => return true,
        }
    }

    false
}

/// Moves a file from the given path to the specified destination.
///
/// `source` and `dest` must be on the same filesystem.
/// If `replace_using_temp` is specified, the destination file will be
/// replaced using the given temporary path.
/// If the existing `dest` file is a currently running long running program,
/// `replace_using_temp` may run into errors cleaning up the temp dir.
/// If that's the case for your use-case, consider not specifying a temp dir to use.
///
/// * Errors:
///     * Io - copying / renaming
#[derive(Debug)]
#[non_exhaustive]
pub struct Move {
    source: path::PathBuf,
    temp: Option<path::PathBuf>,
}
impl Move {
    /// Specify source file. Accepts anything path-like, storing an owned
    /// [`PathBuf`](std::path::PathBuf).
    pub fn from_source(source: impl AsRef<path::Path>) -> Move {
        Self {
            source: source.as_ref().to_path_buf(),
            temp: None,
        }
    }

    /// If specified and the destination file already exists, the "destination"
    /// file will be moved to the given temporary location before the "source"
    /// file is moved to the "destination" file.
    ///
    /// In the event of an `io` error while renaming "source" to "destination",
    /// the temporary file will be moved back to "destination".
    ///
    /// The `temp` dir must be explicitly provided since `rename` operations require
    /// files to live on the same filesystem.
    pub fn replace_using_temp(&mut self, temp: impl AsRef<path::Path>) -> &mut Self {
        self.temp = Some(temp.as_ref().to_path_buf());
        self
    }

    /// Move source file to specified destination
    pub fn to_dest(&self, dest: impl AsRef<path::Path>) -> Result<()> {
        let dest = dest.as_ref();
        match self.temp.as_deref() {
            // Move the existing dest to a temp location so we can move it back if
            // there's an error. If the existing `dest` file is a long running program,
            // this may prevent the temp dir from being cleaned up.
            Some(temp) if dest.exists() => {
                fs::rename(dest, temp)?;
                if let Err(e) = fs::rename(&self.source, dest) {
                    fs::rename(temp, dest)?;
                    return Err(Error::from(e));
                }
            }
            // No temp set, or nothing to preserve at `dest`: just move source into place.
            _ => {
                rename_or_copy(&self.source, dest)?;
            }
        };
        Ok(())
    }
}

/// Rename `source` onto `dest`, falling back to copy when the two are on different filesystems.
///
/// The extraction temp dir is often a tmpfs on Linux while `bin_install_path` lives on the root
/// filesystem, so a plain `fs::rename` returns `CrossesDevices` (EXDEV). On that error the source is
/// copied to a temporary file beside `dest` (same filesystem, so the following rename is atomic),
/// renamed over `dest`, and the original source removed. `fs::copy` preserves the source's
/// permission mode.
fn rename_or_copy(source: &path::Path, dest: &path::Path) -> Result<()> {
    match fs::rename(source, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            let tmp = match dest.file_name() {
                Some(name) => {
                    let mut n = name.to_os_string();
                    n.push(".self_update.tmp");
                    dest.with_file_name(n)
                }
                None => return Err(Error::from(e)),
            };
            fs::copy(source, &tmp)?;
            if let Err(rename_err) = fs::rename(&tmp, dest) {
                let _ = fs::remove_file(&tmp);
                return Err(Error::from(rename_err));
            }
            let _ = fs::remove_file(source);
            Ok(())
        }
        Err(e) => Err(Error::from(e)),
    }
}

/// Transactionally install a *set* of files: either every `(source -> dest)` move is applied, or
/// — on the first failure — all already-applied moves are rolled back, restoring every
/// destination to its prior contents. Use it to update a tool that ships more than one file (a
/// binary plus sidecar libraries/resources) without risking a half-applied update.
///
/// This is the multi-file analogue of [`Move`]. It relies on `rename`, so **every source, every
/// destination, and the `temp` directory must live on the same filesystem** (the same constraint
/// [`Move::replace_using_temp`] has) — in particular the staging dir holding the files you `add`
/// must be co-located with the destinations, not in `$TMPDIR`. The `temp` directory is used to
/// stash each displaced destination so it can be restored on rollback; a [`tempfile::TempDir`] is a
/// convenient choice.
///
/// [`commit`](Self::commit) drains the queued moves as it applies them, so a `MoveAll` is
/// single-use: a second `commit` has nothing left to do and is a no-op returning `Ok(())`. Rollback
/// is best-effort — if a rollback step itself fails it is logged via `log::error!` rather than
/// surfaced, and the error returned to the caller is always the original one that triggered the
/// rollback.
///
/// ```no_run
/// # fn run() -> Result<(), Box<dyn std::error::Error>> {
/// // The stash dir must be on the same filesystem as the destinations (rename can't cross
/// // filesystems), so create it next to them rather than in $TMPDIR.
/// let tmp = tempfile::TempDir::new_in("/usr/local")?;
/// // `new_bin` / `new_lib` are files you already extracted into a staging dir, which must
/// // also be on the destination filesystem (the sources are renamed into place too).
/// let staging = tempfile::TempDir::new_in("/usr/local")?;
/// let new_bin = staging.path().join("app");
/// let new_lib = staging.path().join("libapp.so");
/// self_update::MoveAll::from_temp(tmp.path())
///     .add(new_bin, "/usr/local/bin/app")
///     .add(new_lib, "/usr/local/lib/libapp.so")
///     .commit()?; // all-or-nothing
/// # Ok(())
/// # }
/// ```
///
/// * Errors:
///     * Io - renaming a source into place or stashing an existing destination
#[derive(Debug)]
#[must_use = "queued moves are only applied when `.commit()` is called"]
#[non_exhaustive]
pub struct MoveAll {
    temp: path::PathBuf,
    moves: Vec<(path::PathBuf, path::PathBuf)>,
}

impl MoveAll {
    /// Start a transactional install, stashing displaced destinations under `temp` so they can be
    /// restored if a later move fails. `temp` must be on the same filesystem as every destination.
    /// Accepts anything path-like, storing an owned [`PathBuf`](std::path::PathBuf).
    pub fn from_temp(temp: impl AsRef<path::Path>) -> Self {
        Self {
            temp: temp.as_ref().to_path_buf(),
            moves: Vec::new(),
        }
    }

    /// Queue a `source -> dest` move. Moves are applied by [`commit`](Self::commit) in the order
    /// added.
    pub fn add(
        &mut self,
        source: impl AsRef<path::Path>,
        dest: impl AsRef<path::Path>,
    ) -> &mut Self {
        self.moves
            .push((source.as_ref().to_path_buf(), dest.as_ref().to_path_buf()));
        self
    }

    /// Apply every queued move. On success all destinations have been replaced. On the first
    /// failure, every already-applied move (and the failing one's partial state) is rolled back so
    /// each destination is left with its original contents, and the underlying error is returned.
    ///
    /// The queued moves are drained as they are applied, so calling `commit` again is a no-op that
    /// returns `Ok(())`.
    pub fn commit(&mut self) -> Result<()> {
        // Drain the queue so a second `commit` is a no-op rather than re-running already-applied
        // moves against now-missing sources.
        let moves = std::mem::take(&mut self.moves);

        // For each applied move we remember the destination and where its previous contents (if
        // any) were stashed, so a later failure can restore them in reverse order.
        let mut applied: Vec<Applied> = Vec::with_capacity(moves.len());

        for (i, (source, dest)) in moves.iter().enumerate() {
            // Stash an existing destination so we can move it back on rollback.
            let stash = if dest.exists() {
                let stash = self.temp.join(format!("self_update-stash-{i}"));
                if let Err(e) = fs::rename(dest, &stash) {
                    rollback(&applied);
                    return Err(Error::from(e));
                }
                Some(stash)
            } else {
                None
            };

            // Move the new file into place.
            if let Err(e) = fs::rename(source, dest) {
                // Undo this step's stash before rolling back the earlier ones.
                if let Some(stash) = &stash
                    && let Err(restore_err) = fs::rename(stash, dest)
                {
                    log::error!(
                        "failed to restore {:?} from stash {:?} during rollback: {}",
                        dest,
                        stash,
                        restore_err
                    );
                }
                rollback(&applied);
                return Err(Error::from(e));
            }

            applied.push(Applied {
                dest: dest.clone(),
                stash,
            });
        }

        Ok(())
    }
}

/// A move that [`MoveAll::commit`] has applied, retained so it can be undone on a later failure.
#[derive(Debug)]
struct Applied {
    dest: path::PathBuf,
    stash: Option<path::PathBuf>,
}

/// Best-effort rollback of already-applied moves, in reverse order. For a destination that
/// previously existed, the stashed original is `rename`d back over the newly installed file — a
/// single atomic replace (the same technique [`Move::replace_using_temp`] uses), so the original
/// is never deleted before its restore can fail. For a destination that didn't previously exist
/// (a fresh install), the newly installed file is simply removed. Rollback failures are logged
/// rather than propagated — the original error that triggered the rollback is what callers see.
fn rollback(applied: &[Applied]) {
    for entry in applied.iter().rev() {
        match &entry.stash {
            // Previously existed: atomically restore the original over the new file.
            Some(stash) => {
                if let Err(e) = fs::rename(stash, &entry.dest) {
                    log::error!(
                        "failed to restore {:?} from stash {:?} during rollback: {}",
                        entry.dest,
                        stash,
                        e
                    );
                }
            }
            // Fresh install (nothing to restore): remove the file we added.
            None => {
                if let Err(e) = fs::remove_file(&entry.dest) {
                    log::error!("failed to remove {:?} during rollback: {}", entry.dest, e);
                }
            }
        }
    }
}

/// A download-progress callback: `(bytes_downloaded_so_far, total_bytes_if_known)`.
pub(crate) type DynProgressFn = dyn Fn(u64, Option<u64>) + Send + Sync;

/// Wrapper around a [`DynProgressFn`] so structs holding one can still derive `Clone`/`Debug`.
#[derive(Clone)]
pub(crate) struct ProgressCallback(pub(crate) std::sync::Arc<DynProgressFn>);

impl std::fmt::Debug for ProgressCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProgressCallback(..)")
    }
}

/// A post-update verification callback: given the path to the freshly-extracted binary (before it
/// is installed), returns `Ok(())` to accept it or `Err(..)` to reject it (aborting the update). A
/// returned error's message is carried as the reason of the resulting
/// [`Error::VerificationRejected`](errors::Error::VerificationRejected).
pub(crate) type DynVerifyFn = dyn Fn(&std::path::Path) -> Result<()> + Send + Sync;

/// Wrapper around a [`DynVerifyFn`] so structs holding one can still derive `Clone`/`Debug`.
#[derive(Clone)]
pub(crate) struct VerifyCallback(pub(crate) std::sync::Arc<DynVerifyFn>);

impl std::fmt::Debug for VerifyCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VerifyCallback(..)")
    }
}

/// A custom asset-selection callback: given the release's assets, returns the asset to download
/// (or `None` to fail the update). Overrides the built-in target/identifier substring matching.
pub(crate) type DynAssetMatcher = dyn Fn(&[ReleaseAsset]) -> Option<ReleaseAsset> + Send + Sync;

/// Wrapper around a [`DynAssetMatcher`] so structs holding one can still derive `Clone`/`Debug`.
#[derive(Clone)]
pub(crate) struct AssetMatcher(pub(crate) std::sync::Arc<DynAssetMatcher>);

impl std::fmt::Debug for AssetMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssetMatcher(..)")
    }
}

/// Download things into files
///
/// With optional progress bar
#[non_exhaustive]
pub struct Download {
    show_progress: bool,
    url: String,
    headers: http_client::header::HeaderMap,
    #[cfg(feature = "progress-bar")]
    progress_template: String,
    #[cfg(feature = "progress-bar")]
    progress_chars: String,
    timeout: Option<std::time::Duration>,
    on_progress: Option<ProgressCallback>,
    /// Optional cap on the number of bytes streamed into `dest`. `None` (the default) means no cap,
    /// preserving prior unbounded behavior. When set, the streaming download aborts with an error as
    /// soon as the total bytes written would exceed this many bytes.
    max_download_size: Option<u64>,
    /// Number of times to retry establishing the download request (before any bytes are streamed)
    /// with exponential backoff. `0` (the default) means a single attempt, preserving the prior
    /// no-retry behavior. A failure that occurs *after* streaming has begun is not retried (it would
    /// corrupt the partially-written destination).
    retries: u32,
    retry_base_delay: std::time::Duration,
    retry_max_delay: std::time::Duration,
    /// Optional user-supplied sync HTTP client (used through the trait); `None` => crate default.
    client: Option<std::sync::Arc<dyn http_client::HttpClient>>,
    /// Optional user-supplied async HTTP client; `None` => crate default. Async is reqwest-only.
    #[cfg(feature = "async")]
    async_client: Option<std::sync::Arc<dyn http_client::AsyncHttpClient>>,
    /// Custom TLS root CA certificates to bake into the crate-built client when no client was
    /// injected (see [`add_root_certificate`](Self::add_root_certificate)).
    root_certificates: Vec<Certificate>,
    /// First error from a `request_header(name, value)` argument that wasn't a valid HTTP header.
    /// Deferred like the builders' `request_header` so the setter stays infallible; surfaced from
    /// [`download_to`](Self::download_to) as an `Error::InvalidHeader`.
    header_error: Option<String>,
}

impl std::fmt::Debug for Download {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Download");
        s.field("show_progress", &self.show_progress)
            .field("url", &self.url)
            .field("headers", &self.headers);
        #[cfg(feature = "progress-bar")]
        s.field("progress_template", &self.progress_template)
            .field("progress_chars", &self.progress_chars);
        s.field("timeout", &self.timeout)
            .field(
                "on_progress",
                &self.on_progress.as_ref().map(|_| "<callback>"),
            )
            .field("max_download_size", &self.max_download_size)
            .field("client", &self.client.as_ref().map(|_| "<http_client>"));
        #[cfg(feature = "async")]
        s.field(
            "async_client",
            &self.async_client.as_ref().map(|_| "<async_http_client>"),
        );
        s.field(
            "root_certificates",
            &format_args!("<{} root_certificates>", self.root_certificates.len()),
        );
        s.finish()
    }
}

/// Build the error returned when a streaming download exceeds its configured
/// [`max_download_size`](Download::max_download_size) cap. Reported as an [`Error::Io`] with a
/// message naming the cap so the failure mode is unambiguous.
fn max_download_size_exceeded(cap: u64) -> Error {
    Error::Io(io::Error::other(format!(
        "download exceeded the configured max_download_size cap of {cap} bytes"
    )))
}

impl Download {
    /// Specify download url. Accepts anything string-like (`&str`, `String`, …).
    pub fn from_url(url: impl Into<String>) -> Self {
        Self {
            show_progress: false,
            url: url.into(),
            headers: http_client::header::HeaderMap::new(),
            #[cfg(feature = "progress-bar")]
            progress_template: DEFAULT_PROGRESS_TEMPLATE.to_string(),
            #[cfg(feature = "progress-bar")]
            progress_chars: DEFAULT_PROGRESS_CHARS.to_string(),
            timeout: None,
            on_progress: None,
            max_download_size: None,
            retries: 0,
            retry_base_delay: std::time::Duration::from_millis(100),
            retry_max_delay: std::time::Duration::from_millis(3200),
            client: None,
            #[cfg(feature = "async")]
            async_client: None,
            root_certificates: vec![],
            header_error: None,
        }
    }

    /// Toggle the download progress bar. Named to match the `Update` builder's setter of the same
    /// name.
    pub fn show_download_progress(&mut self, b: bool) -> &mut Self {
        self.show_progress = b;
        self
    }

    /// Set a timeout for the download request. Defaults to no timeout.
    pub fn timeout(&mut self, timeout: std::time::Duration) -> &mut Self {
        self.timeout = Some(timeout);
        self
    }

    /// Cap the number of bytes this download will stream into `dest`. Once the running total of
    /// written bytes exceeds `max_bytes`, the download aborts with an [`Error`] instead of writing
    /// an unbounded amount (useful to defend against a server that streams far more than the
    /// advertised `Content-Length`). Defaults to no cap, so existing behavior is unchanged.
    pub fn max_download_size(&mut self, max_bytes: u64) -> &mut Self {
        self.max_download_size = Some(max_bytes);
        self
    }

    /// Register a callback invoked as the download streams, with
    /// `(bytes_downloaded_so_far, total_bytes)` — `total_bytes` is `None` when the server does
    /// not send a `Content-Length`. Independent of the terminal progress bar
    /// ([`show_download_progress`](Self::show_download_progress)); use it to drive a GUI, structured logging, or
    /// any non-terminal progress display. The callback is `Fn`, so track state via interior
    /// mutability (e.g. an `AtomicU64` or a channel).
    pub fn progress_callback(
        &mut self,
        callback: impl Fn(u64, Option<u64>) + Send + Sync + 'static,
    ) -> &mut Self {
        self.on_progress = Some(ProgressCallback(std::sync::Arc::new(callback)));
        self
    }

    /// Internal: set the progress callback from an already-wrapped `Arc` (used by the update
    /// flow to forward an `Update`'s callback to its download).
    pub(crate) fn set_progress_callback_arc(
        &mut self,
        callback: std::sync::Arc<DynProgressFn>,
    ) -> &mut Self {
        self.on_progress = Some(ProgressCallback(callback));
        self
    }

    /// Set the progress style, as a typed [`ProgressStyle`] (template + chars) so the two strings
    /// can't be transposed.
    #[cfg(feature = "progress-bar")]
    pub fn progress_style(&mut self, style: ProgressStyle) -> &mut Self {
        self.progress_template = style.template;
        self.progress_chars = style.chars;
        self
    }

    /// Replace the entire download request `HeaderMap`. To add a single header without discarding
    /// the others, use [`request_header`](Self::request_header) instead.
    pub fn replace_headers(&mut self, headers: http_client::header::HeaderMap) -> &mut Self {
        self.headers = headers;
        self
    }

    /// Internal: set the download request-establishment retry budget and backoff (used by the
    /// update flow to forward an `Update`'s configured `retries`/`retry_backoff` to the download).
    pub(crate) fn set_retries(
        &mut self,
        retries: u32,
        base: std::time::Duration,
        max: std::time::Duration,
    ) -> &mut Self {
        self.retries = retries;
        self.retry_base_delay = base;
        self.retry_max_delay = max;
        self
    }

    /// Internal: set the injected HTTP clients from already-built `Arc`s (used by the update flow to
    /// forward an `Update`'s injected client to its download).
    pub(crate) fn set_http_client(
        &mut self,
        client: Option<std::sync::Arc<dyn http_client::HttpClient>>,
        #[cfg(feature = "async")] async_client: Option<
            std::sync::Arc<dyn http_client::AsyncHttpClient>,
        >,
    ) -> &mut Self {
        self.client = client;
        #[cfg(feature = "async")]
        {
            self.async_client = async_client;
        }
        self
    }

    /// Add a custom TLS root CA certificate the crate-built HTTP client will trust. Call multiple
    /// times to add more than one. Ignored when an HTTP client is injected via `set_http_client`
    /// (the injected client owns its own TLS config). A malformed certificate surfaces as an
    /// [`Error::InvalidCertificate`] from [`download_to`](Self::download_to).
    ///
    /// **ureq-only builds**: when the `reqwest` feature is disabled, the crate-built ureq client
    /// trusts *only* the supplied certificates (replacing the default Mozilla root set). Supply all
    /// CA certificates you need, including any public roots, or inject a `ureq::Agent` via
    /// `set_http_client` with a merged root set instead.
    pub fn add_root_certificate(&mut self, cert: Certificate) -> &mut Self {
        self.root_certificates.push(cert);
        self
    }

    /// Internal: the configured custom root CA certificates (used by tests to confirm cert
    /// forwarding). Empty unless [`add_root_certificate`](Self::add_root_certificate) was called.
    #[cfg(test)]
    pub(crate) fn root_certificates(&self) -> &[Certificate] {
        &self.root_certificates
    }

    /// Set a download request header, inserting into the existing `HeaderMap`. To add a single
    /// header without discarding the others; to replace the whole map use
    /// [`replace_headers`](Self::replace_headers).
    ///
    /// Accepts anything that converts into a header name/value, so both typed values and plain
    /// strings work: `.request_header("X-Foo", "bar")` or
    /// `.request_header(self_update::http::header::ACCEPT, "application/octet-stream")`. The setter
    /// is infallible; a name or value that is not a valid HTTP header is deferred and surfaced from
    /// [`download_to`](Self::download_to) as an
    /// [`Error::InvalidHeader`], matching the builders'
    /// `request_header` verb.
    pub fn request_header<N, V>(&mut self, name: N, value: V) -> &mut Self
    where
        N: ::core::convert::TryInto<http_client::header::HeaderName>,
        V: ::core::convert::TryInto<http_client::header::HeaderValue>,
    {
        match (name.try_into(), value.try_into()) {
            (Ok(name), Ok(value)) => {
                self.headers.insert(name, value);
            }
            _ => {
                if self.header_error.is_none() {
                    self.header_error =
                        Some("invalid HTTP header passed to `request_header`".to_string());
                }
            }
        }
        self
    }

    /// Surface a deferred `request_header` conversion failure as an `Error::InvalidHeader`.
    fn check_header_error(&self) -> Result<()> {
        if let Some(msg) = &self.header_error {
            return Err(Error::InvalidHeader {
                source: Box::new(errors::MessageError(msg.clone())),
            });
        }
        Ok(())
    }

    /// Download the file behind the given `url` into the specified `dest`.
    /// Show a sliding progress bar if specified.
    /// If the resource doesn't specify a content-length, the progress bar will not be shown
    ///
    /// * Errors:
    ///     * HTTP client network errors
    ///     * Unsuccessful response status
    ///     * Progress-bar errors
    ///     * Reading from response to `BufReader`-buffer
    ///     * Writing from `BufReader`-buffer to `File`
    pub fn download_to<T: io::Write>(&self, mut dest: T) -> Result<()> {
        use io::BufRead;
        self.check_header_error()?;
        let mut headers = self.headers.clone();
        if !headers.contains_key(header::USER_AGENT) {
            headers.insert(
                header::USER_AGENT,
                DEFAULT_USER_AGENT.parse().expect("invalid user-agent"),
            );
        }

        let default;
        let built;
        let client: &dyn http_client::HttpClient = match self.client.as_deref() {
            Some(c) => c,
            None if !self.root_certificates.is_empty() => {
                // No injected client but custom root CAs were supplied: build a client that trusts
                // them. A malformed cert / build failure surfaces here as `Error::InvalidCertificate`.
                built = http_client::client_with_root_certs(&self.root_certificates)
                    .map_err(|source| Error::InvalidCertificate { source })?;
                &*built
            }
            None => {
                default = http_client::default_client();
                &*default
            }
        };
        // Retry only the request-establishment phase (before any bytes are streamed): a failure
        // after streaming begins would corrupt the partially-written destination. With the default
        // `retries == 0` this is a single attempt.
        let resp = backends::retry(
            self.retries,
            self.retry_base_delay,
            self.retry_max_delay,
            || client.get(&self.url, &headers, self.timeout),
            |e, backoff| {
                log::warn!(
                    "self_update: download request to {} failed ({e}); retrying in {backoff}ms",
                    crate::errors::redact_url(&self.url)
                );
                std::thread::sleep(std::time::Duration::from_millis(backoff));
            },
        )?;
        let size = resp
            .headers()
            .get(http_client::header::CONTENT_LENGTH)
            .map(|val| {
                val.to_str()
                    .map(|s| s.parse::<u64>().unwrap_or(0))
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        // `http_client::get` already errored on a non-success status (see `download_to_async`).
        let total = if size == 0 { None } else { Some(size) };
        #[cfg(feature = "progress-bar")]
        let show_progress = if size == 0 { false } else { self.show_progress };

        let mut src = io::BufReader::new(resp.body());
        let mut downloaded: u64 = 0;
        #[cfg(feature = "progress-bar")]
        let mut bar = if show_progress {
            let style = IndicatifProgressStyle::default_bar()
                .template(&self.progress_template)
                .map_err(|e| Error::InvalidProgressStyle {
                    source: Box::new(e),
                })?
                .progress_chars(&self.progress_chars);
            let pb = ProgressBar::new(size);
            pb.set_style(style);
            Some(pb)
        } else {
            None
        };
        loop {
            let n = {
                let buf = src.fill_buf()?;
                dest.write_all(buf)?;
                buf.len()
            };
            if n == 0 {
                break;
            }
            src.consume(n);
            downloaded += n as u64;
            if let Some(cap) = self.max_download_size
                && downloaded > cap
            {
                return Err(max_download_size_exceeded(cap));
            }

            #[cfg(feature = "progress-bar")]
            if let Some(ref mut bar) = bar {
                bar.set_position(min(downloaded, size));
            }
            if let Some(ref cb) = self.on_progress {
                (cb.0)(downloaded, total);
            }
        }
        #[cfg(feature = "progress-bar")]
        if let Some(ref mut bar) = bar {
            bar.finish_with_message("Done");
        }
        Ok(())
    }

    /// Async sibling of [`download_to`](Self::download_to): stream the download into `dest` using
    /// the async (reqwest) transport, driving the same progress bar / callback. `dest` is a
    /// synchronous writer (chunks are written as they arrive); file IO is not offloaded.
    #[cfg(feature = "async")]
    pub async fn download_to_async<T: io::Write>(&self, mut dest: T) -> Result<()> {
        use futures_util::StreamExt;

        self.check_header_error()?;
        let mut headers = self.headers.clone();
        if !headers.contains_key(header::USER_AGENT) {
            headers.insert(
                header::USER_AGENT,
                DEFAULT_USER_AGENT.parse().expect("invalid user-agent"),
            );
        }

        let default;
        let built;
        let client: &dyn http_client::AsyncHttpClient = match self.async_client.as_deref() {
            Some(c) => c,
            None if !self.root_certificates.is_empty() => {
                // No injected async client but custom root CAs were supplied: build one that trusts
                // them. A malformed cert / build failure surfaces here as `Error::InvalidCertificate`.
                built = http_client::async_client_with_root_certs(&self.root_certificates)
                    .map_err(|source| Error::InvalidCertificate { source })?;
                &*built
            }
            None => {
                default = http_client::default_async_client();
                &*default
            }
        };
        // Retry only the request-establishment phase (see `download_to`).
        let resp = backends::retry_async(
            self.retries,
            self.retry_base_delay,
            self.retry_max_delay,
            || client.get(&self.url, &headers, self.timeout),
            |e, backoff| {
                log::warn!(
                    "self_update: download request to {} failed ({e}); retrying in {backoff}ms",
                    crate::errors::redact_url(&self.url)
                );
            },
            |backoff| tokio::time::sleep(std::time::Duration::from_millis(backoff)),
        )
        .await?;
        let size = resp
            .headers()
            .get(http_client::header::CONTENT_LENGTH)
            .map(|val| {
                val.to_str()
                    .map(|s| s.parse::<u64>().unwrap_or(0))
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        // `get_async` already errored on a non-success status.
        let total = if size == 0 { None } else { Some(size) };
        #[cfg(feature = "progress-bar")]
        let show_progress = if size == 0 { false } else { self.show_progress };

        let mut downloaded: u64 = 0;
        #[cfg(feature = "progress-bar")]
        let mut bar = if show_progress {
            let style = IndicatifProgressStyle::default_bar()
                .template(&self.progress_template)
                .map_err(|e| Error::InvalidProgressStyle {
                    source: Box::new(e),
                })?
                .progress_chars(&self.progress_chars);
            let pb = ProgressBar::new(size);
            pb.set_style(style);
            Some(pb)
        } else {
            None
        };

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            dest.write_all(&chunk)?;
            downloaded += chunk.len() as u64;
            if let Some(cap) = self.max_download_size
                && downloaded > cap
            {
                return Err(max_download_size_exceeded(cap));
            }

            #[cfg(feature = "progress-bar")]
            if let Some(ref mut bar) = bar {
                bar.set_position(min(downloaded, size));
            }
            if let Some(ref cb) = self.on_progress {
                (cb.0)(downloaded, total);
            }
        }
        #[cfg(feature = "progress-bar")]
        if let Some(ref mut bar) = bar {
            bar.finish_with_message("Done");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "compression-tar-gz")]
    use flate2::{self, write::GzEncoder};
    #[allow(unused_imports)]
    use std::{
        fs::{self, File},
        io::{self, Read, Write},
        path::{Path, PathBuf},
    };

    #[test]
    fn version_status_is_up_to_date() {
        assert!(VersionStatus::UpToDate("1.2.3".to_string()).is_up_to_date());
        assert!(!VersionStatus::Updated("1.2.3".to_string()).is_up_to_date());
        // `is_updated()` is the complement.
        assert!(VersionStatus::Updated("1.2.3".to_string()).is_updated());
        assert!(!VersionStatus::UpToDate("1.2.3".to_string()).is_updated());
    }

    #[test]
    fn version_status_version_accessor() {
        // version() returns the wrapped string for both variants.
        assert_eq!(
            VersionStatus::UpToDate("1.0.0".to_string()).version(),
            "1.0.0"
        );
        assert_eq!(
            VersionStatus::Updated("2.0.0".to_string()).version(),
            "2.0.0"
        );
    }

    #[test]
    fn version_status_display() {
        // Display is human-readable, not Debug form.
        assert_eq!(
            VersionStatus::UpToDate("1.0.0".to_string()).to_string(),
            "UpToDate(1.0.0)"
        );
        assert_eq!(
            VersionStatus::Updated("2.0.0".to_string()).to_string(),
            "Updated(2.0.0)"
        );
    }

    // `ArchiveKind` renders a friendly, human-readable name via `Display` (used in error messages),
    // not the `Debug` form (which leaks the enum shape like `Tar(Some(Gz))`).
    #[test]
    fn archive_kind_display_is_human_readable() {
        assert_eq!(ArchiveKind::Plain(None).to_string(), "plain");
        assert_eq!(ArchiveKind::Plain(Some(Compression::Gz)).to_string(), "gz");
        assert_eq!(ArchiveKind::Plain(Some(Compression::Xz)).to_string(), "xz");
        #[cfg(feature = "archive-tar")]
        {
            assert_eq!(ArchiveKind::Tar(None).to_string(), "tar");
            assert_eq!(
                ArchiveKind::Tar(Some(Compression::Gz)).to_string(),
                "tar.gz"
            );
            assert_eq!(
                ArchiveKind::Tar(Some(Compression::Xz)).to_string(),
                "tar.xz"
            );
        }
        #[cfg(feature = "archive-zip")]
        assert_eq!(ArchiveKind::Zip.to_string(), "zip");
    }

    // A3/A4/A12/A5: the ergonomic argument types are accepted. These are compile-locks plus light
    // assertions: `Download::from_url` takes `impl Into<String>`; `Extract::from_source` /
    // `Move::from_source` / `MoveAll::from_temp` take `impl AsRef<Path>` (now lifetime-free); the
    // Download header verb is `request_header`; and `progress_style` takes a typed `ProgressStyle`.
    #[test]
    fn ergonomic_constructors_accept_owned_and_borrowed_paths_and_strings() {
        // from_url accepts &str and String.
        let _ = Download::from_url("https://example.com/a.bin");
        let _ = Download::from_url(String::from("https://example.com/b.bin"));

        // Extract::from_source accepts &str, PathBuf, and &Path — and the struct holds no lifetime.
        let _: Extract = Extract::from_source("some/path.tar.gz");
        let _: Extract = Extract::from_source(PathBuf::from("some/path.tar.gz"));
        let owned = PathBuf::from("some/path.tar.gz");
        let _: Extract = Extract::from_source(owned.as_path());

        // Move::from_source / replace_using_temp accept path-like; the type is lifetime-free.
        let mut mv: Move = Move::from_source("src");
        mv.replace_using_temp("tmp");

        // MoveAll::from_temp accepts path-like; lifetime-free.
        let _: MoveAll = MoveAll::from_temp("tmp-dir");
    }

    #[cfg(feature = "progress-bar")]
    #[test]
    fn progress_style_newtype_threads_template_and_chars() {
        // A5: `ProgressStyle::new(template, chars)` builds the typed pair and the Download setter
        // threads both fields through (no transposable two-arg setter).
        let style = ProgressStyle::new("[{bar:40}] {bytes}", "#>-");
        assert_eq!(style.template, "[{bar:40}] {bytes}");
        assert_eq!(style.chars, "#>-");

        let mut dl = Download::from_url("https://example.com/app.tar.gz");
        dl.progress_style(style);
        assert_eq!(dl.progress_template, "[{bar:40}] {bytes}");
        assert_eq!(dl.progress_chars, "#>-");
    }

    #[test]
    fn download_header_accepts_str_name_and_value() {
        let mut dl = Download::from_url("https://example.com/app.tar.gz");
        // Plain string literals must convert into a valid name/value.
        dl.request_header("x-custom-header", "custom-value");
        let stored = dl
            .headers
            .get("x-custom-header")
            .expect("header should be inserted");
        assert_eq!(stored, "custom-value");
    }

    #[test]
    fn download_header_accepts_typed_name_and_value() {
        let mut dl = Download::from_url("https://example.com/app.tar.gz");
        // The typed `HeaderName` / `&str` value form still works.
        dl.request_header(http_client::header::ACCEPT, "application/octet-stream");
        assert_eq!(
            dl.headers.get(http_client::header::ACCEPT).unwrap(),
            "application/octet-stream"
        );
    }

    #[test]
    fn download_header_overwrites_on_repeated_name() {
        // B5: `header()` inserts into the existing map. Calling it twice with the same name must
        // keep the *last* value (insert semantics), not append or keep the first.
        let mut dl = Download::from_url("https://example.com/app.tar.gz");
        dl.request_header("x-dup", "first");
        dl.request_header("x-dup", "second");
        // `get` returns the (single) value; `get_all` must contain exactly one entry.
        assert_eq!(dl.headers.get("x-dup").unwrap(), "second");
        assert_eq!(
            dl.headers.get_all("x-dup").iter().count(),
            1,
            "a repeated header name must overwrite, not accumulate"
        );
    }

    #[test]
    fn replace_headers_wholesale_replaces_after_header_calls() {
        // B5: after building up headers with `header()`, `replace_headers` must discard them all
        // and install only the supplied map (it is a whole-map setter, not a merge).
        let mut dl = Download::from_url("https://example.com/app.tar.gz");
        dl.request_header("x-old-a", "a");
        dl.request_header("x-old-b", "b");

        let mut fresh = http_client::header::HeaderMap::new();
        fresh.insert("x-new", "n".parse().unwrap());
        dl.replace_headers(fresh);

        assert!(
            dl.headers.get("x-old-a").is_none(),
            "replace_headers must drop previously-added headers"
        );
        assert!(dl.headers.get("x-old-b").is_none());
        assert_eq!(dl.headers.get("x-new").unwrap(), "n");
        assert_eq!(
            dl.headers.len(),
            1,
            "replace_headers installs exactly the supplied map"
        );

        // And `header()` still works after a replace, inserting into the new map.
        dl.request_header("x-after", "y");
        assert_eq!(dl.headers.get("x-after").unwrap(), "y");
        assert_eq!(dl.headers.get("x-new").unwrap(), "n");
    }

    #[test]
    fn download_header_rejects_invalid_value() {
        let mut dl = Download::from_url("https://example.com/app.tar.gz");
        // A newline is not a valid header value. The setter is infallible (deferred): the bad
        // header is not inserted, and the error surfaces from `download_to`.
        dl.request_header("x-ok", "bad\nvalue");
        assert!(
            dl.headers.get("x-ok").is_none(),
            "the bad header must not be inserted"
        );
        let err = dl
            .download_to(Vec::<u8>::new())
            .expect_err("a deferred invalid header must surface from download_to");
        assert!(
            matches!(err, Error::InvalidHeader { .. }),
            "expected Error::InvalidHeader, got {:?}",
            err
        );
    }

    #[test]
    fn download_header_rejects_invalid_name() {
        let mut dl = Download::from_url("https://example.com/app.tar.gz");
        // A space is not valid in a header name. The setter is infallible (deferred); the invalid
        // name is rejected before any value insertion, so the map stays empty and the error
        // surfaces from download_to.
        dl.request_header("inva lid", "ok");
        assert!(
            dl.headers.is_empty(),
            "an invalid header name must not leave a partial value inserted"
        );
        let err = dl
            .download_to(Vec::<u8>::new())
            .expect_err("a deferred invalid header name must surface from download_to");
        assert!(matches!(err, Error::InvalidHeader { .. }));
    }

    #[test]
    fn detect_plain() {
        assert_eq!(
            ArchiveKind::Plain(None),
            detect_archive(&PathBuf::from("Something.exe")).unwrap()
        );
    }

    #[test]
    fn move_all_commits_every_move() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        // Two new files to install over two existing destinations.
        let src_a = dir.path().join("src_a");
        let src_b = dir.path().join("src_b");
        fs::write(&src_a, b"new-a").unwrap();
        fs::write(&src_b, b"new-b").unwrap();
        let dst_a = dir.path().join("dst_a");
        let dst_b = dir.path().join("dst_b");
        fs::write(&dst_a, b"old-a").unwrap();
        fs::write(&dst_b, b"old-b").unwrap();

        MoveAll::from_temp(temp.path())
            .add(&src_a, &dst_a)
            .add(&src_b, &dst_b)
            .commit()
            .unwrap();

        assert_eq!(fs::read(&dst_a).unwrap(), b"new-a");
        assert_eq!(fs::read(&dst_b).unwrap(), b"new-b");
    }

    #[test]
    fn move_all_rolls_back_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        // Three moves: the first two are valid and overwrite existing destinations (so both are
        // stashed and applied), the third points at a non-existent source so its move fails. This
        // drives the already-applied first two back through `rollback()` (the stash-restore path).
        let src_a = dir.path().join("src_a");
        let src_b = dir.path().join("src_b");
        fs::write(&src_a, b"new-a").unwrap();
        fs::write(&src_b, b"new-b").unwrap();
        let missing_src = dir.path().join("does_not_exist");

        let dst_a = dir.path().join("dst_a");
        let dst_b = dir.path().join("dst_b");
        let dst_c = dir.path().join("dst_c");
        fs::write(&dst_a, b"old-a").unwrap();
        fs::write(&dst_b, b"old-b").unwrap();
        fs::write(&dst_c, b"old-c").unwrap();

        let res = MoveAll::from_temp(temp.path())
            .add(&src_a, &dst_a)
            .add(&src_b, &dst_b)
            .add(&missing_src, &dst_c)
            .commit();
        assert!(res.is_err(), "a failing move must abort the transaction");

        // Every destination is restored to its original contents — both the applied moves
        // (rolled back via the stash) and the one whose move failed mid-step.
        assert_eq!(
            fs::read(&dst_a).unwrap(),
            b"old-a",
            "the first applied move must be rolled back"
        );
        assert_eq!(
            fs::read(&dst_b).unwrap(),
            b"old-b",
            "the second applied move must be rolled back"
        );
        assert_eq!(
            fs::read(&dst_c).unwrap(),
            b"old-c",
            "the failed move's stashed destination must be restored"
        );
    }

    #[test]
    fn move_all_installs_fresh_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        // Destination does not pre-exist (fresh install, no stash needed).
        let src = dir.path().join("src");
        fs::write(&src, b"fresh").unwrap();
        let dst = dir.path().join("new_dst");

        MoveAll::from_temp(temp.path())
            .add(&src, &dst)
            .commit()
            .unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"fresh");
    }

    #[test]
    fn move_all_second_commit_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        let src = dir.path().join("src");
        fs::write(&src, b"new").unwrap();
        let dst = dir.path().join("dst");
        fs::write(&dst, b"old").unwrap();

        let mut mover = MoveAll::from_temp(temp.path());
        mover.add(&src, &dst);
        mover.commit().unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"new");

        // The queue was drained, so a second commit does nothing and succeeds (rather than trying
        // to re-apply the move against the now-missing source and erroring out).
        mover.commit().unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"new");
    }

    #[test]
    fn download_invokes_progress_callback() {
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        // Serve a known-length body from a loopback server (no external network).
        let body = "x".repeat(20_000);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let served = body.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                served.len(),
                served
            );
            let _ = stream.write_all(resp.as_bytes());
        });

        let progress = Arc::new(Mutex::new(Vec::<(u64, Option<u64>)>::new()));
        let sink_progress = progress.clone();
        let mut out = Vec::new();
        Download::from_url(format!("http://{addr}/file"))
            .progress_callback(move |downloaded, total| {
                sink_progress.lock().unwrap().push((downloaded, total));
            })
            .download_to(&mut out)
            .unwrap();

        assert_eq!(out.len(), 20_000);
        let calls = progress.lock().unwrap();
        assert!(!calls.is_empty(), "callback should have been invoked");
        // `total` reflects the Content-Length on every call.
        assert!(calls.iter().all(|(_, total)| *total == Some(20_000)));
        // `downloaded` is monotonically non-decreasing and reaches the full size.
        let mut last = 0u64;
        for (downloaded, _) in calls.iter() {
            assert!(*downloaded >= last);
            last = *downloaded;
        }
        assert_eq!(calls.last().unwrap().0, 20_000);
    }

    /// A test-double [`HttpResponse`](http_client::HttpResponse) returning a canned body and a
    /// configurable `Content-Length`. Used to prove `download_to` streams the injected client's
    /// body through the trait (`headers()` + `body()`), not a real network response.
    struct DlResponse {
        body: Vec<u8>,
        headers: http_client::header::HeaderMap,
    }

    impl http_client::HttpResponse for DlResponse {
        fn headers(&self) -> &http_client::header::HeaderMap {
            &self.headers
        }
        fn body(self: Box<Self>) -> Box<dyn io::Read> {
            Box::new(io::Cursor::new(self.body))
        }
    }

    /// A test-double [`HttpClient`](http_client::HttpClient) (neither reqwest nor ureq) that records
    /// the requested URL and returns a canned [`DlResponse`].
    struct DlClient {
        body: Vec<u8>,
        content_length: Option<u64>,
        requested: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl http_client::HttpClient for DlClient {
        fn get(
            &self,
            url: &str,
            _headers: &http_client::header::HeaderMap,
            _timeout: Option<std::time::Duration>,
        ) -> Result<Box<dyn http_client::HttpResponse>> {
            self.requested.lock().unwrap().push(url.to_string());
            let mut headers = http_client::header::HeaderMap::new();
            if let Some(len) = self.content_length {
                headers.insert(
                    http_client::header::CONTENT_LENGTH,
                    len.to_string().parse().unwrap(),
                );
            }
            Ok(Box::new(DlResponse {
                body: self.body.clone(),
                headers,
            }))
        }
    }

    /// A flaky [`HttpClient`](http_client::HttpClient) that fails the first `fail_times` GETs with a
    /// transport error, then succeeds — to prove `download_to` retries the request-establishment
    /// phase when a retry budget is configured (B9).
    struct FlakyDlClient {
        body: Vec<u8>,
        fail_times: std::sync::atomic::AtomicU32,
        attempts: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    impl http_client::HttpClient for FlakyDlClient {
        fn get(
            &self,
            _url: &str,
            _headers: &http_client::header::HeaderMap,
            _timeout: Option<std::time::Duration>,
        ) -> Result<Box<dyn http_client::HttpResponse>> {
            use std::sync::atomic::Ordering;
            self.attempts.fetch_add(1, Ordering::SeqCst);
            if self.fail_times.load(Ordering::SeqCst) > 0 {
                self.fail_times.fetch_sub(1, Ordering::SeqCst);
                return Err(Error::HttpStatus {
                    status: 503,
                    url: "u".into(),
                });
            }
            let mut headers = http_client::header::HeaderMap::new();
            headers.insert(
                http_client::header::CONTENT_LENGTH,
                self.body.len().to_string().parse().unwrap(),
            );
            Ok(Box::new(DlResponse {
                body: self.body.clone(),
                headers,
            }))
        }
    }

    #[test]
    fn download_retries_request_establishment_with_configured_budget() {
        // B9: with a retry budget, `download_to` re-establishes the request after a transient
        // failure (before any bytes are streamed) and ultimately succeeds. Two failures then a
        // success => three attempts. A short base/cap keeps the test fast.
        use std::sync::atomic::{AtomicU32, Ordering};
        let body = b"payload-after-retries".to_vec();
        let attempts = std::sync::Arc::new(AtomicU32::new(0));
        let client = std::sync::Arc::new(FlakyDlClient {
            body: body.clone(),
            fail_times: AtomicU32::new(2),
            attempts: attempts.clone(),
        });

        let mut out = Vec::new();
        let mut dl = Download::from_url("https://nonroutable.invalid/asset.bin");
        dl.set_http_client(
            Some(client),
            #[cfg(feature = "async")]
            None,
        );
        dl.set_retries(
            3,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(2),
        );
        dl.download_to(&mut out).unwrap();

        assert_eq!(out, body, "the download succeeds after retrying");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "two failed attempts plus the successful third"
        );
    }

    #[test]
    fn download_without_retry_budget_does_not_retry() {
        // With the default `retries == 0`, a single failure is fatal (one attempt, no retry).
        use std::sync::atomic::{AtomicU32, Ordering};
        let attempts = std::sync::Arc::new(AtomicU32::new(0));
        let client = std::sync::Arc::new(FlakyDlClient {
            body: b"never-reached".to_vec(),
            fail_times: AtomicU32::new(5),
            attempts: attempts.clone(),
        });

        let mut out = Vec::new();
        let mut dl = Download::from_url("https://nonroutable.invalid/asset.bin");
        dl.set_http_client(
            Some(client),
            #[cfg(feature = "async")]
            None,
        );
        let res = dl.download_to(&mut out);
        assert!(
            res.is_err(),
            "no retry budget => the first failure is fatal"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "exactly one attempt with retries == 0"
        );
    }

    #[test]
    fn download_to_uses_injected_http_client_through_the_trait() {
        // Gap #4 (sync Download path): an arbitrary `Arc<dyn HttpClient>` that is NOT reqwest/ureq,
        // injected via `.http_client(...)`, must actually drive `download_to` — the streamed body
        // comes from the fake and the fake records the requested URL. No network is touched (the URL
        // is non-routable).
        let requested = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let body = b"injected-binary-payload".to_vec();
        let client = std::sync::Arc::new(DlClient {
            body: body.clone(),
            content_length: Some(body.len() as u64),
            requested: requested.clone(),
        });

        let mut out = Vec::new();
        let mut dl = Download::from_url("https://nonroutable.invalid/asset.bin");
        dl.set_http_client(
            Some(client),
            #[cfg(feature = "async")]
            None,
        );
        dl.download_to(&mut out).unwrap();

        assert_eq!(out, body, "download_to streamed the injected client's body");
        let urls = requested.lock().unwrap();
        assert_eq!(
            urls.len(),
            1,
            "exactly one GET went through the injected client"
        );
        assert_eq!(urls[0], "https://nonroutable.invalid/asset.bin");
    }

    #[test]
    fn download_to_handles_injected_client_without_content_length() {
        // When the injected response carries no Content-Length, `download_to` must still stream the
        // whole body to completion (size defaults to 0 -> no progress bar, `total == None`) and the
        // progress callback still fires with `total = None`.
        let requested = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let body = b"no-length-body".to_vec();
        let client = std::sync::Arc::new(DlClient {
            body: body.clone(),
            content_length: None,
            requested: requested.clone(),
        });

        let totals = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Option<u64>>::new()));
        let sink = totals.clone();
        let mut out = Vec::new();
        let mut dl = Download::from_url("https://nonroutable.invalid/asset.bin");
        dl.set_http_client(
            Some(client),
            #[cfg(feature = "async")]
            None,
        );
        dl.progress_callback(move |_d, total| sink.lock().unwrap().push(total));
        dl.download_to(&mut out).unwrap();

        assert_eq!(
            out, body,
            "the full body is streamed even with no Content-Length"
        );
        let totals = totals.lock().unwrap();
        assert!(
            totals.iter().all(|t| t.is_none()),
            "with no Content-Length the callback's total must be None, got {:?}",
            totals
        );
    }

    // --- S1: presigned-URL redaction in the download retry warning ---------------------------

    /// A `log::Log` that captures every record's formatted message into a shared global buffer.
    /// Tests filter the buffer by a unique URL host so a single global logger can serve them all.
    struct CaptureLogger;
    static CAPTURE_LOGGER: CaptureLogger = CaptureLogger;

    fn log_capture() -> &'static std::sync::Mutex<Vec<String>> {
        static BUF: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
        BUF.get_or_init(|| std::sync::Mutex::new(Vec::new()))
    }

    impl log::Log for CaptureLogger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            log_capture()
                .lock()
                .unwrap()
                .push(format!("{}", record.args()));
        }
        fn flush(&self) {}
    }

    fn install_capture_logger() {
        static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        INIT.get_or_init(|| {
            // Ignore an error if another harness already set the global logger.
            let _ = log::set_logger(&CAPTURE_LOGGER);
            log::set_max_level(log::LevelFilter::Warn);
        });
    }

    #[test]
    fn download_retry_warning_redacts_presigned_signature() {
        // S1: a download that retries must not leak a presigned S3 `X-Amz-Signature` /
        // `X-Amz-Credential` into the retry warning. Drive `download_to` with a flaky client (one
        // failure, then success) so the retry closure fires exactly one `log::warn!`, and assert the
        // captured line carries neither secret. On the pre-fix code (which logged the raw
        // `self.url`) the signature would appear verbatim and this test fails.
        use std::sync::atomic::AtomicU32;

        install_capture_logger();
        let sig = "abc123-secret-signature-value";
        let cred = "AKIAREDACTTESTONLY";
        let host = "s3-redact-retry-test.invalid";
        let url = format!(
            "https://{host}/app.tar.gz?X-Amz-Credential={cred}%2F20260101\
             &X-Amz-Expires=300&X-Amz-Signature={sig}&X-Amz-SignedHeaders=host"
        );

        let attempts = std::sync::Arc::new(AtomicU32::new(0));
        let client = std::sync::Arc::new(FlakyDlClient {
            body: b"ok".to_vec(),
            fail_times: AtomicU32::new(1),
            attempts: attempts.clone(),
        });

        let mut out = Vec::new();
        let mut dl = Download::from_url(url);
        dl.set_http_client(
            Some(client),
            #[cfg(feature = "async")]
            None,
        );
        dl.set_retries(
            1,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(2),
        );
        dl.download_to(&mut out).unwrap();

        let lines: Vec<String> = log_capture()
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains(host))
            .cloned()
            .collect();
        assert!(
            !lines.is_empty(),
            "the retry closure should have logged a warning for {host}"
        );
        for line in &lines {
            assert!(
                !line.contains(sig),
                "presigned signature leaked into the retry warning: {line}"
            );
            assert!(
                !line.contains(cred),
                "presigned credential leaked into the retry warning: {line}"
            );
        }
    }

    // --- S3: extracted zip modes must not carry setuid/setgid/sticky --------------------------

    #[cfg(all(unix, feature = "archive-zip"))]
    #[test]
    fn extract_zip_masks_setuid_setgid_sticky_bits() {
        // S3: a zip entry archived with a setuid mode (0o4755) must NOT install a setuid file; the
        // extractor masks the mode to `& 0o777`, so the setuid/setgid/sticky bits are dropped while
        // the ordinary rwx bits survive. On the pre-fix code (`from_mode(mode)`) the installed file
        // would be setuid and this test fails.
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("archive.zip");
        {
            let file = fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o4755);
            zip.start_file("payload", opts).unwrap();
            zip.write_all(b"#!/bin/sh\n").unwrap();
            zip.finish().unwrap();
        }

        let out_dir = tmp.path().join("out");
        fs::create_dir_all(&out_dir).unwrap();
        let mut ex = Extract::from_source(&zip_path);
        ex.archive(ArchiveKind::Zip);
        ex.extract_into(&out_dir).unwrap();

        let extracted = out_dir.join("payload");
        let mode = fs::metadata(&extracted).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o7000,
            0,
            "extracted file must carry no setuid/setgid/sticky bits, got mode {mode:o}"
        );
        assert_eq!(
            mode & 0o777,
            0o755,
            "the ordinary rwx bits should be preserved, got mode {mode:o}"
        );
    }

    // --- S4: optional max_download_size cap ---------------------------------------------------

    #[test]
    fn download_max_download_size_aborts_when_body_exceeds_cap() {
        // S4: a body larger than the configured cap aborts the streaming download with an error
        // naming the cap, instead of writing an unbounded amount to `dest`.
        let body = vec![0u8; 4096];
        let client = std::sync::Arc::new(DlClient {
            body: body.clone(),
            content_length: Some(body.len() as u64),
            requested: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        });

        let mut out = Vec::new();
        let mut dl = Download::from_url("https://nonroutable.invalid/big.bin");
        dl.set_http_client(
            Some(client),
            #[cfg(feature = "async")]
            None,
        );
        dl.max_download_size(1024);
        let res = dl.download_to(&mut out);
        assert!(res.is_err(), "a body over the cap must error");
        let msg = res.unwrap_err().to_string();
        assert!(
            msg.contains("max_download_size"),
            "the error should name the cap: {msg}"
        );
    }

    #[test]
    fn download_max_download_size_allows_body_under_cap() {
        // S4: with the default (no cap) unchanged, an explicit cap larger than the body still lets
        // the whole body download successfully.
        let body = vec![7u8; 512];
        let client = std::sync::Arc::new(DlClient {
            body: body.clone(),
            content_length: Some(body.len() as u64),
            requested: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        });

        let mut out = Vec::new();
        let mut dl = Download::from_url("https://nonroutable.invalid/small.bin");
        dl.set_http_client(
            Some(client),
            #[cfg(feature = "async")]
            None,
        );
        dl.max_download_size(1024);
        dl.download_to(&mut out).unwrap();
        assert_eq!(out, body, "a body under the cap downloads in full");
    }

    /// Async test-double response: yields the body as a single `bytes_stream` chunk and as `text`.
    #[cfg(feature = "async")]
    struct DlAsyncResponse {
        body: Vec<u8>,
        headers: http_client::header::HeaderMap,
    }

    #[cfg(feature = "async")]
    impl http_client::AsyncHttpResponse for DlAsyncResponse {
        fn headers(&self) -> &http_client::header::HeaderMap {
            &self.headers
        }
        fn text(self: Box<Self>) -> futures_util::future::BoxFuture<'static, Result<String>> {
            Box::pin(async move { Ok(String::from_utf8_lossy(&self.body).into_owned()) })
        }
        fn bytes_stream(
            self: Box<Self>,
        ) -> futures_util::stream::BoxStream<'static, Result<bytes::Bytes>> {
            Box::pin(futures_util::stream::once(async move {
                Ok(bytes::Bytes::from(self.body))
            }))
        }
    }

    /// Async test-double client (not reqwest) that records the URL and returns [`DlAsyncResponse`].
    #[cfg(feature = "async")]
    struct DlAsyncClient {
        body: Vec<u8>,
        content_length: Option<u64>,
        requested: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[cfg(feature = "async")]
    impl http_client::AsyncHttpClient for DlAsyncClient {
        fn get<'a>(
            &'a self,
            url: &'a str,
            _headers: &'a http_client::header::HeaderMap,
            _timeout: Option<std::time::Duration>,
        ) -> futures_util::future::BoxFuture<'a, Result<Box<dyn http_client::AsyncHttpResponse>>>
        {
            self.requested.lock().unwrap().push(url.to_string());
            let mut headers = http_client::header::HeaderMap::new();
            if let Some(len) = self.content_length {
                headers.insert(
                    http_client::header::CONTENT_LENGTH,
                    len.to_string().parse().unwrap(),
                );
            }
            let body = self.body.clone();
            Box::pin(async move {
                Ok(Box::new(DlAsyncResponse { body, headers })
                    as Box<dyn http_client::AsyncHttpResponse>)
            })
        }
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn download_to_async_uses_injected_async_client_through_the_trait() {
        // Gap #4 (async Download path): an injected `Arc<dyn AsyncHttpClient>` (not reqwest) must
        // drive `download_to_async` via `bytes_stream()`, independently of the sync injection path.
        let requested = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let body = b"async-injected-payload".to_vec();
        let client = std::sync::Arc::new(DlAsyncClient {
            body: body.clone(),
            content_length: Some(body.len() as u64),
            requested: requested.clone(),
        });

        let mut out = Vec::new();
        let mut dl = Download::from_url("https://nonroutable.invalid/asset.bin");
        dl.set_http_client(None, Some(client));
        dl.download_to_async(&mut out).await.unwrap();

        assert_eq!(
            out, body,
            "download_to_async streamed the injected client's body"
        );
        let urls = requested.lock().unwrap();
        assert_eq!(
            urls.len(),
            1,
            "exactly one async GET went through the injected client"
        );
        assert_eq!(urls[0], "https://nonroutable.invalid/asset.bin");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn sync_and_async_injection_are_independent() {
        // Setting only the async client must leave the sync client unset (and vice versa), proving
        // the two injection slots are independent: a `download_to_async` with only an async client
        // injected uses it, and does not fall back to / require the sync slot.
        let requested = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let body = b"only-async".to_vec();
        let async_client = std::sync::Arc::new(DlAsyncClient {
            body: body.clone(),
            content_length: Some(body.len() as u64),
            requested: requested.clone(),
        });

        let mut dl = Download::from_url("https://nonroutable.invalid/asset.bin");
        dl.set_http_client(None, Some(async_client));
        // The sync slot was never set.
        assert!(
            dl.client.is_none(),
            "injecting an async client must not populate the sync client slot"
        );
        assert!(dl.async_client.is_some(), "the async slot is populated");

        let mut out = Vec::new();
        dl.download_to_async(&mut out).await.unwrap();
        assert_eq!(out, body);
    }

    // Regression: `progress_callback` (the byte-level hook) must still fire even when the
    // `progress-bar` feature is disabled. The terminal `indicatif` bar and the callback are
    // orthogonal; disabling the former must not silence the latter.
    #[cfg(not(feature = "progress-bar"))]
    #[test]
    fn progress_callback_fires_without_progress_bar_feature() {
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let body = "y".repeat(8_000);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let served = body.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                served.len(),
                served
            );
            let _ = stream.write_all(resp.as_bytes());
        });

        let calls = Arc::new(Mutex::new(Vec::<(u64, Option<u64>)>::new()));
        let sink = calls.clone();
        let mut out = Vec::new();
        Download::from_url(format!("http://{addr}/file"))
            // `show_download_progress(true)` is intentionally set: with progress-bar OFF it
            // must be a no-op, while the callback below must still fire.
            .show_download_progress(true)
            .progress_callback(move |downloaded, total| {
                sink.lock().unwrap().push((downloaded, total));
            })
            .download_to(&mut out)
            .unwrap();

        assert_eq!(out.len(), 8_000);
        let calls = calls.lock().unwrap();
        assert!(
            !calls.is_empty(),
            "progress_callback must fire even with progress-bar feature disabled"
        );
        assert!(
            calls.iter().all(|(_, total)| *total == Some(8_000)),
            "total should reflect Content-Length"
        );
        assert_eq!(
            calls.last().unwrap().0,
            8_000,
            "final byte count should equal body length"
        );
    }

    #[cfg(feature = "compression-tar-gz")]
    #[test]
    fn detect_plain_gz() {
        assert_eq!(
            ArchiveKind::Plain(Some(Compression::Gz)),
            detect_archive(&PathBuf::from("Something.exe.gz")).unwrap()
        );
    }

    // Without the gzip feature, a plain `.gz` asset must be rejected with `CompressionNotEnabled`,
    // not silently detected as a decodable archive (which would install the compressed bytes).
    #[cfg(not(feature = "compression-tar-gz"))]
    #[test]
    fn detect_plain_gz_without_feature_errors() {
        assert!(matches!(
            detect_archive(&PathBuf::from("Something.exe.gz")),
            Err(Error::CompressionNotEnabled(_))
        ));
    }

    #[cfg(not(feature = "archive-tar"))]
    #[test]
    #[ignore]
    fn detect_tar_gz() {
        println!("WARNING: Please enable 'archive-tar' feature!");
    }
    #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
    #[test]
    fn detect_tar_gz() {
        assert_eq!(
            ArchiveKind::Tar(Some(Compression::Gz)),
            detect_archive(&PathBuf::from("Something.tar.gz")).unwrap()
        );
    }
    // `.tar.gz` with the tar container but no gzip codec must error, not fall through to an opaque
    // failure inside the tar reader.
    #[cfg(all(feature = "archive-tar", not(feature = "compression-tar-gz")))]
    #[test]
    fn detect_tar_gz_without_compression_errors() {
        assert!(matches!(
            detect_archive(&PathBuf::from("Something.tar.gz")),
            Err(Error::CompressionNotEnabled(_))
        ));
    }

    #[cfg(feature = "compression-tar-xz")]
    #[test]
    fn detect_plain_xz() {
        assert_eq!(
            ArchiveKind::Plain(Some(Compression::Xz)),
            detect_archive(&PathBuf::from("Something.exe.xz")).unwrap()
        );
    }

    // Without the xz feature, a plain `.xz` asset must be rejected with `CompressionNotEnabled`
    // rather than silently installed as still-compressed bytes (the original #143 footgun).
    #[cfg(not(feature = "compression-tar-xz"))]
    #[test]
    fn detect_plain_xz_without_feature_errors() {
        assert!(matches!(
            detect_archive(&PathBuf::from("Something.exe.xz")),
            Err(Error::CompressionNotEnabled(_))
        ));
    }

    #[cfg(all(feature = "archive-tar", feature = "compression-tar-xz"))]
    #[test]
    fn detect_tar_xz() {
        // Both the `.tar.xz` double extension and the `.txz` short form resolve to a gzip-free tar.
        assert_eq!(
            ArchiveKind::Tar(Some(Compression::Xz)),
            detect_archive(&PathBuf::from("Something.tar.xz")).unwrap()
        );
        assert_eq!(
            ArchiveKind::Tar(Some(Compression::Xz)),
            detect_archive(&PathBuf::from("Something.txz")).unwrap()
        );
    }

    // `.tar.xz` / `.txz` with the tar container but no xz codec must error, not fall through.
    #[cfg(all(feature = "archive-tar", not(feature = "compression-tar-xz")))]
    #[test]
    fn detect_tar_xz_without_compression_errors() {
        assert!(matches!(
            detect_archive(&PathBuf::from("Something.tar.xz")),
            Err(Error::CompressionNotEnabled(_))
        ));
        assert!(matches!(
            detect_archive(&PathBuf::from("Something.txz")),
            Err(Error::CompressionNotEnabled(_))
        ));
    }

    #[cfg(not(feature = "archive-tar"))]
    #[test]
    #[ignore]
    fn detect_plain_tar() {
        println!("WARNING: Please enable 'archive-tar' feature!");
    }
    #[cfg(feature = "archive-tar")]
    #[test]
    fn detect_plain_tar() {
        assert_eq!(
            ArchiveKind::Tar(None),
            detect_archive(&PathBuf::from("Something.tar")).unwrap()
        );
    }

    #[cfg(not(feature = "archive-zip"))]
    #[test]
    #[ignore]
    fn detect_zip() {
        println!("WARNING: Please enable 'archive-zip' feature!");
    }
    #[cfg(feature = "archive-zip")]
    #[test]
    fn detect_zip() {
        assert_eq!(
            ArchiveKind::Zip,
            detect_archive(&PathBuf::from("Something.zip")).unwrap()
        );
    }

    #[allow(dead_code)]
    fn cmp_content<T: AsRef<Path>>(path: T, s: &str) {
        let mut content = String::new();
        let mut f = File::open(&path).unwrap();
        f.read_to_string(&mut content).unwrap();
        assert!(s == content);
    }

    #[cfg(not(feature = "compression-tar-gz"))]
    #[test]
    #[ignore]
    fn unpack_plain_gzip() {
        println!("WARNING: Please enable 'compression-tar-gz' feature!");
    }
    #[cfg(feature = "compression-tar-gz")]
    #[test]
    fn unpack_plain_gzip() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_gzip_src")
            .tempdir()
            .expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_gzip_outdir")
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_into(out_path)
            .expect("extract fail");
        let out_file = out_path.join("temp");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[cfg(not(feature = "compression-tar-gz"))]
    #[test]
    #[ignore]
    fn unpack_plain_gzip_double_ext() {
        println!("WARNING: Please enable 'compression-tar-gz' feature!");
    }
    #[cfg(feature = "compression-tar-gz")]
    #[test]
    fn unpack_plain_gzip_double_ext() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_gzip_double_ext_src")
            .tempdir()
            .expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.txt.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_gzip_double_ext_outdir")
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_into(out_path)
            .expect("extract fail");
        let out_file = out_path.join("temp.txt");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[cfg(not(all(feature = "archive-tar", feature = "compression-tar-gz")))]
    #[test]
    #[ignore]
    fn unpack_tar_gzip() {
        println!("WARNING: Please enable 'archive-tar compression-tar-gz' features!");
    }
    #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
    #[test]
    fn unpack_tar_gzip() {
        test_extract_into(
            "self_update_unpack_tar_gzip_src",
            "archive.tar.gz",
            ArchiveKind::Tar(Some(Compression::Gz)),
        );
    }

    #[cfg(not(feature = "compression-tar-gz"))]
    #[test]
    #[ignore]
    fn unpack_file_plain_gzip() {
        println!("WARNING: Please enable 'compression-tar-gz' feature!");
    }
    #[cfg(feature = "compression-tar-gz")]
    #[test]
    fn unpack_file_plain_gzip() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_unpack_file_plain_gzip_src")
            .tempdir()
            .expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp = tempfile::Builder::new()
            .prefix("self_update_unpack_file_plain_gzip_outdir")
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_file(out_path, "renamed_file")
            .expect("extract fail");
        let out_file = out_path.join("renamed_file");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[cfg(not(all(feature = "archive-tar", feature = "compression-tar-gz")))]
    #[test]
    #[ignore]
    fn unpack_file_tar_gzip() {
        println!("WARNING: Please enable 'archive-tar compression-tar-gz' features!");
    }
    #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
    #[test]
    fn unpack_file_tar_gzip() {
        test_extract_file(
            "self_update_unpack_file_tar_gzip_src",
            "archive.tar.gz",
            ArchiveKind::Tar(Some(Compression::Gz)),
        );
    }

    // --- xz (#143) round-trips, mirroring the gzip coverage above -----------------------------

    // A plain single-file `.xz` decodes to the file with the `.xz` extension stripped.
    #[cfg(feature = "compression-tar-xz")]
    #[test]
    fn unpack_plain_xz() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_xz_src")
            .tempdir()
            .expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.xz");
        {
            let mut tmp_file = File::create(&fp).expect("temp file create fail");
            lzma_rs::xz_compress(&mut &b"This is a test!"[..], &mut tmp_file)
                .expect("xz encode fail");
        }

        let out_tmp = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_xz_outdir")
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_into(out_path)
            .expect("extract fail");
        let out_file = out_path.join("temp");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    // A plain `.xz` extracted via `extract_file` is written under the requested name.
    #[cfg(feature = "compression-tar-xz")]
    #[test]
    fn unpack_file_plain_xz() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_unpack_file_plain_xz_src")
            .tempdir()
            .expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.xz");
        {
            let mut tmp_file = File::create(&fp).expect("temp file create fail");
            lzma_rs::xz_compress(&mut &b"This is a test!"[..], &mut tmp_file)
                .expect("xz encode fail");
        }

        let out_tmp = tempfile::Builder::new()
            .prefix("self_update_unpack_file_plain_xz_outdir")
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_file(out_path, "renamed_file")
            .expect("extract fail");
        let out_file = out_path.join("renamed_file");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    // A `.tar.xz` unpacks its full tree, exercising the streamed tar-over-xz path end to end.
    #[cfg(all(feature = "archive-tar", feature = "compression-tar-xz"))]
    #[test]
    fn unpack_tar_xz() {
        test_extract_into(
            "self_update_unpack_tar_xz_src",
            "archive.tar.xz",
            ArchiveKind::Tar(Some(Compression::Xz)),
        );
    }

    // A single member of a `.tar.xz` is extractable by path.
    #[cfg(all(feature = "archive-tar", feature = "compression-tar-xz"))]
    #[test]
    fn unpack_file_tar_xz() {
        test_extract_file(
            "self_update_unpack_file_tar_xz_src",
            "archive.tar.xz",
            ArchiveKind::Tar(Some(Compression::Xz)),
        );
    }

    #[cfg(not(feature = "archive-zip"))]
    #[test]
    #[ignore]
    fn unpack_zip() {
        println!("WARNING: Please enable 'archive-zip' feature!");
    }
    #[cfg(feature = "archive-zip")]
    #[test]
    fn unpack_zip() {
        test_extract_into(
            "self_update_unpack_zip_src",
            "archive.zip",
            ArchiveKind::Zip,
        );
    }

    #[cfg(not(feature = "archive-zip"))]
    #[test]
    #[ignore]
    fn unpack_zip_file() {
        println!("WARNING: Please enable 'archive-zip' feature!");
    }
    #[cfg(feature = "archive-zip")]
    #[test]
    fn unpack_zip_file() {
        test_extract_file(
            "self_update_unpack_zip_src",
            "archive.zip",
            ArchiveKind::Zip,
        );
    }

    fn test_extract_into(tmpfile_prefix: &str, src_archive_path: &str, archive_kind: ArchiveKind) {
        let tmp_dir = tempfile::Builder::new()
            .prefix(tmpfile_prefix)
            .tempdir()
            .expect("Failed to create temp dir");

        let tmp_path = tmp_dir.path();
        let archive_file_path = tmp_path.join(src_archive_path);
        let archive_file = File::create(&archive_file_path).expect("Failed to create archive file");

        build_test_archive(archive_file, &archive_file_path, archive_kind);

        let out_tmp = tempfile::Builder::new()
            .prefix(&format!("{}_outdir", tmpfile_prefix))
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();

        Extract::from_source(&archive_file_path)
            .extract_into(out_path)
            .expect("extract fail");

        let out_file = out_path.join("temp.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a test!");

        let out_file = out_path.join("inner_archive/temp2.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a second test!");
    }

    fn test_extract_file(tmpfile_prefix: &str, src_archive_path: &str, archive_kind: ArchiveKind) {
        let tmp_dir = tempfile::Builder::new()
            .prefix(tmpfile_prefix)
            .tempdir()
            .expect("Failed to create temp dir");

        let tmp_path = tmp_dir.path();
        let archive_file_path = tmp_path.join(src_archive_path);
        let archive_file = File::create(&archive_file_path).expect("Failed to create archive file");

        build_test_archive(archive_file, &archive_file_path, archive_kind);

        let out_tmp = tempfile::Builder::new()
            .prefix(&format!("{}_outdir", tmpfile_prefix))
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();

        Extract::from_source(&archive_file_path)
            .extract_file(out_path, "temp.txt")
            .expect("extract fail");
        let out_file = out_path.join("temp.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a test!");

        Extract::from_source(&archive_file_path)
            .extract_file(out_path, "inner_archive/temp2.txt")
            .expect("extract fail");
        let out_file = out_path.join("inner_archive/temp2.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a second test!");
    }

    // A zip whose entry name escapes the output dir (`../escape.txt`) must be rejected, and nothing
    // may be written outside `into_dir`. Guards against zip-slip.
    #[cfg(feature = "archive-zip")]
    #[test]
    fn extract_into_rejects_zip_slip() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("evil.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("../escape.txt", options).expect("start");
            zip.write_all(b"pwned").expect("write");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let out_dir = out_tmp.path().join("into");
        fs::create_dir_all(&out_dir).expect("mkdir");

        let res = Extract::from_source(&archive_path).extract_into(&out_dir);
        assert!(res.is_err(), "a zip-slip entry must be rejected");
        assert!(
            !out_tmp.path().join("escape.txt").exists(),
            "nothing must be written outside the extraction dir"
        );
    }

    // A zip entry carrying an executable unix mode must extract with that mode preserved, so a
    // binary installed from a zip to a custom path stays runnable.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_preserves_zip_unix_mode() {
        use std::os::unix::fs::PermissionsExt;
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("app.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            zip.start_file("app", options).expect("start");
            zip.write_all(b"#!/bin/sh\n").expect("write");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        Extract::from_source(&archive_path)
            .extract_into(out_tmp.path())
            .expect("extract");
        let mode = fs::metadata(out_tmp.path().join("app"))
            .expect("stat")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "the executable bit must be preserved, got mode {:o}",
            mode
        );
    }

    // A zip entry that is a relative symlink (target inside the tree) must be restored as a real
    // symlink, not written out as a regular file containing the target string. The file it points
    // at must be readable through the link. Guards the `.app` framework-symlink regression.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_restores_relative_zip_symlink() {
        use std::os::unix::fs::FileTypeExt as _;
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("links.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // A real file, and a sibling symlink pointing at it by relative path.
            zip.start_file("dir/real.txt", options).expect("start");
            zip.write_all(b"payload").expect("write");
            zip.add_symlink("dir/link.txt", "real.txt", options)
                .expect("add_symlink");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        Extract::from_source(&archive_path)
            .extract_into(out_tmp.path())
            .expect("extract");

        let link_path = out_tmp.path().join("dir/link.txt");
        let meta = fs::symlink_metadata(&link_path).expect("lstat link");
        assert!(
            meta.file_type().is_symlink(),
            "the entry must be restored as a symlink, not a regular file"
        );
        // Sanity: it must not be some other special file type either.
        assert!(!meta.file_type().is_fifo());
        // The link target must be the stored relative path, and reading through it yields the file.
        let target = fs::read_link(&link_path).expect("readlink");
        assert_eq!(target, Path::new("real.txt"));
        let via_link = fs::read_to_string(&link_path).expect("read through link");
        assert_eq!(via_link, "payload");
    }

    // A zip symlink whose target is absolute must be rejected (it would escape the extraction root),
    // and no symlink or file may be left at the entry path.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_rejects_absolute_zip_symlink() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("abs.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.add_symlink("evil", "/etc/passwd", options)
                .expect("add_symlink");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let res = Extract::from_source(&archive_path).extract_into(out_tmp.path());
        assert!(res.is_err(), "an absolute-target symlink must be rejected");
        assert!(
            fs::symlink_metadata(out_tmp.path().join("evil")).is_err(),
            "no link/file may be left behind for a rejected symlink entry"
        );
    }

    // A zip symlink whose relative target climbs above the extraction root with `..` must be
    // rejected, matching the entry-name zip-slip defense.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_rejects_escaping_zip_symlink() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("escape.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // `add_symlink` preserves the raw target string (unlike `add_symlink_from_path`, which
            // would normalize the `..` away), so the escaping target reaches the extractor intact.
            zip.add_symlink("dir/escape", "../../outside", options)
                .expect("add_symlink");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let res = Extract::from_source(&archive_path).extract_into(out_tmp.path());
        assert!(
            res.is_err(),
            "a `..`-escaping symlink target must be rejected"
        );
        assert!(
            fs::symlink_metadata(out_tmp.path().join("dir/escape")).is_err(),
            "no link/file may be left behind for a rejected symlink entry"
        );
    }

    // A `..` target that stays within the root after resolving against the link's parent dir must
    // be allowed (the lexical check must not over-reject legitimate relative links).
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_allows_in_bounds_dotdot_zip_symlink() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("inbounds.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("top.txt", options).expect("start");
            zip.write_all(b"top-payload").expect("write");
            // Link at `dir/up`; target `../top.txt` resolves to `top.txt` inside the root.
            zip.add_symlink("dir/up", "../top.txt", options)
                .expect("add_symlink");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        Extract::from_source(&archive_path)
            .extract_into(out_tmp.path())
            .expect("extract");
        let link_path = out_tmp.path().join("dir/up");
        let meta = fs::symlink_metadata(&link_path).expect("lstat link");
        assert!(meta.file_type().is_symlink(), "must be a symlink");
        let via_link = fs::read_to_string(&link_path).expect("read through link");
        assert_eq!(via_link, "top-payload");
    }

    // extract_file must not write a symlink entry's target string out as the requested file; it
    // errors instead (documented behavior; use extract_into to restore links).
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_file_rejects_zip_symlink_entry() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("single.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.add_symlink("link", "target.txt", options)
                .expect("add_symlink");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let res = Extract::from_source(&archive_path).extract_file(out_tmp.path(), "link");
        assert!(
            res.is_err(),
            "extracting a symlink entry as a file must error"
        );
        assert!(
            fs::symlink_metadata(out_tmp.path().join("link")).is_err(),
            "no file may be written for a rejected symlink entry"
        );
    }

    // --- symlink_target_escapes: lexical edge cases (private fn, adversarial) ---------------
    // These exercise the pure lexical resolver directly so the boundary logic is pinned
    // independently of zip fixture plumbing.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn symlink_target_escapes_lexical_edges() {
        use std::path::Path;
        let esc = symlink_target_escapes;

        // A bare `..` from a top-level link (parent depth 0) climbs above the root -> escapes.
        assert!(
            esc(Path::new(""), Path::new("..")),
            "top-level `..` escapes"
        );
        // `..` from a depth-1 parent resolves exactly to the root -> allowed (boundary, in-bounds).
        assert!(
            !esc(Path::new("dir"), Path::new("..")),
            "`..` back to root is in-bounds"
        );
        // A deep link climbing exactly back to the root then descending stays in-bounds.
        assert!(
            !esc(Path::new("a/b"), Path::new("../../x")),
            "climb exactly to root then descend is in-bounds"
        );
        // One `..` past the root escapes.
        assert!(
            esc(Path::new("a/b"), Path::new("../../..")),
            "climbing one past root escapes"
        );
        // Mixed `./` current-dir components must not be miscounted as depth.
        assert!(
            !esc(Path::new("dir"), Path::new("./real.txt")),
            "`./` component is a no-op, stays in-bounds"
        );
        assert!(
            !esc(Path::new("dir"), Path::new("./../a")),
            "`./` then `..` from depth 1 stays in-bounds"
        );
        // An empty target string has no components -> resolves to the link's own parent, in-bounds.
        assert!(
            !esc(Path::new("dir"), Path::new("")),
            "empty target is in-bounds"
        );
        // Interior `..` that dips and re-descends but never passes the root is in-bounds.
        assert!(
            !esc(Path::new(""), Path::new("a/../b")),
            "dip and re-descend within root is in-bounds"
        );
        // Interior `..` that transiently passes the root escapes even if it would re-descend.
        assert!(
            esc(Path::new(""), Path::new("a/../../b")),
            "transiently passing root escapes"
        );
        // An absolute target always escapes regardless of parent depth.
        assert!(
            esc(Path::new("a/b/c"), Path::new("/etc/passwd")),
            "absolute target escapes"
        );
        assert!(
            esc(Path::new(""), Path::new("/")),
            "bare root target escapes"
        );
    }

    // A symlink chain fully inside the archive (link A -> link B -> real file) must be restored as
    // two real links, readable through the chain. The checks are lexical and per-entry, so an
    // in-tree chain must extract cleanly regardless of order.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_restores_symlink_chain() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("chain.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("real.txt", options).expect("start");
            zip.write_all(b"chain-payload").expect("write");
            // b -> real.txt, a -> b (all siblings at root).
            zip.add_symlink("b", "real.txt", options).expect("b");
            zip.add_symlink("a", "b", options).expect("a");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        Extract::from_source(&archive_path)
            .extract_into(out_tmp.path())
            .expect("extract");
        let a = out_tmp.path().join("a");
        assert!(
            fs::symlink_metadata(&a)
                .expect("lstat a")
                .file_type()
                .is_symlink(),
            "a must be a symlink"
        );
        assert!(
            fs::symlink_metadata(out_tmp.path().join("b"))
                .expect("lstat b")
                .file_type()
                .is_symlink(),
            "b must be a symlink"
        );
        assert_eq!(
            fs::read_to_string(&a).expect("read through chain"),
            "chain-payload"
        );
    }

    // A link entry whose parent directory has no explicit archive entry (and thus no entry ordered
    // before it) must still extract: the loop's `create_dir_all(parent)` must materialize the path.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_symlink_with_implicit_parent_dir() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("implicit.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("target.txt", options).expect("start");
            zip.write_all(b"impl-payload").expect("write");
            // No `nested/` directory entry precedes this link.
            zip.add_symlink("nested/deep/link", "../../target.txt", options)
                .expect("link");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        Extract::from_source(&archive_path)
            .extract_into(out_tmp.path())
            .expect("extract");
        let link = out_tmp.path().join("nested/deep/link");
        assert!(
            fs::symlink_metadata(&link)
                .expect("lstat")
                .file_type()
                .is_symlink(),
            "link must be created under implicitly-created parents"
        );
        assert_eq!(fs::read_to_string(&link).expect("read"), "impl-payload");
    }

    // NOTE on duplicate-path entries: the handoff asked to probe two entries at the same path
    // (regular file then symlink, and symlink then regular file, the latter a possible
    // write-through-link vuln). `zip::ZipWriter` (8.6.0) rejects a second entry at an existing
    // name with `InvalidArchive("Duplicate filename")`, so a duplicate-path fixture cannot be
    // built through the crate's own writer; that code path (the `remove_file` before `symlink`,
    // and `File::create` over a pre-existing link) is only reachable via a hand-forged archive.
    // It is left uncovered here rather than hand-assembling raw zip bytes -- see certification.
    // The stronger, writer-buildable escape is the symlinked-parent bypass below, which needs no
    // duplicate paths.

    // SECURITY: the per-entry lexical `symlink_target_escapes` check alone can be bypassed by
    // first creating a symlinked intermediate directory that aliases to a shallower path. The
    // lexical depth counted for a later link over-estimates the real filesystem depth, so an
    // escaping target would pass the lexical check. The physical-parent verification (canonicalize
    // the entry's parent and require it to equal `canonical_root/<lexical parent>`) is the backstop
    // that rejects any descent through a symlinked ancestor. This pins the rejection.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_rejects_symlink_through_symlinked_parent() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("bypass.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // 1) `d/sl` -> `..`  (link_parent depth 1, `..` -> depth 0, lexically allowed).
            //    Physically aliases `d/sl` to the root.
            zip.add_symlink("d/sl", "..", options).expect("sl");
            // 2) `d/sl/evil` -> `../../x`. Lexically in-bounds, but `d/sl` aliases the root so the
            //    link would land at <root>/evil pointing ABOVE the root. The physical-parent check
            //    rejects it: canonicalize(<root>/d/sl) == <root>, but the expected parent is
            //    <root>/d/sl, so they differ.
            zip.add_symlink("d/sl/evil", "../../x", options)
                .expect("evil");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let res = Extract::from_source(&archive_path).extract_into(out_tmp.path());
        assert!(
            res.is_err(),
            "a symlink descending through a symlinked parent must be rejected"
        );
        // No escaping link may be left behind, at the aliased root location or under `d/sl`.
        assert!(
            fs::symlink_metadata(out_tmp.path().join("evil")).is_err(),
            "no escaping link may be planted at the aliased root path"
        );
        assert!(
            fs::symlink_metadata(out_tmp.path().join("d/sl/evil")).is_err(),
            "no escaping link may be planted under the symlinked parent"
        );
    }

    // A regular-file entry that descends through a symlinked parent must also be rejected: even
    // though the write would land inside the root through the alias (an in-bounds target is all a
    // link can hold), the physical-parent check rejects the descent so nothing is written through
    // an aliased directory.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_rejects_regular_file_through_symlinked_parent() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("filebypass.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // `d/sl` -> `..` aliases to the root; then a regular file under it.
            zip.add_symlink("d/sl", "..", options).expect("sl");
            zip.start_file("d/sl/file.txt", options).expect("start");
            zip.write_all(b"through-alias").expect("write");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let res = Extract::from_source(&archive_path).extract_into(out_tmp.path());
        assert!(
            res.is_err(),
            "a regular file descending through a symlinked parent must be rejected"
        );
        assert!(
            fs::symlink_metadata(out_tmp.path().join("file.txt")).is_err(),
            "no file may be written at the aliased root path"
        );
    }

    // Positive control: a benign tree whose files legitimately descend through REAL directories
    // (a normal nested layout, plus a symlink to a real in-tree directory used only as a leaf link,
    // never descended through) still extracts fine. The physical-parent check must not over-reject.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_allows_files_through_real_directories() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("benign.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("a/b/c/deep.txt", options).expect("start");
            zip.write_all(b"deep-payload").expect("write");
            zip.start_file("a/b/sibling.txt", options).expect("start");
            zip.write_all(b"sibling-payload").expect("write");
            // A symlink to a real in-tree directory (leaf link, not descended through).
            zip.add_symlink("a/link-to-b", "b", options).expect("link");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        Extract::from_source(&archive_path)
            .extract_into(out_tmp.path())
            .expect("extract");
        assert_eq!(
            fs::read_to_string(out_tmp.path().join("a/b/c/deep.txt")).expect("read deep"),
            "deep-payload"
        );
        assert_eq!(
            fs::read_to_string(out_tmp.path().join("a/b/sibling.txt")).expect("read sibling"),
            "sibling-payload"
        );
        // The leaf symlink resolves to the real directory and reads the same content through it.
        assert_eq!(
            fs::read_to_string(out_tmp.path().join("a/link-to-b/sibling.txt"))
                .expect("read through link"),
            "sibling-payload"
        );
    }

    // ADVERSARIAL (deeper chain): a symlinked ancestor aliases the root, then a file entry
    // descends TWO real levels below the alias (`a/sl -> ..`, then `a/sl/b/c/deep.txt`). The
    // lexical depth of the entry (a/sl/b/c) over-counts the physical depth (root/b/c), so the
    // per-entry lexical check would pass; the physical-parent equality check must still reject the
    // descent through the symlinked ancestor. Pins that the guard holds across multi-level descents,
    // not just a single level below the symlink.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_rejects_file_deep_below_symlinked_ancestor() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("deepchain.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // `a/sl` -> `..` aliases `a/sl` to the extraction root.
            zip.add_symlink("a/sl", "..", options).expect("sl");
            // Two levels below the alias. Lexically in-bounds (no `..`), physically root/b/c.
            zip.start_file("a/sl/b/c/deep.txt", options).expect("start");
            zip.write_all(b"deep-through-alias").expect("write");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let res = Extract::from_source(&archive_path).extract_into(out_tmp.path());
        assert!(
            res.is_err(),
            "a file two levels below a symlinked ancestor must be rejected"
        );
        // Nothing may be planted at the aliased root location (root/b/c/deep.txt) either.
        assert!(
            fs::symlink_metadata(out_tmp.path().join("b/c/deep.txt")).is_err(),
            "no file may be written at the aliased (shallower) root path"
        );
        assert!(
            fs::symlink_metadata(out_tmp.path().join("a/sl/b/c/deep.txt")).is_err(),
            "no file may be written below the symlinked ancestor"
        );
    }

    // ADVERSARIAL (symlinked destination): the caller's own `into_dir` may legitimately contain a
    // symlink component (e.g. `/tmp/link-to-real`). `canonical_root` captures its resolved
    // (symlink-free) form up front, so a benign nested archive must extract without being falsely
    // rejected: every entry's physical parent resolves to `canonical_root/<lexical parent>` by
    // construction. Guards against the equality check tripping on a symlink the CALLER supplied
    // rather than one an archive entry created.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_allows_symlinked_destination() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("benign-nested.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("sub/deep/file.txt", options).expect("start");
            zip.write_all(b"nested-payload").expect("write");
            zip.finish().expect("finish");
        }
        // The destination handed to `extract_into` is itself a symlink to the real output dir.
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let real_dest = out_tmp.path().join("real-dest");
        fs::create_dir(&real_dest).expect("mkdir real dest");
        let link_dest = out_tmp.path().join("link-to-dest");
        std::os::unix::fs::symlink(&real_dest, &link_dest).expect("symlink dest");

        Extract::from_source(&archive_path)
            .extract_into(&link_dest)
            .expect("extraction into a symlinked destination must not be rejected");
        // The file lands under the real destination, reachable through the caller's symlink.
        assert_eq!(
            fs::read_to_string(real_dest.join("sub/deep/file.txt")).expect("read real"),
            "nested-payload"
        );
        assert_eq!(
            fs::read_to_string(link_dest.join("sub/deep/file.txt")).expect("read via link"),
            "nested-payload"
        );
    }

    // ADVERSARIAL (unguarded dir branch): the `is_dir()` branch runs `create_dir_all` WITHOUT the
    // physical-parent check, on the reasoning that a symlinked ancestor only aliases in-bounds (its
    // target was validated by `symlink_target_escapes`), so directories created through it stay in
    // bounds, and any later FILE entry under it is rejected by the parent check. This probes that
    // reasoning with a symlink aliasing a real in-tree sibling directory: the dir entry created
    // through the alias must land in-bounds, nothing may be created outside the root, and a file
    // entry descending through the same alias must be rejected even though it too would land
    // in-bounds (the equality check is strict, not a mere prefix/containment check).
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_into_dir_through_symlink_stays_in_bounds_and_file_rejected() {
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("dirthrough.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // Real in-tree directory `a/b` (materialized by a file entry).
            zip.start_file("a/b/keep.txt", options).expect("start keep");
            zip.write_all(b"keep").expect("write keep");
            // `a/sl` -> `b`: a symlink aliasing a real sibling directory (in-bounds target).
            zip.add_symlink("a/sl", "b", options).expect("sl");
            // Directory entry descending through the alias. The unguarded dir branch creates it.
            zip.add_directory("a/sl/planted", options)
                .expect("dir entry");
            // A file entry under the alias must be rejected by the physical-parent equality check
            // even though it would land in-bounds (root/a/b/planted/f.txt).
            zip.start_file("a/sl/planted/f.txt", options)
                .expect("start f");
            zip.write_all(b"through-alias-file").expect("write f");
            zip.finish().expect("finish");
        }
        // Extract into a nested dest so we can assert nothing escapes into the parent.
        let out_tmp = tempfile::tempdir().expect("tempdir");
        let dest = out_tmp.path().join("dest");
        let res = Extract::from_source(&archive_path).extract_into(&dest);
        assert!(
            res.is_err(),
            "a file descending through a symlinked directory must be rejected"
        );
        // The file must not exist at the aliased location, the lexical location, or anywhere.
        assert!(
            fs::symlink_metadata(dest.join("a/b/planted/f.txt")).is_err(),
            "no file may be written at the aliased in-bounds path"
        );
        assert!(
            fs::symlink_metadata(dest.join("a/sl/planted/f.txt")).is_err(),
            "no file may be written at the lexical path under the symlink"
        );
        // The dir branch is unguarded, so `planted` was materialized through the alias -- but it
        // must be IN-BOUNDS (root/a/b/planted), never outside the extraction root.
        if let Ok(meta) = fs::symlink_metadata(dest.join("a/b/planted")) {
            assert!(
                meta.file_type().is_dir(),
                "planted, if present, is a real dir"
            );
        }
        // Nothing may have been created outside `dest`: the extraction parent holds only `dest`.
        let mut stray: Vec<String> = fs::read_dir(out_tmp.path())
            .expect("read parent")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        stray.retain(|name| name != "dest");
        assert!(
            stray.is_empty(),
            "nothing may be created outside the extraction root, found: {:?}",
            stray
        );
    }

    // extract_file on a normal executable-mode regular zip entry is unaffected by the symlink
    // rejection: it extracts and preserves the exec bit.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_file_regular_exec_entry_unaffected() {
        use std::os::unix::fs::PermissionsExt as _;
        let staging = tempfile::tempdir().expect("tempdir");
        let archive_path = staging.path().join("exec.zip");
        {
            let f = File::create(&archive_path).expect("create zip");
            let mut zip = zip::ZipWriter::new(f);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            zip.start_file("bin", options).expect("start");
            zip.write_all(b"#!/bin/sh\n").expect("write");
            zip.finish().expect("finish");
        }
        let out_tmp = tempfile::tempdir().expect("tempdir");
        Extract::from_source(&archive_path)
            .extract_file(out_tmp.path(), "bin")
            .expect("extract_file");
        let out = out_tmp.path().join("bin");
        let mode = fs::metadata(&out).expect("stat").permissions().mode();
        assert!(mode & 0o111 != 0, "exec bit preserved, got {:o}", mode);
        assert_eq!(fs::read_to_string(&out).expect("read"), "#!/bin/sh\n");
    }

    fn build_test_archive<T: AsRef<Path>>(
        mut archive_file: fs::File,
        archive_file_path: T,
        archive_kind: ArchiveKind,
    ) {
        let archive_file_path = archive_file_path.as_ref();

        match archive_kind {
            #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
            ArchiveKind::Tar(Some(Compression::Gz)) => {
                let tmp_tar_path = archive_file_path
                    .parent()
                    .expect("Missing archive file path parent")
                    .join("tar_contents");
                let tmp_tar_inner_path = tmp_tar_path.join("inner_archive");
                fs::create_dir_all(&tmp_tar_inner_path).expect("Failed to create temp tar path");

                let fp = tmp_tar_path.join("temp.txt");
                let mut tmp_file = File::create(fp).expect("temp file create fail");
                tmp_file.write_all(b"This is a test!").unwrap();

                let fp = tmp_tar_inner_path.join("temp2.txt");
                let mut tmp_file = File::create(fp).expect("temp file create fail");
                tmp_file.write_all(b"This is a second test!").unwrap();

                let mut ar = tar::Builder::new(vec![]);
                ar.append_dir_all(".", &tmp_tar_path)
                    .expect("tar append dir all fail");
                let tar_writer = ar.into_inner().expect("failed getting tar writer");

                let mut e = GzEncoder::new(&mut archive_file, flate2::Compression::default());
                io::copy(&mut tar_writer.as_slice(), &mut e)
                    .expect("failed writing from tar archive to gz encoder");
                e.finish().expect("gz finish fail");
            }

            #[cfg(all(feature = "archive-tar", feature = "compression-tar-xz"))]
            ArchiveKind::Tar(Some(Compression::Xz)) => {
                let tmp_tar_path = archive_file_path
                    .parent()
                    .expect("Missing archive file path parent")
                    .join("tar_contents_xz");
                let tmp_tar_inner_path = tmp_tar_path.join("inner_archive");
                fs::create_dir_all(&tmp_tar_inner_path).expect("Failed to create temp tar path");

                let fp = tmp_tar_path.join("temp.txt");
                let mut tmp_file = File::create(fp).expect("temp file create fail");
                tmp_file.write_all(b"This is a test!").unwrap();

                let fp = tmp_tar_inner_path.join("temp2.txt");
                let mut tmp_file = File::create(fp).expect("temp file create fail");
                tmp_file.write_all(b"This is a second test!").unwrap();

                let mut ar = tar::Builder::new(vec![]);
                ar.append_dir_all(".", &tmp_tar_path)
                    .expect("tar append dir all fail");
                let tar_writer = ar.into_inner().expect("failed getting tar writer");

                lzma_rs::xz_compress(&mut tar_writer.as_slice(), &mut archive_file)
                    .expect("failed writing from tar archive to xz encoder");
            }

            #[cfg(feature = "archive-zip")]
            ArchiveKind::Zip => {
                let mut zip = zip::ZipWriter::new(archive_file);
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                zip.start_file("temp.txt", options)
                    .expect("failed starting zip file");
                zip.write_all(b"This is a test!")
                    .expect("failed writing to zip");
                zip.start_file("inner_archive/temp2.txt", options)
                    .expect("failed starting second zip file");
                zip.write_all(b"This is a second test!")
                    .expect("failed writing to second zip");
                zip.finish().expect("failed finishing zip");
            }

            _ => {
                unimplemented!("{:?} not handled", archive_kind);
            }
        }
    }

    // --- extractor `Internal { source: None }` variant-routing -----------------------------
    //
    // These pin the invariant-violation sites in `extract_file`/`extract_into` to EXACTLY
    // `Error::Internal` carrying NO source (the genuine-invariant residue, distinct from the
    // JoinError `Internal { source: Some(..) }`).

    // `extract_file` on a Plain source where the *requested* `file_to_extract` has no file name
    // (e.g. `..`) must route to `Error::Internal { source: None }` ("Extractor source has no
    // file-name"), not an Io error. `file_to_extract` is caller-supplied and need not exist, so
    // this is reachable without a real hostless-path file. (~lib.rs:852)
    #[test]
    fn extract_file_plain_no_file_name_routes_to_internal_without_source() {
        use std::error::Error as _;
        let src_dir = tempfile::tempdir().expect("tempdir");
        let src = src_dir.path().join("payload.bin");
        fs::write(&src, b"hello").expect("write source");

        let out_dir = tempfile::tempdir().expect("out tempdir");

        // `..` has no `file_name()`, firing the invariant branch.
        let err = Extract::from_source(&src)
            .archive(ArchiveKind::Plain(None))
            .extract_file(out_dir.path(), "..")
            .expect_err("a file_to_extract with no file name must error");
        match err {
            Error::Internal {
                ref message,
                ref source,
            } => {
                assert!(
                    source.is_none(),
                    "the no-file-name invariant carries no source, got {:?}",
                    source
                );
                assert!(
                    message.contains("file-name"),
                    "message must describe the missing file name, got: {}",
                    message
                );
            }
            other => panic!("expected Error::Internal, got {:?}", other),
        }
        // Defensive: confirm the variant truly chains no source via the trait too.
        let err = Extract::from_source(&src)
            .archive(ArchiveKind::Plain(None))
            .extract_file(out_dir.path(), "..")
            .unwrap_err();
        assert!(err.source().is_none());
    }

    // `extract_file` on a Tar source where the requested path is not present in the archive must
    // route to `Error::Internal { source: None }` ("Could not find the required path in the
    // archive"), naming the missing path. (~lib.rs:873)
    #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
    #[test]
    fn extract_file_tar_missing_path_routes_to_internal_without_source() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_ws3_tar_missing_src")
            .tempdir()
            .expect("tempdir");
        let archive_file_path = tmp_dir.path().join("archive.tar.gz");
        let archive_file = File::create(&archive_file_path).expect("create archive");
        build_test_archive(
            archive_file,
            &archive_file_path,
            ArchiveKind::Tar(Some(Compression::Gz)),
        );

        let out_tmp = tempfile::tempdir().expect("out tempdir");
        let err = Extract::from_source(&archive_file_path)
            .extract_file(out_tmp.path(), "does/not/exist.txt")
            .expect_err("a path absent from the tar must error");
        match err {
            Error::Internal {
                ref message,
                ref source,
            } => {
                assert!(
                    source.is_none(),
                    "the path-not-found invariant carries no source, got {:?}",
                    source
                );
                assert!(
                    message.contains("Could not find the required path"),
                    "message must describe the missing archive path, got: {}",
                    message
                );
            }
            other => panic!("expected Error::Internal, got {:?}", other),
        }
    }

    // `extract_file` on a Zip source where the requested path is not valid UTF-8 must route to
    // `Error::Internal { source: None }` ("cannot extract file with a non-UTF-8 path"). Reachable
    // on Unix by building an `OsStr` from raw non-UTF-8 bytes. (~lib.rs:903)
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn extract_file_zip_non_utf8_path_routes_to_internal_without_source() {
        use std::os::unix::ffi::OsStrExt;

        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_ws3_zip_nonutf8_src")
            .tempdir()
            .expect("tempdir");
        let archive_file_path = tmp_dir.path().join("archive.zip");
        let archive_file = File::create(&archive_file_path).expect("create archive");
        build_test_archive(archive_file, &archive_file_path, ArchiveKind::Zip);

        let out_tmp = tempfile::tempdir().expect("out tempdir");
        // 0xFF is never valid UTF-8.
        let bad = std::ffi::OsStr::from_bytes(b"bad\xFFname");
        let err = Extract::from_source(&archive_file_path)
            .extract_file(out_tmp.path(), bad)
            .expect_err("a non-UTF-8 zip path must error");
        match err {
            Error::Internal {
                ref message,
                ref source,
            } => {
                assert!(
                    source.is_none(),
                    "the non-UTF-8-path invariant carries no source, got {:?}",
                    source
                );
                assert!(
                    message.contains("non-UTF-8 path"),
                    "message must describe the non-UTF-8 path, got: {}",
                    message
                );
            }
            other => panic!("expected Error::Internal, got {:?}", other),
        }
    }
}
