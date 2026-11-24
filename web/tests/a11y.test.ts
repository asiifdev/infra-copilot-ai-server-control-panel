// An automated accessibility pass (axe-core) over every route, in both locales and both themes,
// plus the hostile-data pass: every place where server or model text appears must render it inert.
// jsdom has no layout, so rules that need rendering (colour contrast) are off here; the palette is
// checked by scripts/check-palette.py and the layout rules by the browser check in ADR 0031.

import axe from 'axe-core';
import { flushSync } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { hostileShort, id } from '../mock/fixtures';
import Shell from '../src/Shell.svelte';
import { i18n, type Locale } from '../src/lib/i18n.svelte';
import { palette } from '../src/lib/palette.svelte';
import { router } from '../src/lib/router.svelte';
import { theme, type Theme } from '../src/lib/theme.svelte';
import { eventually, installMock, readyApp, render, resetSession } from './helpers';

const CHAT = id(300);

const ROUTES = [
  '#/',
  '#/nodes',
  '#/nodes/node-alpha',
  '#/findings',
  '#/assistant',
  `#/assistant/${CHAT}`,
  '#/proposals',
  '#/plans',
  '#/plans/move-alpha',
  '#/transports',
  '#/memory',
  '#/settings',
];

afterEach(() => {
  vi.unstubAllGlobals();
  resetSession();
});

async function openRoute(hash: string, options?: Parameters<typeof installMock>[0], locale: Locale = 'en', mode: Theme = 'light') {
  const mock = installMock(options);
  await readyApp(mock, locale);
  theme.set(mode, false);
  // In hostile mode the first node is named with an attack string; open that one.
  window.location.hash = options?.hostile ? hash.replace('node-alpha', encodeURIComponent(hostileShort(0))) : hash;
  const stop = router.start();
  const root = render(Shell);
  await eventually(() => expect(root.querySelector('.spinner, [data-loading]')).toBeNull());
  return { root, stop, mock };
}

describe('accessibility (axe-core)', () => {
  for (const locale of ['en', 'id'] as const) {
    for (const mode of ['light', 'dark'] as const) {
      for (const hash of ROUTES) {
        it(`${hash} in ${locale}, ${mode} theme has no violations`, async () => {
          const { root, stop } = await openRoute(hash, undefined, locale, mode);
          const results = await axe.run(document.documentElement, {
            rules: { 'color-contrast': { enabled: false }, 'scrollable-region-focusable': { enabled: true } },
          });
          const report = results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`);
          expect(report).toEqual([]);
          expect(root.querySelector('main')).not.toBeNull();
          stop();
        });
      }
    }
  }
});

describe('accessibility of the overlays (axe-core)', () => {
  for (const locale of ['en', 'id'] as const) {
    for (const mode of ['light', 'dark'] as const) {
      it(`the command palette in ${locale}, ${mode} theme has no violations`, async () => {
        const { stop } = await openRoute('#/', undefined, locale, mode);
        palette.show();
        flushSync();
        const results = await axe.run(document.documentElement, { rules: { 'color-contrast': { enabled: false } } });
        expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`)).toEqual([]);
        palette.hide();
        stop();
      });

      it(`the confirmation dialog on the Approvals page in ${locale}, ${mode} theme has no violations`, async () => {
        const { root, stop } = await openRoute('#/proposals', undefined, locale, mode);
        const approve = [...root.querySelectorAll('section button')].find((b) => /Setujui|Approve/.test(b.textContent ?? '')) as HTMLButtonElement;
        // The second pending card is the risky one.
        const buttons = [...root.querySelectorAll('section button')].filter((b) => /Setujui|Approve/.test(b.textContent ?? ''));
        ((buttons[1] ?? approve) as HTMLButtonElement).click();
        flushSync();
        expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
        const results = await axe.run(document.documentElement, { rules: { 'color-contrast': { enabled: false } } });
        expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(' | ')}`)).toEqual([]);
        stop();
      });
    }
  }
});

describe('structure by construction', () => {
  it('has a skip link first, landmarks, one h1 per page and the document language', async () => {
    const { root, stop } = await openRoute('#/nodes');
    expect(root.querySelector('a')?.getAttribute('href')).toBe('#main-content');
    expect(root.querySelector('header')).not.toBeNull();
    expect(root.querySelector('nav[aria-label]')).not.toBeNull();
    expect(root.querySelector('main#main-content')).not.toBeNull();
    expect(root.querySelectorAll('h1').length).toBe(1);
    expect(document.documentElement.lang).toBe('en');
    expect(document.title).toBe('Nodes - Nodivara');
    stop();
  });

  it('marks the current page in the navigation and moves focus to the heading on a route change', async () => {
    const { root, stop } = await openRoute('#/nodes');
    expect(root.querySelector('nav a[aria-current="page"]')?.textContent).toBe('Nodes');
    window.location.hash = '#/findings';
    window.dispatchEvent(new HashChangeEvent('hashchange'));
    await eventually(() => expect(document.activeElement?.textContent).toBe('Findings'));
    expect(document.activeElement?.id).toBe('page-title');
    expect(document.activeElement?.textContent).toBe('Findings');
    expect(root.querySelector('nav a[aria-current="page"]')?.textContent).toBe('Findings');
    expect(document.title).toBe('Findings - Nodivara');
    stop();
  });

  it('updates the document title and language with the locale', async () => {
    const { stop } = await openRoute('#/memory');
    await i18n.load('id');
    expect(document.title).toBe('Memori - Nodivara');
    expect(document.documentElement.lang).toBe('id');
    stop();
  });
});

describe('hostile server text is inert on every page', () => {
  for (const hash of ROUTES) {
    it(`${hash}`, async () => {
      const { root, stop } = await openRoute(hash, { hostile: true });
      expect(root.querySelector('script, iframe, object, embed, img, svg:not([data-icon]), a[href^="javascript"]')).toBeNull();
      for (const el of root.querySelectorAll('*')) {
        for (const attr of el.attributes) {
          expect(attr.name.startsWith('on'), `${el.tagName} ${attr.name}`).toBe(false);
          expect(attr.value.toLowerCase().startsWith('javascript:'), `${el.tagName} ${attr.name}`).toBe(false);
        }
      }
      expect((window as unknown as { __pwned?: unknown }).__pwned).toBeUndefined();
      expect(root.textContent).not.toMatch(/[\u202A-\u202E\u2066-\u2069\u200B]/);
      stop();
    });
  }

  it('shows the attack strings as visible text (so the page is not simply dropping them)', async () => {
    const { root, stop } = await openRoute('#/findings', { hostile: true });
    root.querySelectorAll('details').forEach((d) => (d.open = true));
    expect(root.textContent).toContain('<img src=x onerror="window.__pwned = true">');
    expect(root.textContent).toContain('[click](javascript:window.__pwned=1) **bold**');
    expect(root.textContent).toContain('<a href="javascript:window.__pwned=1">link</a>');
    stop();
  });
});
