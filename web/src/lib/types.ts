// Response shapes of the local API (ADR 0029). Everything here is untrusted data: the UI renders
// it as text only. Shapes the ADR leaves open (memories, proposals, conversations, messages) are
// the smallest reading that fits; deviations are listed in ADR 0031.

export interface SessionInfo {
  user: { email: string; name: string };
  org: { name: string; slug: string };
  locale: string;
  master_key_configured: boolean;
  version: string;
  /** True only when the owner started the server with `--enable-execution`. */
  execution_enabled: boolean;
}

export interface InventoryCounts {
  listener: number;
  process: number;
  cron: number;
  env_ref: number;
}

export interface NodeSummary {
  name: string;
  kind: string;
  status: string;
  access?: string;
  latest_inventory: { taken_at: string; status: string; counts: InventoryCounts } | null;
}

export interface InventoryItem {
  kind: string;
  key: string;
  data: unknown;
}

export interface InventoryView {
  taken_at: string;
  status: string;
  access?: string;
  warnings: string[];
  items: InventoryItem[];
}

export type Severity = 'info' | 'important' | 'critical';

export interface Finding {
  rule_id: string;
  severity: string;
  subject: { kind: string; key: string };
  title: string;
  explanation: string;
  advice: string;
  note: string | null;
  evidence: { kind: string; key: string }[];
}

export interface FindingsView {
  comparison: { source: string; target: string; created_at: string } | null;
  findings: Finding[];
}

export interface PlanSummary {
  id: string;
  name: string;
  status: string;
  source_node: string;
  target_node: string;
  created_at: string;
}

export type PlanStepStatus =
  | 'pending'
  | 'proposed'
  | 'approved'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'skipped'
  | 'rolled_back';

/** One step of a plan. `seq` counts from 0; `depends_on` lists the `seq` of earlier steps. */
export interface PlanStep {
  seq: number;
  kind: string;
  depends_on: number[];
  status: PlanStepStatus;
  /** Inventory steps only: "source" or "target". */
  side?: string;
  /** Restore and verify steps only: the database key from the plan. */
  database?: string;
}

/** A finding a gate mentions (untrusted text). `reason` is set on accepted critical findings. */
export interface GateFinding {
  rule_id: string;
  subject: { kind: string; key: string };
  reason?: string;
}

export interface PlanDetail extends PlanSummary {
  spec: unknown;
  steps: PlanStep[];
  rendered: string;
  gates: Record<string, unknown>;
}

/** Last connection check of a transport: numbers and booleans only (the server drops strings). */
export type TransportCheck = { ok?: boolean; [key: string]: unknown };

export interface TransportSummary {
  id: string;
  name: string;
  kind: string;
  verified_at: string | null;
  last_check: TransportCheck | null;
}

export type TransportView = TransportSummary;

export interface ProviderView {
  name: string;
  base_url: string;
  model: string;
  params: unknown;
  has_key: boolean;
  is_default: boolean;
}

export interface ProviderInput {
  name: string;
  base_url: string;
  model: string;
  params?: Record<string, unknown>;
  key?: string;
  is_default?: boolean;
}

export type ProviderTestResult =
  | { ok: true; latency_ms: number; model: string }
  | { ok: false; code: string; message: string };

export interface MemoryNote {
  id: string;
  content: string;
  /** org_fact | task | preference */
  kind: string;
  /** person | assistant */
  source: string;
  /** pending | active */
  status: string;
  conversation_id: string | null;
  created_at: string;
  updated_at: string;
}

/** The closed action list: the only thing a proposal can run (ADR 0037). */
export type ProposalAction =
  | {
      kind: 'restart_service';
      manager: 'systemd';
      service: string;
      target_label: string;
    }
  | {
      kind: 'update_env_var';
      path: string;
      key: string;
      value: '[REDACTED]';
      target_label: string;
    }
  | {
      kind: 'delete_managed_file';
      path: string;
      target_label: string;
    };

export interface ProposalDryRun {
  steps: string[];
  current_state: string;
  diff: string | null;
  dry_run_at: string;
}

export interface ProposalApproval {
  decision: 'approved' | 'rejected';
  typed_confirmation: boolean;
  created_at: string;
}

export interface ProposalRollback {
  status: string;
  verified: boolean;
  reason: string;
  output: string;
  started_at: string;
  finished_at: string | null;
}

/** What happened when an approved proposal was run. `output` is untrusted text from the target. */
export interface ProposalExecution {
  status: 'running' | 'queued' | 'succeeded' | 'failed';
  verified: boolean;
  /** ok | not_installed | timeout | command_failed | not_active_after | io | action_invalid | interrupted */
  reason: string | null;
  output: string;
  started_at: string;
  finished_at: string | null;
}

export interface ProposalView {
  id: string;
  tool: string;
  /** 1 to 4 */
  tier: number;
  title: string;
  rationale: string;
  suggested_action: string;
  impact: string;
  rule_id: string | null;
  subject_key: string | null;
  /** pending | approved | rejected | executed | failed | expired */
  status: string;
  conversation_id: string | null;
  created_at: string;
  expires_at: string;
  /** What running would do; null for a recommendation only. */
  action: ProposalAction | null;
  dry_run: ProposalDryRun | null;
  approval: ProposalApproval | null;
  has_backup: boolean;
  /** Name of the target node of the action. */
  node: string | null;
  execution: ProposalExecution | null;
  rollback: ProposalRollback | null;
  approvals_count?: number;
  required_approvals?: number;
  user_has_approved?: boolean;
}

export interface Conversation {
  id: string;
  title: string;
  archived: boolean;
  created_at: string;
}

export interface ChatMessage {
  seq: number;
  role: string;
  content: string;
  created_at: string;
  /** Names of the tools the assistant called in this message. */
  tools: string[];
}

/** Events of POST /api/conversations/{id}/turn. */
export type TurnEvent =
  | { type: 'delta'; text: string }
  | { type: 'tool'; name: string; tier: number | string; status: string; duration_ms?: number }
  | { type: 'done'; rounds?: number; tool_calls?: number; usage?: { estimated?: boolean; tokens?: number } }
  | { type: 'error'; code: string; message: string };

export interface MonitorView {
  id: string;
  name: string;
  kind: 'http' | 'tcp' | 'tls';
  target: string;
  current_status: string;
  last_checked_at: string | null;
}

export interface IncidentView {
  id: string;
  monitor_id: string;
  started_at: string;
  resolved_at: string | null;
  cause: string;
  severity: string;
  acknowledged_at: string | null;
}

export interface ChannelView {
  id: string;
  kind: 'telegram' | 'webhook';
  name: string;
  has_secret: boolean;
  is_active: boolean;
}

export interface NodeLogsView {
  node_id: string;
  service: string;
  line_count: number;
  truncated: boolean;
  content: string;
}

export interface AgentTokenResult {
  node_id: string;
  token_id: string;
  token: string;
}

export interface LocalSite {
  id: string;
  domain: string;
  upstream_port: number;
  config_path: string;
  status: string;
  created_at: string;
  php_version?: string | null;
  tls_enabled?: boolean;
}
