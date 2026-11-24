// Pages and helpers fed with the shapes the real server sends (tests/recorded/real-api.json), not
// with shapes invented from the contract text.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { toasts } from '../src/lib/toast.svelte';
import Memory from '../src/pages/Memory.svelte';
import Plans from '../src/pages/Plans.svelte';
import Proposals from '../src/pages/Proposals.svelte';
import Transports from '../src/pages/Transports.svelte';
import { fromHistory } from '../src/lib/chat.svelte';
import { readCheck } from '../src/lib/transport';
import { formatBytes } from '../src/lib/format';
import type { ChatMessage, MemoryNote, ProposalView } from '../src/lib/types';
import { eventually, installMock, readyApp, render, resetSession } from './helpers';

const recorded = JSON.parse(readFileSync(join(import.meta.dirname, 'recorded', 'real-api.json'), 'utf8')) as Record<string, { body?: unknown }>;

afterEach(() => {
  vi.unstubAllGlobals();
  resetSession();
});

describe('transport check', () => {
  it('reads the recorded real value', () => {
    const entry = (recorded.transports?.body as { last_check: unknown }[])[0]!;
    expect(readCheck(entry.last_check)).toMatchObject({ ok: true });
  });

  it('reads the full report shape and degrades on anything else', () => {
    const full = readCheck({ ok: false, quota: { total: 2048, used: 1024, free: 1024 }, quota_unsupported: false, speed_bytes_per_sec: 4096, speed_leftover: false });
    expect(full).toEqual({ ok: false, freeBytes: 1024, totalBytes: 2048, speedBytesPerSec: 4096 });
    expect(readCheck(null)).toBeNull();
    expect(readCheck('boom')).toBeNull();
    expect(readCheck([])).toBeNull();
    expect(readCheck({ ok: 'yes', problem: 'x' })).toMatchObject({ ok: null });
    expect(readCheck({ ok: true, quota: { free: -1 } })).toMatchObject({ freeBytes: null });
  });

  it('formats bytes', () => {
    expect(formatBytes(0, 'en')).toBe('0 B');
    expect(formatBytes(1536, 'en')).toBe('1.5 KB');
    expect(formatBytes(5 * 1024 ** 3, 'id')).toBe('5 GB');
  });

  it('shows the last check readably in both languages, never [object Object]', async () => {
    for (const locale of ['en', 'id'] as const) {
      const mock = installMock();
      mock.state.transports[0]!.last_check = { ok: true, quota: { total: 2048, used: 1024, free: 1024 }, speed_bytes_per_sec: 2048 };
      await readyApp(mock, locale);
      const root = render(Transports);
      await eventually(() => expect(root.querySelectorAll('tbody tr').length).toBe(3));
      expect(root.textContent).not.toContain('[object Object]');
      const rows = [...root.querySelectorAll('tbody tr')].map((row) => row.textContent ?? '');
      const word = (en: string, id: string) => (locale === 'en' ? en : id);
      expect(rows[0]).toContain(word('Passed', 'Berhasil'));
      expect(rows[0]).toContain('1 KB');
      expect(rows[1]).toContain(word('Failed', 'Gagal'));
      expect(rows[2]).toContain(word('Never', 'Belum pernah'));
      vi.unstubAllGlobals();
    }
  });
});

describe('stored conversation rows', () => {
  const row = (seq: number, role: string, content: string, tools: string[] = []): ChatMessage => ({ seq, role, content, created_at: 'now', tools });

  it('turns the real tool row into chips on the answer that follows', () => {
    const real = recorded.messages_after_turn?.body as ChatMessage[];
    const shown = fromHistory(real);
    expect(shown.map((m) => m.role)).toEqual(['user', 'assistant']);
    expect(shown[1]?.tools).toEqual([{ name: 'list_nodes' }]);
    expect(shown[1]?.text).toContain('You have 2 servers');
  });

  it('keeps tool names of a turn that ended without an answer, and skips system and tool rows', () => {
    const shown = fromHistory([row(1, 'user', 'a'), row(2, 'assistant', '', ['x']), row(3, 'tool', 'ignored'), row(4, 'system', 's')]);
    expect(shown.map((m) => [m.role, m.text, m.tools.length])).toEqual([
      ['user', 'a', 0],
      ['assistant', '', 1],
    ]);
  });
});

describe('Memory page with real notes', () => {
  it('shows kind and source, approves a note and deletes one after confirmation', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(Memory);
    await eventually(() => expect(root.textContent).toContain('Backups are kept for 30 days.'));
    expect(root.textContent).toContain('Suggested by the assistant');
    expect(root.textContent).toContain('Personal preference');
    const approve = [...root.querySelectorAll('button')].find((b) => b.textContent?.includes('Approve')) as HTMLButtonElement;
    approve.click();
    await eventually(() => expect(mock.state.memories.filter((m: MemoryNote) => m.status === 'pending').length).toBe(1));
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'success')).toBe(true));
    const del = [...root.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Delete') as HTMLButtonElement;
    del.click();
    await eventually(() => expect(document.querySelector('[role="alertdialog"]')).not.toBeNull());
    const confirm = [...document.querySelectorAll('[role="alertdialog"] button')].pop() as HTMLButtonElement;
    confirm.click();
    await eventually(() => expect(mock.state.memories.length).toBe(3));
    expect(mock.state.requests.some((r) => r.method === 'DELETE' && /\/api\/memories\/[0-9a-f-]{36}$/.test(r.path))).toBe(true);
  });
});

describe('Proposals on the Approvals page', () => {
  it('shows title, rationale, suggested action and tool of real proposals', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(Proposals);
    await eventually(() => expect(root.textContent).toContain('Restart the web service'));
    const proposal = mock.state.proposals[0] as ProposalView;
    expect(root.textContent).toContain(proposal.rationale);
    expect(root.textContent).toContain(proposal.suggested_action);
    expect(root.textContent).toContain('propose_restart');
    expect(root.textContent).toContain('Approving records your decision. Nothing runs yet.');
  });

  it('no longer lists proposals on the Plans page, only a link to Approvals', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(Plans);
    await eventually(() => expect(root.textContent).toContain('move-alpha'));
    expect(root.textContent).not.toContain('Restart the web service');
    expect(root.querySelector('a[href="#/proposals"]')).not.toBeNull();
  });
});
