English | [Bahasa Indonesia](faq.id.md)

# Frequently asked questions

Last updated: 2026-11-20. Nodivara is **pre-alpha**: features described here as planned are not
available yet.

## What is Nodivara?

Nodivara is an open-source server control panel, aimed at the same job as aaPanel and cPanel,
with a built-in AI agent that proposes changes and waits for human approval. It is written in Rust
with a Svelte interface, targets Linux servers and a desktop app, and is pre-alpha: nothing is
installable yet.

## Can I install it today?

No. There is no release and no installer. From source you can run a read-only inventory, a
parity check of two inventories and a local command line (`infra`) that stores inventories, keeps
migration plan drafts and transport settings, and can talk to an AI provider you configure. The
uptime monitor is a library nothing runs yet, and nothing can approve or execute a change.
Everything else in the README is planned.

## What is an AI server management agent, in this project?

It is an AI model, using the OpenAI-compatible provider you configure, that can read the state of
your server through a closed list of tools and propose actions. It does not run a free shell. Any
action that changes or destroys something becomes a proposal that a person approves first.

## Can the AI run commands on my server on its own?

No for anything that changes a server. Actions are ranked in five tiers, from read-only to
destructive. Only tiers 0 and 1 (reading and planning) run automatically. From tier 2 up, a stored
proposal needs a human approval, and destructive actions also need the target's name typed in.

## Are my secrets sent to the AI model?

By design, no. Secrets such as passwords in URLs, `.env` values, tokens and private keys are
redacted before anything is logged, stored, shown, or sent to a model. The redaction code exists in
this repository and has tests, and provider keys and transport credentials are encrypted at rest
and never accepted as command-line arguments. The assistant has not yet been run against a live
provider, so the whole path is tested only against local test doubles.

## How is it different from aaPanel, cPanel, 1Panel, Coolify or Dokploy?

Those products exist and ship today; Nodivara does not yet. The intended differences are the
audited approval workflow for AI actions, a pre-cutover parity check for server migrations, and an
error analysis that cites its evidence. See the dated, sourced [comparison](comparison.md).

## What is a migration parity check?

It compares what runs on the old server with what runs on the new one and reports what was left
behind before you switch the old one off: a service nobody listed, a `localhost` that points at the
wrong machine, a DNS record that never moved. A read-only version with seven rules runs today from
source (`infra collect` and `infra compare`); it does not prove two servers are equivalent, and the
report lists which rules were checked. More rules and the approval-gated parts are planned.

## Which operating systems does it support?

The target is Linux servers first. A desktop edition for macOS, Linux and Windows is planned using
Tauri. Linux comes first, then macOS, then Windows. None is released.

## Which programming languages and frameworks can it deploy?

Planned: Node, PHP, Python, Go and .NET, with stack detection from the repository and a one-click
deploy. This is not built yet.

## Does it support Indonesian and English?

Yes by design: all user-facing text goes through translation catalogs in English and Indonesian,
and every public document has both versions.

## What license is it under?

Provisionally AGPL-3.0-only, pending a lawyer's review before the first public release. See
[LICENSING.md](../LICENSING.md) for the plain-language summary and the open questions.

## Will there be paid editions?

Planned, as an open-core model: a free open-source core, plus separately offered paid editions
(extra limits, licensing, hosted cloud). Nothing in the open core is meant to be disabled on
purpose. Prices are not set.

## How can I contribute or report a security problem?

Read [CONTRIBUTING.md](../CONTRIBUTING.md) for contributions. Report security problems privately,
never in a public issue; see [SECURITY.md](../SECURITY.md).
