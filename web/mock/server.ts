// A faithful mock of the local API of ADR 0029: every endpoint, the same error body, the same
// session and CSRF rules, the real i18n catalog (crates/i18n/locales) and server-sent events with
// realistic timing. It is plain TypeScript over web-standard Headers/Response, so the dev server
// (mock/vite-plugin.ts) and the tests (createMock().fetch) share it.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { buildFixtures, HOSTILE, id as fakeId, type Fixtures } from './fixtures.ts';
import type { ChatMessage, MemoryNote, ProviderInput, ProviderView } from '../src/lib/types';

export const MOCK_TOKEN = 'mock-valid-token-0001';
export const MOCK_EXPIRED_TOKEN = 'mock-expired-token-0001';
export const MOCK_ORIGIN = 'http://127.0.0.1:5173';
const BODY_LIMIT = 64 * 1024;

export interface MockOptions {
  /** Free-text fields carry attack strings (see fixtures.ts). */
  hostile?: boolean;
  /** NODIVARA_MASTER_KEY present on the server. Default true. */
  masterKey?: boolean;
  /** Delay between stream events. Default 0 (tests); the dev server uses a few dozen ms. */
  turnDelayMs?: number;
  /** Cut the event stream into tiny chunks to exercise the decoder. */
  splitChunks?: boolean;
  /** Line ending of the event stream. */
  eol?: '\n' | '\r\n' | '\r';
  /** Serve the catalog without a session (the sign-in page needs text before the exchange). */
  openCatalog?: boolean;
  /** `infra serve --enable-execution` was given. Default false, like the real server. */
  executionEnabled?: boolean;
  /** A run ends as failed (command_failed) instead of succeeded. */
  executionFails?: boolean;
}

export interface MockRequest {
  method: string;
  path: string;
  query: URLSearchParams;
  headers: Headers;
  body: string;
  signal?: AbortSignal;
}

export interface LoggedRequest {
  method: string;
  path: string;
  headers: Record<string, string>;
  body: unknown;
}

function loadCatalog(locale: 'en' | 'id', prefix = 'ui.'): Record<string, string> {
  const source = readFileSync(join(import.meta.dirname, '..', '..', 'crates', 'i18n', 'locales', `${locale}.txt`), 'utf8');
  const out: Record<string, string> = {};
  for (const raw of source.split('\n')) {
    const line = raw.trim();
    if (line === '' || line.startsWith('#')) continue;
    const eq = line.indexOf('=');
    if (eq < 0) continue;
    const key = line.slice(0, eq).trim();
    if (key.startsWith(prefix)) out[key] = line.slice(eq + 1).trim();
  }
  return out;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

interface Scripted {
  event: string;
  data: Record<string, unknown>;
}

function turnScript(message: string, hostile: boolean): Scripted[] {
  if (message.includes('error:')) {
    return [
      { event: 'delta', data: { text: 'Let me look at that. ' } },
      { event: 'error', data: { code: 'ai_transport', message: 'The provider could not be reached. Check the address, your network and that the service is running.' } },
    ];
  }
  if (message.includes('hostile') || hostile) {
    return [
      { event: 'tool', data: { name: HOSTILE[0], tier: 0, status: 'ok', duration_ms: 3 } },
      ...HOSTILE.map((text) => ({ event: 'delta', data: { text: `${text}\n` } })),
      { event: 'done', data: { rounds: 1, tool_calls: 1 } },
    ];
  }
  if (message.includes('huge')) {
    return [
      ...Array.from({ length: 4 }, () => ({ event: 'delta', data: { text: 'a'.repeat(100_000) } })),
      { event: 'done', data: { rounds: 1, tool_calls: 0 } },
    ];
  }
  if (message.includes('slow')) {
    return [
      ...Array.from({ length: 200 }, (_, i) => ({ event: 'delta', data: { text: `word${i} ` } })),
      { event: 'done', data: { rounds: 1, tool_calls: 0 } },
    ];
  }
  return [
    { event: 'tool', data: { name: 'search_inventory', tier: 0, status: 'ok', duration_ms: 12 } },
    { event: 'delta', data: { text: 'Port 8080 is listed on the old ' } },
    { event: 'delta', data: { text: 'server but not on the new one.\n' } },
    { event: 'delta', data: { text: 'Start the service, then compare again. é\u{1F600}' } },
    { event: 'done', data: { rounds: 1, tool_calls: 1, usage: { estimated: true, tokens: 42 } } },
  ];
}

export function createMock(options: MockOptions = {}) {
  const fixtures: Fixtures = buildFixtures({ hostile: options.hostile });
  const catalogs = { en: loadCatalog('en'), id: loadCatalog('id') };
  // The server's own error texts, so a message in the mock is the message of the real server.
  const errorTexts = { en: loadCatalog('en', 'api.error.'), id: loadCatalog('id', 'api.error.') };
  const state = {
    ...fixtures,
    sites: [{ id: fakeId(901), domain: 'demo.test', upstream_port: 3000, config_path: '/etc/nodivara/sites/demo.test.conf', status: 'proposed', created_at: '2026-11-27T10:00:00Z' }],
    masterKey: options.masterKey ?? true,
    executionEnabled: options.executionEnabled ?? false,
    executionFails: options.executionFails ?? false,
    token: { value: MOCK_TOKEN, used: false },
    sessions: new Set<string>(),
    activeTurns: new Set<string>(),
    /** Every request the mock saw, for tests (bodies are kept so tests can prove what was sent). */
    requests: [] as LoggedRequest[],
    nextConversation: 2,
    nextSession: 1,
  };
  const delay = options.turnDelayMs ?? 0;
  const eol = options.eol ?? '\n';

  function localeOf(req: MockRequest): 'en' | 'id' {
    const lang = (req.query.get('lang') ?? req.headers.get('accept-language') ?? 'en').slice(0, 2).toLowerCase();
    return lang === 'id' ? 'id' : 'en';
  }

  function json(status: number, body: unknown, extra: Record<string, string> = {}): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store', ...extra },
    });
  }

  function fail(req: MockRequest, status: number, code: string): Response {
    const locale = localeOf(req);
    return json(status, { error: { code, message: errorTexts[locale][`api.error.${code}`] ?? code } });
  }

  function sessionOf(req: MockRequest): string | null {
    const cookie = req.headers.get('cookie') ?? '';
    const match = /(?:^|;\s*)nv_session=([^;]+)/.exec(cookie);
    return match?.[1] && state.sessions.has(match[1]) ? match[1] : null;
  }

  function stream(req: MockRequest, script: Scripted[], onEnd: () => void): Response {
    const encoder = new TextEncoder();
    let cancelled = false;
    let chunkIndex = 0;
    const readable = new ReadableStream<Uint8Array>({
      async start(controller) {
        const abort = () => {
          cancelled = true;
          try {
            controller.error(new DOMException('The operation was aborted.', 'AbortError'));
          } catch {
            // already closed
          }
        };
        req.signal?.addEventListener('abort', abort, { once: true });
        try {
          for (const item of script) {
            if (cancelled) return;
            if (delay > 0) await sleep(delay);
            if (cancelled) return;
            const text = `event: ${item.event}\ndata: ${JSON.stringify(item.data)}\n\n`.replaceAll('\n', eol);
            const bytes = encoder.encode(text);
            if (options.splitChunks) {
              for (let i = 0; i < bytes.length; ) {
                const size = (chunkIndex++ % 5) + 1;
                controller.enqueue(bytes.slice(i, i + size));
                i += size;
              }
            } else {
              controller.enqueue(bytes);
            }
            await Promise.resolve();
          }
          if (!cancelled) controller.close();
        } finally {
          req.signal?.removeEventListener('abort', abort);
          onEnd();
        }
      },
      cancel() {
        cancelled = true;
      },
    });
    return new Response(readable, {
      headers: { 'Content-Type': 'text/event-stream; charset=utf-8', 'Cache-Control': 'no-store' },
    });
  }

  async function route(req: MockRequest): Promise<Response> {
    const { method, path } = req;
    const parts = path
      .split('/')
      .filter(Boolean)
      .map((segment) => {
        try {
          return decodeURIComponent(segment);
        } catch {
          return segment;
        }
      }); // ['api', ...]
    if (path === '/healthz') return json(200, { status: 'ok' });
    if (parts[0] !== 'api') return fail(req, 404, 'not_found');

    // CSRF: every request that changes data needs the custom header (and JSON when it has a body).
    let body: unknown;
    if (method !== 'GET') {
      if (req.headers.get('x-nodivara') !== '1') return fail(req, 403, 'csrf_header');
      if (req.body.length > BODY_LIMIT) return fail(req, 413, 'body_too_large');
      if (req.body !== '') {
        if (!(req.headers.get('content-type') ?? '').toLowerCase().startsWith('application/json')) {
          return fail(req, 415, 'bad_content_type');
        }
        try {
          body = JSON.parse(req.body);
        } catch {
          return fail(req, 400, 'bad_request');
        }
      }
    }

    const sub = parts.slice(1);
    if (method === 'POST' && sub.join('/') === 'auth/exchange') {
      const token = (body as { token?: unknown } | undefined)?.token;
      if (typeof token !== 'string' || token !== state.token.value || state.token.used) {
        return fail(req, 401, 'invalid_token');
      }
      state.token.used = true; // one-time
      const id = `mock-session-${state.nextSession++}`;
      state.sessions.add(id);
      return new Response(null, {
        status: 204,
        headers: { 'Set-Cookie': `nv_session=${id}; HttpOnly; SameSite=Strict; Path=/` },
      });
    }
    if (method === 'GET' && sub[0] === 'i18n' && sub.length === 2 && (options.openCatalog ?? true)) {
      const locale = sub[1] === 'id' ? 'id' : 'en';
      return json(200, catalogs[locale]);
    }

    const session = sessionOf(req);
    if (!session) return fail(req, 401, 'unauthenticated');

    const [head, name, action] = sub;
    const query = req.query;
    switch (head) {
      case 'auth':
        if (method === 'POST' && name === 'logout') {
          state.sessions.delete(session);
          return new Response(null, { status: 204, headers: { 'Set-Cookie': 'nv_session=; Max-Age=0; Path=/' } });
        }
        break;
      case 'session':
        return json(200, {
          user: { email: 'owner@example.test', name: 'Local Owner' },
          org: { name: 'Example Org', slug: 'example' },
          locale: localeOf(req),
          master_key_configured: state.masterKey,
          version: '0.0.0-mock',
          execution_enabled: state.executionEnabled,
        });
      case 'i18n':
        if (sub.length === 2) return json(200, catalogs[sub[1] === 'id' ? 'id' : 'en']);
        break;
      case 'nodes': {
        if (method === 'POST' && name && action === 'agent-token') {
          return json(200, { node_id: name, token_id: fakeId(950), token: 'nv_agent_rotated_token_123' });
        }
        if (method !== 'GET') break;
        if (!name) return json(200, state.nodes);
        if (action === 'logs') {
          const service = query.get('service') ?? '';
          if (!service) return fail(req, 400, 'invalid_service');
          return json(200, { node_id: name, service, line_count: 2, truncated: false, content: `line 1 ${service}\nline 2 ok` });
        }
        if (action === 'inventory') {
          const view = state.inventory[name];
          if (!view) return fail(req, 404, 'not_found');
          const kind = query.get('kind');
          const limit = Number(query.get('limit') ?? '1000');
          const items = view.items.filter((i) => !kind || i.kind === kind).slice(0, Number.isFinite(limit) ? limit : 1000);
          return json(200, { ...view, items });
        }
        break;
      }
      case 'findings': {
        if (method !== 'GET') break;
        const min = query.get('min_severity');
        const rank: Record<string, number> = { info: 0, important: 1, critical: 2 };
        const findings = state.findings.filter((f) => !min || (rank[f.severity] ?? 0) >= (rank[min] ?? 0));
        return json(200, {
          comparison: { source: 'node-alpha', target: 'node-gamma', created_at: '2026-11-21T11:00:00Z' },
          findings,
        });
      }
      case 'plans': {
        if (method === 'GET') {
          if (!name) {
            return json(
              200,
              state.plans.map(({ id: pid, name: n, status, source_node, target_node, created_at }) => ({
                id: pid,
                name: n,
                status,
                source_node,
                target_node,
                created_at,
              })),
            );
          }
          const plan = state.plans.find((p) => p.name === name || p.id === name);
          return plan ? json(200, plan) : fail(req, 404, 'not_found');
        }
        if (method === 'POST' && name && action === 'approve') {
          const plan = state.plans.find((p) => p.id === name || p.name === name);
          if (!plan) return fail(req, 404, 'not_found');
          if (plan.status !== 'draft' && plan.status !== 'reviewed') return fail(req, 409, 'plan_not_approvable');
          plan.status = 'approved';
          return json(200, { id: plan.id, name: plan.name, status: plan.status });
        }
        if (method === 'POST' && name && action === 'run') {
          if (!state.executionEnabled) return fail(req, 409, 'execution_disabled');
          const plan = state.plans.find((p) => p.id === name || p.name === name);
          if (!plan) return fail(req, 404, 'not_found');
          if (plan.status !== 'approved' && plan.status !== 'paused') return fail(req, 409, 'plan_not_approved');
          if (state.executionFails) {
            plan.status = 'paused';
            return fail(req, 409, 'plan_gate_blocked');
          }
          plan.status = 'completed';
          plan.steps = plan.steps.map((s) => ({ ...s, status: 'succeeded' }));
          return json(200, {
            id: plan.id,
            name: plan.name,
            status: plan.status,
            steps: plan.steps.map((s) => ({ seq: s.seq, kind: s.kind, status: s.status, reason: null })),
          });
        }
        break;
      }
      case 'transports': {
        if (method === 'GET' && !name) return json(200, state.transports);
        if (method === 'POST' && name && action === 'credentials') {
          if (!state.masterKey) return fail(req, 409, 'master_key_missing');
          const transport = state.transports.find((t) => t.id === name || t.name === name);
          if (!transport) return fail(req, 404, 'not_found');
          if (transport.kind === 'local') return fail(req, 409, 'credentials_not_accepted');
          const creds = (body as { credentials?: unknown } | undefined)?.credentials;
          if (
            typeof creds !== 'object' ||
            creds === null ||
            Array.isArray(creds) ||
            Object.keys(creds).length === 0 ||
            Object.values(creds).some((v) => typeof v !== 'string')
          ) {
            return fail(req, 400, 'bad_credentials');
          }
          transport.verified_at = null;
          return json(200, { id: transport.id, name: transport.name, updated: true, verified_at: null });
        }
        break;
      }
      case 'monitors': {
        if (method === 'GET' && !name) return json(200, state.monitors);
        if (method === 'POST' && !name) {
          const inp = (body ?? {}) as { name?: string; kind?: 'http' | 'tcp' | 'tls'; target?: string };
          if (!inp.name || !inp.kind || !inp.target) return fail(req, 400, 'invalid_input');
          const m = { id: fakeId(600 + state.monitors.length + 1), name: inp.name, kind: inp.kind, target: inp.target, current_status: 'pending', last_checked_at: null };
          state.monitors.push(m);
          return json(201, m);
        }
        if (method === 'DELETE' && name) {
          state.monitors = state.monitors.filter((m) => m.id !== name);
          return new Response(null, { status: 204 });
        }
        break;
      }
      case 'incidents': {
        if (method === 'GET' && !name) return json(200, state.incidents);
        if (method === 'POST' && name && action === 'ack') {
          const inc = state.incidents.find((i) => i.id === name);
          if (!inc) return fail(req, 404, 'not_found');
          inc.acknowledged_at = new Date().toISOString();
          return json(200, { id: inc.id, acknowledged: true });
        }
        break;
      }
      case 'channels': {
        if (method === 'GET' && !name) return json(200, state.channels);
        if (method === 'POST' && name === 'telegram' && action === 'link-code') {
          return json(201, { id: fakeId(850), code: 'ABCD1234EFGH', command: '/link ABCD1234EFGH' });
        }
        if (method === 'POST' && !name) {
          const inp = (body ?? {}) as { kind?: 'telegram' | 'webhook'; name?: string; token?: string };
          if (!inp.name || !inp.kind) return fail(req, 400, 'invalid_input');
          if (inp.token && !state.masterKey) return fail(req, 409, 'master_key_missing');
          const c = { id: fakeId(800 + state.channels.length + 1), kind: inp.kind, name: inp.name, has_secret: Boolean(inp.token), is_active: true };
          state.channels.push(c);
          return json(201, c);
        }
        if (method === 'DELETE' && name) {
          state.channels = state.channels.filter((c) => c.id !== name);
          return new Response(null, { status: 204 });
        }
        break;
      }
      case 'sites': {
        if (method === 'GET' && !name) return json(200, { sites: state.sites });
        if (method === 'POST' && !name) {
          const inp = (body ?? {}) as { domain?: string; upstream_port?: number };
          if (!inp.domain || !inp.domain.endsWith('.test')) return fail(req, 400, 'invalid_test_domain');
          if (typeof inp.upstream_port !== 'number' || inp.upstream_port < 1 || inp.upstream_port > 65535) return fail(req, 400, 'invalid_upstream_port');
          const site = { id: fakeId(900 + state.sites.length + 1), domain: inp.domain, upstream_port: inp.upstream_port, config_path: `/etc/nodivara/sites/${inp.domain}.conf`, status: 'proposed', created_at: '2026-11-27T12:00:00Z' };
          state.sites.push(site);
          return json(201, { site, config_preview: `server { server_name ${inp.domain}; proxy_pass http://127.0.0.1:${inp.upstream_port}; }` });
        }
        break;
      }
      case 'providers': {
        if (method === 'GET' && !name) return json(200, state.providers);
        if (method === 'POST' && !name) {
          const input = body as ProviderInput | undefined;
          if (
            !input ||
            typeof input.name !== 'string' ||
            typeof input.base_url !== 'string' ||
            typeof input.model !== 'string' ||
            input.name === ''
          ) {
            return fail(req, 400, 'invalid_input');
          }
          if (input.key !== undefined && !state.masterKey) return fail(req, 409, 'master_key_missing');
          // The server only adds: an existing name is refused (replace = delete, then add), and a
          // second default is refused. The first provider becomes the default by itself.
          if (state.providers.some((p) => p.name === input.name)) return fail(req, 409, 'already_exists');
          if (input.is_default === true && state.providers.some((p) => p.is_default)) {
            return fail(req, 409, 'default_exists');
          }
          const view: ProviderView = {
            name: input.name,
            base_url: input.base_url,
            model: input.model,
            params: input.params ?? {},
            has_key: input.key !== undefined,
            is_default: input.is_default === true || state.providers.length === 0,
          };
          state.providers.push(view);
          return json(201, view); // never echoes the key
        }
        if (method === 'DELETE' && name) {
          const at = state.providers.findIndex((p) => p.name === name);
          if (at < 0) return fail(req, 404, 'not_found');
          state.providers.splice(at, 1);
          return new Response(null, { status: 204 });
        }
        if (method === 'POST' && name && action === 'test') {
          const provider = state.providers.find((p) => p.name === name);
          if (!provider) return fail(req, 404, 'not_found');
          if (provider.base_url.includes('unreachable')) {
            return json(200, {
              ok: false,
              code: 'ai_transport',
              message: errorTexts[localeOf(req)]['api.error.ai_transport'] ?? '',
            });
          }
          return json(200, { ok: true, latency_ms: 120, model: provider.model });
        }
        break;
      }
      case 'memories': {
        if (method === 'GET' && !name) {
          const status = query.get('status') ?? 'active';
          return json(200, state.memories.filter((m) => m.status === status));
        }
        const note = state.memories.find((m) => String(m.id) === name);
        if (method === 'POST' && action === 'approve') {
          if (!note) return fail(req, 404, 'not_found');
          (note as MemoryNote).status = 'active';
          return new Response(null, { status: 204 });
        }
        if (method === 'DELETE' && name) {
          if (!note) return fail(req, 404, 'not_found');
          state.memories.splice(state.memories.indexOf(note), 1);
          return new Response(null, { status: 204 });
        }
        break;
      }
      case 'proposals': {
        if (method === 'GET' && !name) {
          const status = query.get('status');
          return json(200, state.proposals.filter((p) => !status || p.status === status));
        }
        if (method === 'POST' && name && action === 'decision') {
          const proposal = state.proposals.find((p) => p.id === name);
          if (!proposal) return fail(req, 404, 'not_found');
          const { decision, typed_confirmation } = (body as { decision?: unknown; typed_confirmation?: unknown } | undefined) ?? {};
          if (decision !== 'approve' && decision !== 'reject') return fail(req, 400, 'invalid_input');
          // Only a pending proposal can be decided, and only before it expires. Nothing is executed.
          if (proposal.status === 'pending' && new Date(proposal.expires_at).getTime() < Date.now()) proposal.status = 'expired';
          if (proposal.status === 'expired') return fail(req, 409, 'proposal_expired');
          if (proposal.status !== 'pending') return fail(req, 409, 'proposal_decided');
          if (decision === 'approve' && proposal.tier === 4) {
            if (!proposal.action) return fail(req, 409, 'action_invalid');
            if (typed_confirmation !== proposal.action.target_label) return fail(req, 400, 'typed_confirmation_mismatch');
          }
          proposal.status = decision === 'approve' ? 'approved' : 'rejected';
          proposal.approval = {
            decision: decision === 'approve' ? 'approved' : 'rejected',
            typed_confirmation: decision === 'approve' && proposal.tier === 4,
            created_at: new Date().toISOString(),
          };
          return json(200, { id: proposal.id, status: proposal.status });
        }
        if (method === 'POST' && name && action === 'dry-run') {
          const proposal = state.proposals.find((p) => p.id === name);
          if (!proposal) return fail(req, 404, 'not_found');
          if (!proposal.action) return fail(req, 409, 'proposal_no_action');
          const dry = {
            steps: [`Inspect ${proposal.action.target_label}`, `Preview ${proposal.action.kind}`],
            current_state: 'active (mock)',
            diff: `--- a/${proposal.action.target_label}\n+++ b/${proposal.action.target_label}\n@@ -1 +1 @@\n-old\n+new`,
            dry_run_at: new Date().toISOString(),
          };
          proposal.dry_run = dry;
          return json(200, { id: proposal.id, ...dry });
        }
        if (method === 'POST' && name && action === 'execute') {
          // The same order of checks as the server: switched off, unknown, then the state.
          if (!state.executionEnabled) return fail(req, 409, 'execution_disabled');
          const proposal = state.proposals.find((p) => p.id === name);
          if (!proposal) return fail(req, 404, 'not_found');
          if (proposal.status === 'executed' || proposal.status === 'failed') return fail(req, 409, 'proposal_already_run');
          if (proposal.status === 'approved' && new Date(proposal.expires_at).getTime() < Date.now()) proposal.status = 'expired';
          if (proposal.status === 'expired') return fail(req, 409, 'proposal_expired');
          if (proposal.status !== 'approved') return fail(req, 409, 'proposal_not_approved');
          if (!proposal.action) return fail(req, 409, 'proposal_no_action');
          const started = new Date().toISOString();
          const ok = !state.executionFails;
          proposal.execution = {
            status: ok ? 'succeeded' : 'failed',
            verified: ok,
            reason: ok ? 'ok' : 'command_failed',
            output: ok ? 'active' : `Failed to restart ${proposal.action.target_label}: Unit not found.`,
            started_at: started,
            finished_at: new Date().toISOString(),
          };
          proposal.has_backup = proposal.tier >= 3;
          proposal.status = ok ? 'executed' : 'failed';
          return json(200, { id: proposal.id, status: proposal.status, has_backup: proposal.has_backup, execution: proposal.execution });
        }
        if (method === 'POST' && name && action === 'rollback') {
          if (!state.executionEnabled) return fail(req, 409, 'execution_disabled');
          const proposal = state.proposals.find((p) => p.id === name);
          if (!proposal) return fail(req, 404, 'not_found');
          if (!proposal.execution) return fail(req, 409, 'proposal_not_executed');
          if (proposal.execution.status === 'running') return fail(req, 409, 'execution_running');
          if (!proposal.has_backup) return fail(req, 409, 'backup_missing');
          if (proposal.rollback) return fail(req, 409, 'already_rolled_back');
          const ok = !state.executionFails;
          proposal.rollback = {
            status: ok ? 'succeeded' : 'failed',
            verified: ok,
            reason: ok ? 'ok' : 'command_failed',
            output: ok ? 'restored' : 'rollback failed',
            started_at: new Date().toISOString(),
            finished_at: new Date().toISOString(),
          };
          return json(200, { id: proposal.id, rollback: proposal.rollback });
        }
        break;
      }
      case 'conversations': {
        if (method === 'GET' && !name) return json(200, state.conversations);
        if (method === 'POST' && !name) {
          const id = fakeId(300 + state.nextConversation++);
          const title = (body as { title?: unknown } | undefined)?.title;
          const conversation = {
            id,
            title: typeof title === 'string' && title !== '' ? title : 'New conversation',
            archived: false,
            created_at: '2026-11-23T08:00:00.000Z',
          };
          state.conversations.unshift(conversation);
          state.messages[id] = [];
          return json(201, conversation);
        }
        const history = name ? state.messages[name] : undefined;
        if (!name || !history) return fail(req, 404, 'not_found');
        if (method === 'GET' && action === 'messages') return json(200, history);
        if (method === 'POST' && action === 'turn') {
          const message = (body as { message?: unknown } | undefined)?.message;
          if (typeof message !== 'string' || message.trim() === '') return fail(req, 400, 'message_empty');
          if (state.activeTurns.has(name)) return fail(req, 409, 'busy');
          state.activeTurns.add(name);
          const at = () => new Date().toISOString();
          history.push({ seq: history.length + 1, role: 'user', content: message, created_at: at(), tools: [] });
          const script = turnScript(message, options.hostile === true);
          return stream(req, script, () => {
            state.activeTurns.delete(name);
            // Like the server: the tool calls of a turn are stored on an assistant row of their
            // own (empty text), then the answer.
            const tools = script.filter((x) => x.event === 'tool').map((x) => String(x.data.name));
            if (tools.length > 0) {
              history.push({ seq: history.length + 1, role: 'assistant', content: '', created_at: at(), tools });
            }
            const text = script
              .filter((x) => x.event === 'delta')
              .map((x) => String(x.data.text))
              .join('');
            history.push({ seq: history.length + 1, role: 'assistant', content: text, created_at: at(), tools: [] });
          });
        }
        break;
      }
      default:
        break;
    }
    return fail(req, 404, 'not_found');
  }

  async function handle(req: MockRequest): Promise<Response> {
    let parsed: unknown = undefined;
    try {
      parsed = req.body === '' ? undefined : JSON.parse(req.body);
    } catch {
      parsed = undefined;
    }
    state.requests.push({
      method: req.method,
      path: req.path,
      headers: Object.fromEntries(req.headers.entries()),
      body: parsed,
    });
    return route(req);
  }

  /**
   * A fetch that behaves like the browser against this mock: same-origin only, a cookie jar for
   * Set-Cookie, and an abort that errors the response stream. Use as globalThis.fetch in tests.
   */
  const cookies = new Map<string, string>();
  async function mockFetch(input: string | URL | Request, init: RequestInit = {}): Promise<Response> {
    const url = new URL(typeof input === 'string' || input instanceof URL ? input : input.url, MOCK_ORIGIN);
    if (url.origin !== MOCK_ORIGIN) throw new TypeError('cross-origin request refused by the mock');
    const headers = new Headers(init.headers);
    if (cookies.size > 0 && init.credentials !== 'omit') {
      headers.set('cookie', [...cookies].map(([k, v]) => `${k}=${v}`).join('; '));
    }
    if (init.signal?.aborted) throw new DOMException('The operation was aborted.', 'AbortError');
    const response = await handle({
      method: (init.method ?? 'GET').toUpperCase(),
      path: url.pathname,
      query: url.searchParams,
      headers,
      body: typeof init.body === 'string' ? init.body : '',
      signal: init.signal ?? undefined,
    });
    const setCookie = response.headers.get('set-cookie');
    if (setCookie) {
      const [pair = ''] = setCookie.split(';');
      const eq = pair.indexOf('=');
      const key = pair.slice(0, eq);
      const value = pair.slice(eq + 1);
      if (value === '') cookies.delete(key);
      else cookies.set(key, value);
    }
    return response;
  }

  return { handle, fetch: mockFetch as typeof fetch, state, cookies };
}

export type Mock = ReturnType<typeof createMock>;
