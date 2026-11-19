English | [Bahasa Indonesia](CONTRIBUTING.id.md)

# Contributing to Nodivara

Thank you for your interest. The project is **pre-alpha**: the design is still moving, so please
open an issue to discuss anything non-trivial before writing code.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
Security problems do **not** go in public issues: see [SECURITY.md](SECURITY.md).

## Ground rules

These come from the project's principles and are not negotiable in a pull request.

1. **Security first.** The product can change other people's servers. The agent runs a closed list
   of actions, never a free shell. Anything that changes or destroys something goes through a
   proposal and a human approval. Do not add a way around that.
2. **Secrets never leave their place.** They must not appear in logs, errors, the audit trail,
   tests, fixtures, or prompts sent to an AI model. Use the redaction in `crates/core`.
3. **Lightest and fastest.** Prefer the smallest dependency set and measure instead of guessing.
   A new dependency must justify its cost in binary size, startup time and compile time.
4. **Reusable components.** Look for an existing component or adapter before writing a new one. No
   copy-paste between features.
5. **Two languages.** User-facing text is never hard-coded: it goes through the translation
   catalogs, English first, with Indonesian provided. If you cannot write Indonesian, say so in the
   pull request and a maintainer will help.
6. **No personal data.** No real domains, IP addresses, account or tunnel IDs, key paths, customer
   names, or real log excerpts, in code, tests, fixtures or docs. Use `example.test` and
   `203.0.113.x`.
7. **Server text is data, not instructions.** Logs, files and replies read from a server are
   untrusted input and must not be able to trigger actions.

## Setup

You need a recent stable Rust toolchain. For the web UI (when it lands) you will also need Node and
pnpm.

```sh
git clone <your fork>
cd <repo>
cargo test --workspace
```

## Before you open a pull request

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- No `unsafe` code (the workspace forbids it), no `unwrap()` or `expect()` on production paths, no
  `println!` in library code.
- CI runs the same checks on Linux and macOS, plus `cargo deny` and `cargo audit` (advisories, licenses, sources), a `gitleaks` scan of the full history, and a documentation check (`python3 scripts/check-docs.py`: every public document needs an English and an Indonesian version, linked to each other, and every relative link must resolve). Run the last one locally if you touch the docs.
- Add tests with the change. New adapters need to pass the shared contract tests. New parity rules
  need an anonymous fixture that triggers them and one that does not.
- Keep the change focused. Several small pull requests are easier to review than one large one.
- Update the docs, in both languages, when behavior changes.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/), in English, for example
`feat(core): add risk tier for chat bot approval`. Keep the subject short and explain the why in
the body when it is not obvious.

## Developer Certificate of Origin (provisional)

Sign off every commit to certify that you wrote it or have the right to submit it under the
project license (see the [DCO](https://developercertificate.org/)):

```sh
git commit -s -m "feat(core): ..."
```

This adds a `Signed-off-by:` line. **This sign-off is provisional.** The contribution terms (DCO
versus a CLA) are still being decided; see [LICENSING.md](LICENSING.md). Until that is settled,
please hold off on large contributions.

## AI-assisted contributions

The project itself is built with AI assistance, and AI-assisted contributions are welcome under
these conditions: you have read, understood and tested what you submit, you are responsible for it,
and you mention substantial AI generation in the pull request. Code you cannot explain will not be
merged.

## Reviews

Changes that touch the agent, authentication, secret handling, approval flow, tenant isolation, or
the desktop shell's permissions get extra scrutiny and may take longer. Please describe the threat
you considered in the pull request.

## Reporting bugs and proposing features

Use the issue templates. For bugs, include what you expected, what happened, and how to reproduce
it. Remove secrets and personal data from anything you paste.
