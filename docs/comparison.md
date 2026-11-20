English | [Bahasa Indonesia](comparison.id.md)

# Comparison with other server panels

Last updated: 2026-11-19. Facts below were read from the vendors' own pages on that date; prices
and features change, so check the linked sources before relying on them.

**Nodivara is pre-alpha: only a read-only parity check and a local command line run today, from source, and the AI agent and approval workflow in its row are not available.** This page is not a claim
that it is better than any product listed. The other products ship today.

## Overview

| Product | Type and license | Price (as of 2026-11-19, excluding tax) | AI features, as published by the vendor | Source |
|---|---|---|---|---|
| cPanel & WHM | Commercial panel | Solo $29.99/month (billed monthly), $27.46/month (billed annually); Admin $35.99; Pro $53.99; Premier $69.99. Prices are scheduled to change on 2026-12-16 | AI Assistant (acts on a customer's behalf, with the customer's confirmation first), MCP connectivity with scoped permissions and an audit trail, AI App Hosting, AI Builder | [pricing](https://www.cpanel.net/pricing/), [cPanel AI](https://www.cpanel.net/cpanel-ai/) |
| aaPanel | Free edition plus Pro; license listed as "Other" on GitHub | Pro: $198/year, $699 lifetime, $28.80/month | None found in the pages read | [pricing](https://www.aapanel.com/new/pricing.html) |
| 1Panel | Open core, GPL-3.0 | Pro: $80/year or $200 perpetual | Manages AI apps and agents on the server (for example OpenClaw), per its feature list | [Pro edition](https://1panel.pro/pro-edition) |
| Coolify | Open source, Apache-2.0 | Self-hosted free with all features; Cloud $5/month for 2 servers, plus $3 per extra server | None found in the pages read | [pricing](https://coolify.io/pricing) |
| Dokploy | Open core: Apache-2.0 plus a proprietary directory | Cloud Hobby $4.50/server/month; Startup from $15/month | Bring-your-own-key AI assistant; log analysis is read-only per its docs | [pricing](https://dokploy.com/pricing) |
| **Nodivara** | **Open core, provisional AGPL-3.0-only** | **Not set** | **Available from source (2026-11-20):** read-only migration parity check (7 rules). **Planned:** approval-gated AI agent, error analysis that cites evidence | [README](../README.md) |

## What Nodivara intends to do differently

These are goals, not shipped features.

- **Every state-changing action goes through a stored proposal and a human approval**, ranked in
  five risk tiers. Destructive actions need the target's name typed in.
- **A parity check before a server migration is finished**, listing what was left behind (a read-only version with seven rules runs today).
- **Error analysis that cites the evidence it used** and separates fact from guess.
- **Secrets never reach the AI model**, enforced by redaction before logging, storage and prompts.

## What the other products do well today

- cPanel, aaPanel, and 1Panel are mature panels with large user bases, and cPanel now ships AI
  features with confirmation before actions and an audit trail on its MCP connectivity.
- Coolify and Dokploy are popular self-hosted deployment platforms with free self-hosting.
- Nodivara has no users, no release, and no track record yet.

## How the facts were gathered

Vendor pricing and feature pages were read on 2026-11-19. Some feature descriptions come from the
vendors' own marketing pages, which may not show everything a product does. "None found in the
pages read" means the pages read did not mention an AI feature, not that none exists. The fuller
research, with its limits, is in [market-research.md](market-research.md).
