# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project intends to follow
[Semantic Versioning](https://semver.org/) once there is a release.

The changelog is kept in English only, as a single source of truth, so the two language versions
cannot drift apart. Release notes may get an Indonesian summary.

## [Unreleased]

### Added

- `core-domain` crate: the five risk tiers and the approval rules attached to them, and secret
  redaction (URL passwords, `name=value` and JSON secrets, bearer tokens, well-known token shapes,
  private key blocks), with tests.
- Design documents, architecture decision records, and market research.
- Project documents for open source: README, contributing guide, code of conduct, security policy,
  and licensing notes, each in English and Indonesian.
- Provisional license: `AGPL-3.0-only` (see `LICENSING.md`).
- Continuous integration: format, lint, tests on Linux and macOS, dependency policy (`deny.toml`), advisory scan, full-history secret scan, and a documentation check.
- Funding and support policy (`FUNDING.md`), in English and Indonesian.
