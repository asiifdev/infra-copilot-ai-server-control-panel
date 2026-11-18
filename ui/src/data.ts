import type { MessageKey } from './i18n';

export type Status = 'online' | 'degraded' | 'offline';
export type RiskTier = 1 | 2 | 3 | 4;

export interface NodeRow {
  id: number;
  name: string;
  status: Status;
  risk: RiskTier;
  lastSeenMinutes: number;
}

const STATUSES: readonly Status[] = ['online', 'degraded', 'offline'];
const RISKS: readonly RiskTier[] = [1, 2, 3, 4];

/** Deterministic dummy rows so every measurement run renders the same table. */
export function makeRows(count: number): NodeRow[] {
  const rows: NodeRow[] = [];
  for (let i = 0; i < count; i++) {
    rows.push({
      id: i,
      name: `node-${String(i + 1).padStart(4, '0')}`,
      status: STATUSES[(i * 7) % STATUSES.length] ?? 'online',
      risk: RISKS[(i * 5) % RISKS.length] ?? 1,
      lastSeenMinutes: (i * 13) % 600,
    });
  }
  return rows;
}

export const statusKey = (s: Status): MessageKey => `status.${s}`;
export const riskKey = (r: RiskTier): MessageKey => `risk.${r}`;
