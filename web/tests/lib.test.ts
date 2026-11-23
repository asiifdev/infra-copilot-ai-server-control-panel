import { afterEach, describe, expect, it, vi } from 'vitest';
import { setApiLocale } from '../src/lib/api';
import { detectLocale, format, i18n, t } from '../src/lib/i18n.svelte';
import { hrefFor, parseHash, router, type Route } from '../src/lib/router.svelte';
import { safeText, inlineText, TEXT_LIMIT } from '../src/lib/text';
import { theme } from '../src/lib/theme.svelte';
import { toasts } from '../src/lib/toast.svelte';
import { Resource } from '../src/lib/resource.svelte';
import { ApiError } from '../src/lib/api';
import { installMock, signIn } from './helpers';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  setApiLocale('en');
});

describe('i18n', () => {
  it('substitutes placeholders, keeps unknown ones and unescapes doubled braces', () => {
    expect(format('Hello {name}, {n} items', { name: 'A', n: 3 })).toBe('Hello A, 3 items');
    expect(format('Missing {x}', {})).toBe('Missing {x}');
    expect(format('{{ "a": 1 }}')).toBe('{ "a": 1 }');
    expect(format('open {brace')).toBe('open {brace');
  });

  it('does not interpret substituted values (no second pass)', () => {
    expect(format('Hi {name}', { name: '{other}' })).toBe('Hi {other}');
  });

  it('falls back to the key when a message is missing or the catalog is not loaded', () => {
    expect(t('ui.not.there')).toBe('ui.not.there');
  });

  it('loads the served catalog in both locales and sets the document language', async () => {
    const mock = installMock();
    await signIn(mock);
    await i18n.load('en');
    expect(t('ui.nav.nodes')).toBe('Nodes');
    expect(document.documentElement.lang).toBe('en');
    await i18n.load('id', true);
    expect(t('ui.nav.nodes')).toBe('Node');
    expect(t('ui.nav.overview')).toBe('Ringkasan');
    expect(document.documentElement.lang).toBe('id');
    expect(localStorage.getItem('nv.locale')).toBe('id');
    expect(t('ui.overview.nodes_count', { count: 3 })).toBe('3 node');
  });

  it('keeps working (with keys) when the catalog cannot be loaded', async () => {
    vi.stubGlobal('fetch', async () => new Response('{}', { status: 500 }));
    await i18n.load('en');
    expect(t('ui.nav.nodes')).toBeTypeOf('string');
  });

  it('detects the locale from ?lang, then the stored choice, then the browser', () => {
    expect(detectLocale('?lang=id', ['en-US'])).toBe('id');
    expect(detectLocale('?lang=xx', ['id-ID'])).toBe('id');
    expect(detectLocale('', ['fr-FR', 'en-GB'])).toBe('en');
    localStorage.setItem('nv.locale', 'id');
    expect(detectLocale('', ['en-US'])).toBe('id');
    expect(detectLocale('?lang=en', ['id'])).toBe('en');
    expect(detectLocale('', [])).toBe('id');
  });
});

describe('router', () => {
  it('parses every route and falls back to the overview', () => {
    expect(parseHash('')).toEqual({ name: 'overview' });
    expect(parseHash('#/')).toEqual({ name: 'overview' });
    expect(parseHash('#/nodes')).toEqual({ name: 'nodes' });
    expect(parseHash('#/nodes/web%201')).toEqual({ name: 'node', node: 'web 1' });
    expect(parseHash('#/findings')).toEqual({ name: 'findings' });
    expect(parseHash('#/assistant')).toEqual({ name: 'assistant', conversation: null });
    expect(parseHash('#/assistant/c1')).toEqual({ name: 'assistant', conversation: 'c1' });
    expect(parseHash('#/plans/p%2F1')).toEqual({ name: 'plan', plan: 'p/1' });
    expect(parseHash('#/transports')).toEqual({ name: 'transports' });
    expect(parseHash('#/memory')).toEqual({ name: 'memory' });
    expect(parseHash('#/settings')).toEqual({ name: 'settings' });
    expect(parseHash('#/nope')).toEqual({ name: 'overview' });
    expect(parseHash('#t=sometoken')).toEqual({ name: 'overview' });
    expect(parseHash('#/nodes/a/b')).toEqual({ name: 'overview' });
  });

  it('refuses malformed or oversized parameters', () => {
    expect(parseHash('#/nodes/%E0%A4%A')).toEqual({ name: 'nodes' });
    expect(parseHash(`#/nodes/${'a'.repeat(500)}`)).toEqual({ name: 'nodes' });
    expect(parseHash('#/plans/')).toEqual({ name: 'plans' });
  });

  it('round-trips hrefs, including hostile names', () => {
    const routes: Route[] = [
      { name: 'overview' },
      { name: 'node', node: '<script>x</script>/../' },
      { name: 'assistant', conversation: 'c 1' },
      { name: 'plan', plan: 'p&q=1' },
      { name: 'settings' },
    ];
    for (const route of routes) expect(parseHash(hrefFor(route))).toEqual(route);
  });

  it('follows hashchange and navigates', async () => {
    const stop = router.start();
    router.go({ name: 'findings' });
    window.dispatchEvent(new HashChangeEvent('hashchange'));
    expect(router.route.name).toBe('findings');
    stop();
  });
});

describe('theme', () => {
  it('sets and removes data-theme and persists only the choice', () => {
    theme.set('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    theme.set('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    theme.set('system');
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(localStorage.getItem('nv.theme')).toBe('system');
    localStorage.setItem('nv.theme', 'dark');
    theme.init();
    expect(theme.choice).toBe('dark');
    localStorage.setItem('nv.theme', '<bad>');
    theme.init();
    expect(theme.choice).toBe('system');
  });
});

describe('toasts', () => {
  it('shows, dismisses and expires non-error notices; keeps errors until dismissed', () => {
    vi.useFakeTimers();
    const ok = toasts.push('success', 'saved');
    const bad = toasts.push('error', 'failed');
    expect(toasts.items.length).toBe(2);
    vi.advanceTimersByTime(8000);
    expect(toasts.items.map((x) => x.id)).toEqual([bad]);
    toasts.dismiss(bad);
    expect(toasts.items).toEqual([]);
    expect(ok).not.toBe(bad);
  });

  it('shows at most a few at a time', () => {
    for (let i = 0; i < 10; i += 1) toasts.push('error', `n${i}`);
    expect(toasts.items.length).toBeLessThanOrEqual(4);
  });
});

describe('text helpers', () => {
  it('removes bidi controls and zero-width characters but keeps emoji joiners', () => {
    expect(safeText('a\u202Eb\u200Bc\u2066d\u2069e').text).toBe('abcde');
    expect(safeText('\u{1F468}\u200D\u{1F469}').text).toBe('\u{1F468}\u200D\u{1F469}');
  });

  it('cuts huge strings and flags it', () => {
    const result = safeText('A'.repeat(TEXT_LIMIT + 10));
    expect(result.truncated).toBe(true);
    expect(result.text.length).toBe(TEXT_LIMIT);
  });

  it('flattens and bounds inline text; tolerates non-strings', () => {
    expect(inlineText('a\n\n  b\tc')).toBe('a b c');
    expect(inlineText('x'.repeat(500), 10)).toHaveLength(11);
    expect(inlineText(null)).toBe('');
    expect(inlineText(42)).toBe('42');
  });
});

describe('Resource', () => {
  it('ignores a slower older request once a newer one started', async () => {
    const resource = new Resource<string>();
    let release: (value: string) => void = () => {};
    resource.load(() => new Promise<string>((resolve) => (release = resolve)));
    const first = release;
    resource.load(async () => 'new');
    await vi.waitFor(() => expect(resource.data).toBe('new'));
    first('old');
    await Promise.resolve();
    expect(resource.data).toBe('new');
  });

  it('keeps an ApiError and turns anything else into internal', async () => {
    const resource = new Resource<string>();
    resource.load(async () => {
      throw new ApiError(404, 'not_found', '');
    });
    await vi.waitFor(() => expect(resource.status).toBe('error'));
    expect(resource.error?.code).toBe('not_found');
    resource.load(async () => {
      throw new Error('boom');
    });
    await vi.waitFor(() => expect(resource.error?.code).toBe('internal'));
  });
});
