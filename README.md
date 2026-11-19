English | [Bahasa Indonesia](README.id.md)

<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="brand/nodivara-lockup-dark.png">
  <img src="brand/nodivara-lockup-light.png" alt="Nodivara logo: a lime and silver ribbon forming the letter N, next to the word Nodivara" width="620">
</picture>

### The server control panel with an AI agent that proposes, and **you** approve.

[![CI](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/actions/workflows/ci.yml/badge.svg)](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/actions/workflows/ci.yml)
![Status: pre-alpha](https://img.shields.io/badge/status-pre--alpha-b6f205?style=flat-square&labelColor=0d1020)
![License: AGPL-3.0-only, provisional](https://img.shields.io/badge/license-AGPL--3.0--only%20%28provisional%29-b6f205?style=flat-square&labelColor=0d1020)
![Built with Rust and Svelte 5](https://img.shields.io/badge/built%20with-Rust%20%2B%20Svelte%205-b6f205?style=flat-square&labelColor=0d1020)
![Languages: English and Indonesian](https://img.shields.io/badge/UI-EN%20%7C%20ID-b6f205?style=flat-square&labelColor=0d1020)

[What works today](#what-works-today) · [How it works](#how-it-works) · [Built to be trusted](#built-to-be-trusted) · [Roadmap](#roadmap) · [FAQ](docs/faq.md) · [Compare](docs/comparison.md)

</div>

---

**Nodivara is an open-source server control panel, aimed at the same job as aaPanel and cPanel, with a built-in AI agent that proposes changes and waits for human approval.** It is written in Rust with a Svelte interface, targets Linux servers and a desktop app, and is pre-alpha: nothing is installable yet.

Last updated: 2026-11-19.

## What works today

> **Honest status.** There is no installable release, no agent and no panel yet. What exists is a read-only inventory and a parity check you can run from source, plus the libraries the uptime monitor will be built from.

| Works now (from source) | Built as libraries, not running yet | Planned |
|---|---|---|
| Collect a Linux inventory, read-only: ports, processes, cron, PM2, systemd, Docker | Uptime checks (HTTP, TCP, TLS expiry) with SSRF protection | The AI agent and approval workflow |
| Compare two inventories with **7 parity rules** and read the report in English or Indonesian | Incident engine and Telegram alerts | The web panel and desktop app |
| Exit code 1 when a critical gap is found, so it fits a migration checklist | Database schema with tenant isolation and a hash-chained audit log | Git deploys, DNS and Cloudflare Tunnel |

### Try it

```sh
git clone https://github.com/asiifdev/infra-copilot-ai-server-control-panel.git
cd infra-copilot-ai-server-control-panel

cargo run -q -p cli -- collect > source.json        # on the old server (read-only)
cargo run -q -p cli -- collect > target.json        # on the new server
cargo run -q -p cli -- compare source.json target.json --lang en
```

Real output, from the anonymous test fixtures in this repository (shortened):

```text
6 finding(s) from 7 rule(s):

[Critical] PAR-001: Port 7700 has no counterpart on the target
  On the source, searchd listens on port 7700 (bind address 127.0.0.1). The target has no listener
  on that port. If searchd did not move, whatever depends on it will fail after cutover; ...
  Advice: Check whether searchd is meant to run on the target. If so, install and start it there ...
  (listener tcp:7700)

[Important] PAR-008: A scheduled job has no counterpart on the target
  The source runs a scheduled job for root (0 3 * * *): backup-db. The target has no matching job. ...

...
This is a read-only comparison of two inventories. It does not prove the servers are equivalent:
only these rules were checked: PAR-001, PAR-002, PAR-006, PAR-007, PAR-008, PAR-009, PAR-010.
```

The report says what it did **not** check. That honesty is deliberate: a parity tool that claims more than it proves is worse than none.

## How it works

<p align="center">
  <img src="docs/assets/operating-loop.svg" alt="The Nodivara operating loop in seven steps: connect, understand, diagnose, propose, approve, execute, verify. Understand is available now; connect and diagnose are being built; the rest are planned. Approve is the human step." width="960">
</p>

The agent will not get a shell. It will pick from a **closed list of actions**, ranked from read-only to destructive. Anything from tier 2 up becomes a **Proposal** with its reason, target, risk and impact, and waits for you.

| Tier | Example | How it runs |
|---|---|---|
| 0 Read | list services, read logs, collect inventory | automatically; no secrets |
| 1 Plan | draft a migration plan | automatically; only creates a Proposal, changes nothing |
| 2 Light change | restart a non-critical app | Proposal, one-click approval |
| 3 Risky change | edit `.env`, change DNS, restore a database | Proposal, dry-run, impact summary, automatic backup; approved **in the panel only** |
| 4 Destructive | delete data, `DROP`, delete backups | Proposal, type the target's name, a second person in the cloud edition; **panel only**, never automatic |

*This is the design (see [docs/plan.md](docs/plan.md)); the tiers and redaction types exist in code, the agent that uses them does not yet.*

## Why Nodivara

| | What it means | Status |
|---|---|---|
| **Approval-gated AI** | The agent shows reason, target, risk and impact. Nothing runs until you approve. | planned |
| **Migration parity** | Compare the old and new server and find what was left behind: a service nobody listed, a `localhost` pointing at the wrong machine, a DNS record that never moved. | **works today** (7 rules, read-only) |
| **Evidence-based diagnosis** | The AI must cite the logs, inventory and processes behind a diagnosis, and say when evidence is missing. | planned |

Nodivara is not a feature-count race. For how it differs from other panels, and what we do **not** claim, read the [comparison](docs/comparison.md).

## Built to be trusted

A tool that can change servers has to earn trust with rules you can check. These are enforced in this repository today, with links to the code or decision:

| Rule | Where it holds | State |
|---|---|---|
| No free shell: only a fixed list of read-only commands, no shell, time and size limits | [`collectors/exec.rs`](crates/collectors/src/exec.rs), [decision 0014](docs/decisions/0014-pelaksana-perintah-terbatas.md) | implemented, tested |
| Secrets never in logs, reports or AI input | [`core/redact.rs`](crates/core/src/redact.rs); environment values are parsed in memory and dropped ([0015](docs/decisions/0015-env-ref-dan-versi-layanan.md)) | implemented, tested |
| Text from servers is data, never instructions | alerts carry closed enums, never server text ([`monitor`](crates/monitor), [`notifier`](crates/notifier)) | implemented, tested |
| The monitor cannot be turned into an SSRF probe | address check at the exact connect call, redirect and size caps ([`net`](crates/net), [0016](docs/decisions/0016-klien-http-tls-monitor.md)) | implemented, tested |
| A planted symlink cannot redirect a read | path walk by directory descriptor, devices never opened ([`rootfs.rs`](crates/collectors/src/rootfs.rs), [0017](docs/decisions/0017-rustix-untuk-rootfs.md)) | implemented, tested |
| Tenants cannot see each other's data; tampering shows | tenant-scoped queries and a hash-chained audit log ([`db`](crates/db)) | implemented, tested |
| Supply chain watched on every push | `cargo deny`, `cargo audit`, full-history secret scan, Linux and macOS tests | in [CI](.github/workflows/ci.yml) |
| Human approval for anything that changes a server | proposal flow | **planned** with the agent |

Found a vulnerability? Please report it privately: see [SECURITY.md](SECURITY.md).

## Roadmap

| Stage | What | Status |
|---|---|---|
| M0 Foundation | domain types, i18n, database schema, size and speed budget | done |
| M1 Inventory and parity, read-only | collectors, parity rules, CLI, uptime monitor and Telegram alerts | in progress: collectors, rules and CLI done; monitor libraries done, not wired in |
| M2 Read-only AI assistant | OpenAI-compatible provider, chat, memory, tools of tier 0-1 | planned |
| M3 Migration planner | wizard, backup and transport, written plan | planned |
| M4 Execution with approval | tiers 2-4, dry-run, rollback, verification, audit | planned |
| M5 Server panel | sites, PHP versions, databases | planned |
| M6 Commercial | licenses, hosted cloud, billing | planned |
| M7 Open release | cleanup, documentation, final license and name check | planned |

## Principles

- **Lightest and fastest.** One Rust binary, a Svelte 5 UI compiled into it, SQLite by default. Desktop edition with Tauri. No extra runtime on your server just to run the panel.
- **Least privilege.** A closed list of actions, ranked by risk, with approval for anything that changes something.
- **Secrets stay secret.** Never logged, never shown, never sent to an AI model.
- **Open core, honestly.** The core is open source. Paid editions will be offered separately; nothing in the open core is crippled on purpose.
- **English and Indonesian from day one.** All user-facing text goes through translation catalogs.

## Quick answers

**Is Nodivara ready to use?** No. It is pre-alpha. The read-only inventory and parity check run from source; there is no installer, agent or panel yet.

**Will the AI change my server by itself?** By design, no: it proposes, you approve, and the riskiest tiers can only be approved in the panel. The agent does not exist yet, so today nothing can change a server.

**What is it written in?** Rust for the core and agent, Svelte 5 for the interface, SQLite by default.

More in the [FAQ](docs/faq.md).

<details>
<summary><b>Repository layout</b></summary>

```
crates/core         shared domain types: risk tiers, secret redaction (no external dependencies)
crates/i18n         en/id message catalog with stable keys and fallback
crates/inventory    inventory model and the Collector / ParityRule contracts
crates/collectors   read-only Linux collectors (listeners, processes, cron, PM2, systemd, Docker)
crates/parity       parity rules PAR-001, PAR-002, PAR-006, PAR-007, PAR-008, PAR-009, PAR-010, comparer and localized report
crates/net          SSRF-safe async HTTP/TLS client (certificate inspection) for the monitor
crates/monitor      uptime incident engine (thresholds, maintenance, uptime %), pure logic
crates/notifier     NotificationChannel contract, Telegram send channel, localized alerts
crates/watch        glue: probes via net, incidents via monitor, Telegram alerts via notifier (library)
crates/cli          `infra collect` and `infra compare` (offline, read-only)
crates/db           SQLite schema, tenant-scoped queries, hash-chained audit log
crates/proto-server throwaway prototype used to measure binary size, memory and startup
ui/                 Svelte 5 + Vite prototype page (embedded into the prototype binary)
brand/              Nodivara logo files (optimized); colours and usage in docs/brand.md
docs/design.md      technical design: schema, inventory format, agent protocol, approval flow
docs/decisions/     architecture decision records
docs/market-research.md   market research with facts separated from inference
scripts/            helper scripts (including measure.sh for the performance budget)
.claude/ .agents/   configuration and skills for AI-assisted development
```

</details>

## Build and test

Requires a recent stable Rust toolchain.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Documentation

- [FAQ](docs/faq.md) | [Comparison with other panels](docs/comparison.md) | [Brand and colours](docs/brand.md)
- [Design](docs/design.md) and [decision records](docs/decisions/)
- [Contributing](CONTRIBUTING.md) | [Code of Conduct](CODE_OF_CONDUCT.md) | [Security policy](SECURITY.md)
- [Licensing](LICENSING.md) | [Funding and support](FUNDING.md) | [Changelog](CHANGELOG.md)

Each document has an Indonesian version next to it (`*.id.md`), except the license text and the changelog (see [LICENSING.md](LICENSING.md)).

## License

Provisionally [AGPL-3.0-only](LICENSE), pending a final decision before the first public release. See [LICENSING.md](LICENSING.md) for what that means and what is still open. The name "Nodivara" is chosen by the owner, who reports it is still available and unique; no formal trademark search has been done yet.
