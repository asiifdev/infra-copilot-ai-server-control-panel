// The mock and the TypeScript types were first written from the contract text, and the first run
// against the real server showed they disagreed. tests/recorded/real-api.json holds what the real
// server (infra serve on demo data) answered. These tests make the mock answer the same requests
// and compare the status, the content type and the exact set of keys, so the mock cannot drift
// from the server again without a recorded change.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { id } from '../mock/fixtures';
import { createMock, MOCK_TOKEN, type Mock } from '../mock/server';
import { installMock, signIn } from './helpers';

interface Entry {
  method: string;
  path: string;
  status: number;
  content_type: string | null;
  body?: unknown;
  raw?: string;
}
const recorded = JSON.parse(readFileSync(join(import.meta.dirname, 'recorded', 'real-api.json'), 'utf8')) as Record<string, Entry>;
const real = (name: string): Entry => {
  const entry = recorded[name];
  if (!entry) throw new Error(`no recording named ${name}`);
  return entry;
};

/** Paths whose content is free-form (not part of the contract's key set). */
const FREE = new Set(['data', 'spec', 'params', 'usage']);
/** Keys the real server only sends in situations the demo data does not have. */
const OPTIONAL: Record<string, string[]> = { 'steps[]': ['side', 'database'] };

type Shape = { keys?: Record<string, Shape>; item?: Shape; kind: string };

/** The set of keys at every level (union over array items), stopping at free-form fields. */
function shapeOf(value: unknown, field = ''): Shape {
  if (Array.isArray(value)) {
    const merged: Shape = { kind: 'array' };
    for (const item of value) merged.item = mergeShape(merged.item, shapeOf(item, `${field}[]`));
    return merged;
  }
  if (value !== null && typeof value === 'object') {
    const keys: Record<string, Shape> = {};
    for (const [key, child] of Object.entries(value)) {
      keys[key] = FREE.has(key) ? { kind: 'free' } : shapeOf(child, key === field ? key : key);
    }
    return { kind: 'object', keys };
  }
  return { kind: value === null ? 'null' : typeof value };
}

function mergeShape(a: Shape | undefined, b: Shape): Shape {
  if (!a) return b;
  if (a.kind !== b.kind) {
    // null next to a value (an optional or empty field): keep the richer one.
    return a.kind === 'null' ? b : b.kind === 'null' ? a : a;
  }
  const merged: Shape = { kind: a.kind };
  if (a.item || b.item) merged.item = a.item && b.item ? mergeShape(a.item, b.item) : (a.item ?? b.item);
  if (a.keys || b.keys) {
    merged.keys = { ...(a.keys ?? {}) };
    for (const [key, shape] of Object.entries(b.keys ?? {})) merged.keys[key] = mergeShape(merged.keys[key], shape);
  }
  return merged;
}

/** Lists every difference in key sets between the recorded and the mock shape, with its path. */
function diff(expected: Shape, actual: Shape, path: string, out: string[]): void {
  if (expected.kind === 'free' || actual.kind === 'free') return;
  if (expected.kind === 'null' || actual.kind === 'null') return; // empty list or nullable field
  if (expected.kind !== actual.kind) {
    out.push(`${path}: type ${expected.kind} (real) vs ${actual.kind} (mock)`);
    return;
  }
  if (expected.item && actual.item) diff(expected.item, actual.item, `${path}[]`, out);
  if (expected.keys && actual.keys) {
    const optional = OPTIONAL[`${path.split('.').pop() ?? ''}`] ?? [];
    for (const key of Object.keys(expected.keys)) {
      if (!(key in actual.keys) && !optional.includes(key)) out.push(`${path}.${key}: in the real answer, missing in the mock`);
    }
    for (const key of Object.keys(actual.keys)) {
      if (!(key in expected.keys) && !optional.includes(key)) out.push(`${path}.${key}: in the mock, never sent by the real server`);
    }
    for (const [key, shape] of Object.entries(expected.keys)) {
      const other = actual.keys[key];
      if (other) diff(shape, other, `${path}.${key}`, out);
    }
  }
}

async function ask(mock: Mock, method: string, path: string, body?: unknown, headers: Record<string, string> = {}) {
  const init: RequestInit = { method, headers: { 'X-Nodivara': '1', ...headers } };
  if (body !== undefined) {
    init.body = JSON.stringify(body);
    (init.headers as Record<string, string>)['Content-Type'] = 'application/json';
  }
  const response = await mock.fetch(path, init);
  const text = await response.text();
  let parsed: unknown = undefined;
  try {
    parsed = text === '' ? undefined : JSON.parse(text);
  } catch {
    parsed = undefined;
  }
  return { response, text, body: parsed };
}

function expectSameShape(name: string, actualBody: unknown): void {
  const entry = real(name);
  const problems: string[] = [];
  diff(shapeOf(entry.body), shapeOf(actualBody), name, problems);
  expect(problems, `${name}: mock differs from the recorded real answer`).toEqual([]);
}

function expectSameEnvelope(name: string, response: Response): void {
  const entry = real(name);
  expect(response.status, `${name} status`).toBe(entry.status);
  expect((response.headers.get('content-type') ?? '').split(';')[0], `${name} content type`).toBe((entry.content_type ?? '').split(';')[0]);
}

async function signedIn(): Promise<Mock> {
  const mock = installMock();
  await signIn(mock);
  return mock;
}

describe('the comparison itself', () => {
  it('reports a missing key, an extra key and a changed type', () => {
    const problems: string[] = [];
    diff(shapeOf({ a: 1, b: 'x', c: [{ k: 1 }] }), shapeOf({ a: 'one', d: 1, c: [{ k: 1, z: 1 }] }), 'x', problems);
    expect(problems).toEqual(
      expect.arrayContaining([
        expect.stringContaining('x.b: in the real answer'),
        expect.stringContaining('x.d: in the mock'),
        expect.stringContaining('x.a: type number'),
        expect.stringContaining('x.c[].z: in the mock'),
      ]),
    );
  });

  it('would catch a mock whose conversation lacks a recorded key', () => {
    const problems: string[] = [];
    const entry = real('conversations_before').body as unknown[];
    diff(shapeOf(entry), shapeOf([{ id: 'x', title: 't', created_at: 'now' }]), 'conversations', problems);
    expect(problems.join()).toContain('archived');
  });
});

describe('the mock answers with the keys of the real server', () => {
  const reads: [string, string][] = [
    ['session', '/api/session'],
    ['nodes', '/api/nodes'],
    ['inventory', '/api/nodes/node-alpha/inventory'],
    ['findings', '/api/findings'],
    ['plans', '/api/plans'],
    ['plan', '/api/plans/move-alpha'],
    ['transports', '/api/transports'],
    ['providers', '/api/providers'],
    ['memories_pending', '/api/memories?status=pending'],
    ['memories_active', '/api/memories?status=active'],
    ['proposals', '/api/proposals'],
    ['conversations_before', '/api/conversations'],
  ];
  for (const [name, path] of reads) {
    it(`GET ${real(name).path} (${name})`, async () => {
      const mock = await signedIn();
      const { response, body } = await ask(mock, 'GET', path);
      expectSameEnvelope(name, response);
      expectSameShape(name, body);
    });
  }

  it('nullable fields take both forms in the mock too (empty inventory, no last check)', async () => {
    const mock = await signedIn();
    const { body } = await ask(mock, 'GET', '/api/nodes');
    const nodes = body as { latest_inventory: unknown }[];
    expect(nodes.some((n) => n.latest_inventory === null)).toBe(true);
    expect(nodes.some((n) => n.latest_inventory !== null)).toBe(true);
  });

  it('creating a conversation answers 201 with the real keys, and a title is optional', async () => {
    const mock = await signedIn();
    const { response, body } = await ask(mock, 'POST', '/api/conversations', {});
    expectSameEnvelope('conversation_create', response);
    expectSameShape('conversation_create', body);
    expect((body as { title: string }).title).toBe('New conversation');
    const titled = await ask(mock, 'POST', '/api/conversations', { title: 'My title' });
    expect((titled.body as { title: string }).title).toBe('My title');
  });

  it('a turn streams the recorded event kinds and stores rows with the recorded keys', async () => {
    const mock = await signedIn();
    const { text } = await ask(mock, 'POST', `/api/conversations/${id(301)}/turn`, { message: 'hello' }, { Accept: 'text/event-stream' });
    const events = (raw: string) =>
      raw
        .split('\n\n')
        .filter((chunk) => chunk.trim() !== '')
        .map((chunk) => {
          const name = /^event: (.*)$/m.exec(chunk)?.[1] ?? '';
          const data = JSON.parse(/^data: (.*)$/m.exec(chunk)?.[1] ?? 'null') as unknown;
          return { name, data };
        });
    const realEvents = events(real('turn').raw ?? '');
    const mockEvents = events(text);
    for (const name of ['tool', 'delta', 'done']) {
      const expected = realEvents.find((e) => e.name === name);
      const actual = mockEvents.find((e) => e.name === name);
      expect(expected, `recorded ${name}`).toBeDefined();
      expect(actual, `mock ${name}`).toBeDefined();
      const problems: string[] = [];
      diff(shapeOf(expected?.data), shapeOf(actual?.data), `event ${name}`, problems);
      expect(problems).toEqual([]);
    }
    const { body } = await ask(mock, 'GET', `/api/conversations/${id(301)}/messages`);
    expectSameShape('messages_after_turn', body);
    const rows = body as { role: string; content: string; tools: string[] }[];
    // As on the real server: the user row, a tool row with empty text, then the answer.
    expect(rows.map((r) => r.role)).toEqual(['user', 'assistant', 'assistant']);
    expect(rows[1]).toMatchObject({ content: '', tools: ['search_inventory'] });
  });

  it('an empty conversation lists no messages, as recorded', async () => {
    const mock = await signedIn();
    const { response, body } = await ask(mock, 'GET', `/api/conversations/${id(301)}/messages`);
    expectSameEnvelope('messages_empty', response);
    expect(body).toEqual(real('messages_empty').body);
  });

  it('adding a provider answers 201 with the real keys; the first one is the default', async () => {
    const mock = await signedIn();
    mock.state.providers.length = 0;
    const input = { name: 'first', base_url: 'https://llm.example.test/v1', model: 'm' };
    const { response, body } = await ask(mock, 'POST', '/api/providers', input);
    expectSameEnvelope('provider_add', response);
    expectSameShape('provider_add', body);
    expect((body as { is_default: boolean }).is_default).toBe(true);
  });

  it('approving and deleting a memory answer 204 with no body', async () => {
    const mock = await signedIn();
    const approve = await ask(mock, 'POST', `/api/memories/${id(101)}/approve`);
    expect(approve.response.status).toBe(real('memory_approve').status);
    expect(approve.text).toBe('');
    const remove = await ask(mock, 'DELETE', `/api/memories/${id(102)}`);
    expect(remove.response.status).toBe(real('memory_delete').status);
    expect(remove.text).toBe('');
  });

  it('a provider test answers the recorded keys for success and failure', async () => {
    const mock = await signedIn();
    const ok = await ask(mock, 'POST', '/api/providers/primary/test');
    expectSameEnvelope('provider_test', ok.response);
    expectSameShape('provider_test', ok.body);
    mock.state.providers[0]!.base_url = 'https://unreachable.example.test/v1';
    const failed = await ask(mock, 'POST', '/api/providers/primary/test');
    expectSameShape('provider_test_second', failed.body);
    expect((failed.body as { ok: boolean }).ok).toBe(false);
  });
});

describe('the mock refuses with the codes and statuses of the real server', () => {
  const codeOf = (body: unknown) => (body as { error: { code: string; message: string } }).error;

  it('answers the same error body shape and code for every recorded refusal', async () => {
    const mock = createMock();
    vi.stubGlobal('fetch', mock.fetch);

    const wrongToken = await ask(mock, 'POST', '/api/auth/exchange', { token: 'x'.repeat(64) });
    expect(wrongToken.response.status).toBe(real('exchange_bad').status);
    expect(codeOf(wrongToken.body).code).toBe(codeOf(real('exchange_bad').body).code);
    expect(codeOf(wrongToken.body).message).toBe(codeOf(real('exchange_bad').body).message);

    const noSession = await ask(mock, 'GET', '/api/nodes');
    expect(noSession.response.status).toBe(real('unauth_nodes').status);
    expect(codeOf(noSession.body)).toEqual(codeOf(real('unauth_nodes').body));

    const exchanged = await mock.fetch('/api/auth/exchange', {
      method: 'POST',
      headers: { 'X-Nodivara': '1', 'Content-Type': 'application/json' },
      body: JSON.stringify({ token: MOCK_TOKEN }),
    });
    expect(exchanged.status).toBe(real('exchange').status);

    const noHeader = await mock.fetch('/api/conversations', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
    expect(noHeader.status).toBe(real('csrf_no_header').status);
    expect(codeOf(await noHeader.json())).toEqual(codeOf(real('csrf_no_header').body));

    const missing = await ask(mock, 'GET', '/api/nodes/nope/inventory');
    expect(missing.response.status).toBe(real('inventory_404').status);
    expect(codeOf(missing.body)).toEqual(codeOf(real('inventory_404').body));

    const duplicate = await ask(mock, 'POST', '/api/providers', { name: 'primary', base_url: 'https://llm.example.test/v1', model: 'm' });
    expect(duplicate.response.status).toBe(real('provider_add_dup').status);
    expect(codeOf(duplicate.body)).toEqual(codeOf(real('provider_add_dup').body));

    const invalid = await ask(mock, 'POST', '/api/providers', { name: '', base_url: 'x', model: '' });
    expect(invalid.response.status).toBe(real('provider_add_bad').status);
    expect(codeOf(invalid.body)).toEqual(codeOf(real('provider_add_bad').body));

    const empty = await ask(mock, 'POST', `/api/conversations/${id(301)}/turn`, { message: '' });
    expect(empty.response.status).toBe(real('turn_bad').status);
    expect(codeOf(empty.body)).toEqual(codeOf(JSON.parse(real('turn_bad').raw ?? '{}')));

    const approve404 = await ask(mock, 'POST', `/api/memories/${id(999)}/approve`);
    expect(approve404.response.status).toBe(real('memory_approve_404').status);
    expect(codeOf(approve404.body)).toEqual(codeOf(real('memory_approve_404').body));
  });

  it('refuses a second default with default_exists (a real server rule the contract text omitted)', async () => {
    const mock = await signedIn();
    const { response, body } = await ask(mock, 'POST', '/api/providers', {
      name: 'other',
      base_url: 'https://llm.example.test/v1',
      model: 'm',
      is_default: true,
    });
    expect(response.status).toBe(409);
    expect(codeOf(body).code).toBe('default_exists');
  });
});
