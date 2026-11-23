import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Assistant from '../src/pages/Assistant.svelte';
import Settings from '../src/pages/Settings.svelte';
import { i18n } from '../src/lib/i18n.svelte';
import { toasts } from '../src/lib/toast.svelte';
import type { Mock } from '../mock/server';
import { byText, eventually, installMock, readyApp, render, resetSession } from './helpers';

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
    const root = render(Assistant, { conversation: 'c1' });
    await eventually(() => expect(root.querySelectorAll('.messages > li').length).toBe(2)); // stored history
    return root;
  }

  async function ask(root: HTMLElement, text: string) {
    type(root.querySelector('#composer') as HTMLTextAreaElement, text);
    button(root, 'Send').click();
    flushSync();
  }

  it('lists conversations, shows the stored messages and a labelled live log', async () => {
    const root = await open();
    expect(root.textContent).toContain('Why is port 8080 missing?');
    const log = root.querySelector('[role="log"]') as HTMLElement;
    expect(log.getAttribute('aria-live')).toBe('polite');
    expect(root.querySelector('nav a[aria-current="page"]')?.textContent).toContain('Why is port 8080 missing?');
  });

  it('streams the answer in, shows tool chips with tier and status, and finishes', async () => {
    const root = await open();
    await ask(root, 'hello');
    await eventually(() => expect(root.textContent).toContain('Start the service, then compare again.'));
    await eventually(() => expect((button(root, 'Send')).disabled).toBe(true)); // empty draft after sending
    expect(root.querySelector('.chips')?.textContent).toContain('search_inventory');
    expect(root.querySelector('.chips')?.textContent).toContain('Tier 0');
    expect(root.textContent).not.toContain('Stop');
    expect(root.querySelectorAll('.messages > li').length).toBe(4);
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
    mock.state.activeTurns.add('c1');
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
    expect(root.querySelector('script, img, svg, iframe, object, embed, a[href^="javascript"]')).toBeNull();
    const all = root.querySelectorAll('*');
    for (const el of all) {
      for (const attr of el.attributes) expect(attr.name.startsWith('on'), `${el.tagName} ${attr.name}`).toBe(false);
    }
    expect((window as unknown as { __pwned?: unknown }).__pwned).toBeUndefined();
    expect(root.textContent).toContain('<script>window.__pwned = true</script>');
    const blocks = [...root.querySelectorAll('.block')].map((b) => b.textContent ?? '');
    expect(blocks.some((text) => text.includes('line one\nline two'))).toBe(true);
    // Bidi override and zero-width characters are removed, the visible text stays.
    expect(root.textContent).not.toMatch(/[\u202A-\u202E\u2066-\u2069\u200B]/);
    // A 300 000 character string is cut and says so.
    expect(root.textContent).toContain('This text is very long and was cut short.');
  });

  it('creates a conversation and navigates to it', async () => {
    const root = await open();
    button(root, 'New conversation').click();
    await eventually(() => expect(window.location.hash).toBe('#/assistant/c3'));
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
    button(root, 'Save provider').click();
    await eventually(() => expect(root.textContent).toContain('second'));
    const post = mock.state.requests.filter((r) => r.method === 'POST' && r.path === '/api/providers');
    expect(post.length).toBe(1);
    expect((post[0]?.body as { key: string }).key).toBe(secret); // it was sent...
    expect(field(root, 'provider-key').value).toBe(''); // ...and cleared
    nowhereVisible(root);
    expect(JSON.stringify(mock.state.providers)).not.toContain(secret); // the mock never kept it either
    expect(toasts.items.map((x) => x.text)).toContain('The provider was saved.');
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
    button(root, 'Save provider').click();
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
    button(root, 'Save provider').click();
    flushSync();
    expect(root.textContent).toContain('Enter a valid JSON object, or leave this empty.');
    expect(root.textContent).toContain('Enter a full address that starts with https:// or http://.');
    expect(root.textContent).toContain('Enter the model name.');
    expect(root.textContent).toContain('Enter a name with letters');
    expect(mock.state.requests.length).toBe(before);
    type(field(root, 'provider-params'), '[1,2]');
    button(root, 'Save provider').click();
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
    button(root, 'Save provider').click();
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
    expect(root.textContent).toContain('Could not reach the provider.'); // the server detail, as text
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
