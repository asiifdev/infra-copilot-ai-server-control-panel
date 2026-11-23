// Reading a plan detail (GET /api/plans/{name}) for display: step kinds, sides and gates get
// localized labels; anything the UI does not know yet degrades to a readable fallback made of
// the raw name as plain text. Everything here is untrusted data; the caller renders it as text.
// The shapes come from crates/server/src/api/data.rs and crates/plan/src/steps.rs, and
// tests/contract.test.ts fails when the server sends a gate key that has no label here.

import { describeData } from './format';
import type { GateFinding, PlanStep } from './types';

// Literal keys on purpose: the key-coverage test finds them.
export const STEP_LABELS: Record<string, string> = {
  inventory: 'ui.plan.step_inventory',
  backup: 'ui.plan.step_backup',
  transfer: 'ui.plan.step_transfer',
  restore: 'ui.plan.step_restore',
  verify: 'ui.plan.step_verify',
  delta: 'ui.plan.step_delta',
  cutover: 'ui.plan.step_cutover',
  post_verify: 'ui.plan.step_post_verify',
  decommission_source: 'ui.plan.step_decommission_source',
};

export const SIDE_LABELS: Record<string, string> = {
  source: 'ui.plan.side_source',
  target: 'ui.plan.side_target',
};

export const GATE_LABELS: Record<string, string> = {
  cutover_allowed: 'ui.plan.gate_cutover_allowed',
  shutdown_allowed: 'ui.plan.gate_shutdown_allowed',
  inventory_incomplete: 'ui.plan.gate_inventory_incomplete',
  comparison_current: 'ui.plan.gate_comparison_current',
  open_critical: 'ui.plan.gate_open_critical',
  accepted_critical: 'ui.plan.gate_accepted_critical',
  important: 'ui.plan.gate_important',
  info: 'ui.plan.gate_info',
};

export interface StepRow {
  /** Number shown to people: the place in the list, from 1 (the same numbers as the plan text). */
  number: number;
  seq: number;
  kind: string;
  side: string | null;
  database: string | null;
  /** Numbers (as shown) of the steps this one waits for. */
  after: number[];
}

/** Steps in the order the server sent them, with dependencies translated to shown numbers. */
export function stepRows(steps: unknown): StepRow[] {
  if (!Array.isArray(steps)) return [];
  const raw = steps.filter((s): s is PlanStep => typeof s === 'object' && s !== null);
  const numberOf = new Map<number, number>();
  raw.forEach((step, index) => {
    if (typeof step.seq === 'number') numberOf.set(step.seq, index + 1);
  });
  return raw.map((step, index) => ({
    number: index + 1,
    seq: typeof step.seq === 'number' ? step.seq : index,
    kind: typeof step.kind === 'string' ? step.kind : '',
    side: typeof step.side === 'string' ? step.side : null,
    database: typeof step.database === 'string' ? step.database : null,
    after: (Array.isArray(step.depends_on) ? step.depends_on : [])
      .filter((d): d is number => typeof d === 'number')
      .map((d) => numberOf.get(d) ?? d + 1),
  }));
}

export type GateView =
  | { type: 'bool'; value: boolean }
  | { type: 'count'; value: number }
  | { type: 'findings'; items: GateFinding[] }
  | { type: 'text'; value: string };

function isFinding(value: unknown): value is GateFinding {
  if (typeof value !== 'object' || value === null) return false;
  const item = value as Partial<GateFinding>;
  return typeof item.rule_id === 'string' && typeof item.subject === 'object' && item.subject !== null;
}

/** What a gate value is: yes/no, a number, a list of findings, or something to show as text. */
export function gateView(value: unknown): GateView {
  if (typeof value === 'boolean') return { type: 'bool', value };
  if (typeof value === 'number' && Number.isFinite(value)) return { type: 'count', value };
  if (Array.isArray(value) && value.every(isFinding)) return { type: 'findings', items: value };
  if (Array.isArray(value)) return { type: 'count', value: value.length };
  if (typeof value === 'string') return { type: 'text', value };
  return { type: 'text', value: describeData(value, 120) };
}
