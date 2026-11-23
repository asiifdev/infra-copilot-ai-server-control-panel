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

  const plans: PlanDetail[] = [
    {
      name: 'move-alpha',
      status: 'draft',
      spec: { source: 'node-alpha', target: 'node-gamma' },
      steps: [
        { kind: 'backup', side: 'source' },
        { kind: 'restore', side: 'target', note: text('check ports', 6) },
      ],
      rendered: long('Plan move-alpha (draft)\n1. Back up the source.\n2. Restore on the target.', 0),
      gates: { open_critical: [{ kind: 'listener', key: 'tcp:8080' }], accepted_critical: [], important: 1, info: 1, inventory_incomplete: false },
    },
    { name: 'move-beta', status: 'draft', spec: {}, steps: [], rendered: 'Plan move-beta (draft)', gates: { important: 0 } },
  ];

  const transports: TransportSummary[] = [
    { name: 'drive-main', kind: 'rclone', verified_at: '2026-11-18T09:00:00Z', last_check: '2026-11-22T09:00:00Z' },
    { name: 'sftp-spare', kind: 'sftp', verified_at: null, last_check: null },
  ];

  const providers: ProviderView[] = [
    { name: 'primary', base_url: 'https://llm.example.test/v1', model: 'model-large', params: { temperature: 0.2 }, has_key: true, is_default: true },
  ];

  const memories: MemoryNote[] = [
    { id: 1, content: long('The production database is read-only on weekends.', 4), status: 'pending', created_at: '2026-11-22T07:00:00Z' },
    { id: 2, content: long('Backups are kept for 30 days.', 8), status: 'pending', created_at: '2026-11-22T07:05:00Z' },
    { id: 3, content: long('The staging server is called node-gamma.', 7), status: 'active', created_at: '2026-11-19T07:00:00Z' },
  ];

  const proposals: ProposalView[] = [
    { id: 1, action: text('Restart the web service', 4), reason: long('It stopped answering health checks.', 1), impact: long('About ten seconds of downtime.', 2), status: 'pending', tier: 2 },
    { id: 2, action: 'Rotate the log files', reason: 'Disk is 92 percent full.', impact: 'None expected.', status: 'rejected', tier: 'light_change' },
  ];

  const conversations: Conversation[] = [
    { id: 'c1', title: text('Why is port 8080 missing?', 5), created_at: '2026-11-22T08:00:00Z' },
    { id: 'c2', title: '', created_at: '2026-11-22T09:00:00Z' },
  ];
  const messages: Record<string, ChatMessage[]> = {
    c1: [
      { role: 'user', content: 'Why is port 8080 missing?' },
      { role: 'assistant', content: long('The web service is not running on the new server.', 4) },
    ],
    c2: [],
  };

  return { nodes, inventory, findings, plans, transports, providers, memories, proposals, conversations, messages };
}
