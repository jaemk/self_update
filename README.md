# self_update


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
```

The `manifest` backend (`manifest` feature) serves releases from a `manifest.json` file hosted
on any static file server. The tool author publishes the manifest at a stable URL; assets may be
absolute URLs or relative paths resolved against that URL. Asset `digest` fields (`sha256:<hex>`)
plug into the existing checksum verification path when the `checksums` feature is on. See
`specs/ref-manifest-backend.md` for the full schema.

```rust
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
```

Separate utilities are also exposed (**NOTE**: the following example extracts a `.tar.gz`, which
_requires_ both the `archive-tar` and `compression-tar-gz` features -- `archive-tar` reads the tar
archive and `compression-tar-gz` decodes the gzip layer; see the [features](#features) section
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

### Bundle installs (macOS `.app`)

A macOS application is a *directory* bundle, so replacing only the executable inside
`MyApp.app/Contents/MacOS/` leaves stale resources behind and breaks the bundle's code signature.
Set `bundle_path_in_archive` to name the bundle directory inside the release archive and the whole
tree is installed as one unit:

```rust
fn update() -> Result<(), Box<dyn std::error::Error>> {
    self_update::backends::github::Update::configure()
        .repo_owner("me")
        .repo_name("myapp")
        .bin_name("myapp")
        .current_version(self_update::cargo_crate_version!())
        // The bundle directory inside the archive; `{{ bin }}` / `{{ target }}` / `{{ version }}`
        // substitutions work here exactly as in `bin_path_in_archive`.
        .bundle_path_in_archive("MyApp.app")
        // Optional on macOS: defaults to the nearest `.app` ancestor of the running executable.
        .bundle_install_path("/Applications/MyApp.app")
        .build()?
        .update()?;
    Ok(())
}
```

How the swap works, and what it guarantees:

- The archive is extracted in full into a temporary directory **inside the install path's parent**,
  so every rename is on one filesystem (there is no cross-device fallback, and the parent needs
  room for one more copy of the bundle). A symlinked `bundle_install_path` is resolved first, so the
  tree behind the link is replaced, the link survives, and staging still lands beside the real tree.
- The installed tree is stashed, then the staged tree is renamed into place. A failure at any step
  restores the original bundle, and the error names the bundle path. Once the final rename lands the
  update is committed.
- When the running executable lives inside the bundle it is renamed aside first, so the old tree
  holds no running image. After a successful update the running executable's path holds the new
  bundle's executable, and the process can relaunch itself with `restart()` (see
  [Restarting after an update](#restarting-after-an-update)).
- Bundle mode replaces a directory, so combining it with an explicit `bin_install_path` or
  `bin_path_in_archive` is rejected by `build()` (`Error::ConflictingConfig`), and setting
  `bundle_install_path` without `bundle_path_in_archive` is an `Error::MissingField` rather than a
  silently discarded path. `bin_name` is still required: it selects the asset and feeds `{{ bin }}`.
- The `verify_binary` hook receives the **staged bundle root**, which is what
  `codesign --verify --deep` wants; a rejection aborts before anything is replaced.
- The crate never signs, notarizes, or staples: ship an already-signed (and, for Gatekeeper,
  notarized) `.app` and the swap preserves exactly what you shipped. A quarantined app running from
  a read-only App Translocation mount cannot update itself in place; that is detected up front as
  `Error::AppTranslocated`, and the fix is to move the app (which clears the quarantine) and
  relaunch it.

Directory bundles on linux and windows go through the same code path. On windows the swap fails,
and rolls back, if the process holds files inside the bundle open beyond its own executable (a DLL
loaded from the bundle, for example). `.deb` / `.msi` packages are a different shape entirely --
hand the downloaded file to `dpkg -i` / `msiexec /i` yourself; the crate's replace-and-verify
semantics do not apply to a system installer.

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
- **A digest resolved from a sums asset of the same release**, via
  `checksum_from_asset("SHA256SUMS")`. The named asset is fetched before the artifact is
  downloaded, and the entry for the selected asset supplies the digest. The usual `SHA256SUMS`
  shapes are accepted (coreutils text and binary modes, leading paths, the BSD tag form, `#`
  comments, and a whole-file bare digest), and the algorithm comes from the digest's length, so a
  `SHA512SUMS` asset needs no extra configuration. A release with no such asset, or no entry for
  the artifact, is an `Error::ChecksumSourceInvalid` rather than a skipped check. This is the one
  to reach for on gitlab / gitea / s3, whose APIs publish no per-asset digest.

Both complement the `signatures` feature (zipsign), which verifies authenticity rather than a
published digest.

```rust
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

Or let the updater fetch and parse the release's own sums asset:

```rust
fn update() -> Result<(), Box<dyn std::error::Error>> {
    self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        .checksum_from_asset("SHA256SUMS")
        .build()?
        .update()?;
    Ok(())
}
```

### Verification hooks

Two hooks let you gate an update with your own check. They differ in *what file they see*, which is
the whole reason both exist:

- `verify_archive(|archive: &Path| ..)` runs on the **downloaded archive**, after the crate's own
  content gates (checksum, release digest, signature) and before anything is extracted. This is
  where an external attestation or signature check belongs, since those are issued over the released
  file itself: `gh attestation verify <archive> --repo owner/repo`, `cosign verify-blob`, and so on.
  A rejection is `Error::ArchiveVerificationRejected`.
- `verify_binary(|new_exe: &Path| ..)` runs on the **extracted binary**, immediately before it
  replaces the installed one. This is where a smoke test belongs, typically running
  `new_exe --version` and checking the output. A rejection is `Error::VerificationRejected`.

Either returning `Err(..)` aborts the update with nothing installed. Full order:
`verify_checksum` -> release digest -> signature -> `verify_archive` -> extract -> `verify_binary`
-> replace.

```rust
fn update() -> Result<(), Box<dyn std::error::Error>> {
    self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        .verify_archive(|archive: &std::path::Path| {
            let ok = std::process::Command::new("gh")
                .args(["attestation", "verify"])
                .arg(archive)
                .args(["--repo", "jaemk/self_update"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                Ok(())
            } else {
                Err(self_update::Error::archive_verification_rejected(
                    "no build-provenance attestation for this artifact",
                ))
            }
        })
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

```rust
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

### Authentication

Every forge backend's `Update` **and** `ReleaseList` builder -- github, gitlab, gitea, gitee, eight
builders in all -- takes an authorization token. A token is what reaches a private repository at
all, and what lifts the host's anonymous request budget (see
[Rate limits and `Error::RateLimited`](#rate-limits-and-errorratelimited) below). There are two
setters:

* `auth_token(t)` -- a token your application already holds.
* `auth_token_from_env()` -- take it from the backend's conventional environment variables, using
  the first that is set and non-empty (surrounding whitespace is trimmed); a variable that *is* set
  but is not valid UTF-8 is treated the same as unset, since it could not become an HTTP header
  value either way:
  * **github**: `GH_TOKEN`, then `GITHUB_TOKEN` (matching the `gh` CLI's documented precedence).
  * **gitlab**: `GITLAB_TOKEN`.
  * **gitea**: `GITEA_TOKEN`.
  * **gitee**: `GITEE_TOKEN`.

The lookup happens when you call `auth_token_from_env()`, not at request time: it reads the process
environment exactly once, at that call. A `std::env::set_var` made afterward -- before `build()`,
before `update()` -- has no effect on an already-built value; call the setter again (or set the
variable earlier) if that ordering matters to you.

```rust
let status = self_update::backends::github::Update::configure()
    .repo_owner("jaemk")
    .repo_name("self_update")
    .bin_name("self_update_example")
    .current_version(self_update::cargo_crate_version!())
    // Uses a token when the environment supplies one; unauthenticated when it does not.
    .auth_token_from_env()
    .build()?
    .update()?;
```

**Precedence: an explicit `auth_token(..)` always wins, in either call order.** The environment is a
*fallback* that only fills an unset token, so `auth_token(t).auth_token_from_env()` and
`auth_token_from_env().auth_token(t)` both end up with `t`, and an ambient `*_TOKEN` can never
displace the credential your application provisioned. When no variable is set the call is a no-op --
the token is left as it was and the request goes out exactly as before -- so it is safe to place
unconditionally in an application that also runs outside CI or a corporate network.

`has_auth_token()` (on the same eight builders) reports whether an authorization token is
*configured* on this builder, from either setter. This is configuration, not a prediction: at
request time the token is withheld unless the URL's host matches the configured API host or an
`allow_auth_host` entry over https (loopback is allowed over plain http, for a local mirror or a
test stub), and a user-supplied `Authorization` header via `request_header` takes precedence over
it, silently. On gitea an env-sourced token is additionally withheld unless the configured host was
acknowledged (below). None of that is reflected by `has_auth_token()` -- it reports presence only,
never validity and never the value -- the builders' `Debug` renders the token as `"<token>"`, so
logging a builder does not leak an ambient CI credential.

Reading the environment is opt-in: the crate never does it on its own, since the configured API base
can be a self-hosted host and sending a user's token there should be your decision. Two caveats to
"safe to call unconditionally":

- A variable that is *set* but stale, expired, revoked, or scoped to a different resource makes the
  request **fail** where an anonymous request against a public repository would have succeeded --
  typically a generic `Error::Unauthorized`, with nothing in the error naming the environment as the
  cause. If a working update check starts failing right after you add `auth_token_from_env()`,
  check the variable's value first.
- A token that *is* picked up but cannot be encoded as an HTTP header value (a stray newline, for
  example) is not caught by `build()` -- it surfaces as
  [`Error::InvalidAuthToken`](crate::errors::Error::InvalidAuthToken) at **request** time, and that
  error's message does not mention the environment either.

Both of the crate's own diagnostics about the token it picked up -- the "using the auth token from
$X" pickup and the off-host warning below -- are emitted via `log::debug!` / `log::warn!` only.
Neither prints anything on its own; they are invisible unless your application has installed a
`log` implementation (`env_logger`, `tracing-log`, etc.).

**The variable set does not change with the host.** A custom `api_base_url` / `host` -- GitHub
Enterprise, a self-hosted GitLab -- is still served by exactly the variables above, so an ambient
`GITHUB_TOKEN` is sent to whatever host the builder points at. When an env-sourced token is about to
be bound to a host other than the backend's canonical one (`api.github.com`, `gitlab.com`,
`gitee.com`), `build()` emits a `log::warn!` naming the host, and still sends the token -- on
github/gitlab/gitee this is a warning, not a block. If the off-canonical host is a deliberate GitHub
Enterprise / self-hosted GitLab target, either silence the warning by acknowledging the host with
`allow_auth_host(..)`, or skip the environment lookup and set the token explicitly instead:
`auth_token(std::env::var("GITLAB_TOKEN")?)`. Note also that `gh` reads `GH_ENTERPRISE_TOKEN` /
`GITHUB_ENTERPRISE_TOKEN` for a GitHub Enterprise host and this crate does not, so an enterprise
`api_base_url` still needs one of the variables above (or an explicit `auth_token(..)`).

**gitea is the exception to warn-and-send.** It is always self-hosted, so it has no canonical host
to compare an env-sourced token's destination against. Rather than send `GITEA_TOKEN` to whatever
host the application happens to be pointed at with no signal at all, gitea *withholds* the token
instead: the request goes out anonymous, `build()` still returns `Ok`, and a `log::warn!` names the
host and the same two remedies as above. Get it sent anyway by acknowledging the host, either with
`allow_auth_host(host)` or by setting the token explicitly with `auth_token(..)` (which always takes
precedence, on every backend).

GitHub answers **404**, not 401 or 403, when a token cannot see a private repository -- it hides the
repository's existence rather than distinguishing "forbidden" from "not found". That 404 surfaces as
[`Error::NotFound`](crate::errors::Error::NotFound), so a repository you can normally read looks
like it does not exist rather than like a permission problem; check the token's scope before
assuming a typo in the repo name. Reading a private repository's releases needs the classic `repo`
scope (a fine-grained token needs `Contents: Read-only`) -- the "no scopes needed" note below is for
lifting a *public* repository's rate limit only.

`CI_JOB_TOKEN` is deliberately **not** read on gitlab, even though every GitLab CI job exports it:
this backend sends `Authorization: Bearer`, which is not GitLab's job-token mechanism (the
`JOB-TOKEN` header / `job_token` parameter), and job tokens are project-scoped -- reading it would
turn a working anonymous fetch of a public project into a 401/403 inside CI. Pass it explicitly with
`auth_token(..)` if you want it.

### Rate limits and `Error::RateLimited`

A rate-limited response surfaces as [`Error::RateLimited`](crate::errors::Error::RateLimited),
distinct from the `Error::Unauthorized` a genuine credential failure produces -- the rule below is
the same on **every** backend, not just github (the numbers in [GitHub rate
limits](#github-rate-limits) below are github-specific; the classification is not). A response with
headers in hand is classified as `RateLimited` when it is a **429** (RFC 6585 defines that status as
rate limiting, so it always lands here, with or without quota headers), or a **403** carrying either
a zero remaining-quota header (`x-ratelimit-remaining: 0`, or gitlab's `RateLimit-Remaining: 0`) or a
usable `Retry-After` -- that last case is GitHub's *secondary* rate limit, which answers 403 +
`Retry-After` while `x-ratelimit-remaining` is still nonzero. A bare 403 with no such header stays
`Unauthorized`.

Back off by [`Error::rate_limit_delay()`](crate::errors::Error::rate_limit_delay), which resolves
the wait to an `Option<Duration>`: the server's `Retry-After` when it sent one, otherwise
`reset_at` minus now, and `None` when the window has already elapsed or nothing is known. Reading
the raw fields instead is the footgun -- on GitHub's *primary* limit only `x-ratelimit-reset` is
sent, so `retry_after.unwrap_or_default()` sleeps zero and burns more quota. Both server-supplied
values are clamped to a 24h ceiling; beyond it they resolve to `None`, so a hostile `Retry-After`
cannot park an update channel indefinitely -- but the wait can legitimately be *up to* that 24h
ceiling, so blocking a thread on it is rarely the right call for an interactive application (see the
example below).

The retry/backoff setters do **not** apply to a `RateLimited` response. `Error::RateLimited` is
never retried: the wait is the server's to dictate (`Retry-After`, or the reset header), and it can
be far longer than any backoff this crate would apply, so the error is returned immediately and the
decision to sleep, reschedule, or give up stays with the caller instead of being spent inside the
loop.

```rust
fn check() -> Result<(), Box<dyn std::error::Error>> {
    let update = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("self_update_example")
        .current_version(self_update::cargo_crate_version!())
        .auth_token_from_env()
        .build()?;

    match update.update() {
        Ok(status) => println!("update status: `{}`", status.version()),
        Err(err @ self_update::Error::RateLimited { .. }) => {
            // rate_limit_delay() can resolve to a wait as long as 24h, so blocking this thread on
            // it is rarely the right call for an interactive app. Skip this run and let the next
            // scheduled check (e.g. through `UpdateCheckGuard` above) try again, rather than
            // sleeping here -- if you do want to block instead, sleep on `err.rate_limit_delay()`
            // and retry `update.update()` yourself.
            let _ = err.rate_limit_delay();
            println!("rate limited; retrying on the next scheduled check");
        }
        Err(err) => return Err(err.into()),
    }
    Ok(())
}
```

### GitHub rate limits

Requests to the GitHub REST API are rate limited by GitHub itself, not by this crate:

- **Unauthenticated** requests are limited to **60 per hour per source IP**; **authenticated**
  requests (a token via `auth_token` / `auth_token_from_env`, see
  [Authentication](#authentication)) get **5000 per hour**. A token needs no scopes to raise the
  limit for a public repository (a private repository needs the scope noted above regardless of the
  limit).
- That budget is counted **per source IP, not per application**. Behind a shared egress IP -- a
  NAT'd corporate network, a CI runner pool, a VPN exit -- it is pooled across everyone on that IP
  and can be spent entirely by other people, so a lightly-used application still sees 403s there.
- An update check costs **one** API request (the latest-release lookup, or one request per page of a
  paginated listing). The asset **download** itself is a CDN redirect and does not count against the
  core API limit.
- To avoid it: set a token, and check less often -- the
  [`UpdateCheckGuard`](crate::check_interval::UpdateCheckGuard) above throttles how often you check.

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

```rust
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

A fully custom transport also owns the job of **classifying** a non-2xx response. Prefer
[`Error::http_status_error_with_headers(status, url, &headers)`](crate::errors::Error::http_status_error_with_headers)
over the header-blind [`Error::http_status_error`](crate::errors::Error::http_status_error): the
header-blind form still maps a **429** to
[`Error::RateLimited`](crate::errors::Error::RateLimited) -- the status alone is the signal. What it
cannot do is promote a **403** (with no headers in hand a 403 stays `Unauthorized`) or recover the
`reset_at` / `retry_after` fields, so `rate_limit_delay()` on one of its errors is always `None`.
See [Rate limits and `Error::RateLimited`](#rate-limits-and-errorratelimited) above for the full
classification rule. The built-in reqwest and ureq clients (including an injected `ureq::Agent`)
all use the header-aware form, so they classify identically.

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

**TLS certificate errors behind a corporate proxy (`ureq` + `rustls`).** Many company networks
terminate outbound HTTPS at an intercepting proxy that re-signs traffic with an internal CA. That CA
is installed in the machine's trust store, so `curl` and the system browsers accept it, but the
ureq client's default root store is `RootCerts::WebPki` (Mozilla's bundled roots), which ignores the
machine entirely, so every request fails to verify. Enable the `native-certs` feature to move the
ureq client onto the OS trust store instead:

```toml
self_update = { version = "1.2", features = ["ureq", "rustls", "native-certs"] }
```

The reqwest client needs nothing: its rustls setup already verifies against the OS trust store.
`native-certs` has no effect on an injected `ureq::Agent` either, since that agent owns its own TLS
config, so set `RootCerts::PlatformVerifier` on it yourself. On Linux the OS trust store honors
`SSL_CERT_FILE` / `SSL_CERT_DIR`, so those env vars work as an escape hatch once the feature is on.
To trust exactly one internal CA and nothing else, skip the feature and pass the certificate to
[`add_root_certificate`](crate::backends::github::UpdateBuilder::add_root_certificate). Note that on
a ureq build that *replaces* the trust store rather than adding to it.


License: MIT
