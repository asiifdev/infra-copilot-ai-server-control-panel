// The app shell: navigation with the approvals badge, the phone menu disclosure, the command
// palette (keyboard flow, focus trap and return) and the overview dashboard.

import { flushSync } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Shell from '../src/Shell.svelte';
import { fuzzyScore } from '../src/lib/fuzzy';
import { i18n } from '../src/lib/i18n.svelte';
import { palette } from '../src/lib/palette.svelte';
import { router } from '../src/lib/router.svelte';
import { relativeTime } from '../src/lib/time';
import { theme } from '../src/lib/theme.svelte';
import Overview from '../src/pages/Overview.svelte';
import { eventually, installMock, readyApp, render, resetSession } from './helpers';

afterEach(() => {
  vi.unstubAllGlobals();
  palette.hide();
  resetSession();
});

async function openShell(hash = '#/nodes') {
  const mock = installMock();
  await readyApp(mock);
  window.location.hash = hash;
  const stop = router.start();
  const root = render(Shell);
  await eventually(() => expect(root.querySelector('[data-loading]')).toBeNull());
  return { mock, root, stop };
}

function key(target: EventTarget, init: KeyboardEventInit) {
  target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }));
  flushSync();
}

describe('navigation', () => {
  it('groups the pages and lists Approvals with its live pending count', async () => {
    const { root, stop } = await openShell();
    const groups = [...root.querySelectorAll('.group-label')].map((g) => g.textContent);
    expect(groups).toEqual(['Monitor', 'Work', 'Workspace']);
    const links = [...root.querySelectorAll('nav a')].map((a) => a.textContent?.replace(/\s+/g, ' ').trim());
    expect(links.slice(0, 3)).toEqual(['Overview', 'Nodes', 'Findings']);
    await eventually(() => expect(root.querySelector('a[href="#/proposals"] .count')?.textContent).toBe('2'));
    expect(root.querySelector('a[href="#/proposals"] .visually-hidden')?.textContent).toBe('2 waiting for a decision');
    expect(root.querySelector('nav svg[aria-hidden="true"]')).not.toBeNull();
    stop();
  });

  it('opens the Approvals page and marks it as current', async () => {
    const { root, stop } = await openShell('#/proposals');
    expect(root.querySelector('nav a[aria-current="page"]')?.textContent).toContain('Approvals');
    expect(document.title).toBe('Approvals - Nodivara');
    expect(root.querySelector('h1')?.textContent).toBe('Approvals');
    stop();
  });

  it('has a menu button that discloses the navigation, closes on navigation and on Escape', async () => {
    const { root, stop } = await openShell();
    const button = root.querySelector('#menu-button') as HTMLButtonElement;
    const nav = root.querySelector('#primary-nav') as HTMLElement;
    expect(button.getAttribute('aria-controls')).toBe('primary-nav');
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(nav.classList.contains('open')).toBe(false);
    button.click();
    flushSync();
    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(nav.classList.contains('open')).toBe(true);
    key(window, { key: 'Escape' });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(button);
    button.click();
    flushSync();
    (root.querySelector('nav a[href="#/findings"]') as HTMLAnchorElement).click();
    flushSync();
    expect(button.getAttribute('aria-expanded')).toBe('false');
    await eventually(() => expect(window.location.hash).toBe('#/findings')); // the click's own navigation
    stop();
  });

  it('uses only classes for styling (no inline style attributes in the rendered page)', async () => {
    const { root, stop } = await openShell('#/');
    await eventually(() => expect(root.querySelectorAll('section.tile').length).toBe(4));
    expect(root.querySelector('[style]')).toBeNull();
    stop();
  });
});

describe('command palette', () => {
  it('opens with Ctrl+K, filters, moves with the arrow keys and opens a page with Enter', async () => {
    const { root, stop } = await openShell('#/');
    const trigger = root.querySelector('.find') as HTMLButtonElement;
    trigger.focus();
    key(window, { key: 'k', ctrlKey: true });
    const dialog = document.querySelector('[role="dialog"][aria-modal="true"]') as HTMLElement;
    expect(dialog).not.toBeNull();
    expect(dialog.getAttribute('aria-labelledby')).toBe('palette-title');
    const input = dialog.querySelector('input[role="combobox"]') as HTMLInputElement;
    expect(document.activeElement).toBe(input);
    expect(input.getAttribute('aria-expanded')).toBe('true');
    expect(dialog.querySelector('[role="listbox"]')).not.toBeNull();
    const options = () => [...dialog.querySelectorAll('[role="option"]')];
    expect(options().length).toBe(12); // nine pages and three actions
    input.value = 'sett';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    expect(options()[0]?.textContent).toContain('Settings');
    expect(options()[0]?.getAttribute('aria-selected')).toBe('true');
    expect(input.getAttribute('aria-activedescendant')).toBe(options()[0]?.id);
    key(input, { key: 'Enter' });
    expect(window.location.hash).toBe('#/settings');
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    stop();
  });

  it('wraps with ArrowDown and ArrowUp and shows a message when nothing matches', async () => {
    const { stop } = await openShell('#/');
    palette.show();
    flushSync();
    const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
    const input = dialog.querySelector('input') as HTMLInputElement;
    const selected = () => dialog.querySelector('[role="option"][aria-selected="true"]')?.textContent;
    const first = selected();
    key(input, { key: 'ArrowUp' });
    expect(selected()).not.toBe(first);
    key(input, { key: 'ArrowDown' });
    expect(selected()).toBe(first);
    input.value = 'zzzzqq';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    expect(dialog.textContent).toContain('Nothing matches what you typed.');
    expect(dialog.querySelectorAll('[role="option"]').length).toBe(0);
    stop();
  });

  it('closes with Escape and returns the focus to the button that opened it', async () => {
    const { root, stop } = await openShell('#/');
    const trigger = root.querySelector('.find') as HTMLButtonElement;
    trigger.focus();
    trigger.click();
    flushSync();
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    key(document.querySelector('[role="dialog"] input') as HTMLElement, { key: 'Escape' });
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    stop();
  });

  it('keeps Tab inside the dialog', async () => {
    const { stop } = await openShell('#/');
    palette.show();
    flushSync();
    const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
    const close = dialog.querySelector('button') as HTMLButtonElement;
    close.focus();
    const event = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    close.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(dialog.querySelector('input'));
    stop();
  });

  it('runs quick actions: switch theme, switch language, new conversation', async () => {
    const { mock, stop } = await openShell('#/');
    const run = async (label: string) => {
      palette.show();
      flushSync();
      const input = document.querySelector('[role="dialog"] input') as HTMLInputElement;
      input.value = label;
      input.dispatchEvent(new Event('input', { bubbles: true }));
      flushSync();
      key(input, { key: 'Enter' });
    };
    await run('Switch theme');
    expect(theme.choice).toBe('light'); // system -> light
    await run('Switch language');
    await eventually(() => expect(i18n.locale).toBe('id'));
    await i18n.load('en');
    await run('New conversation');
    await eventually(() => expect(mock.state.requests.some((r) => r.method === 'POST' && r.path === '/api/conversations')).toBe(true));
    await eventually(() => expect(window.location.hash).toMatch(/^#\/assistant\/.+/));
    stop();
  });

  it('is fully translated (Indonesian)', async () => {
    const { stop } = await openShell('#/');
    await i18n.load('id');
    palette.show();
    flushSync();
    const text = document.querySelector('[role="dialog"]')?.textContent ?? '';
    expect(text).toContain('Halaman');
    expect(text).toContain('Percakapan baru');
    expect(text).toContain('Persetujuan');
    stop();
  });
});

describe('overview dashboard', () => {
  it('shows the four tiles with big numbers and the attention list with pending approvals first', async () => {
    const mock = installMock();
    await readyApp(mock);
    const root = render(Overview);
    await eventually(() => expect(root.querySelectorAll('.tile').length).toBe(4));
    const tiles = [...root.querySelectorAll('.tile')];
    expect(tiles[0]?.querySelector('.value')?.textContent).toBe('2');
    expect(tiles[0]?.textContent).toContain('waiting for your decision');
    expect(tiles[0]?.querySelector('a')?.getAttribute('href')).toBe('#/proposals');
    await eventually(() => expect(root.textContent).toContain('Needs your attention'));
    const first = root.querySelector('.items li');
    expect(first?.textContent).toContain('Approvals waiting for your decision: 2');
    expect(first?.querySelector('a')?.getAttribute('href')).toBe('#/proposals');
    const actions = [...root.querySelectorAll('.actions a')].map((a) => a.textContent?.trim());
    expect(actions).toEqual(['Ask the assistant', 'Add a provider', 'Review approvals']);
  });

  it('says that everything is calm when nothing needs attention', async () => {
    const mock = installMock();
    mock.state.proposals = [];
    mock.state.findings.length = 0;
    for (const node of mock.state.nodes) node.status = 'online';
    await readyApp(mock);
    const root = render(Overview);
    await eventually(() => expect(root.textContent).toContain('Everything looks calm'));
  });
});

describe('helpers', () => {
  it('fuzzyScore prefers prefixes, then word starts, then substrings, then scattered letters', () => {
    expect(fuzzyScore('nod', 'Nodes')).toBe(0);
    expect(fuzzyScore('con', 'New conversation')).toBe(1);
    expect(fuzzyScore('ver', 'New conversation')).toBeGreaterThan(1);
    expect(fuzzyScore('nde', 'Nodes')).toBeGreaterThan(5);
    expect(fuzzyScore('xyz', 'Nodes')).toBe(-1);
    expect(fuzzyScore('', 'Nodes')).toBe(0);
  });

  it('relativeTime speaks the language of the page and says whether it is past', () => {
    const now = Date.parse('2026-11-24T12:00:00Z');
    expect(relativeTime('2026-11-26T12:00:00Z', 'en', now)).toEqual({ past: false, text: 'in 2 days' });
    expect(relativeTime('2026-11-24T09:00:00Z', 'en', now)).toEqual({ past: true, text: '3 hours ago' });
    expect(relativeTime('2026-11-24T15:00:00Z', 'id', now)?.text).toContain('3 jam');
    expect(relativeTime('nonsense', 'en', now)).toBeNull();
    expect(relativeTime(null, 'en', now)).toBeNull();
  });
});
