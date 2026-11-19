English | [Bahasa Indonesia](README.id.md)

# Infra Copilot

[![CI](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/actions/workflows/ci.yml/badge.svg)](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/actions/workflows/ci.yml)

**Infra Copilot is an open-source server control panel, aimed at the same job as aaPanel and cPanel, with a built-in AI agent that proposes changes and waits for human approval.** It is written in Rust with a Svelte interface, targets Linux servers and a desktop app, and is pre-alpha: nothing is installable yet.

Last updated: 2026-11-19.

> **Status: pre-alpha.** There is no installable release yet. This repository currently holds the
> design, the architecture decisions and the first foundation crates (domain types, message catalog, database schema), a size and speed prototype, and an offline command line that collects a Linux inventory read-only and compares two inventories with three parity rules. There is no agent, panel or monitor yet. "Infra Copilot" is a provisional
> name and may change.

## What it is

Infra Copilot aims to be a lightweight panel for running and migrating servers, in the spirit of
aaPanel and cPanel, with an AI agent that understands the state of the machine and acts only with
human approval. It runs on Linux servers (VPS or dedicated) and, later, as a desktop app.

Everything below is **planned**, not shipped:

- **Manage a server:** services, apps, sites, databases, backups, logs, cron, files.
- **Deploy from Git, one click:** detect the stack (Node, PHP, Python, Go, .NET), build on the
  server, switch traffic without downtime, roll back. An advanced mode for full control.
- **Monitor uptime and errors:** checks from the server and from the outside, grouped error events,
  and AI root-cause analysis with the evidence it relied on and recommended steps.
- **Telegram bot:** alerts, commands, and action buttons, in your language.
- **Migrate with a parity check:** inventory the old and the new server, then report what was left
  behind (a service nobody listed, a `localhost` that points at the wrong machine, a DNS record that
  never moved) before you switch off the old server.
- **DNS and Cloudflare Tunnel:** manage records and tunnels, always with a snapshot and a rollback.
- **An AI agent you can audit:** an OpenAI-compatible provider of your choice, tool calling,
  memory, and an approval workflow. It proposes; you decide.

## Principles

- **Lightest and fastest.** One Rust binary, a Svelte 5 UI compiled into it, SQLite by default.
  Desktop edition with Tauri. No extra runtime on your server just to run the panel.
- **Least privilege.** The agent runs a closed list of actions, not a free shell. Actions are
  ranked from read-only to destructive, and anything that changes or destroys something goes
  through a proposal and a human approval.
- **Secrets stay secret.** They are never logged, never shown, and never sent to an AI model.
- **Open core, honestly.** The core is open source. Paid editions (extra limits, licensing, hosted
  cloud) will be offered separately; nothing in the open core is crippled on purpose.
- **English and Indonesian from day one.** All user-facing text goes through translation catalogs.

## Editions (planned)

| Edition | Where it runs | Status |
|---|---|---|
| Server | Linux VPS / dedicated, UI in the browser | planned |
| Desktop | macOS, Linux, Windows (Tauri) | planned |
| Cloud | Hosted by the project | planned |

## Repository layout

```
crates/core         shared domain types: risk tiers, secret redaction (no external dependencies)
crates/i18n         en/id message catalog with stable keys and fallback
crates/inventory    inventory model and the Collector / ParityRule contracts
crates/collectors   read-only Linux collectors (listeners, processes, cron, PM2, systemd, Docker)
crates/parity       parity rules PAR-001, PAR-002, PAR-006, PAR-007, PAR-008, PAR-009, PAR-010, comparer and localized report
crates/net          SSRF-safe async HTTP/TLS client (certificate inspection) for the monitor
crates/monitor      uptime incident engine (thresholds, maintenance, uptime %), pure logic
crates/notifier     NotificationChannel contract, Telegram send channel, localized alerts
crates/cli          `infra collect` and `infra compare` (offline, read-only)
crates/db           SQLite schema, tenant-scoped queries, hash-chained audit log
crates/proto-server throwaway prototype used to measure binary size, memory and startup
ui/                 Svelte 5 + Vite prototype page (embedded into the prototype binary)
docs/design.md      technical design: schema, inventory format, agent protocol, approval flow
docs/decisions/     architecture decision records
docs/market-research.md   market research with facts separated from inference
scripts/            helper scripts (including measure.sh for the performance budget)
.claude/ .agents/   configuration and skills for AI-assisted development
```

## Build and test

Requires a recent stable Rust toolchain.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

There is nothing to run yet beyond the tests.

## Documentation

- [FAQ](docs/faq.md) | [Comparison with other panels](docs/comparison.md)
- [Design](docs/design.md) and [decision records](docs/decisions/)
- [Contributing](CONTRIBUTING.md) | [Code of Conduct](CODE_OF_CONDUCT.md) | [Security policy](SECURITY.md)
- [Licensing](LICENSING.md) | [Funding and support](FUNDING.md) | [Changelog](CHANGELOG.md)

Each document has an Indonesian version next to it (`*.id.md`), except the license text and the
changelog (see [LICENSING.md](LICENSING.md)).

## License

Provisionally [AGPL-3.0-only](LICENSE), pending a final decision before the first public release.
See [LICENSING.md](LICENSING.md) for what that means and what is still open.
