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

async function open(options?: Parameters<typeof installMock>[0]) {
  const mock = installMock(options);
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
    expect(pressed).toEqual(['Pending 2:true', 'Approved 4:false', 'Rejected 1:false', 'Expired 1:false', 'All 8:false']);
    expect(root.textContent).toContain('Approving records your decision. Nothing runs yet.');
  });

  it('switches the filter and hides the decision buttons of decided proposals', async () => {
    const { root } = await open();
    press(root, 'button', 'Expired 1');
    await eventually(() => expect(root.textContent).toContain('Clear the page cache'));
    expect(root.querySelectorAll('section button').length).toBe(0);
    press(root, 'button', 'All 8');
    expect(root.querySelectorAll('h2').length).toBe(8);
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

const runs = (mock: ReturnType<typeof installMock>) => mock.state.requests.filter((r) => r.path.endsWith('/execute'));
const dialogOf = () => document.querySelector('[role="alertdialog"]') as HTMLElement;

async function openApproved(options?: Parameters<typeof installMock>[0]) {
  const opened = await open(options);
  press(opened.root, 'button', 'Approved 4');
  await eventually(() => expect(opened.root.textContent).toContain('Restart nginx'));
  return opened;
}

describe('Running an approved action', () => {
  it('keeps executed and failed proposals findable under Approved, with the same counts', async () => {
    const { root } = await openApproved();
    const titles = [...root.querySelectorAll('section h2')].map((h) => h.textContent);
    expect(titles).toEqual(expect.arrayContaining(['Add a swap file', 'Restart nginx', 'Restart the queue worker', 'Restart the mail relay']));
    expect(titles.length).toBe(4);
  });

  it('shows the action as a fact: what, which manager, which node', async () => {
    const { root } = await openApproved();
    expect(card(root, 'Restart nginx').textContent).toContain('Restart nginx.service (systemd) on this-mac');
  });

  it('disables Run and explains how to turn execution on when the server has it off', async () => {
    const { root, mock } = await openApproved();
    const nginx = card(root, 'Restart nginx');
    const button = byText(nginx, 'button', 'Run') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    const hint = nginx.querySelector(`#${button.getAttribute('aria-describedby')}`) as HTMLElement;
    expect(hint.textContent).toContain('Running is switched off on this server');
    expect(hint.querySelector('code')?.textContent).toBe('infra serve --enable-execution');
    button.click();
    flushSync();
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    expect(runs(mock).length).toBe(0);
  });

  it('says that a recommendation without an action has nothing to run', async () => {
    const { root } = await openApproved({ executionEnabled: true });
    const swap = card(root, 'Add a swap file');
    expect(swap.textContent).toContain('This is a recommendation only; there is nothing to run.');
    expect(swap.querySelector('button')).toBeNull();
  });

  it('offers Run only on approved proposals with an action', async () => {
    const { root } = await openApproved({ executionEnabled: true });
    const withRun = [...root.querySelectorAll('section')].filter((c) => byText(c, 'button', 'Run')).map((c) => c.querySelector('h2')?.textContent);
    expect(withRun).toEqual(['Restart nginx']);
  });

  it('asks first with a dialog that says exactly what happens, and cancel sends nothing', async () => {
    const { root, mock } = await openApproved({ executionEnabled: true });
    const button = byText(card(root, 'Restart nginx'), 'button', 'Run') as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    expect(button.hasAttribute('aria-describedby')).toBe(false);
    press(card(root, 'Restart nginx'), 'button', 'Run');
    const dialog = dialogOf();
    expect(dialog.querySelector('h2')?.textContent).toBe('Run this action now?');
    const text = dialog.textContent ?? '';
    expect(text).toContain('restart nginx.service (systemd) on this-mac');
    expect(text).toContain('briefly unavailable');
    expect(text).toContain('cannot be undone');
    expect(text).toContain('never retried automatically');
    expect(runs(mock).length).toBe(0);
    // A deliberate action, not a destructive-looking button.
    expect(byText(dialog, 'button', 'Yes, run it now')?.className).toContain('primary');
    press(dialog, 'button', 'Cancel');
    expect(dialogOf()).toBeNull();
    expect(runs(mock).length).toBe(0);
    expect((mock.state.proposals[5] as ProposalView).status).toBe('approved');
  });

  it('runs after the confirmation, waits for the server, then shows the verified result', async () => {
    const { root, mock } = await openApproved({ executionEnabled: true });
    press(card(root, 'Restart nginx'), 'button', 'Run');
    press(dialogOf(), 'button', 'Yes, run it now');
    await eventually(() => expect(runs(mock).length).toBe(1));
    expect(runs(mock)[0]?.body).toBeUndefined();
    expect(runs(mock)[0]?.headers['x-nodivara']).toBe('1');
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'success' && x.text.includes('ran and was verified'))).toBe(true));
    expect((mock.state.proposals[5] as ProposalView).status).toBe('executed');
    await eventually(() => expect(byText(card(root, 'Restart nginx'), 'button', 'Run')).toBeNull());
    const text = card(root, 'Restart nginx').textContent ?? '';
    expect(text).toContain('Succeeded');
    expect(text).toContain('Verified: the service was active afterwards.');
    expect(dialogOf()).toBeNull();
  });

  it('reports a failed run: error toast, Failed badge, the reason sentence and the output', async () => {
    const { root, mock } = await openApproved({ executionEnabled: true, executionFails: true });
    press(card(root, 'Restart nginx'), 'button', 'Run');
    press(dialogOf(), 'button', 'Yes, run it now');
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'error' && x.text.includes('The action failed'))).toBe(true));
    expect((mock.state.proposals[5] as ProposalView).status).toBe('failed');
    await eventually(() => expect(card(root, 'Restart nginx').textContent).toContain('The command failed. See the output below.'));
    const text = card(root, 'Restart nginx').textContent ?? '';
    expect(text).toContain('Failed');
    expect(text).toContain('Not verified');
    expect(text).toContain('Failed to restart nginx.service: Unit not found.');
    expect(byText(card(root, 'Restart nginx'), 'button', 'Run')).toBeNull(); // never retried
  });

  const conflicts: [string, (p: ProposalView, mock: ReturnType<typeof installMock>) => void, string][] = [
    ['execution_disabled', (_p, mock) => (mock.state.executionEnabled = false), 'switched off'],
    ['proposal_already_run', (p) => (p.status = 'executed'), 'already run'],
    ['proposal_not_approved', (p) => (p.status = 'rejected'), 'Only an approved proposal'],
    ['proposal_expired', (p) => (p.expires_at = '2020-01-01T00:00:00.000Z'), 'expired'],
    ['proposal_no_action', (p) => (p.action = null), 'recommendation only'],
  ];
  for (const [code, change, message] of conflicts) {
    it(`handles 409 ${code}: explains it, closes the dialog and reloads the list`, async () => {
      const { root, mock } = await openApproved({ executionEnabled: true });
      press(card(root, 'Restart nginx'), 'button', 'Run');
      change(mock.state.proposals[5] as ProposalView, mock); // the world changed while the dialog was open
      press(dialogOf(), 'button', 'Yes, run it now');
      await eventually(() => expect(toasts.items.some((x) => x.kind === 'error' && x.text.includes(message))).toBe(true));
      expect(dialogOf()).toBeNull();
      expect(runs(mock).length).toBe(1);
      expect((mock.state.proposals[5] as ProposalView).execution).toBeNull(); // nothing ran
    });
  }

  it('answers the mock contract: CSRF header, unknown id, and the stored errors', async () => {
    const mock = installMock({ executionEnabled: true });
    await readyApp(mock);
    const id = (mock.state.proposals[5] as ProposalView).id;
    const noHeader = await mock.fetch(`/api/proposals/${id}/execute`, { method: 'POST' });
    expect(noHeader.status).toBe(403);
    const unknown = await mock.fetch('/api/proposals/nope/execute', { method: 'POST', headers: { 'X-Nodivara': '1' } });
    expect(unknown.status).toBe(404);
    const pending = await mock.fetch(`/api/proposals/${(mock.state.proposals[0] as ProposalView).id}/execute`, { method: 'POST', headers: { 'X-Nodivara': '1' } });
    expect(pending.status).toBe(409);
    expect(((await pending.json()) as { error: { code: string } }).error.code).toBe('proposal_not_approved');
    const swap = await mock.fetch(`/api/proposals/${(mock.state.proposals[4] as ProposalView).id}/execute`, { method: 'POST', headers: { 'X-Nodivara': '1' } });
    expect(((await swap.json()) as { error: { code: string } }).error.code).toBe('proposal_no_action');
    const ok = await mock.fetch(`/api/proposals/${id}/execute`, { method: 'POST', headers: { 'X-Nodivara': '1' } });
    expect(ok.status).toBe(200);
    expect(((await ok.json()) as { status: string }).status).toBe('executed');
    const again = await mock.fetch(`/api/proposals/${id}/execute`, { method: 'POST', headers: { 'X-Nodivara': '1' } });
    expect(((await again.json()) as { error: { code: string } }).error.code).toBe('proposal_already_run');
  });

  it('shows executed and failed results with badge, verified, reason, time and output', async () => {
    const { root } = await openApproved();
    const done = card(root, 'Restart the queue worker').textContent ?? '';
    expect(done).toContain('Succeeded');
    expect(done).toContain('Verified: the service was active afterwards.');
    expect(done).toContain('The action ran and its result was checked.');
    expect(done).toMatch(/Finished .*2026/);
    expect(done).toContain('active');
    const failed = card(root, 'Restart the mail relay').textContent ?? '';
    expect(failed).toContain('Failed');
    expect(failed).toContain('The command failed. See the output below.');
    expect(failed).toContain('Unit mail-relay.service not found.');
  });

  it('renders hostile output and service names as inert text', async () => {
    const { root } = await open({ hostile: true });
    press(root, 'button', 'Approved 4');
    await eventually(() => expect(root.querySelectorAll('section h2').length).toBe(4));
    expect(root.querySelector('script, img, iframe, svg:not([data-icon]), a')).toBeNull();
    expect(root.textContent).toContain('<script>window.__pwned = true</script>');
    expect((window as unknown as { __pwned?: boolean }).__pwned).toBeUndefined();
  });

  it('counts approved actions in the same way after running (nothing disappears)', async () => {
    const { root } = await openApproved({ executionEnabled: true });
    press(card(root, 'Restart nginx'), 'button', 'Run');
    press(dialogOf(), 'button', 'Yes, run it now');
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'success')).toBe(true));
    await eventually(() => {
      const group = root.querySelector('[role="group"]') as HTMLElement;
      expect(group.textContent?.replace(/\s+/g, ' ')).toContain('Approved 4');
    });
  });
});
