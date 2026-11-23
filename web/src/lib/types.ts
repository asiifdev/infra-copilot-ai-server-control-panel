// Response shapes of the local API (ADR 0029). Everything here is untrusted data: the UI renders
// it as text only. Shapes the ADR leaves open (memories, proposals, conversations, messages) are
// the smallest reading that fits; deviations are listed in ADR 0031.

export interface SessionInfo {
  user: { email: string; name: string };
  org: { name: string; slug: string };
  locale: string;
  master_key_configured: boolean;
  version: string;
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
  name: string;
  status: string;
}

export interface PlanStep {
  [key: string]: unknown;
}

export interface PlanDetail {
  name: string;
  status: string;
  spec: unknown;
  steps: PlanStep[];
  rendered: string;
  gates: Record<string, unknown>;
}

export interface TransportSummary {
  name: string;
  kind: string;
  verified_at: string | null;
  last_check: string | null;
}

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
  id: string | number;
  content: string;
  status: string;
  created_at?: string;
}

export interface ProposalView {
  id: string | number;
  action: string;
  reason: string;
  impact: string;
  status: string;
  tier?: number | string;
  created_at?: string;
}

export interface Conversation {
  id: string | number;
  title: string;
  created_at?: string;
}

export interface ChatMessage {
  id?: string | number;
  role: string;
  content: string;
  created_at?: string;
}

/** Events of POST /api/conversations/{id}/turn. */
export type TurnEvent =
  | { type: 'delta'; text: string }
  | { type: 'tool'; name: string; tier: number | string; status: string; duration_ms?: number }
  | { type: 'done'; rounds?: number; tool_calls?: number; usage?: unknown }
  | { type: 'error'; code: string; message: string };
