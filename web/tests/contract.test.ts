// Drift guards against the Rust server's source: the UI must have a label for every error code and
// every plan gate the server can send today. When the server grows one, these fail until the UI
// (and the catalog) learn it. They read the server source on purpose: it is the contract.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { errorKey, KNOWN_ERROR_CODES } from '../src/lib/errors';
import { GATE_LABELS, SIDE_LABELS, STEP_LABELS } from '../src/lib/plan';

const root = join(import.meta.dirname, '..', '..');
const read = (path: string) => readFileSync(join(root, path), 'utf8');

describe('error codes', () => {
  const table = read('crates/server/src/errors.rs');
  const codes = [...table.matchAll(/entry!\(\s*"([a-z_]+)"/g)].map((m) => m[1]!);

  it('finds the server table', () => {
    expect(codes.length).toBeGreaterThan(40);
    expect(codes).toContain('csrf_header');
    expect(codes).toContain('already_exists');
  });

  it('has a specific message in the UI for every code the server can send', () => {
    const unmapped = codes.filter((code) => errorKey(code) === 'ui.error.unknown');
    expect(unmapped).toEqual([]);
  });

  it('does not keep a code the server no longer has, except the ones the client raises itself', () => {
    const clientOnly = new Set(['unauthorized', 'forbidden', 'payload_too_large', 'network', 'bad_response', 'provider_unreachable', 'provider_auth', 'provider_error']);
    const stale = KNOWN_ERROR_CODES.filter((code) => !codes.includes(code) && !clientOnly.has(code));
    expect(stale).toEqual([]);
  });
});

describe('plan gates, step kinds and sides', () => {
  const data = read('crates/server/src/api/data.rs');
  const gateKeys = [...data.matchAll(/gate_map\s*\.insert\(\s*"([a-z_]+)"/g)].map((m) => m[1]!);

  it('reads the gate keys of the server', () => {
    expect(gateKeys.sort()).toEqual(
      ['accepted_critical', 'comparison_current', 'cutover_allowed', 'important', 'info', 'inventory_incomplete', 'open_critical', 'shutdown_allowed'].sort(),
    );
  });

  it('has a ui label for every gate key the server sends today', () => {
    expect(gateKeys.filter((key) => !GATE_LABELS[key])).toEqual([]);
  });

  it('has a ui label for every step kind and side of the plan crate', () => {
    const steps = read('crates/plan/src/steps.rs');
    const kinds = [...steps.matchAll(/Self::\w+ => "([a-z_]+)",/g)].map((m) => m[1]!);
    expect(kinds.length).toBe(9);
    expect(kinds.filter((kind) => !STEP_LABELS[kind])).toEqual([]);
    expect(Object.keys(SIDE_LABELS).sort()).toEqual(['source', 'target']);
    expect(steps).toContain('Source,');
    expect(steps).toContain('Target,');
  });
});

describe('favicon', () => {
  it('is linked from index.html and is a plain, script-free SVG (the CSP allows images from self only)', () => {
    const page = read('web/index.html');
    expect(page).toContain('<link rel="icon" href="/favicon.svg" type="image/svg+xml" />');
    const icon = read('web/public/favicon.svg');
    expect(icon.startsWith('<svg')).toBe(true);
    expect(icon).not.toMatch(/<script|onload|onerror|href=|<style|<image|javascript:/i);
    expect(icon.length).toBeLessThan(1024);
  });
});
