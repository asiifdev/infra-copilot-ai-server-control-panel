// Reads the last connection check of a transport. The server keeps numbers and booleans only
// (crates/server/src/api/data.rs `numbers_only`), shaped like
// {ok, quota: {total, used, free} | null, quota_unsupported, speed_bytes_per_sec, speed_leftover}.
// Anything else degrades to "unknown result": nothing is guessed and no key is trusted.

import type { TransportCheck } from './types';

export interface CheckView {
  /** true passed, false failed, null when the result has no usable `ok`. */
  ok: boolean | null;
  freeBytes: number | null;
  totalBytes: number | null;
  speedBytesPerSec: number | null;
}

const count = (value: unknown): number | null =>
  typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null;

export function readCheck(check: unknown): CheckView | null {
  if (check === null || typeof check !== 'object' || Array.isArray(check)) return null;
  const value = check as TransportCheck;
  const quota = typeof value.quota === 'object' && value.quota !== null ? (value.quota as Record<string, unknown>) : {};
  return {
    ok: typeof value.ok === 'boolean' ? value.ok : null,
    freeBytes: count(quota.free),
    totalBytes: count(quota.total),
    speedBytesPerSec: count(value.speed_bytes_per_sec),
  };
}
