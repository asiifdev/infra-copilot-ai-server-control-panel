#!/usr/bin/env python3
"""PreToolUse guard for Tauri work (see CLAUDE.md "Tauri v2" and .claude/skills/tauri-v2-*).

Tauri v1 and v2 differ a lot, and v1 knowledge produces code that looks right but is wrong. This
hook does two things for Write, Edit, MultiEdit and Bash:

1. When the target is Tauri related, it injects a short reminder to load the Tauri v2 skills first.
2. When the content being written contains a pattern that exists only in Tauri v1, it denies the
   write and says what to use instead.

Escape hatch for a deliberate exception: put the text `tauri-guard: allow` in the written content.
Documentation under .claude/, docs/, research_notes/, reports/ and any *.md file is never denied,
because it describes the v1 to v2 differences on purpose.

Input: the hook JSON on stdin. Output: the PreToolUse JSON on stdout. Errors never block the tool.
"""

import json
import re
import sys

SKILLS = (
    "tauri-v2-fundamentals (dasar, command, event, state), "
    "tauri-v2-security (capabilities, permissions, CSP, AppManifest), "
    "tauri-v2-sidecar-plugins (sidecar, plugin, tray), "
    "tauri-v2-build-release (build, signing, updater, CI)"
)

REMINDER = (
    "Proyek ini memakai Tauri v2, dan pengetahuan Tauri v1 salah untuk v2. "
    "Sebelum menulis atau mengubah kode Tauri, muat skill yang relevan: " + SKILLS + ". "
    "Verifikasi API dengan dokumentasi terkini (Context7 /tauri-apps/tauri-docs), jangan dari ingatan. "
    "Command buatan sendiri boleh dipanggil semua jendela secara bawaan: batasi dengan AppManifest::commands."
)

# (regex, what to use in Tauri v2). Matched against the content being written.
V1_ONLY = [
    (r"@tauri-apps/api/tauri\b", "impor `invoke` dari `@tauri-apps/api/core`"),
    (
        r"@tauri-apps/api/(shell|fs|http|dialog|notification|clipboard|os|process|updater|globalShortcut|cli)\b",
        "modul itu pindah ke plugin resmi (`@tauri-apps/plugin-*`) dan butuh izin di capabilities",
    ),
    (r"\bappWindow\b", "gunakan `getCurrentWindow()` dari `@tauri-apps/api/window`"),
    (r"\btauri::api::", "modul `tauri::api` dihapus; pakai plugin atau `tauri::path`"),
    (r"\"allowlist\"", "allowlist v1 diganti capabilities di `src-tauri/capabilities/`"),
    (r"\bdevPath\b", "kunci build v2 adalah `devUrl`"),
    (r"\bdistDir\b", "kunci build v2 adalah `frontendDist`"),
    (r"\bapi-all\b", "fitur Cargo `api-all` tidak ada di v2; izin diatur lewat capabilities"),
]
V1_TAURI_KEY = (r"\"tauri\"\s*:\s*\{", "di `tauri.conf.json` v2 kunci `tauri` bernama `app`")

DOC_PATH = re.compile(r"(/\.claude/|/docs/|/research_notes/|/reports/|/references/|\.md$)")
TAURI_PATH = re.compile(r"(/src-tauri/|(^|/)tauri\.conf\.json5?$|(^|/)Tauri\.toml$)")
TAURI_CONTENT = re.compile(r"(@tauri-apps/|\btauri::|tauri_plugin_|tauri-plugin-)")
TAURI_BASH = re.compile(
    r"(cargo\s+tauri|(npm|pnpm|yarn|bun|npx)\s+(run\s+)?tauri\b|create-tauri-app"
    r"|(^|[\s;&|])tauri\s+(dev|build|init|add|plugin|migrate|android|ios|icon|info|signer|bundle)\b"
    r"|cargo\s+(add|install)\s+\S*tauri)"
)


def emit(payload):
    sys.stdout.write(json.dumps({"hookSpecificOutput": dict(hookEventName="PreToolUse", **payload)}))


def written_text(tool, tool_input):
    if tool == "Write":
        return tool_input.get("content") or ""
    if tool == "Edit":
        return tool_input.get("new_string") or ""
    if tool == "MultiEdit":
        return "\n".join((e or {}).get("new_string") or "" for e in tool_input.get("edits") or [])
    return ""


def main():
    try:
        data = json.load(sys.stdin)
    except Exception:
        return  # malformed input: never block the tool
    tool = data.get("tool_name")
    tool_input = data.get("tool_input") or {}

    if tool == "Bash":
        if TAURI_BASH.search(tool_input.get("command") or ""):
            emit({"additionalContext": REMINDER})
        return

    if tool not in ("Write", "Edit", "MultiEdit"):
        return

    path = tool_input.get("file_path") or ""
    text = written_text(tool, tool_input)
    related = bool(TAURI_PATH.search(path) or TAURI_CONTENT.search(text))
    if not related:
        return

    if not DOC_PATH.search(path) and "tauri-guard: allow" not in text:
        hits = [hint for pattern, hint in V1_ONLY if re.search(pattern, text)]
        if re.search(r"(^|/)tauri\.conf\.json5?$", path) and re.search(V1_TAURI_KEY[0], text):
            hits.append(V1_TAURI_KEY[1])
        if hits:
            reason = (
                "Ditolak oleh tauri-guard: konten memuat pola yang hanya ada di Tauri v1. "
                + "; ".join(hits)
                + ". Muat skill tauri-v2-fundamentals dan verifikasi dengan Context7 sebelum menulis ulang. "
                "Jika ini pengecualian yang disengaja, sertakan teks `tauri-guard: allow` di konten."
            )
            emit({"permissionDecision": "deny", "permissionDecisionReason": reason})
            return

    emit({"additionalContext": REMINDER})


if __name__ == "__main__":
    main()
