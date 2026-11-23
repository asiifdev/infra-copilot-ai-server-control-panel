import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api, ApiError, buildUrl, onUnauthorized, setApiLocale, streamTurn } from '../src/lib/api';
import type { TurnEvent } from '../src/lib/types';
import { installMock, signIn } from './helpers';
import { id } from '../mock/fixtures';

const CHAT = id(300);

afterEach(() => {
  vi.unstubAllGlobals();
  onUnauthorized(null);
  setApiLocale('en');
});

describe('api client', () => {
  it('answers 401 before the token exchange and calls the unauthorized handler', async () => {
    installMock();
    const handler = vi.fn();
    onUnauthorized(handler);
    await expect(api.session()).rejects.toMatchObject({ status: 401, code: 'unauthenticated' });
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it('exchange: a bad token is invalid_token and is not treated as an expired session', async () => {
    installMock();
    const handler = vi.fn();
    onUnauthorized(handler);
    await expect(api.exchange('not-the-token-at-all')).rejects.toMatchObject({ status: 401, code: 'invalid_token' });
    expect(handler).not.toHaveBeenCalled();
  });

  it('exchange: a good token works once; the token is one-time', async () => {
    const mock = installMock();
    await signIn(mock);
    expect((await api.session()).org.slug).toBe('example');
    await expect(api.exchange('mock-valid-token-0001')).rejects.toMatchObject({ code: 'invalid_token' });
  });

  it('sends X-Nodivara on every non-GET request and never on GET', async () => {
    const mock = installMock();
    await signIn(mock);
    mock.state.requests.length = 0;
    await api.nodes();
    await api.createConversation('t');
    await api.approveMemory(id(101));
    await api.deleteMemory(id(102));
    await api.logout();
    const byMethod = (m: string) => mock.state.requests.filter((r) => r.method === m);
    expect(byMethod('GET').every((r) => r.headers['x-nodivara'] === undefined)).toBe(true);
    const changing = mock.state.requests.filter((r) => r.method !== 'GET');
    expect(changing.length).toBe(4);
    expect(changing.every((r) => r.headers['x-nodivara'] === '1')).toBe(true);
  });

  it('the mock refuses a change without the header (so the client is proven to send it)', async () => {
    const mock = installMock();
    await signIn(mock);
    const response = await mock.fetch('/api/conversations', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
    expect(response.status).toBe(403);
  });

  it('maps the contract error body to code and keeps the status', async () => {
    const mock = installMock();
    await signIn(mock);
    await expect(api.plan('missing')).rejects.toMatchObject({ status: 404, code: 'not_found' });
    await expect(api.inventory('nope')).rejects.toBeInstanceOf(ApiError);
  });

  it('turns a malformed error body into a status-derived code', async () => {
    vi.stubGlobal('fetch', async () => new Response('<html>oops</html>', { status: 502 }));
    await expect(api.nodes()).rejects.toMatchObject({ status: 502, code: 'http_502' });
    vi.stubGlobal('fetch', async () => new Response(JSON.stringify({ error: { code: '<script>', message: 'x'.repeat(2000) } }), { status: 500 }));
    const error = await api.nodes().catch((e: ApiError) => e);
    expect((error as ApiError).code).toBe('http_500');
    expect((error as ApiError).message.length).toBeLessThanOrEqual(500);
  });

  it('maps a network failure to the network code', async () => {
    vi.stubGlobal('fetch', async () => {
      throw new TypeError('failed');
    });
    await expect(api.nodes()).rejects.toMatchObject({ code: 'network', status: 0 });
  });

  it('adds the locale to every request and encodes path parts', async () => {
    const mock = installMock();
    await signIn(mock);
    setApiLocale('id');
    const seen: string[] = [];
    const inner = globalThis.fetch;
    vi.stubGlobal('fetch', (input: string, init?: RequestInit) => {
      seen.push(String(input));
      return inner(input, init);
    });
    await api.plans();
    await api.inventory('a/b c', 'cron', 5).catch(() => undefined);
    expect(seen[0]).toBe('/api/plans?lang=id');
    expect(seen[1]).toBe('/api/nodes/a%2Fb%20c/inventory?kind=cron&limit=5&lang=id');
  });

  it('never sends a request outside /api (same-origin only)', async () => {
    for (const bad of ['//evil.test/api/x', 'https://evil.test/api/x', '/other/path', 'api/nodes', '/api', '/apix/nodes']) {
      expect(() => buildUrl(bad), bad).toThrow(ApiError);
    }
    expect(buildUrl('/api/nodes')).toBe('/api/nodes?lang=en');
  });

  it('sends cookies same-origin and no referrer', async () => {
    let seen: RequestInit | undefined;
    vi.stubGlobal('fetch', async (_url: string, init?: RequestInit) => {
      seen = init;
      return new Response('[]', { status: 200 });
    });
    await api.nodes();
    expect(seen?.credentials).toBe('same-origin');
    expect(seen?.referrerPolicy).toBe('no-referrer');
    expect(seen?.cache).toBe('no-store');
  });

  it('supports abort through the signal', async () => {
    vi.stubGlobal('fetch', (_url: string, init?: RequestInit) =>
      new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')));
      }),
    );
    const controller = new AbortController();
    const pending = api.nodes(controller.signal);
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
  });
});

describe('streamTurn', () => {
  let events: TurnEvent[];
  beforeEach(() => {
    events = [];
  });

  it('reports delta, tool and done events in order', async () => {
    const mock = installMock();
    await signIn(mock);
    await streamTurn(CHAT, 'hello', (e) => events.push(e));
    expect(events.map((e) => e.type)).toEqual(['tool', 'delta', 'delta', 'delta', 'done']);
    const text = events.filter((e) => e.type === 'delta').map((e) => (e as { text: string }).text).join('');
    expect(text).toContain('Port 8080');
    expect(text).toContain('é\u{1F600}');
  });

  it('gives the same result for every line ending and tiny chunks', async () => {
    for (const eol of ['\n', '\r\n', '\r'] as const) {
      const mock = installMock({ eol, splitChunks: true });
      await signIn(mock);
      const seen: TurnEvent[] = [];
      await streamTurn(CHAT, 'hello', (e) => seen.push(e));
      expect(seen.map((e) => e.type)).toEqual(['tool', 'delta', 'delta', 'delta', 'done']);
      vi.unstubAllGlobals();
    }
  });

  it('refuses a second turn in the same conversation with 409 busy', async () => {
    const mock = installMock({ turnDelayMs: 5 });
    await signIn(mock);
    const first = streamTurn(CHAT, 'slow please', () => {});
    await new Promise((r) => setTimeout(r, 20));
    await expect(streamTurn(CHAT, 'again', () => {})).rejects.toMatchObject({ status: 409, code: 'busy' });
    const controller = new AbortController();
    controller.abort();
    void first.catch(() => undefined);
  });

  it('aborts a stream in flight', async () => {
    const mock = installMock({ turnDelayMs: 5 });
    await signIn(mock);
    const controller = new AbortController();
    const seen: TurnEvent[] = [];
    const pending = streamTurn(CHAT, 'slow please', (e) => seen.push(e), controller.signal);
    await new Promise((r) => setTimeout(r, 40));
    controller.abort();
    await expect(pending).rejects.toBeDefined();
    expect(seen.length).toBeGreaterThan(0);
    expect(seen.length).toBeLessThan(200);
    await new Promise((r) => setTimeout(r, 20));
    expect(mock.state.activeTurns.size).toBe(0);
  });

  it('ignores events it does not know and malformed data', async () => {
    const body = 'event: nope\ndata: {}\n\nevent: delta\ndata: not json\n\nevent: delta\ndata: {"text":"ok"}\n\nevent: done\ndata: {}\n\n';
    vi.stubGlobal('fetch', async () => new Response(body, { status: 200 }));
    await streamTurn(CHAT, 'x', (e) => events.push(e));
    expect(events).toEqual([{ type: 'delta', text: 'ok' }, { type: 'done', rounds: undefined, tool_calls: undefined, usage: undefined }]);
  });

  it('sanitises an error event code', async () => {
    const body = 'event: error\ndata: {"code":"<img onerror=1>","message":"m"}\n\n';
    vi.stubGlobal('fetch', async () => new Response(body, { status: 200 }));
    await streamTurn(CHAT, 'x', (e) => events.push(e));
    expect(events[0]).toMatchObject({ type: 'error', code: 'internal' });
  });
});
