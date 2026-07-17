# Manifest backend (reference)

Status: pending

## Scope

The manifest backend in `src/backends/manifest.rs`, gated on the `manifest` Cargo feature. It
provides a service-agnostic release backend: the tool author publishes a `manifest.json` at any
stable HTTPS URL (a GitHub Pages site, an S3 bucket, a CDN, a plain nginx directory -- any static
file server), and the updater fetches it to list releases. No forge API, no platform-specific
bucket listing, no new dependencies. This file documents the intended behavior as a canonical
reference for the implementation.

## Behavior

### Builders

Two public items gated on the `manifest` feature:

- `ManifestSource` / `ManifestSourceBuilder`: a concrete `ReleaseSource` (and `AsyncReleaseSource`
  under the `async` feature) that fetches and parses a `manifest.json` from a given URL using the
  crate's HTTP clients, honoring `RequestConfig` (timeout, `request_header`, retries). It can be
  used directly with `backends::custom::Update` when custom pipeline control is needed (e.g.
  combining a manifest source with a bespoke asset-matcher or verify hook).
- `Update` / `UpdateBuilder`: the all-in-one facade. `Update::configure()` returns an
  `UpdateBuilder`. The backend setters are: `manifest_url` (required). The shared common surface
  is provided by `impl_common_builder_setters!()`: `bin_name`, `current_version`, `target`,
  `timeout`, `retries`, `request_header`, `show_download_progress`, `no_confirm`, `show_output`,
  `auth_token`, `verify_checksum`, `verify_release_digest`, `asset_matcher`, `verify`, `checksum`,
  `reqwest_client`, `reqwest_async_client`, `ureq_agent`, `http_client`, `http_client_async`,
  and their shared accessors. `build()` validates `request.check()` (surfacing a deferred
  `request_header` parse error as `Error::InvalidHeader`), then requires `manifest_url`
  (`Error::MissingField { field: "manifest_url" }`), then calls `common.build()` (which requires
  `current_version` and `bin_name`). `build_async()` (feature `async`) returns the async wrapper.
  The concrete `Update` is `Send` and exposes the inherent verbs (`update`, `update_extended`,
  `get_latest_release`, `get_newer_releases`, `get_release_version`, `is_update_available`), so
  `.build()?.update()?` needs no trait import.

### Manifest schema

The canonical schema (version 1):

```json
{
  "schema": 1,
  "releases": [
    {
      "version": "1.2.3",
      "date": "2026-07-16T00:00:00Z",
      "notes_url": "https://example.net/releases/1.2.3",
      "assets": [
        {
          "name": "app-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
          "url": "app-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
          "digest": "sha256:..."
        }
      ]
    }
  ]
}
```

Field semantics, top level:

- `schema` (integer, required): the schema version. Currently must be exactly `1`. A manifest
  whose `schema` field is absent, of the wrong type, or any value other than `1` is rejected with
  `Error::InvalidResponse` naming the schema version.
- `releases` (array, required): the list of releases; may be empty (yields `Error::NoReleaseFound`
  from the update path).

Field semantics, per release entry:

- `version` (string, required): a semver version string. An entry whose `version` is not valid
  semver is skipped with a debug log and does not appear in the returned release list (same
  behavior as the forge backends skipping non-semver tags).
- `date` (string, optional): an ISO-8601 / RFC-3339 datetime string; maps to `Release::date`.
  Absent or unparseable values are treated as absent (the field is `None`).
- `notes_url` (string, optional): a URL for the release notes page; maps to
  `Release::release_notes_url()`. Absent when not set.
- `assets` (array, required per entry): zero or more asset objects.

Field semantics, per asset:

- `name` (string, required): the asset filename; maps to `ReleaseAsset::name()`.
- `url` (string, required): the asset download URL; may be absolute or relative. See URL
  resolution below.
- `digest` (string, optional): a content digest in `algorithm:hex` form. The value is mapped
  verbatim to `ReleaseAsset::digest()` and plugs into the existing release-digest verification
  path (the same check the github backend uses when the `checksums` feature is on; see
  `ref-signatures-and-checksums.md`). The verification layer supports `sha256:` and `sha512:`;
  an unsupported algorithm errors at verify time rather than being silently skipped, so a digest
  the manifest author supplied is never dropped. Absent when the field is missing.

Unknown fields at any level of the document are silently ignored (forward compatibility).

### URL resolution

The `url` field of each asset may be an absolute URL (contains `://`) or a relative path. Relative
URLs resolve against the manifest URL's directory: the manifest URL is truncated at the last `/`
and the relative URL is appended verbatim. For example, a manifest at
`https://example.net/releases/manifest.json` and an asset url of
`app-1.2.3-x86_64-unknown-linux-gnu.tar.gz` resolves to
`https://example.net/releases/app-1.2.3-x86_64-unknown-linux-gnu.tar.gz`. Path segments of `..`
in a relative URL are not specially handled (they are included in the resolved URL as-is).

### Schema versioning and forward compatibility

`schema: 1` is the only currently accepted version. The `schema` field is read before any other
field; a `schema` that is not recognized (absent, wrong type, or any value other than 1) causes
`Error::InvalidResponse` with a message naming the received schema value. This ensures that a
future schema change (adding required fields or restructuring `releases`) is a clean failure for
older clients rather than a silent mis-parse.

Unknown fields anywhere in the document are ignored (forward compatibility). A future `schema: 2`
may add required fields or restructure the document; clients built against `schema: 1` will reject
it with `Error::InvalidResponse` rather than silently misread it.

### Non-semver release skipping

Release entries whose `version` field is absent or is not a valid semver string are skipped at
parse time with a `debug!` log line naming the skipped version. They do not appear in the returned
`Vec<Release>`. This matches the forge backends' treatment of non-semver tags.

### Digest verification

Each asset's optional `digest` field (`sha256:<hex>`) maps to `ReleaseAsset::with_digest`. Under
the `checksums` feature, the update pipeline verifies the downloaded artifact against it before
installing -- automatically, on by default, when the asset carries a digest. The caller can opt
out with `verify_release_digest(false)`. The behavior is identical to the github backend's
per-asset digest check. See `ref-signatures-and-checksums.md` for the full verification pipeline.

### Sync and async

`ManifestSource` implements `ReleaseSource` unconditionally (under the `manifest` feature).
Under the `async` feature it also implements `AsyncReleaseSource`. `Update::build()` drives the
blocking path; `Update::build_async()` (feature `async`) drives the async path. Network IO
is handled by the crate's shared sync/async HTTP clients (`ureq` / reqwest blocking for sync;
reqwest async for async).

### Errors

- `manifest_url` not set at `build()`: `Error::MissingField { field: "manifest_url" }`.
- `bin_name` or `current_version` not set: `Error::MissingField` from `common.build()`.
- A completed non-2xx response from the manifest fetch: structured by status via `status_to_error`
  (404 -> `Error::NotFound`, 401/403 -> `Error::Unauthorized`, other non-2xx ->
  `Error::HttpStatus`); a connection/TLS/timeout failure is `Error::Transport`.
- Unrecognized `schema` value or missing required fields: `Error::InvalidResponse`.
- An empty releases list after filtering (no releases at all, or all non-semver): the
  `Error::NoReleaseFound` from the standard update-selection helpers.

## Public surface

Under the `manifest` feature:

- `manifest::ManifestSource`, `manifest::ManifestSourceBuilder` (the `ReleaseSource`
  implementation; usable independently with `backends::custom::Update`).
- `manifest::Update`, `manifest::UpdateBuilder`: the all-in-one facade.
  `Update::configure()` -> `UpdateBuilder`. `build()` -> `Update`. `build_async()` (feature
  `async`) -> async wrapper.
- `Update` is `Send`; exposes inherent verbs (`update`, `update_extended`, `get_latest_release`,
  `get_newer_releases`, `get_release_version`, `is_update_available`) and, under `async`, the
  `*_async` siblings via the `AsyncReleaseUpdate` default methods.
- No `ReleaseList` struct: standalone listing uses `ManifestSource` directly (as a `ReleaseSource`
  implementation) or uses the inherent listing verbs on `Update`.

The manifest backend adds no new Cargo dependencies; all HTTP, JSON parsing, and checksum
verification reuse existing crate infrastructure.

## Invariants and regression checklist

- `manifest_url` is required; absent -> `Error::MissingField { field: "manifest_url" }` from `build()`.
- `bin_name` and `current_version` are required (common setters); absent -> `Error::MissingField` from `common.build()`.
- `schema != 1` (including absent, wrong type, `0`, or any integer > 1) -> `Error::InvalidResponse`.
- Unknown fields at any level are ignored; the parser must not reject a future schema extension
  for a document whose `schema` is still `1`.
- Non-semver `version` entries are dropped with a debug log, not an error.
- Relative asset `url` values resolve against the manifest URL truncated at the last `/`; `..`
  segments are not handled specially.
- `digest` values map verbatim to `ReleaseAsset::with_digest`. Under `checksums`, the update
  pipeline verifies the download against it; an unsupported algorithm prefix errors at verify
  time, it is never silently dropped.
- A non-2xx manifest fetch is always `Err`; the backend never parses an error body as a release
  list.
- `ManifestSource` implements `ReleaseSource`; under `async` it also implements
  `AsyncReleaseSource`.
- No new Cargo dependencies are added by the `manifest` feature.

## Tests

Expected in `src/backends/manifest.rs` (`#[cfg(test)] mod tests`), driven by a loopback TCP stub
(no external network):

- Schema validation: `schema: 1` accepted; `schema: 0`, `schema: 2`, and missing `schema` yield
  `Error::InvalidResponse`; unknown top-level fields are ignored.
- Non-semver entries: a release with a non-semver `version` is skipped (debug log); a valid
  version in the same manifest is kept.
- URL resolution: absolute URLs are used verbatim; relative URLs resolve against the manifest
  URL's directory. A manifest at `.../dir/manifest.json` and asset url `foo.tar.gz` resolves to
  `.../dir/foo.tar.gz`.
- Digest: an asset with `digest: "sha256:<hex>"` carries that digest on the `ReleaseAsset`;
  an asset without `digest` has `None`.
- Optional fields: `date` and `notes_url` absent -> `None`; present -> mapped.
- Empty releases list -> `Error::NoReleaseFound` from the update selection.
- Non-2xx responses -> structured status error (`NotFound`, `Unauthorized`, `HttpStatus`).
- `manifest_url` missing at `build()` -> `Error::MissingField`.
- Loopback stub tests for `get_latest_release`, `get_newer_releases`, `get_release_version`,
  and `is_update_available` (sync); and the async equivalents under the `async` feature.
- `ManifestSource` used directly with `backends::custom::Update` to confirm
  `ReleaseSource` interop.

## Related

- `ref-custom-backend.md` (`ManifestSource` can be used as a `ReleaseSource` with the custom backend)
- `ref-signatures-and-checksums.md` (digest verification pipeline that `digest` fields plug into)
- `ref-release-model.md` (the `Release` / `ReleaseAsset` model)
- `ref-feature-flags.md` (the `manifest` feature flag)
- `transport-control.md` (timeout, retries, custom headers honored by `ManifestSource`)
- `error-network-vs-http-semantics.md` (non-2xx -> structured status; transport failure -> `Error::Transport`)
