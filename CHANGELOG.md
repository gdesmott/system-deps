# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [7.0.3](https://github.com/gdesmott/system-deps/compare/v7.0.2...v7.0.3) - 2024-09-22

### Other

- Update cfg-expr to 0.17

## [7.0.2](https://github.com/gdesmott/system-deps/compare/v7.0.1...v7.0.2) - 2024-08-16

### Fixed
- fix new clippy warning

### Other
- Add `cargo:rerun-if-changed=Cargo.toml` to config probe
- Fix some errors in docs
- update to cfg-expr 0.16

## [7.0.1](https://github.com/gdesmott/system-deps/compare/v7.0.0...v7.0.1) - 2024-06-18

### Other
- bump minimal pkg-config version to 0.3.25

## [7.0.0](https://github.com/gdesmott/system-deps/compare/v6.2.2...v7.0.0) - 2024-06-17

### Fixed
- fix new clippy warning

### Other
- Add support for linker flags
- reformat
- update itertools dep

## [6.2.2](https://github.com/gdesmott/system-deps/compare/v6.2.1...v6.2.2) - 2024-03-19

### Other
- Update to heck 0.5, version-compare 0.2 and itertools 0.12

## [6.2.1](https://github.com/gdesmott/system-deps/compare/v6.2.0...v6.2.1) - 2024-03-14

### Other
- Don't ignore transitive imports when probing a static library
- ignore version_range_unsatisfied
- fix 'optional' test on Ubuntu CI
- use release-plz
