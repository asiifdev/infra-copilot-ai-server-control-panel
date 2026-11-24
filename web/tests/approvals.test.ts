// The Approvals page: filtering, the decision flow (approve, reject, the extra confirmation for a
// risky change) and what happens when the server answers 409.

import { flushSync } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProposalView } from '../src/lib/types';
import { toasts } from '../src/lib/toast.svelte';
import Proposals from '../src/pages/Proposals.svelte';
import { byText, eventually, installMock, readyApp, render, resetSession } from './helpers';

afterEach(() => {
  vi.unstubAllGlobals();
  resetSession();
});

async function open() {
  const mock = installMock();
  await readyApp(mock);
  const root = render(Proposals);
  await eventually(() => expect(root.querySelectorAll('section h2').length).toBeGreaterThan(0));
  return { mock, root };
}

function card(root: HTMLElement, title: string): HTMLElement {
  const heading = [...root.querySelectorAll('section h2')].find((h) => h.textContent?.includes(title));
  const found = heading?.closest('section');
  if (!found) throw new Error(`no card for ${title}`);
  return found;
}

function press(root: ParentNode, selector: string, text: string) {
  const button = byText(root, selector, text);
  if (!button) throw new Error(`no ${selector} with ${text}`);
  button.click();
  flushSync();
}

const decisions = (mock: ReturnType<typeof installMock>) => mock.state.requests.filter((r) => r.path.endsWith('/decision'));

describe('Approvals page', () => {
  it('lists pending proposals first and counts every state in the filter', async () => {
    const { root } = await open();
    const titles = [...root.querySelectorAll('section h2')].map((h) => h.textContent);
    expect(titles.some((x) => x?.includes('Restart the web service'))).toBe(true);
    expect(titles.some((x) => x?.includes('Move traffic'))).toBe(true);
    expect(titles.some((x) => x?.includes('Rotate the log files'))).toBe(false); // rejected
    const group = root.querySelector('[role="group"]') as HTMLElement;
    const pressed = [...group.querySelectorAll('button')].map((b) => `${b.textContent?.replace(/\s+/g, ' ').trim()}:${b.getAttribute('aria-pressed')}`);
    expect(pressed).toEqual(['Pending 2:true', 'Approved 1:false', 'Rejected 1:false', 'Expired 1:false', 'All 5:false']);
    expect(root.textContent).toContain('Approving records your decision. Nothing runs yet.');
  });

  it('switches the filter and hides the decision buttons of decided proposals', async () => {
    const { root } = await open();
    press(root, 'button', 'Expired 1');
    await eventually(() => expect(root.textContent).toContain('Clear the page cache'));
    expect(root.querySelectorAll('section button').length).toBe(0);
    press(root, 'button', 'All 5');
    expect(root.querySelectorAll('h2').length).toBe(5);
  });

  it('shows tier, rule, subject, tool and a relative expiry', async () => {
    const { root } = await open();
    const text = card(root, 'Restart the web service').textContent ?? '';
    expect(text).toContain('Tier 2');
    expect(text).toContain('Rule PAR-001');
    expect(text).toContain('Subject tcp:8080');
    expect(text).toContain('propose_restart');
    expect(text).toMatch(/Expires in 2 days|Expires in 1 day|Expires in 48 hours/);
  });

  it('approves a light change with one click, sends only the decision and says nothing ran', async () => {
    const { mock, root } = await open();
    press(card(root, 'Restart the web service'), 'button', 'Approve');
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'success' && x.text.includes('Nothing was run'))).toBe(true));
    const sent = decisions(mock);
    expect(sent.length).toBe(1);
    expect(sent[0]?.body).toEqual({ decision: 'approve' });
    expect(sent[0]?.headers['x-nodivara']).toBe('1');
    expect((mock.state.proposals[0] as ProposalView).status).toBe('approved');
    await eventually(() => expect(root.textContent).not.toContain('Restart the web service'));
  });

  it('asks for a second confirmation before approving a risky change (tier 3 or more)', async () => {
    const { mock, root } = await open();
    press(card(root, 'Move traffic'), 'button', 'Approve');
    const dialog = document.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog).not.toBeNull();
    expect(dialog.textContent).toContain('risky');
    expect(decisions(mock).length).toBe(0); // nothing sent yet
    press(dialog, 'button', 'Cancel');
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    expect(decisions(mock).length).toBe(0);
    press(card(root, 'Move traffic'), 'button', 'Approve');
    press(document.querySelector('[role="alertdialog"]') as HTMLElement, 'button', 'Yes, approve');
    await eventually(() => expect(decisions(mock).length).toBe(1));
    expect(decisions(mock)[0]?.body).toEqual({ decision: 'approve' });
    expect((mock.state.proposals[2] as ProposalView).status).toBe('approved');
  });

  it('asks before rejecting, and rejects after the confirmation', async () => {
    const { mock, root } = await open();
    press(card(root, 'Restart the web service'), 'button', 'Reject');
    const dialog = document.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog.textContent).toContain('Reject this proposal?');
    expect(decisions(mock).length).toBe(0);
    press(dialog, 'button', 'Reject');
    await eventually(() => expect(decisions(mock).length).toBe(1));
    expect(decisions(mock)[0]?.body).toEqual({ decision: 'reject' });
    expect((mock.state.proposals[0] as ProposalView).status).toBe('rejected');
    await eventually(() => expect(toasts.items.some((x) => x.text === 'Proposal rejected.')).toBe(true));
  });

  it('handles 409 proposal_decided: explains it and reloads the list', async () => {
    const { mock, root } = await open();
    (mock.state.proposals[0] as ProposalView).status = 'approved'; // someone else decided meanwhile
    press(card(root, 'Restart the web service'), 'button', 'Approve');
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'error' && x.text.includes('already decided'))).toBe(true));
    await eventually(() => expect(root.textContent).not.toContain('Restart the web service'));
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
  });

  it('handles 409 proposal_expired and keeps working', async () => {
    const { mock, root } = await open();
    (mock.state.proposals[0] as ProposalView).expires_at = '2020-01-01T00:00:00.000Z';
    press(card(root, 'Restart the web service'), 'button', 'Approve');
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'error' && x.text.includes('expired'))).toBe(true));
    await eventually(() => expect(root.textContent).not.toContain('Restart the web service'));
    expect(card(root, 'Move traffic')).toBeTruthy(); // the other card is still there
  });

  it('shows a friendly empty state when nothing is pending', async () => {
    const mock = installMock();
    mock.state.proposals = [];
    await readyApp(mock);
    const root = render(Proposals);
    await eventually(() => expect(root.textContent).toContain('You are all caught up'));
  });

  it('refuses a decision without the CSRF header and a bad decision value (mock contract)', async () => {
    const mock = installMock();
    await readyApp(mock);
    const id = (mock.state.proposals[0] as ProposalView).id;
    const noHeader = await mock.fetch(`/api/proposals/${id}/decision`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{"decision":"approve"}' });
    expect(noHeader.status).toBe(403);
    const bad = await mock.fetch(`/api/proposals/${id}/decision`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Nodivara': '1' }, body: '{"decision":"run"}' });
    expect(bad.status).toBe(400);
  });
});
