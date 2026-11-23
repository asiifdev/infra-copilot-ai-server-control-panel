<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { inlineText } from '../lib/text';

  type Tone = 'success' | 'warning' | 'danger' | 'info' | 'neutral';
  type Kind = 'node' | 'severity' | 'tier' | 'state';

  interface Props {
    kind: Kind;
    /** Raw value from the API (untrusted); unknown values are shown as plain text. */
    value: string | number;
  }

  let { kind, value }: Props = $props();

  // Literal keys on purpose: the key-coverage test finds them.
  const NODE: Record<string, [string, Tone]> = {
    pending: ['ui.status.pending', 'warning'],
    online: ['ui.status.online', 'success'],
    offline: ['ui.status.offline', 'danger'],
    revoked: ['ui.status.revoked', 'neutral'],
  };
  const SEVERITY: Record<string, [string, Tone]> = {
    info: ['ui.severity.info', 'info'],
    important: ['ui.severity.important', 'warning'],
    critical: ['ui.severity.critical', 'danger'],
  };
  const TIER: Record<string, [string, Tone]> = {
    '0': ['ui.tier.read', 'success'],
    '1': ['ui.tier.plan', 'info'],
    '2': ['ui.tier.light', 'warning'],
    '3': ['ui.tier.risky', 'danger'],
    '4': ['ui.tier.destructive', 'danger'],
    read: ['ui.tier.read', 'success'],
    plan: ['ui.tier.plan', 'info'],
    light_change: ['ui.tier.light', 'warning'],
    risky_change: ['ui.tier.risky', 'danger'],
    destructive: ['ui.tier.destructive', 'danger'],
  };
  const STATE: Record<string, [string, Tone]> = {
    draft: ['ui.state.draft', 'info'],
    pending: ['ui.state.pending', 'warning'],
    active: ['ui.state.active', 'success'],
    approved: ['ui.state.approved', 'success'],
    rejected: ['ui.state.rejected', 'neutral'],
    passed: ['ui.transports.check_ok', 'success'],
    failed: ['ui.transports.check_failed', 'danger'],
    executed: ['ui.state.executed', 'success'],
    expired: ['ui.state.expired', 'neutral'],
    ok: ['ui.state.ok', 'success'],
    running: ['ui.state.running', 'info'],
    done: ['ui.state.done', 'success'],
    error: ['ui.state.error', 'danger'],
    complete: ['ui.state.complete', 'success'],
    partial: ['ui.state.partial', 'warning'],
    ready: ['ui.state.ready', 'success'],
  };
  const TABLES: Record<Kind, Record<string, [string, Tone]>> = {
    node: NODE,
    severity: SEVERITY,
    tier: TIER,
    state: STATE,
  };
  const ICONS: Record<Tone, string> = {
    success: '✓',
    warning: '▲',
    danger: '✕',
    info: 'ℹ',
    neutral: '○',
  };

  const entry = $derived(TABLES[kind][String(value).toLowerCase()]);
  const tone = $derived<Tone>(entry ? entry[1] : 'neutral');
  const label = $derived(entry ? t(entry[0]) : inlineText(value, 40));
</script>

<span class={['badge', tone]}>
  <span class="icon" aria-hidden="true">{ICONS[tone]}</span>
  <span>{label}</span>
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--nv-space-1);
    padding: 0 var(--nv-space-2);
    border: var(--nv-border-width) solid currentColor;
    border-radius: var(--nv-radius-sm);
    font-size: var(--nv-text-sm);
    font-weight: 600;
    background: var(--nv-surface);
    white-space: nowrap;
  }
  .success {
    color: var(--nv-success);
  }
  .warning {
    color: var(--nv-warning);
  }
  .danger {
    color: var(--nv-danger);
  }
  .info {
    color: var(--nv-info);
  }
  .neutral {
    color: var(--nv-text-muted);
  }
</style>
