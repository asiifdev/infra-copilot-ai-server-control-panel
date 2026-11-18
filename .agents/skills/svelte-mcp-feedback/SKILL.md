---
name: svelte-mcp-feedback
description: Report Svelte MCP autofixer opportunities when the user corrects a Svelte or SvelteKit mistake the agent made. Use after corrections to .svelte components, .svelte.ts/.svelte.js modules, or SvelteKit features where a reusable check or fix could prevent the mistake. Do not trigger for initial implementation requests, unrelated bugs, or personal style preferences.
---

# Svelte MCP feedback

Turn a user-corrected mistake into actionable feedback for the Svelte MCP autofixer in `sveltejs/ai-tools`.

## 1. Identify the autofixer opportunity

Use the conversation and the code you just worked on to identify:

- The incorrect code you produced and the user's correction.
- The Svelte or SvelteKit rule that explains the correction.
- A reusable code pattern the MCP could detect, and the fix or suggestion it could provide.

Include SvelteKit JavaScript/TypeScript files such as `+page.ts`, `+layout.server.ts`, `+server.ts`, `hooks.server.ts`, and `svelte.config.js`; feedback is not limited to `.svelte` files.

Only proceed if there is a concrete, generalizable Svelte/SvelteKit mistake. A changed product requirement, a personal naming preference, or an unrelated application bug does not justify an autofixer issue. If the correction is ambiguous, clarify it while continuing the user's task.

Apply the user's correction and complete the requested work. Reporting feedback should not leave the original mistake unfixed.

## 2. Capture a minimal reproduction

Reduce the before/after code to the smallest example that preserves the mistake. Use generic names and sample data instead of copying unrelated project code, credentials, or conversation history.

If the Svelte MCP is available, check its relevant documentation and run `svelte-autofixer` on the incorrect example when it supports that file type. Record whether it missed the issue, suggested an incorrect fix, or already reported the issue and you overlooked it. Do not claim the tool missed something without checking its output. If the tool is unavailable or does not support the file type, say so; a proposed new rule can still be useful.

If an existing diagnostic already clearly covers the correction, follow it instead of opening a new-rule request. Report only a concrete gap, such as an unclear diagnostic, a wrong suggested fix, or a missed case.

Prepare a title like `Autofixer: detect <incorrect pattern>` and a Markdown body with:

1. **Summary** — the mistake and why it matters.
2. **Incorrect code** — a fenced minimal example with the relevant filename/file type.
3. **User correction / expected code** — the corrected example and a brief explanation.
4. **Proposed detection and fix** — the recognizable pattern, suggested transformation or guidance, and cases where a fix would be ambiguous.
5. **Current MCP behavior** — the actual relevant output, or explicitly "not checked" with the reason.
6. **Environment** — Svelte, SvelteKit, MCP, and model versions when known; do not invent missing versions.

The report is ready when a maintainer can understand the pattern and expected outcome without access to the original project.

## 3. Save a local draft (EDITED LOCALLY: no outbound posting)

> **Local edit by the project owner.** The upstream skill creates a public GitHub issue (`gh issue create`) or opens a prefilled issue form. In this project that is disabled: nothing is sent to GitHub or any other external service by this skill.

Write the finished report (title + Markdown body from step 2) to a local file, `docs/svelte-feedback-drafts/<YYYY-MM-DD>-<short-slug>.md`, creating the folder if needed. Do **not** run `gh issue create`, do **not** open a browser or a prefilled issue URL, and do **not** call any network tool for this purpose.

Then tell the user in one or two sentences: the correction you identified, the path of the draft, and that they can review it and submit it themselves if they want. Never submit it on their behalf. Return to the user's original task.

If the user explicitly asks you to post a specific draft in the current conversation, show the exact title and body first and wait for a clear yes before doing anything outward-facing.
