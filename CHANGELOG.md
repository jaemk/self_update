# Changelog

## [Unreleased]
### Added

### Changed

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

