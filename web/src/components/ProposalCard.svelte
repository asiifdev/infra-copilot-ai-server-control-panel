<script lang="ts">
  import { formatDateTime } from '../lib/format';
  import { i18n, t } from '../lib/i18n.svelte';
  import { relativeTime } from '../lib/time';
  import type { ProposalView } from '../lib/types';
  import { inlineText } from '../lib/text';
  import Button from './Button.svelte';
  import Card from './Card.svelte';
  import Icon from './Icon.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import TextBlock from './TextBlock.svelte';

  interface Props {
    proposal: ProposalView;
    /** Heading level of the card title: 2 on a page of its own list, 3 when nested under another heading. */
    level?: 2 | 3;
    /** A decision for this proposal is being sent. */
    busy?: boolean;
    /** When both are given and the proposal is pending, the card offers the decision. */
    onapprove?: () => void;
    onreject?: () => void;
    /** Whether the server allows running (`infra serve --enable-execution`). */
    executionEnabled?: boolean;
    /** Offered on an approved proposal that has an action. */
    onrun?: () => void;
  }

  let { proposal, level = 3, busy = false, onapprove, onreject, executionEnabled = false, onrun }: Props = $props();

  const pending = $derived(proposal.status === 'pending');
  const decidable = $derived(pending && onapprove !== undefined && onreject !== undefined);
  const expiry = $derived(relativeTime(proposal.expires_at, i18n.locale));
  const tone = $derived(proposal.tier >= 3 ? 'danger' : proposal.tier === 2 ? 'warning' : undefined);
  const titleText = $derived(inlineText(proposal.title, 120));
  const approved = $derived(proposal.status === 'approved');
  const runnable = $derived(approved && proposal.action !== null && onrun !== undefined);
  const execution = $derived(proposal.execution);
  const hintId = $derived(`run-off-${proposal.id}`);
  // A command the owner types, not text to translate.
  const ENABLE_COMMAND = 'infra serve --enable-execution';

  function actionText(action: NonNullable<ProposalView['action']>): string {
    if (action.kind !== 'restart_service') return t('ui.proposal.action_other', { kind: inlineText((action as { kind: unknown }).kind, 40) });
    return t('ui.proposal.restart_service', {
      service: inlineText(action.service, 100),
      manager: inlineText(action.manager, 20),
      node: inlineText(proposal.node, 80) || t('ui.proposal.node_unknown'),
    });
  }

  // One literal key per reason code of the server; an unknown code gets the generic sentence.
  function reasonText(reason: string): string {
    switch (reason) {
      case 'ok':
        return t('ui.execution.reason_ok');
      case 'not_installed':
        return t('ui.execution.reason_not_installed');
      case 'timeout':
        return t('ui.execution.reason_timeout');
      case 'command_failed':
        return t('ui.execution.reason_command_failed');
      case 'not_active_after':
        return t('ui.execution.reason_not_active_after');
      case 'io':
        return t('ui.execution.reason_io');
      case 'action_invalid':
        return t('ui.execution.reason_action_invalid');
      case 'interrupted':
        return t('ui.execution.reason_interrupted');
      default:
        return t('ui.execution.reason_other');
    }
  }
</script>

<Card title={titleText} {level} {tone}>
  {#snippet actions()}
    <StatusBadge kind="state" value={proposal.status} />
    <StatusBadge kind="tier" value={proposal.tier} />
  {/snippet}
  <ul class="meta plain muted">
    <li>{t('ui.proposal.tool', { tool: inlineText(proposal.tool, 64) })}</li>
    {#if proposal.rule_id}<li>{t('ui.proposals.rule', { rule: inlineText(proposal.rule_id, 40) })}</li>{/if}
    {#if proposal.subject_key}<li>{t('ui.proposals.subject', { subject: inlineText(proposal.subject_key, 80) })}</li>{/if}
    <li class="when">
      <Icon name="clock" size="sm" />
      {#if pending && expiry}
        {t(expiry.past ? 'ui.proposals.expired_ago' : 'ui.proposals.expires', { when: expiry.text })}
      {:else}
        {t('ui.proposals.created', { when: formatDateTime(proposal.created_at, i18n.locale) ?? inlineText(proposal.created_at, 40) })}
      {/if}
    </li>
  </ul>
  <dl class="facts">
    <dt>{t('ui.proposal.reason')}</dt>
    <dd><TextBlock text={proposal.rationale} /></dd>
    <dt>{t('ui.proposal.action')}</dt>
    <dd><TextBlock text={proposal.suggested_action} /></dd>
    <dt>{t('ui.proposal.impact')}</dt>
    <dd><TextBlock text={proposal.impact} /></dd>
    {#if proposal.action}
      <dt>{t('ui.proposal.prepared')}</dt>
      <dd class="prepared">{actionText(proposal.action)}</dd>
    {/if}
    {#if execution}
      <dt>{t('ui.proposal.result')}</dt>
      <dd class="result">
        <p class="status">
          <StatusBadge kind="state" value={execution.status} />
          <span class="muted">
            {#if execution.finished_at}
              {t('ui.execution.finished', { when: formatDateTime(execution.finished_at, i18n.locale) ?? inlineText(execution.finished_at, 40) })}
            {:else}
              {t('ui.execution.started', { when: formatDateTime(execution.started_at, i18n.locale) ?? inlineText(execution.started_at, 40) })}
            {/if}
          </span>
        </p>
        {#if execution.reason}<p>{reasonText(execution.reason)}</p>{/if}
        {#if execution.status !== 'running'}
          <p class="muted">{t(execution.verified ? 'ui.execution.verified_yes' : 'ui.execution.verified_no')}</p>
        {/if}
      </dd>
      {#if execution.output.trim() !== ''}
        <dt>{t('ui.proposal.output')}</dt>
        <dd><TextBlock text={execution.output} mono /></dd>
      {/if}
    {/if}
  </dl>
  {#if decidable}
    <div class="decide">
      <p class="muted note">{t('ui.proposals.nothing_runs')}</p>
      <div class="row">
        <Button variant="primary" icon="check" onclick={onapprove} disabled={busy}>{t('ui.proposals.approve')}</Button>
        <Button variant="secondary" onclick={onreject} disabled={busy}>{t('ui.proposals.reject')}</Button>
      </div>
    </div>
  {/if}
  {#if approved && proposal.action === null}
    <p class="muted note">{t('ui.proposals.recommendation_only')}</p>
  {:else if runnable}
    <div class="decide">
      {#if !executionEnabled}
        <p id={hintId} class="muted note">{t('ui.proposals.run_off')} <code>{ENABLE_COMMAND}</code>.</p>
      {/if}
      <div class="row">
        <Button variant="primary" icon="arrow" onclick={onrun} disabled={busy || !executionEnabled} describedby={executionEnabled ? undefined : hintId}>{t('ui.proposals.run')}</Button>
      </div>
    </div>
  {/if}
</Card>

<style>
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nv-space-1) var(--nv-space-4);
    font-size: var(--nv-text-sm);
  }
  .when {
    display: inline-flex;
    align-items: center;
    gap: var(--nv-space-1);
  }
  .facts {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--nv-space-1) var(--nv-space-4);
    margin: 0;
  }
  dt {
    font-weight: 650;
    font-size: var(--nv-text-sm);
    color: var(--nv-text-muted);
    margin-block-start: var(--nv-space-2);
  }
  dd {
    margin: 0;
    min-width: 0;
  }
  .decide {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
    padding-block-start: var(--nv-space-3);
    border-block-start: var(--nv-border-width) solid var(--nv-border);
  }
  .prepared {
    overflow-wrap: anywhere;
  }
  .result {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-1);
  }
  .result p {
    max-width: none;
  }
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--nv-space-2);
  }
  code {
    overflow-wrap: anywhere;
  }
  .note {
    font-size: var(--nv-text-sm);
  }
  @media (min-width: 40rem) {
    .facts {
      grid-template-columns: 8rem minmax(0, 1fr);
    }
    dt {
      margin-block-start: 0;
    }
  }
</style>
