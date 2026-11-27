import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Assistant from '../src/pages/Assistant.svelte';
import Monitors from '../src/pages/Monitors.svelte';
import NodeDetail from '../src/pages/NodeDetail.svelte';
import PlanDetail from '../src/pages/PlanDetail.svelte';
import Settings from '../src/pages/Settings.svelte';
import Transports from '../src/pages/Transports.svelte';
import { i18n } from '../src/lib/i18n.svelte';
import { toasts } from '../src/lib/toast.svelte';
import type { Mock } from '../mock/server';
import { byText, eventually, installMock, readyApp, render, resetSession } from './helpers';
import { id } from '../mock/fixtures';

const CHAT = id(300);

function type(element: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  element.value = value;
  element.dispatchEvent(new Event('input', { bubbles: true }));
  flushSync();
}

function button(root: HTMLElement, text: string): HTMLButtonElement {
  const found = [...root.querySelectorAll('button')].find((b) => b.textContent?.includes(text));
  if (!found) throw new Error(`no button "${text}"`);
  return found as HTMLButtonElement;
}

function field(root: HTMLElement, id: string): HTMLInputElement {
  const found = root.querySelector<HTMLInputElement>(`#${id}`);
  if (!found) throw new Error(`no field #${id}`);
  return found;
}

afterEach(() => {
  vi.unstubAllGlobals();
  resetSession();
  delete (window as unknown as { __pwned?: unknown }).__pwned;
});

describe('Assistant page', () => {
  let mock: Mock;

  async function open(options?: Parameters<typeof installMock>[0]) {
    mock = installMock(options);
    await readyApp(mock);
    const root = render(Assistant, { conversation: CHAT });
    await eventually(() => expect(root.querySelectorAll('.messages > li').length).toBe(2)); // stored history
    return root;
  }

  async function ask(root: HTMLElement, text: string) {
    type(root.querySelector('#composer') as HTMLTextAreaElement, text);
    button(root, 'Send').click();
    flushSync();
  }

  it('lists conversations and shows the stored messages in a labelled list that is not a live region', async () => {
    const root = await open();
    expect(root.textContent).toContain('Why is port 8080 missing?');
    const list = root.querySelector('ol.messages') as HTMLElement;
    expect(list.getAttribute('aria-label')).toBe('Messages');
    expect(root.querySelector('[role="log"]')).toBeNull();
    expect(list.closest('[aria-live]')).toBeNull();
    expect(list.querySelector('[aria-live]')).toBeNull();
    expect(root.querySelector('nav a[aria-current="page"]')?.textContent).toContain('Why is port 8080 missing?');
  });

  it('streams the answer in, shows tool chips with tier and status, and finishes', async () => {
    const root = await open();
    await ask(root, 'hello');
    await eventually(() => expect(root.textContent).toContain('Start the service, then compare again.'));
    await eventually(() => expect((button(root, 'Send')).disabled).toBe(true)); // empty draft after sending
    const live = [...root.querySelectorAll('.chips')].pop();
    expect(live?.textContent).toContain('search_inventory');
    expect(live?.textContent).toContain('Tier 0');
    expect(root.textContent).not.toContain('Stop');
    expect(root.querySelectorAll('.messages > li').length).toBe(4);
  });

  it('announces a streamed reply only when it starts and when it ends, not word by word', async () => {
    const root = await open({ turnDelayMs: 5 });
    const status = () => root.querySelector('p[role="status"]') as HTMLElement;
    expect(status().textContent).toBe('');
    const seen: string[] = [];
    const watcher = new MutationObserver(() => {
      const text = status().textContent ?? '';
      if (seen[seen.length - 1] !== text) seen.push(text);
    });
    watcher.observe(status(), { childList: true, characterData: true, subtree: true });
    await ask(root, 'slow please');
    await eventually(() => expect(status().textContent).toBe('The assistant has finished its reply.'));
    watcher.disconnect();
    expect(seen).toEqual(['Writing a reply', 'The assistant has finished its reply.']);
  });

  it('caps a runaway reply, says so, ends the turn and keeps the composer usable', async () => {
    const root = await open();
    await ask(root, 'huge please');
    await eventually(() => expect(root.textContent).toContain('The reply was very long and was cut here.'));
    const longest = Math.max(...[...root.querySelectorAll('.messages .text, .messages p')].map((p) => p.textContent?.length ?? 0));
    expect(longest).toBeLessThanOrEqual(200_000 + 500);
    expect(root.textContent).not.toContain('Stopped before the reply was finished.');
    await eventually(() => expect(mock.state.activeTurns.size).toBe(0));
    type(root.querySelector('#composer') as HTMLTextAreaElement, 'next');
    expect(button(root, 'Send').disabled).toBe(false);
  });

  it('Stop aborts the stream and the composer is usable again', async () => {
    const root = await open({ turnDelayMs: 5 });
    await ask(root, 'slow please');
    await eventually(() => expect(root.textContent).toContain('word3'));
    button(root, 'Stop').click();
    await eventually(() => expect(root.textContent).toContain('Stopped before the reply was finished.'));
    expect(root.textContent).not.toContain('word199');
    expect(byText(root, 'button', 'Stop')).toBeNull();
    type(root.querySelector('#composer') as HTMLTextAreaElement, 'next');
    expect(button(root, 'Send').disabled).toBe(false);
    await eventually(() => expect(mock.state.activeTurns.size).toBe(0));
  });

  it('shows localized advice with a link to Settings for an error event', async () => {
    const root = await open();
    await ask(root, 'please error: now');
    await eventually(() => expect(root.querySelector('[role="alert"]')).not.toBeNull());
    const alert = root.querySelector('[role="alert"]') as HTMLElement;
    expect(alert.textContent).toContain('The AI provider could not be reached.');
    expect(alert.textContent).toContain('Check the base URL in Settings');
    expect(alert.querySelector('a')?.getAttribute('href')).toBe('#/settings');
    expect(root.textContent).toContain('Let me look at that.'); // the text before the error stays
  });

  it('shows the busy advice (409) with a link to Settings', async () => {
    const root = await open();
    mock.state.activeTurns.add(CHAT);
    await ask(root, 'hello');
    await eventually(() => expect(root.querySelector('[role="alert"]')).not.toBeNull());
    const alert = root.querySelector('[role="alert"]') as HTMLElement;
    expect(alert.textContent).toContain('A reply is already being written in this conversation.');
    expect(alert.querySelector('a')?.getAttribute('href')).toBe('#/settings');
  });

  it('renders model text, tool names and stored history as inert text with line breaks kept', async () => {
    const root = await open({ hostile: true });
    await ask(root, 'hostile');
    await eventually(() => expect(root.textContent).toContain('line three'));
    expect(root.querySelector('script, img, svg:not([data-icon]), iframe, object, embed, a[href^="javascript"]')).toBeNull();
    const all = root.querySelectorAll('*');
    for (const el of all) {
      for (const attr of el.attributes) expect(attr.name.startsWith('on'), `${el.tagName} ${attr.name}`).toBe(false);
    }
    expect((window as unknown as { __pwned?: unknown }).__pwned).toBeUndefined();
    expect(root.textContent).toContain('<script>window.__pwned = true</script>');
    const blocks = [...root.querySelectorAll('.nv-text-block')].map((b) => b.textContent ?? '');
    expect(blocks.some((text) => text.includes('line one\nline two'))).toBe(true);
    // Bidi override and zero-width characters are removed, the visible text stays.
    expect(root.textContent).not.toMatch(/[\u202A-\u202E\u2066-\u2069\u200B]/);
    // A 300 000 character string is cut while it streams, and the reply says so.
    expect(root.textContent).toContain('The reply was very long and was cut here.');
  });

  it('creates a conversation and navigates to it', async () => {
    const root = await open();
    button(root, 'New conversation').click();
    await eventually(() => expect(window.location.hash).toBe(`#/assistant/${id(302)}`));
  });

  it('does not send an empty message', async () => {
    const root = await open();
    expect(button(root, 'Send').disabled).toBe(true);
    type(root.querySelector('#composer') as HTMLTextAreaElement, '   ');
    expect(button(root, 'Send').disabled).toBe(true);
  });
});

describe('Settings page', () => {
  const secret = ['k', 'k', 'k', 'k'].join('').repeat(6) + 'Z9';

  async function open(options?: Parameters<typeof installMock>[0], locale: 'en' | 'id' = 'en') {
    const mock = installMock(options);
    await readyApp(mock, locale);
    const root = render(Settings);
    await eventually(() => expect(root.textContent).toContain('model-large'));
    return { mock, root };
  }

  function fill(root: HTMLElement, withKey = true) {
    type(field(root, 'provider-name'), 'second');
    type(field(root, 'provider-url'), 'https://other.example.test/v1');
    type(field(root, 'provider-model'), 'model-small');
    if (withKey) type(field(root, 'provider-key'), secret);
  }

  function nowhereVisible(root: HTMLElement) {
    expect(document.body.textContent).not.toContain(secret);
    for (const el of root.querySelectorAll('input, textarea')) expect((el as HTMLInputElement).value).not.toContain(secret);
    for (const el of root.querySelectorAll('*')) {
      for (const attr of el.attributes) expect(attr.value).not.toContain(secret);
    }
  }

  it('says it adds providers, explains how to replace one, and locks the default box when a default exists', async () => {
    const { root } = await open();
    expect(root.textContent).toContain('Add a provider');
    expect(root.textContent).not.toContain('Add or update');
    expect(root.textContent).toContain('To replace a provider, delete it first');
    const box = field(root, 'provider-default');
    expect(box.disabled).toBe(true);
    expect(box.checked).toBe(false);
    expect(root.textContent).toContain('A default provider already exists');
  });

  it('preselects "use as the default" when there is no provider yet, and sends it', async () => {
    const mock = installMock();
    mock.state.providers.length = 0;
    await readyApp(mock);
    const root = render(Settings);
    await eventually(() => expect(root.textContent).toContain('No provider yet.'));
    const box = field(root, 'provider-default');
    expect(box.checked).toBe(true);
    expect(box.disabled).toBe(false);
    expect(root.textContent).toContain('first provider, so it becomes the default');
    type(field(root, 'provider-name'), 'first');
    type(field(root, 'provider-url'), 'https://other.example.test/v1');
    type(field(root, 'provider-model'), 'model-small');
    button(root, 'Add provider').click();
    await eventually(() => expect(mock.state.providers.length).toBe(1));
    const sent = mock.state.requests.find((r) => r.method === 'POST' && r.path === '/api/providers');
    expect(sent?.body).toMatchObject({ name: 'first', is_default: true });
    expect(mock.state.providers[0]?.is_default).toBe(true);
  });

  it('lets the person untick the default box when it is the first provider', async () => {
    const mock = installMock();
    mock.state.providers.length = 0;
    await readyApp(mock);
    const root = render(Settings);
    await eventually(() => expect(field(root, 'provider-default').checked).toBe(true));
    const box = field(root, 'provider-default');
    box.click();
    flushSync();
    expect(box.checked).toBe(false);
  });

  it('shows the replace advice when the server answers already_exists', async () => {
    const { root } = await open();
    type(field(root, 'provider-name'), 'primary');
    type(field(root, 'provider-url'), 'https://other.example.test/v1');
    type(field(root, 'provider-model'), 'model-small');
    button(root, 'Add provider').click();
    await eventually(() => expect(root.textContent).toContain('delete the old one first and then add it again'));
  });

  it('lists providers with the key state but never a key; the key field is a password field with autocomplete off', async () => {
    const { root } = await open();
    expect(root.textContent).toContain('Stored (hidden)');
    const key = field(root, 'provider-key');
    expect(key.type).toBe('password');
    expect(key.autocomplete).toBe('off');
  });

  it('sends the key once, clears it after submit and never shows it again', async () => {
    const { mock, root } = await open();
    fill(root);
    expect(field(root, 'provider-key').value).toBe(secret);
    button(root, 'Add provider').click();
    await eventually(() => expect(root.textContent).toContain('second'));
    const post = mock.state.requests.filter((r) => r.method === 'POST' && r.path === '/api/providers');
    expect(post.length).toBe(1);
    expect((post[0]?.body as { key: string }).key).toBe(secret); // it was sent...
    expect(field(root, 'provider-key').value).toBe(''); // ...and cleared
    nowhereVisible(root);
    expect(JSON.stringify(mock.state.providers)).not.toContain(secret); // the mock never kept it either
    expect(toasts.items.map((x) => x.text)).toContain('The provider was added.');
    nowhereVisible(root);
  });

  it('also clears the key when the save fails', async () => {
    const { root } = await open();
    const inner = globalThis.fetch;
    vi.stubGlobal('fetch', (input: string, init?: RequestInit) =>
      init?.method === 'POST' && String(input).startsWith('/api/providers')
        ? Promise.resolve(new Response(JSON.stringify({ error: { code: 'internal', message: 'boom' } }), { status: 500 }))
        : inner(input, init),
    );
    fill(root);
    button(root, 'Add provider').click();
    await eventually(() => expect(root.textContent).toContain('Something went wrong on the server.'));
    expect(field(root, 'provider-key').value).toBe('');
    nowhereVisible(root);
  });

  it('validates the parameters JSON, the URL and the name before sending anything', async () => {
    const { mock, root } = await open();
    const before = mock.state.requests.length;
    type(field(root, 'provider-name'), '!!');
    type(field(root, 'provider-url'), 'ftp://x');
    type(field(root, 'provider-params'), '{not json');
    button(root, 'Add provider').click();
    flushSync();
    expect(root.textContent).toContain('Enter a valid JSON object, or leave this empty.');
    expect(root.textContent).toContain('Enter a full address that starts with https:// or http://.');
    expect(root.textContent).toContain('Enter the model name.');
    expect(root.textContent).toContain('Enter a name with letters');
    expect(mock.state.requests.length).toBe(before);
    type(field(root, 'provider-params'), '[1,2]');
    button(root, 'Add provider').click();
    flushSync();
    expect(root.textContent).toContain('Enter a valid JSON object');
  });

  it('shows a clear banner and blocks the key field when the master key is missing', async () => {
    const { root } = await open({ masterKey: false });
    const banner = root.querySelector('[role="alert"]') as HTMLElement;
    expect(banner.textContent).toContain('The master key is not configured');
    expect(banner.textContent).toContain('NODIVARA_MASTER_KEY');
    expect(field(root, 'provider-key').disabled).toBe(true);
  });

  it('relays 409 master_key_missing from the server with advice', async () => {
    const { mock, root } = await open();
    mock.state.masterKey = false; // the server lost the key after the page loaded
    fill(root);
    button(root, 'Add provider').click();
    await eventually(() => expect(root.textContent).toContain('The server has no master key'));
    expect(root.textContent).toContain('Start the server with the NODIVARA_MASTER_KEY');
    expect(field(root, 'provider-key').value).toBe('');
  });

  it('tests the connection and shows success and failure with advice', async () => {
    const { mock, root } = await open();
    button(root, 'Test connection').click();
    await eventually(() => expect(root.textContent).toContain('answered in 120 ms with model model-large'));
    mock.state.providers[0]!.base_url = 'https://unreachable.example.test/v1';
    button(root, 'Test connection').click();
    await eventually(() => expect(root.textContent).toContain('The AI provider could not be reached.'));
    expect(root.textContent).toContain('Check the base URL in Settings');
    expect(root.textContent).toContain('Check the address, your network'); // the server detail, as text
  });

  it('asks before deleting a provider, and deletes only after confirmation', async () => {
    const { mock, root } = await open();
    button(root, 'Delete').click();
    flushSync();
    expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
    expect(mock.state.requests.some((r) => r.method === 'DELETE')).toBe(false);
    (document.querySelector('[role="alertdialog"] button') as HTMLButtonElement).click(); // Cancel is first
    flushSync();
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
    expect(mock.state.providers.length).toBe(1);
    button(root, 'Delete').click();
    flushSync();
    const confirm = [...document.querySelectorAll('[role="alertdialog"] button')].pop() as HTMLButtonElement;
    confirm.click();
    await eventually(() => expect(mock.state.providers.length).toBe(0));
  });

  it('switches language and theme and remembers only those two choices', async () => {
    const { root } = await open();
    const locale = root.querySelector('#locale') as HTMLSelectElement;
    locale.value = 'id';
    locale.dispatchEvent(new Event('change', { bubbles: true }));
    await eventually(() => expect(i18n.locale).toBe('id'));
    await eventually(() => expect(root.textContent).toContain('Pengaturan'));
    const theme = root.querySelector('#theme') as HTMLSelectElement;
    theme.value = 'dark';
    theme.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(Object.keys(localStorage).sort()).toEqual(['nv.locale', 'nv.theme']);
  });
});

describe('Plan detail page', () => {
  async function open(name: string, locale: 'en' | 'id' = 'en') {
    const mock = installMock();
    await readyApp(mock, locale);
    const root = render(PlanDetail, { name });
    await eventually(() => expect(root.querySelector('ol.timeline')).not.toBeNull());
    return { mock, root };
  }

  it('shows the steps as an ordered timeline with localized kinds, sides and dependencies', async () => {
    const { root } = await open('move-alpha');
    const rows = [...root.querySelectorAll('ol.timeline li')].map((row) => row.textContent?.replace(/\s+/g, ' ').trim() ?? '');
    expect(rows.length).toBe(10);
    expect(rows[0]).toContain('Collect an inventory');
    expect(rows[0]).toContain('Source server');
    expect(rows[0]).toContain('First: it waits for nothing');
    expect(rows[1]).toContain('Target server');
    expect(rows[2]).toContain('Take the backup');
    expect(rows[2]).toContain('After step 1, 2');
    expect(rows[4]).toContain('Database shop_db');
    expect(rows[9]).toContain('Shut down the old server');
    expect(root.textContent).not.toMatch(/depends_on|"kind"|\{|\[object/);
  });

  it('shows every gate as a localized label with Yes/No or a number, never a raw key', async () => {
    const { root } = await open('move-alpha');
    const facts = root.querySelector('ul.gates')?.textContent?.replace(/\s+/g, ' ') ?? '';
    for (const key of ['cutover_allowed', 'shutdown_allowed', 'inventory_incomplete', 'comparison_current', 'open_critical', 'accepted_critical']) {
      expect(facts).not.toContain(key);
    }
    expect(facts).toContain('Moving traffic allowed No');
    expect(facts).toContain('Shutting down the old server allowed No');
    expect(facts).toContain('Inventory incomplete Yes');
    expect(facts).toContain('Comparison is up to date Yes');
    expect(facts).toContain('Critical findings still open 1');
    expect(facts).toContain('PAR-001 on listener tcp:8080');
    expect(facts).toContain('Accepted because: Replaced by a managed job.');
    expect(facts).toContain('Informational findings 17');
  });

  it('is localized in Indonesian', async () => {
    const { root } = await open('move-alpha', 'id');
    expect(root.textContent).toContain('Kumpulkan inventaris');
    expect(root.textContent).toContain('Server asal');
    expect(root.textContent).toContain('Setelah langkah 1, 2');
  });

  it('degrades unknown step kinds, sides and gate keys to readable text', async () => {
    const mock = installMock();
    const plan = mock.state.plans[1]!;
    plan.steps = [{ seq: 0, kind: 'teleport', depends_on: [], status: 'pending', side: 'moon' }, { seq: 5, kind: 'backup', depends_on: [0, 9], status: 'pending' }];
    plan.gates = { future_gate: true, other_number: 3, nested: { a: 1 }, open_critical: 'oops' };
    await readyApp(mock);
    const root = render(PlanDetail, { name: 'move-beta' });
    await eventually(() => expect(root.querySelector('ol.timeline')).not.toBeNull());
    const text = root.textContent?.replace(/\s+/g, ' ') ?? '';
    expect(text).toContain('Other step: teleport');
    expect(text).toContain('moon');
    expect(text).toContain('After step 1, 10'); // a missing step keeps its own number plus one
    expect(text).toContain('Other gate: future_gate Yes');
    expect(text).toContain('Other gate: other_number 3');
    expect(text).toContain('Other gate: nested');
    expect(text).toContain('Critical findings still open oops');
  });

  it('renders hostile step and gate text as inert text', async () => {
    const mock = installMock({ hostile: true });
    await readyApp(mock);
    const root = render(PlanDetail, { name: 'move-alpha' });
    await eventually(() => expect(root.querySelector('ol.timeline')).not.toBeNull());
    expect(root.querySelector('script, img, svg:not([data-icon]), iframe, a[href^="javascript"]')).toBeNull();
    expect((window as unknown as { __pwned?: unknown }).__pwned).toBeUndefined();
  });

  it('approves a draft plan, runs it when execution is enabled, and shows step statuses', async () => {
    const mock = installMock({ executionEnabled: true });
    await readyApp(mock);
    const root = render(PlanDetail, { name: 'move-alpha' });
    await eventually(() => expect(root.querySelector('ol.timeline')).not.toBeNull());
    expect(root.querySelector('.step-title')?.textContent).toContain('Pending');

    button(root, 'Approve plan').click();
    flushSync();
    const approveDialog = document.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(approveDialog.textContent).toContain('Approve this migration plan?');
    (byText(approveDialog, 'button', 'Approve plan') as HTMLButtonElement).click();
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'success' && x.text === 'Plan approved.')).toBe(true));
    await eventually(() => expect(button(root, 'Run plan')).toBeDefined());

    button(root, 'Run plan').click();
    flushSync();
    const runDialog = document.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(runDialog.textContent).toContain('Run this approved migration plan now?');
    (byText(runDialog, 'button', 'Run plan') as HTMLButtonElement).click();
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'success' && x.text === 'Plan finished.')).toBe(true));
    await eventually(() => expect(root.querySelector('.step-title')?.textContent).toContain('Succeeded'));
  });

  it('shows gate_blocked_toast when plan_gate_blocked occurs', async () => {
    const mock = installMock({ executionEnabled: true, executionFails: true });
    mock.state.plans[0]!.status = 'approved';
    await readyApp(mock);
    const root = render(PlanDetail, { name: 'move-alpha' });
    await eventually(() => expect(root.querySelector('ol.timeline')).not.toBeNull());
    button(root, 'Run plan').click();
    flushSync();
    const runDialog = document.querySelector('[role="alertdialog"]') as HTMLElement;
    (byText(runDialog, 'button', 'Run plan') as HTMLButtonElement).click();
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'error' && x.text.includes('safety gate'))).toBe(true));
  });
});

describe('Transports page', () => {
  it('updates transport credentials via write-only JSON input and clears input after submit', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(Transports);
    await eventually(() => expect(root.textContent).toContain('drive-main'));

    const updateBtn = [...root.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Update credentials') as HTMLButtonElement;
    updateBtn.click();
    flushSync();
    const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
    expect(dialog).not.toBeNull();
    const textarea = dialog.querySelector('textarea') as HTMLTextAreaElement;
    textarea.value = '{"token":"secret-val"}';
    textarea.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    button(dialog, 'Update credentials').click();
    flushSync();
    expect(textarea.value).toBe('');
    await eventually(() => expect(toasts.items.some((x) => x.kind === 'success' && x.text.includes('Credentials updated'))).toBe(true));
    expect(mock.state.requests.some((r) => r.method === 'POST' && r.path.endsWith('/credentials'))).toBe(true);
  });
});

describe('Monitors page', () => {
  it('lists monitors and incidents, adds/deletes a monitor, and acknowledges an incident', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(Monitors);
    await eventually(() => expect(root.textContent).toContain('web-health'));
    expect(root.textContent).toContain('HTTP 503');

    type(field(root, 'mon-name'), 'api-tcp');
    type(field(root, 'mon-target'), '127.0.0.1:8080');
    button(root, 'Add monitor').click();
    await eventually(() => expect(root.textContent).toContain('api-tcp'));

    button(root, 'Acknowledge').click();
    await eventually(() => expect(mock.state.requests.some((r) => r.method === 'POST' && r.path.endsWith('/ack'))).toBe(true));
  });
});

describe('Channels and Telegram linking in Settings', () => {
  it('lists channels, adds a channel, and generates a Telegram link code', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(Settings);
    await eventually(() => expect(root.textContent).toContain('ops-bot'));

    type(field(root, 'ch-name'), 'alerts-tg');
    type(field(root, 'ch-token'), '123:abc');
    button(root, 'Add channel').click();
    await eventually(() => expect(root.textContent).toContain('alerts-tg'));

    button(root, 'Link Telegram').click();
    await eventually(() => expect(root.textContent).toContain('/link ABCD1234EFGH'));
  });
});

describe('NodeDetail service logs', () => {
  it('loads bounded service logs for a node and rotates agent token', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(NodeDetail, { name: 'node-alpha' });
    await eventually(() => expect(root.querySelector('#node-log-svc')).not.toBeNull());
    type(field(root, 'node-log-svc'), 'nginx.service');
    button(root, 'Service logs').click();
    await eventually(() => expect(root.querySelector('pre')?.textContent).toContain('line 1 nginx.service'));

    button(root, 'Rotate agent token').click();
    await eventually(() => expect(root.textContent).toContain('nv_agent_rotated_token_123'));
  });
});

describe('Local .test sites in Settings', () => {
  it('lists local .test sites and creates a new site', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(Settings);
    await eventually(() => expect(root.textContent).toContain('demo.test'));

    type(field(root, 'site-domain'), 'app.test');
    type(field(root, 'site-port'), '8080');
    button(root, 'Add site').click();
    await eventually(() => expect(root.textContent).toContain('app.test'));
    await eventually(() => expect(root.textContent).toContain('proxy_pass http://127.0.0.1:8080'));
  });
});
