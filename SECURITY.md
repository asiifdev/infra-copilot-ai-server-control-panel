English | [Bahasa Indonesia](SECURITY.id.md)

# Security policy

Infra Copilot is designed to hold privileged access to servers, so a vulnerability here can be
serious. We take reports seriously and ask you to report them privately.

## Status

The project is **pre-alpha** and has no released version, so no version is supported yet. Once
releases exist, this section will list which versions receive security fixes.

## How to report

**Do not open a public issue, pull request or discussion for a security problem.**

Report privately using one of these:

1. **GitHub private vulnerability reporting** on this repository (the "Report a vulnerability"
   button under the Security tab), once it is enabled.
2. **Email:** **[security contact: to be set by the maintainer before the repository is made
   public]**

Please include what you found, where, how to reproduce it, and what you think the impact is. Do not
include real secrets, real customer data, or access to systems you do not own.

## What we especially want to hear about

- Anything that lets an action run **without the required approval**, or lowers an action's risk
  tier.
- Any way to read, log, or leak a **secret**, or send one to an AI model.
- **Prompt injection** that makes the agent act on text read from a server, a log, or a file.
- Breaking **isolation between organizations** (tenant isolation).
- Command or argument injection through the agent, the sidecar, or the desktop shell.
- Weak spots in authentication, sessions, the Telegram bot's account linking and callbacks, or
  update signing.
- Overly broad permissions in the desktop shell's capabilities.

## What to expect

We will acknowledge your report, investigate, keep you informed, and credit you in the fix if you
wish. Specific response-time targets will be published at the first release; until then we make no
promise about timing beyond a good-faith effort.

## Responsible disclosure

Please give us a reasonable chance to fix the problem before you disclose it publicly, and avoid
accessing data that is not yours, degrading a service, or testing against systems you do not own.
This policy is not legal advice and does not grant permission for testing against third parties.
