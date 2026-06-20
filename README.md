# self_update


[![Build status](https://ci.appveyor.com/api/projects/status/xlkq8rd73cla4ixw/branch/master?svg=true)](https://ci.appveyor.com/project/jaemk/self-update/branch/master)
[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

Supported backends: **GitHub**, **GitLab**, **Gitea**, and **S3** (Amazon S3, Google GCS,
DigitalOcean Spaces, or any S3-compatible endpoint). Each exposes the same `Update`
(configure → build → update) and `ReleaseList` builder API.

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
> settings only — the defaults are unchanged. Note the status block is printed *before* the
> confirmation prompt, so suppressing one does not suppress the other.

## Usage

Update (replace) the current executable with the latest release downloaded
from `https://api.github.com/repos/jaemk/self_update/releases/latest`.
Note, the [`trust`](https://github.com/japaric/trust) project provides a nice setup for
producing release-builds via CI (travis/appveyor).

### Features

Exactly **one** HTTP client and **one** TLS backend must be selected (they are mutually
exclusive — enabling both, or neither, is a compile error):

* `reqwest` (default): use the [`reqwest`](https://docs.rs/reqwest) HTTP client;
* `ureq`: use the [`ureq`](https://docs.rs/ureq) HTTP client instead (set `default-features = false`);
* `default-tls` (default): native TLS for the selected client;
* `rustls`: use a [pure rust TLS implementation](https://github.com/rustls/rustls) instead. This feature does _not_ support 32bit macOS.

The following optional [cargo features](https://doc.rust-lang.org/cargo/reference/manifest.html#the-features-section)
are _disabled_ by default; activate the one(s) your release files need:

* `archive-tar`: Support for _tar_ archive format;
* `archive-zip`: Support for _zip_ archive format;
* `compression-flate2`: Support for _gzip_ compression;
* `compression-zip-deflate`: Support for _zip_'s _deflate_ compression format;
* `compression-zip-bzip2`: Support for _zip_'s _bzip2_ compression format;
* `signatures`: Use [zipsign](https://github.com/Kijewski/zipsign) to verify `.zip` and `.tar.gz` artifacts. Artifacts are assumed to have been signed using zipsign;
* `checksums`: Verify a downloaded artifact against a known SHA-256/SHA-512 checksum (e.g. from a `SHA256SUMS` file) before installing it;
* `s3-auth`: Sign S3 requests (AWS SigV4) to update from private buckets via the S3 backend;
* `async`: Add async (`*_async`) update methods alongside the unchanged blocking API. tokio-only and reqwest-only (incompatible with `ureq`); see [Async](#async) below.

The S3 backend needs **no feature** — it is always compiled. (A no-op `s3` alias feature exists only so `features = ["s3"]` resolves for symmetry with the other backends; only private-bucket request signing needs an actual feature, `s3-auth`.)

### Example

Run the following example to see `self_update` in action:

`cargo run --example github --features "archive-tar archive-zip compression-flate2 compression-zip-deflate"`.

There are equivalent examples for the other backends (`gitlab`, `gitea`, `s3`), e.g.:

`cargo run --example gitlab --features "archive-tar archive-zip compression-flate2 compression-zip-deflate"`.

which runs something roughly equivalent to:

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

Amazon S3, Google GCS, and DigitalOcean Spaces, as well as any S3 compatible server are also supported
through the `S3` backend to check for new releases.  Provided a `bucket_name`
and `asset_prefix` string, `self_update` will look up all matching files using the following format
as a convention for the filenames: `[directory/]<asset name>-<semver>-<platform/target>.<extension>`.
Leading directories will be stripped from the file name allowing the use of subdirectories in the S3 bucket,
and any file not matching the format, or not matching the provided prefix string, will be ignored.

```rust
use self_update::cargo_crate_version;

fn update() -> Result<(), Box<dyn ::std::error::Error>> {
    let status = self_update::backends::s3::Update::configure()
        // .end_point(self_update::backends::s3::EndPoint::GCS)
        // .end_point("https://s3.example.com")
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
```

Separate utilities are also exposed (**NOTE**: the following example extracts a `.tar.gz`, which
_requires_ both the `archive-tar` and `compression-flate2` features -- `archive-tar` reads the tar
archive and `compression-flate2` decodes the gzip layer; see the [features](#features) section
above). It downloads, extracts, and replaces the running binary
by hand; the staging directory and the in-place replacement use the [`tempfile`](https://crates.io/crates/tempfile)
and [`self_replace`](https://crates.io/crates/self-replace) crates, which you add as your own dependencies
(they are no longer re-exported from `self_update`):

```rust
fn update() -> Result<(), Box<dyn std::error::Error>> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    // get the first available release
    let asset = releases[0]
        .asset_for(&self_update::get_target(), None)
        .unwrap();

    let tmp_dir = tempfile::Builder::new()
            .prefix("self_update")
            .tempdir_in(::std::env::current_dir()?)?;
    let tmp_tarball_path = tmp_dir.path().join(&asset.name);
    let tmp_tarball = ::std::fs::File::create(&tmp_tarball_path)?;

    self_update::Download::from_url(&asset.download_url)
        .header(self_update::http::header::ACCEPT, "application/octet-stream")?
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
`compression-flate2` features.

```rust
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

With the `checksums` feature, pass a known digest (e.g. one published in a `SHA256SUMS` file
alongside the release) and the crate verifies the downloaded artifact against it **before**
installing — a mismatch aborts the update. The algorithm is chosen by the
`Checksum` variant (`Sha256` / `Sha512`); it complements the `signatures`
feature (zipsign), which verifies authenticity rather than a published digest.

```rust
fn update() -> Result<(), Box<dyn std::error::Error>> {
    self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        // hex digest, obtained out of band (e.g. parsed from the release's SHA256SUMS)
        .checksum(self_update::Checksum::Sha256("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".into()))
        .build()?
        .update()?;
    Ok(())
}
```

### Listing releases (`ReleaseList`)

Each built-in backend exposes a `ReleaseList` builder for fetching the list of available releases
without performing an update. There is **no single unifying `self_update::ReleaseList` type**:
every backend has its own, distinct `ReleaseList` (the fields and request shape differ per host),
so they are reached through their backend modules rather than re-exported at the crate root:

* [`backends::github::ReleaseList`](backends::github::ReleaseList)
* [`backends::gitlab::ReleaseList`](backends::gitlab::ReleaseList)
* [`backends::gitea::ReleaseList`](backends::gitea::ReleaseList)
* [`backends::s3::ReleaseList`](backends::s3::ReleaseList)

The custom backend has no `ReleaseList` by design: listing is performed entirely by your
[`ReleaseSource`] (or [`AsyncReleaseSource`]) implementation, which already returns
[`Release`] values directly.

### Custom backends

To update from a host the built-in backends (`github`, `gitlab`, `gitea`, `s3`) don't cover —
another forge, a private artifact registry, a plain HTTP directory — implement the
`ReleaseSource` trait (three fetch methods that say *where releases come from*) and drive a full
update through the `backends::custom` backend, which reuses the crate's compare → select-asset →
download → verify → extract → install flow. You build `Release`s with `Release::builder` and
`ReleaseAsset::new`; the `ReleaseUpdate` trait stays sealed.

`ReleaseSource` is **synchronous**. For a natively-async source, implement `AsyncReleaseSource`
(the same three fetches as `async fn`) and drive it through
`backends::custom::AsyncUpdate` + `build_async()`; to reuse a
`Clone` sync source from the async API, wrap it in
`backends::custom::Blocking`.

```rust
use self_update::{Release, ReleaseAsset, ReleaseSource, cargo_crate_version};

struct MyHost;
impl ReleaseSource for MyHost {
    fn get_latest_release(&self) -> self_update::Result<Release> {
        Ok(Release::builder()
            .version("1.2.3")
            .asset(ReleaseAsset::new("app-x86_64-unknown-linux-gnu.tar.gz", "https://host/app.tar.gz"))
            .build()?)
    }
    fn get_latest_releases(&self, _current: &str) -> self_update::Result<Vec<Release>> {
        Ok(vec![self.get_latest_release()?])
    }
    fn get_release_version(&self, _ver: &str) -> self_update::Result<Release> {
        self.get_latest_release()
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
returns a concrete `Update` with async (`*_async`) verbs — `update_async()`,
`update_extended_async()`, and `get_latest_release_async()` — so a `tokio` application can update
without wrapping the blocking calls in `spawn_blocking`. The blocking API is unchanged; the async
path is purely additive. It is **tokio-only and reqwest-only** (ureq has no async story, so `async`
is incompatible with `ureq`). Network IO becomes async; the extract/replace step stays synchronous.

```rust
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

### Custom HTTP client

The `.timeout()` / `.request_header()` / `.retries()` builder knobs cover most transport needs, but
for full control — custom TLS roots / mTLS, connection pooling, redirect policy, proxy-with-auth, or
simply reusing your application's existing client — you can hand the crate a **pre-built client**.
It is used for both the release listing and the download. The setters are client-specific (the
client types differ and are mutually exclusive): `reqwest_client` (a blocking
`reqwest::blocking::Client`, used by the blocking API), `reqwest_async_client`
(an async `reqwest::Client`, used by the `*_async` verbs), and `ureq_agent` (a
`ureq::Agent`). The selected client crate is re-exported (`self_update::reqwest` /
`self_update::ureq`) so you don't need a separate dependency to name the type.

When you inject a client, `.request_header()` still applies, and `.retries()` still applies to the
release-listing requests (the download is never retried), and for `reqwest` the per-request
`.timeout()` is layered on too; but `HTTP(S)_PROXY` env and the crate's TLS feature are left entirely
to your client (and a `ureq::Agent` owns its own timeout, so `.timeout()` does not apply to an
injected agent — configure it on the agent). `reqwest_client` feeds the sync verbs and
`reqwest_async_client` the async ones — injecting only one and calling the other half just uses the
crate's per-call client for that half.

```rust
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

When using cross compilation tools such as cross if you want to use rustls and not openssl

```toml
self_update = { version = "1", features = ["rustls"], default-features = false }
```

**TLS certificate errors on Linux (`default-tls` / OpenSSL).** With the native-TLS backend,
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


License: MIT
