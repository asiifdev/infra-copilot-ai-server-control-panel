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
- `i18n` crate: en/id message catalog with stable keys, English fallback and a scanner that rejects
  hardcoded text in the UI.
- `db` crate: SQLite schema for organizations, users, memberships, API tokens, nodes, agents and node
  secrets; tenant-scoped queries; hash-chained append-only audit log with chain verification.
- `proto-server` crate and `ui/` prototype (Svelte 5 + Vite embedded in an axum binary) with
  `scripts/measure.sh`, used to set the performance budget (proposed in decision 0010).
- Decision records 0007 (core libraries), 0008 (own message catalog), 0009 (SQLite schema) and 0010
  (performance budget), all proposed; 0011 (own migrator, latest `rusqlite`), accepted.

### Changed

- `db` now uses `rusqlite` 0.43 with its own `user_version` migrator instead of `rusqlite_migration` (decision 0011); a database newer than the build is refused.
- `cargo-deny` allows workspace path dependencies; the docs check skips `ui/node_modules`.
- Replaced `chunks_exact` with `as_chunks` in the SHA-256 code and removed an unused dependency from `i18n`, found by the latest stable clippy.
- Contributor rules: documentation is updated in the same change, and commits are pushed right after they are made when the owner asks for a commit; CI must be green after each push; toolchain and dependencies track the latest stable versions.
