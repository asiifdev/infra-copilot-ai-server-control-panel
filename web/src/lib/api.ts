// Typed client for the local API (ADR 0029). Same-origin only, cookie session (HttpOnly, never
// readable here), X-Nodivara on every non-GET request. Errors carry a fixed code, never server
// text that is shown without the catalog. No secret is ever kept: the provider key goes out in
// one request body and the caller clears it.

import { readSse } from './sse';
import type {
  AgentTokenResult,
  ChannelView,
  ChatMessage,
  Conversation,
  FindingsView,
  IncidentView,
  InventoryView,
  LocalSite,
  MemoryNote,
  MonitorView,
  NodeLogsView,
  NodeSummary,
  PlanDetail,
  PlanStepStatus,
  PlanSummary,
  ProposalDryRun,
  ProposalExecution,
  ProposalRollback,
  ProposalView,
  ProviderInput,
  ProviderTestResult,
  ProviderView,
  SessionInfo,
  TransportSummary,
  TurnEvent,
} from './types';

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

const REQUEST_TIMEOUT_MS = 30_000;
const CODE_PATTERN = /^[a-z][a-z0-9_]{0,40}$/;

let locale = 'en';
let unauthorizedHandler: (() => void) | null = null;

export function setApiLocale(value: string): void {
  locale = value;
}

/** Called once when a request other than the token exchange answers 401. */
export function onUnauthorized(handler: (() => void) | null): void {
  unauthorizedHandler = handler;
}

export function buildUrl(path: string, query?: Record<string, string | number | undefined>): string {
  // Same-origin by construction: a path must be absolute on this origin and under /api.
  if (!path.startsWith('/api/') || path.startsWith('//')) {
    throw new ApiError(0, 'bad_request', 'path must be a same-origin /api path');
  }
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value !== undefined && value !== '') params.set(key, String(value));
  }
  params.set('lang', locale);
  return `${path}?${params.toString()}`;
}

function linkAbort(source: AbortSignal | undefined, target: AbortController): () => void {
  if (!source) return () => {};
  if (source.aborted) {
    target.abort();
    return () => {};
  }
  const onAbort = () => target.abort();
  source.addEventListener('abort', onAbort, { once: true });
  return () => source.removeEventListener('abort', onAbort);
}

async function errorFrom(response: Response): Promise<ApiError> {
  let code = `http_${response.status}`;
  let message = '';
  try {
    const parsed: unknown = JSON.parse(await response.text());
    const error = (parsed as { error?: { code?: unknown; message?: unknown } } | null)?.error;
    if (typeof error?.code === 'string' && CODE_PATTERN.test(error.code)) code = error.code;
    if (typeof error?.message === 'string') message = error.message.slice(0, 500);
  } catch {
    // Not the contract error body: keep the status-derived code.
  }
  if (response.status === 401 && code === 'http_401') code = 'unauthorized';
  return new ApiError(response.status, code, message);
}

interface RequestOptions {
  body?: unknown;
  query?: Record<string, string | number | undefined>;
  signal?: AbortSignal;
  /** Do not treat a 401 as an expired session (used by the token exchange). */
  quiet401?: boolean;
  /** Leave the response open (event stream); the caller reads the body. */
  stream?: boolean;
}

async function send(method: string, path: string, options: RequestOptions = {}): Promise<Response> {
  const url = buildUrl(path, options.query);
  const headers: Record<string, string> = { Accept: options.stream ? 'text/event-stream' : 'application/json' };
  let body: string | undefined;
  if (method !== 'GET') {
    headers['X-Nodivara'] = '1';
    if (options.body !== undefined) {
      headers['Content-Type'] = 'application/json';
      body = JSON.stringify(options.body);
    }
  }
  const controller = new AbortController();
  const unlink = linkAbort(options.signal, controller);
  const timer = options.stream ? null : setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  let response: Response;
  try {
    response = await globalThis.fetch(url, {
      method,
      headers,
      body,
      credentials: 'same-origin',
      cache: 'no-store',
      referrerPolicy: 'no-referrer',
      signal: controller.signal,
    });
  } catch (cause) {
    if (options.signal?.aborted) throw cause;
    throw new ApiError(0, controller.signal.aborted ? 'timeout' : 'network', '');
  } finally {
    if (timer) clearTimeout(timer);
    if (!options.stream) unlink();
  }
  if (!response.ok) {
    unlink();
    const error = await errorFrom(response);
    if (error.status === 401 && !options.quiet401) unauthorizedHandler?.();
    throw error;
  }
  return response;
}

async function json<T>(method: string, path: string, options?: RequestOptions): Promise<T> {
  const response = await send(method, path, options);
  try {
    return (await response.json()) as T;
  } catch {
    throw new ApiError(response.status, 'bad_response', '');
  }
}

async function empty(method: string, path: string, options?: RequestOptions): Promise<void> {
  await send(method, path, options);
}

const enc = encodeURIComponent;
const get = <T>(path: string, signal?: AbortSignal, query?: Record<string, string | number | undefined>) =>
  json<T>('GET', path, { query, signal });
const post = <T>(path: string, body?: unknown, signal?: AbortSignal) =>
  json<T>('POST', path, { body, signal });
const del = (path: string) => empty('DELETE', path);
const idPath = (base: string, id: string | number, sub = '') => `/api/${base}/${enc(String(id))}${sub}`;
const prop = (id: string | number, sub: string) => idPath('proposals', id, `/${sub}`);

export const api = {
  exchange: (token: string, signal?: AbortSignal) =>
    empty('POST', '/api/auth/exchange', { body: { token }, signal, quiet401: true }),
  logout: () => empty('POST', '/api/auth/logout'),
  session: (signal?: AbortSignal) => get<SessionInfo>('/api/session', signal),
  i18n: (loc: string, signal?: AbortSignal) => get<Record<string, string>>(`/api/i18n/${enc(loc)}`, signal),
  nodes: (signal?: AbortSignal) => get<NodeSummary[]>('/api/nodes', signal),
  inventory: (name: string, kind?: string, limit?: number, signal?: AbortSignal) =>
    get<InventoryView>(`/api/nodes/${enc(name)}/inventory`, signal, { kind, limit }),
  nodeLogs: (id: string, service: string, lines = 50, signal?: AbortSignal) =>
    get<NodeLogsView>(`/api/nodes/${enc(id)}/logs`, signal, { service, lines }),
  rotateAgentToken: (id: string) => post<AgentTokenResult>(`/api/nodes/${enc(id)}/agent-token`),
  sites: (signal?: AbortSignal) => get<{ sites: LocalSite[] }>('/api/sites', signal),
  createSite: (domain: string, upstream_port: number, opts?: { php_version?: string; tls_enabled?: boolean }) =>
    post<{ site: LocalSite; config_preview: string }>('/api/sites', {
      domain,
      upstream_port,
      ...(opts?.php_version ? { php_version: opts.php_version } : {}),
      ...(opts?.tls_enabled !== undefined ? { tls_enabled: opts.tls_enabled } : {}),
    }),
  findings: (minSeverity?: string, limit?: number, signal?: AbortSignal) =>
    get<FindingsView>('/api/findings', signal, { min_severity: minSeverity, limit }),
  plans: (signal?: AbortSignal) => get<PlanSummary[]>('/api/plans', signal),
  plan: (name: string, signal?: AbortSignal) => get<PlanDetail>(`/api/plans/${enc(name)}`, signal),
  approvePlan: (id: string) => post<{ id: string; name: string; status: string }>(`/api/plans/${enc(id)}/approve`),
  runPlan: (id: string) =>
    post<{ id: string; name: string; status: string; steps: { seq: number; kind: string; status: PlanStepStatus; reason: string | null }[] }>(`/api/plans/${enc(id)}/run`),
  transports: (signal?: AbortSignal) => get<TransportSummary[]>('/api/transports', signal),
  updateTransportCredentials: (id: string, credentials: Record<string, string>) =>
    post<{ id: string; name: string; updated: boolean; verified_at: null }>(`/api/transports/${enc(id)}/credentials`, { credentials }),
  providers: (signal?: AbortSignal) => get<ProviderView[]>('/api/providers', signal),
  saveProvider: (input: ProviderInput) => post<ProviderView>('/api/providers', input),
  deleteProvider: (name: string) => del(`/api/providers/${enc(name)}`),
  testProvider: (name: string, signal?: AbortSignal) => post<ProviderTestResult>(`/api/providers/${enc(name)}/test`, undefined, signal),
  memories: (status: 'pending' | 'active', signal?: AbortSignal) => get<MemoryNote[]>('/api/memories', signal, { status }),
  approveMemory: (id: string | number) => empty('POST', idPath('memories', id, '/approve')),
  deleteMemory: (id: string | number) => del(idPath('memories', id)),
  proposals: (status?: string, signal?: AbortSignal) => get<ProposalView[]>('/api/proposals', signal, { status }),
  decideProposal: (id: string | number, decision: 'approve' | 'reject', typedConfirmation?: string, comment?: string) =>
    post<{ id: string; status: string }>(prop(id, 'decision'), {
      decision,
      ...(typedConfirmation !== undefined ? { typed_confirmation: typedConfirmation } : {}),
      ...(comment !== undefined ? { comment } : {}),
    }),
  dryRunProposal: (id: string | number) => post<{ id: string } & ProposalDryRun>(prop(id, 'dry-run')),
  executeProposal: (id: string | number) =>
    post<{ id: string; status: string; has_backup?: boolean; execution?: ProposalExecution; agent_job?: { id: string; status: string } }>(prop(id, 'execute')),
  rollbackProposal: (id: string | number) => post<{ id: string; rollback: ProposalRollback }>(prop(id, 'rollback')),
  conversations: (signal?: AbortSignal) => get<Conversation[]>('/api/conversations', signal),
  createConversation: (title?: string) => post<Conversation>('/api/conversations', { title }),
  messages: (id: string | number, signal?: AbortSignal) => get<ChatMessage[]>(idPath('conversations', id, '/messages'), signal),
  monitors: (signal?: AbortSignal) => get<MonitorView[]>('/api/monitors', signal),
  createMonitor: (input: { name: string; kind: string; target: string; interval_sec?: number; timeout_sec?: number; allow_private?: boolean }) =>
    post<MonitorView>('/api/monitors', input),
  deleteMonitor: (id: string) => del(`/api/monitors/${enc(id)}`),
  incidents: (signal?: AbortSignal) => get<IncidentView[]>('/api/incidents', signal),
  ackIncident: (id: string) => post<{ id: string; acknowledged: boolean }>(`/api/incidents/${enc(id)}/ack`),
  channels: (signal?: AbortSignal) => get<ChannelView[]>('/api/channels', signal),
  createChannel: (input: { kind: string; name: string; config?: Record<string, unknown>; token?: string }) =>
    post<ChannelView>('/api/channels', input),
  deleteChannel: (id: string) => del(`/api/channels/${enc(id)}`),
  telegramLinkCode: () => post<{ id: string; code: string; command: string }>('/api/channels/telegram/link-code'),
};

const num = (v: unknown) => (typeof v === 'number' ? v : undefined);
const str = (v: unknown) => (typeof v === 'string' ? v : undefined);

function parseTurnEvent(name: string | null, data: string): TurnEvent | null {
  let v: Record<string, unknown>;
  try {
    const p: unknown = JSON.parse(data);
    if (typeof p !== 'object' || p === null) return null;
    v = p as Record<string, unknown>;
  } catch {
    return null;
  }
  if (name === 'delta') return typeof v.text === 'string' ? { type: 'delta', text: v.text } : null;
  if (name === 'tool') return { type: 'tool', name: String(v.name ?? ''), tier: typeof v.tier === 'number' || typeof v.tier === 'string' ? v.tier : 0, status: String(v.status ?? ''), duration_ms: num(v.duration_ms) };
  if (name === 'done') return { type: 'done', rounds: num(v.rounds), tool_calls: num(v.tool_calls), usage: typeof v.usage === 'object' && v.usage !== null ? (v.usage as { estimated?: boolean; tokens?: number }) : undefined };
  if (name === 'error') {
    const code = str(v.code);
    return { type: 'error', code: code && CODE_PATTERN.test(code) ? code : 'internal', message: (str(v.message) ?? '').slice(0, 500) };
  }
  return null;
}

/**
 * Sends one assistant turn and reports its events as they arrive. Resolves when the stream ends.
 * Rejects with ApiError for a refusal before the stream (409 busy, 401, ...). An abort through the
 * signal rejects with the abort error of fetch/read; callers treat it as "stopped".
 */
export async function streamTurn(
  conversationId: string | number,
  message: string,
  onEvent: (event: TurnEvent) => void,
  signal?: AbortSignal,
): Promise<void> {
  const response = await send('POST', idPath('conversations', conversationId, '/turn'), {
    body: { message },
    signal,
    stream: true,
  });
  if (!response.body) throw new ApiError(response.status, 'bad_response', '');
  await readSse(response.body, (raw) => {
    const event = parseTurnEvent(raw.event, raw.data);
    if (event) onEvent(event);
  });
}
