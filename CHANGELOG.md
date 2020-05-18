# Changelog

## [Unreleased]
### Added
### Changed
### Removed

## [0.15.0]
### Added
- Handling for `.tgz` files
### Changed
- Support version tags with or without leading `v`
- S3, support path prefixes that contain directories
### Removed

## [0.14.0]
### Added
- Expose `body` string in `Release` data
### Changed
### Removed

## [0.13.0]
### Added
- Feature flag `rustls` to enable using [rustls](https://github.com/ctz/rustls) instead of native openssl implementations.
### Changed
### Removed

## [0.12.0]
### Added
### Changed
- Make all archive and compression dependencies optional, available behind
  feature flags, and off by default. The feature flags are listed in the
  README. The common github-release use-case (tar.gz) requires the features
  `archive-tar compression-flate2`
- Make the `update` module public
### Removed

## [0.11.1]
### Added
### Changed
- add rust highlighting tag to doc example
### Removed

## [0.11.0]
### Added
### Changed
- set executable bits on non-windows
### Removed

## [0.10.0]
### Added
### Changed
- update reqwest to 0.10, add default user-agent to requests
- update indicatif to 0.13
### Removed

## [0.9.0]
### Added
- support for Amazon S3 as releases backend server
### Changed
- use `Update` trait in GitHub backend implementation for code re-usability
### Removed

## [0.8.0]
### Added

### Changed
- use the system temp directory on windows

### Removed

## [0.7.0]
### Added
### Changed
- accept `auth_token` in `Update` to allow obtaining releases from private GitHub repos
- use GitHub api url instead of browser url to download assets so that auth can be used for private repos
- accept headers in `Download` that can be used in GET request to download url (required for passing in auth token for private GitHub repos)
### Removed

## [0.6.0]
### Added
### Changed
- use indicatif instead of pbr
- update to rust 2018
- determine target arch at build time
### Removed


## [0.5.1]
### Added
- expose a more detailed `GitHubUpdateStatus`

### Changed
### Removed


## [0.5.0]
### Added
- zip archive support
- option to extract a single file

### Changed
- renamed github-updater `bin_path_in_tarball` to `bin_path_in_archive`

### Removed


## [0.4.5]
### Added
- freebsd support

### Changed

### Removed


## [0.4.4]
### Added

### Changed
- bump reqwest

### Removed


## [0.4.3]
### Added

### Changed
- Update readme - mention `trust` for producing releases
- Update `version` module docs

### Removed
- `macro` module is no longer public
    - `cargo_crate_version!` is still exported


## [0.4.2]
### Added
- `version` module for comparing semver tags more explicitly

### Changed
- Add deprecation warning for replacing `should_update` with `version::bump_is_compatible`
- Update the github `update` method to display the compatibility of new release versions.

### Removed
