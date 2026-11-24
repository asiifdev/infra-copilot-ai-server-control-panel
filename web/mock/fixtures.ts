// Generic, fake fixtures for the mock API. No real domains, ids, names or keys: hosts end in
// ".test", names are placeholders. In hostile mode every free-text field carries an attack string
// so the tests can prove the UI renders it inert.

import type {
  ChatMessage,
  Conversation,
  Finding,
  InventoryItem,
  MemoryNote,
  NodeSummary,
  PlanDetail,
  PlanStep,
  ProposalView,
  ProviderView,
  TransportSummary,
} from '../src/lib/types';

/** Strings a hostile server, tool or model might send. Every one must render as inert text. */
export const HOSTILE: string[] = [
  '<script>window.__pwned = true</script>',
  '<img src=x onerror="window.__pwned = true">',
  '"><svg onload=window.__pwned=1>',
  'javascript:window.__pwned=1',
  '[click](javascript:window.__pwned=1) **bold** `code` <b>tag</b>',
  '<a href="javascript:window.__pwned=1">link</a>',
  '\u202Egnp.exe\u202C and \u200Bzero\u200Bwidth \u2066isolate\u2069',
  'line one\nline two\r\nline three',
  `${'A'.repeat(300_000)}`,
];

/** A fake UUID: the server uses 36-character ids. */
export function id(n: number): string {
  return `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
}

export function hostile(index: number): string {
  return HOSTILE[index % HOSTILE.length] ?? '';
}

/** A hostile string that is short enough for table cells and titles. */
export function hostileShort(index: number): string {
  return hostile(index % (HOSTILE.length - 1));
}

export interface Fixtures {
  nodes: NodeSummary[];
  inventory: Record<string, { taken_at: string; status: string; warnings: string[]; items: InventoryItem[] }>;
  findings: Finding[];
  plans: PlanDetail[];
  transports: TransportSummary[];
  providers: ProviderView[];
  memories: MemoryNote[];
  proposals: ProposalView[];
  conversations: Conversation[];
  messages: Record<string, ChatMessage[]>;
}

export function buildFixtures(options: { hostile?: boolean } = {}): Fixtures {
  const h = options.hostile === true;
  const text = (plain: string, i: number) => (h ? hostileShort(i) : plain);
  const long = (plain: string, i: number) => (h ? hostile(i) : plain);

  const nodes: NodeSummary[] = [
    {
      name: text('node-alpha', 0),
      kind: 'ssh',
      status: 'online',
      latest_inventory: {
        taken_at: '2026-11-20T08:30:00Z',
        status: 'complete',
        counts: { listener: 4, process: 6, cron: 2, env_ref: 5 },
      },
    },
    { name: 'node-beta', kind: 'ssh', status: 'offline', latest_inventory: null },
    {
      name: 'node-gamma',
      kind: 'local',
      status: 'pending',
      latest_inventory: {
        taken_at: '2026-11-21T10:00:00Z',
        status: 'partial',
        counts: { listener: 1, process: 2, cron: 0, env_ref: 1 },
      },
    },
  ];

  const item = (kind: string, key: string, data: unknown): InventoryItem => ({ kind, key, data });
  const items: InventoryItem[] = [
    item('listener', 'tcp:8080', { proto: 'tcp', port: 8080, bind: '0.0.0.0', process: text('web', 1), exposed: 'public' }),
    item('listener', 'tcp:5432', { proto: 'tcp', port: 5432, bind: '127.0.0.1', process: 'db', exposed: 'loopback' }),
    item('process', 'pm2:web-app', { cmd: 'node server.js', cwd: '/srv/app', mode: 'prod', supervised: true }),
    item('cron', 'cron:backup', { schedule: '0 3 * * *', command: text('backup.sh', 2) }),
    item('env_ref', 'env:web-app:DATABASE_URL', { app: 'web-app', name: 'DATABASE_URL' }),
  ];
  const inventory = {
    [nodes[0]?.name ?? 'node-alpha']: { taken_at: '2026-11-20T08:30:00Z', status: 'complete', warnings: [text('Cron could not be read for one user.', 3)], items },
    'node-gamma': { taken_at: '2026-11-21T10:00:00Z', status: 'partial', warnings: [], items: items.slice(0, 2) },
  };

  const findings: Finding[] = [
    {
      rule_id: 'parity.port.missing',
      severity: 'critical',
      subject: { kind: 'listener', key: 'tcp:8080' },
      title: text('Port 8080 is not served on the new server', 4),
      explanation: long('The old server listens on port 8080 but the new one does not.', 7),
      advice: long('Start the service on the new server, then compare again.', 1),
      note: h ? hostile(2) : null,
      evidence: [{ kind: 'listener', key: 'tcp:8080' }],
    },
    {
      rule_id: 'parity.cron.missing',
      severity: 'important',
      subject: { kind: 'cron', key: 'cron:backup' },
      title: text('A scheduled job is missing', 5),
      explanation: long('The backup job exists only on the old server.', 3),
      advice: long('Recreate the job on the new server.', 4),
      note: null,
      evidence: [{ kind: 'cron', key: 'cron:backup' }],
    },
    {
      rule_id: 'parity.env.extra',
      severity: 'info',
      subject: { kind: 'env_ref', key: 'env:web-app:DEBUG' },
      title: 'An extra environment reference exists on the new server',
      explanation: long('The new server has one reference the old one does not.', 5),
      advice: long('No action is needed if this is intended.', 6),
      note: null,
      evidence: [],
    },
  ];

  const step = (seq: number, kind: string, depends_on: number[], extra: Partial<PlanStep> = {}): PlanStep => ({
    seq,
    kind,
    depends_on,
    ...extra,
  });
  const steps: PlanStep[] = [
    step(0, 'inventory', [], { side: 'source' }),
    step(1, 'inventory', [], { side: 'target' }),
    step(2, 'backup', [0, 1]),
    step(3, 'transfer', [2]),
    step(4, 'restore', [3], { database: text('shop_db', 6) }),
    step(5, 'verify', [4], { database: 'shop_db' }),
    step(6, 'delta', [5]),
    step(7, 'cutover', [6]),
    step(8, 'post_verify', [7]),
    step(9, 'decommission_source', [8]),
  ];
  const plans: PlanDetail[] = [
    {
      name: 'move-alpha',
      status: 'draft',
      source_node: 'node-alpha',
      target_node: 'node-gamma',
      created_at: '2026-11-22T10:00:00Z',
      spec: { source: { node_id: id(1) }, target: { node_id: id(2) } },
      steps,
      rendered: long('# Migration plan: move-alpha\n\n## Steps, in the order they run\n1. Collect an inventory\n', 0),
      // The keys the server sends today (crates/server/src/api/data.rs), in its order.
      gates: {
        cutover_allowed: false,
        shutdown_allowed: false,
        inventory_incomplete: true,
        comparison_current: true,
        open_critical: [{ rule_id: 'PAR-001', subject: { kind: 'listener', key: text('tcp:8080', 4) } }],
        accepted_critical: [
          { rule_id: 'PAR-004', subject: { kind: 'cron', key: 'cron:backup' }, reason: long('Replaced by a managed job.', 3) },
        ],
        important: 1,
        info: 17,
      },
    },
    {
      name: 'move-beta',
      status: 'draft',
      source_node: 'node-beta',
      target_node: 'node-gamma',
      created_at: '2026-11-22T11:00:00Z',
      spec: {},
      steps: [],
      rendered: '# Migration plan: move-beta',
      gates: {
        cutover_allowed: true,
        shutdown_allowed: false,
        inventory_incomplete: false,
        comparison_current: false,
        open_critical: [],
        accepted_critical: [],
        important: 0,
        info: 0,
      },
    },
  ];

  const transports: TransportSummary[] = [
    { name: 'drive-main', kind: 'rclone', verified_at: '2026-11-18T09:00:00Z', last_check: { ok: true } },
    { name: 'sftp-spare', kind: 'sftp', verified_at: null, last_check: { ok: false } },
    { name: 'rsync-new', kind: 'rsync', verified_at: null, last_check: null },
  ];

  const providers: ProviderView[] = [
    { name: 'primary', base_url: 'https://llm.example.test/v1', model: 'model-large', params: { temperature: 0.2 }, has_key: true, is_default: true },
  ];

  const memory = (n: number, kind: string, source: string, status: string, content: string): MemoryNote => ({
    id: id(100 + n),
    content,
    kind,
    source,
    status,
    conversation_id: null,
    created_at: '2026-11-22T07:00:00.000Z',
    updated_at: '2026-11-22T07:00:00.000Z',
  });
  const memories: MemoryNote[] = [
    memory(1, 'task', 'assistant', 'pending', long('The production database is read-only on weekends.', 4)),
    memory(2, 'org_fact', 'assistant', 'pending', long('Backups are kept for 30 days.', 8)),
    memory(3, 'org_fact', 'person', 'active', long('The staging server is called node-gamma.', 7)),
    memory(4, 'preference', 'person', 'active', 'Prefers short answers.'),
  ];

  const proposal = (
    n: number,
    tool: string,
    tier: number,
    status: string,
    title: string,
    rationale: string,
    action: string,
    impact: string,
    run: Pick<ProposalView, 'action' | 'node' | 'execution'> = { action: null, node: null, execution: null },
  ): ProposalView => ({
    id: id(200 + n),
    tool,
    tier,
    title,
    rationale,
    suggested_action: action,
    impact,
    rule_id: 'PAR-001',
    subject_key: 'tcp:8080',
    status,
    conversation_id: id(300),
    created_at: '2026-11-22T07:00:00.000Z',
    // Pending proposals expire two days from now (the clock of the test run), the others are history.
    expires_at: new Date(Date.now() + (status === 'pending' || status === 'approved' ? 2 * 86_400_000 : -86_400_000)).toISOString(),
    ...run,
  });
  const restart = (service: string, execution: ProposalView['execution'] = null): Pick<ProposalView, 'action' | 'node' | 'execution'> => ({
    action: { kind: 'restart_service', manager: 'systemd', service },
    node: 'this-mac',
    execution,
  });
  const proposals: ProposalView[] = [
    proposal(1, 'propose_restart', 2, 'pending', text('Restart the web service', 4), long('It stopped answering health checks.', 1), long('Restart the service.', 5), long('About ten seconds of downtime.', 2)),
    proposal(2, 'propose_rotate_logs', 1, 'rejected', 'Rotate the log files', 'Disk is 92 percent full.', 'Rotate the logs.', 'None expected.'),
    proposal(3, 'propose_move_traffic', 3, 'pending', 'Move traffic to the new server', 'The comparison found no blocking differences.', 'Switch the tunnel to the new server.', 'All visitors reach the new server.'),
    proposal(4, 'propose_clear_cache', 1, 'expired', 'Clear the page cache', 'The cache is stale.', 'Clear it.', 'Pages load slower for a minute.'),
    // Approved, recommendation only: nothing to run.
    proposal(5, 'propose_add_swap', 2, 'approved', 'Add a swap file', 'Memory use is high.', 'Create a 2 GB swap file.', 'Uses 2 GB of disk.'),
    // Approved with an action: waits for a person to run it.
    proposal(6, 'propose_restart_service', 2, 'approved', 'Restart nginx', 'Health checks keep failing.', 'Restart the nginx service.', 'The web service is briefly unavailable. A restart cannot be undone.', restart(text('nginx.service', 3))),
    proposal(7, 'propose_restart_service', 2, 'executed', 'Restart the queue worker', 'The queue is stuck.', 'Restart the worker.', 'The worker is briefly unavailable. A restart cannot be undone.', restart('queue-worker.service', {
      status: 'succeeded',
      verified: true,
      reason: 'ok',
      output: long('active', 7),
      started_at: '2026-11-23T09:00:00.000Z',
      finished_at: '2026-11-23T09:00:02.000Z',
    })),
    proposal(8, 'propose_restart_service', 2, 'failed', 'Restart the mail relay', 'Mail is not being delivered.', 'Restart the relay.', 'Mail is briefly delayed. A restart cannot be undone.', restart('mail-relay.service', {
      status: 'failed',
      verified: false,
      reason: 'command_failed',
      output: h ? HOSTILE.slice(0, 8).join('\n') : 'Failed to restart mail-relay.service: Unit mail-relay.service not found.',
      started_at: '2026-11-23T10:00:00.000Z',
      finished_at: '2026-11-23T10:00:01.000Z',
    })),
  ];

  const conversations: Conversation[] = [
    { id: id(300), title: text('Why is port 8080 missing?', 5), archived: false, created_at: '2026-11-22T08:00:00.000Z' },
    { id: id(301), title: 'New conversation', archived: false, created_at: '2026-11-22T09:00:00.000Z' },
  ];
  const messages: Record<string, ChatMessage[]> = {
    [id(300)]: [
      { seq: 1, role: 'user', content: 'Why is port 8080 missing?', created_at: '2026-11-22T08:00:01.000Z', tools: [] },
      { seq: 2, role: 'assistant', content: '', created_at: '2026-11-22T08:00:02.000Z', tools: ['search_inventory'] },
      { seq: 4, role: 'assistant', content: long('The web service is not running on the new server.', 4), created_at: '2026-11-22T08:00:03.000Z', tools: [] },
    ],
    [id(301)]: [],
  };

  return { nodes, inventory, findings, plans, transports, providers, memories, proposals, conversations, messages };
}
