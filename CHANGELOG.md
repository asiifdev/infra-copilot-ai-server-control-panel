# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project intends to follow
[Semantic Versioning](https://semver.org/) once there is a release.

The changelog is kept in English only, as a single source of truth, so the two language versions
cannot drift apart. Release notes may get an Indonesian summary.

## [Unreleased]

### Added

- M2-c, the AI provider (`crates/ai`, ADR 0022): an OpenAI-compatible chat client over `net` with function calling, Server-Sent Events streaming (a decoder that is byte-split safe, fragmented tool calls merged by index, `<think>` blocks removed), strict `params` parsing, bounded answers, retries that never repeat visible text, `Retry-After` support and a circuit breaker. Errors have fixed text and never carry the key or server text. 42 tests, including a real-socket SSE round trip; a live smoke test is opt-in (`--ignored`) and not yet run against Muse.
- `net`: `post_json_with` (validated extra headers; `Authorization` marked sensitive; framing and injection headers refused) and `post_json_stream` (body read chunk by chunk with idle, total and byte limits) for Server-Sent Events. Ten new tests against local listeners; the 28 existing ones are unchanged.
- Secret storage at rest (ADR 0021): crate `vault` seals secrets with AES-256-GCM (`ring`, fresh random nonce, associated data binding org, purpose and key id), wipes key material on drop, and supports key rotation. Migration 0005 adds org-level `secrets` and `ai_providers` tables. `Secret` now lives in `core-domain` (notifier re-exports it).
- M2-b, read-only AI tools: database tables for inventories, items, comparisons and findings (migration 0004, every child row tied to its org by a composite foreign key); a `ReadStore` seam with a SQLite implementation (`DbStore`) and `persist_inventory` / `persist_comparison`; and five tier 0 tools (`list_nodes`, `get_inventory_summary`, `list_inventory_items`, `list_findings`, `explain_finding`) whose explanations come from the i18n catalog. Tests cover cross-org reads and writes, injection-looking node names and both locales.
- M2-a, the AI tool layer (`crates/agent`, ADR 0020): the `AiTool` contract, closed typed argument schemas, a registry that refuses any tool above tier 1 and any tool declaring an identity field (`org_id`, `user_id`, ...), and an executor that validates arguments, redacts and caps results, and fences them as untrusted data. No model is connected yet; 26 tests cover refusal paths.

- A visual README in English and Indonesian: logo hero, honest status table, an operating-loop diagram (`docs/assets/operating-loop.svg`, every step labelled NOW, BUILDING or PLANNED), a real CLI output sample, a table of the safety rules enforced today with links to the code, and the roadmap.

- Discoverability: a lead line and a one-line mechanism in both READMEs, a "Who is Nodivara for?" section by role, the same canonical definition in the README, FAQ and `llms.txt`, and a 1280x640 social preview image (`brand/nodivara-social-1280x640.png`; GitHub only accepts it through the repository settings).
- Light-theme logo files (lockup, mark, square) in `brand/`; the README picks the light or dark lockup with `<picture>`.
- Product name **Nodivara** (decision 0019) and the brand palette: colour tokens sampled from the logo with light and dark themes (`ui/src/tokens.css`), a contrast checker run in CI (WCAG AAA 7:1 for every text token on every surface, 3:1 for UI boundaries) (`scripts/check-palette.py`), optimized logo files in `brand/` and a usage guide (`docs/brand.md`). The repository, crate and binary names are unchanged. A trademark and domain check is still pending.

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

- `inventory` crate: inventory model (`Inventory`, `Item`, `Finding`, `Status`) and the `Collector` and `ParityRule` contracts, following `docs/design.md` 3.3, 4, 5 and 12.4 (decision 0013).
- `collectors` crate: read-only Linux collectors for listening ports, processes and cron jobs, reading through a replaceable `RootFs` (tested on fixture trees); command lines and cron commands are redacted and `/proc/<pid>/environ` is never read.
- `parity` crate: rules PAR-001 (listener without counterpart), PAR-008 (cron job without counterpart) and PAR-010 (port open to the internet), a deterministic comparer, and a renderer that takes all text from the en/id catalog. A partial inventory never proves absence: findings are capped at Info.
- PAR-006 (application serving a port runs a development server or outside any supervisor, target side only). The process collector now sets `mode` to `dev` for well-known development servers and `supervised` from the cgroup (systemd service, container scope, login session) and from supervisor ancestors such as PM2; `prod` is never claimed and unknown stays null.
- `collectors`: systemd (running services and timers), Docker (running containers) and PM2 (apps saved in `dump.pm2`, read from files only) collectors, plus a closed, read-only command runner for `docker` and `systemctl` (decision 0014). `infra collect` runs programs only on this machine; with `--root` they are skipped.
- `env_ref` items: PM2 and Docker environment values are parsed in memory into a target host and port and then dropped (never stored, logged or shown); references to local listeners get `resolves_to`. Docker process items get `version` from the image tag (decision 0015).
- Parity rules PAR-002 (a container reference to `localhost` while the service listens on the host), PAR-007 (a reference to a local port nothing listens on) and PAR-009 (different major version of the same container image).
- `cli` crate: `infra collect` and `infra compare` (exit code 1 on a critical finding), with English and Indonesian output.
- `net` crate: async HTTP/TLS client for the monitor (decision 0016): GET and JSON POST with redirect, size and time caps, an address check at the exact connect call (private, loopback, link-local and metadata ranges are refused by default), TCP connect timing and certificate inspection that also works for expired or self-signed servers.
- `monitor` crate: pure uptime incident engine (failure and recovery thresholds, maintenance windows, repeat and flapping guards, uptime percentage); probe results carry no server text.
- `notifier` crate: `NotificationChannel` contract with a shared contract suite, a send-only Telegram channel (MarkdownV2 escaping, closed error set, token never in any error or Debug output), localized en/id alerts, quiet hours and throttling.
- `watch` crate: runs monitor probes through `net` (concurrent, bounded, per-check timeouts), feeds the incident engine and sends localized alerts through a `NotificationChannel` (Telegram over `net`). The bot token is read at send time from a named environment variable behind a `SecretSource` trait (a stop-gap, decision 0018); monitors on private addresses need an explicit per-monitor opt-in.

### Security

- `RootFs` now resolves every path with a directory-descriptor walk (`openat` with `O_NOFOLLOW` per component, links expanded inside the root with `readlinkat`, at most 40 links) and, on Linux, `openat2(RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS)` first. A process that can write inside the root can no longer swap a parent directory for a symlink between the check and the open (closes the limit accepted in decision 0013; decision 0017). The new `rustix` dependency (features `std`, `fs`) is used only by `collectors`; the public API is unchanged and `read_to_string_nolink` no longer needs a check-after-open.
- `env_ref` extraction ignores variables named like credentials (`PASS`, `SECRET`, `TOKEN`, `KEY`, `AUTH`, `CRED`, `PRIVATE`), trusts a bare `host:port` only for reference-named variables, and discards URLs whose userinfo was cut by an unencoded `/`, `?` or `#`, so a username or password digits can never become a host or port.
- `RootFs` never opens a device node for a file read: on Linux the entry is opened with `O_PATH`, typed on that descriptor and reopened through `/proc/self/fd` only when it is a regular file (falling back to the walk where `/proc` is missing); the walk checks the type just before opening, and the opened descriptor is checked again. File reads also use `O_NOCTTY`.
- PM2 collector refuses symbolic links at `.pm2` and at the dump, reads `/root` first and records the owning account, so a user-writable home cannot redirect the read or shadow the administrator's apps. The dump is opened with a check after open (`RootFs::read_to_string_nolink`), so a link swapped in between the check and the read is refused too; parent directories remain a documented limit (decision 0013).
- Redaction now treats `PASS` and `AUTH` named values as secrets, and collectors hide separate-argument secrets in process command lines and cron commands (`-p secret`, `--password x`, `curl -u user:pw`). Found by independent review, covered by tests.
- Redaction of nested quoted commands (`sh -c '...'`), `--user`/`--proxy-user` values and cron hints; `infra compare` sanitizes control characters from untrusted inventories so they cannot repaint the terminal or forge report lines.
- Collectors resolve symlinks inside the chosen root only, so a symlink in a mounted or untrusted tree cannot read host files; a crontab or process name with multi-byte characters can no longer make the whole collection fail.

### Changed

- `db` hashes the audit chain with the `sha2` crate instead of in-house SHA-256 (decision 0012); decisions 0008, 0009 and 0010 are accepted.
- `.gitignore` now covers local databases, `measure.json`, and tool scratch directories.
- CI now has a `perf-budget` job (Linux and macOS) that builds the prototype, measures it with `scripts/measure.sh` and fails when `scripts/check-budget.py` finds a metric over `docs/perf-budget.json`.
- `db` now uses `rusqlite` 0.43 with its own `user_version` migrator instead of `rusqlite_migration` (decision 0011); a database newer than the build is refused.
- `cargo-deny` allows workspace path dependencies; the docs check skips `ui/node_modules`.
- Replaced `chunks_exact` with `as_chunks` in the SHA-256 code and removed an unused dependency from `i18n`, found by the latest stable clippy.
- Contributor rules: documentation is updated in the same change, and commits are pushed right after they are made when the owner asks for a commit; CI must be green after each push; toolchain and dependencies track the latest stable versions.
