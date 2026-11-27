<script lang="ts">
  import { dateText } from '../lib/format';
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
    /** Offered on a proposal that has an action to compute a dry-run preview. */
    ondryrun?: () => void;
    /** Offered on an executed proposal that has a backup and has not been rolled back. */
    onrollback?: () => void;
  }

  let { proposal, level = 3, busy = false, onapprove, onreject, executionEnabled = false, onrun, ondryrun, onrollback }: Props = $props();

  const pending = $derived(proposal.status === 'pending');
  const approved = $derived(proposal.status === 'approved');
  const expiry = $derived(relativeTime(proposal.expires_at, i18n.locale));
  const decidable = $derived(pending && onapprove !== undefined && onreject !== undefined);
  const runnable = $derived(approved && proposal.action !== null && onrun !== undefined);
  const canDryRun = $derived((pending || approved) && proposal.action !== null && ondryrun !== undefined);
  const rollbackable = $derived(
    proposal.has_backup && proposal.rollback === null && proposal.execution !== null && proposal.execution.status !== 'running' && onrollback !== undefined,
  );
  // A command the owner types, not text to translate.
  const ENABLE_COMMAND = 'infra serve --enable-execution';

  function actionText(action: NonNullable<ProposalView['action']>): string {
    if (action.kind === 'restart_service') {
      return t('ui.proposal.restart_service', {
        service: inlineText(action.service, 100),
        manager: inlineText(action.manager, 20),
        node: inlineText(proposal.node, 80) || t('ui.proposal.node_unknown'),
      });
    }
    return t('ui.proposal.action_other', {
      kind: inlineText(`${action.kind}: ${action.target_label}`, 120),
    });
  }

  const REASONS: Record<string, string> = {
    ok: 'ui.execution.reason_ok',
    not_installed: 'ui.execution.reason_not_installed',
    timeout: 'ui.execution.reason_timeout',
    command_failed: 'ui.execution.reason_command_failed',
    not_active_after: 'ui.execution.reason_not_active_after',
    io: 'ui.execution.reason_io',
    action_invalid: 'ui.execution.reason_action_invalid',
    interrupted: 'ui.execution.reason_interrupted',
  };

  function reasonText(reason: string): string {
    return t(REASONS[reason] ?? 'ui.execution.reason_other');
  }
</script>

{#snippet outcomeView(labelKey: string, item: { status: string; verified: boolean; reason: string | null; output: string; started_at: string; finished_at: string | null }, showBackup: boolean)}
  <dt>{t(labelKey)}</dt>
  <dd class="nv-prop-result">
    <p class="nv-prop-status">
      <StatusBadge kind="state" value={item.status} />
      <span class="muted">
        {#if item.finished_at}
          {t('ui.execution.finished', { when: dateText(item.finished_at, i18n.locale) })}
        {:else}
          {t('ui.execution.started', { when: dateText(item.started_at, i18n.locale) })}
        {/if}
      </span>
    </p>
    {#if item.reason}<p>{reasonText(item.reason)}</p>{/if}
    {#if item.status === 'queued'}
      <p class="muted">{t('ui.execution.queued')}</p>
    {:else if item.status !== 'running'}
      <p class="muted">{t(item.verified ? 'ui.execution.verified_yes' : 'ui.execution.verified_no')}</p>
    {/if}
    {#if showBackup}
      <p class="muted">{t('ui.proposals.backup_ready')}</p>
    {/if}
  </dd>
  {#if item.output.trim() !== ''}
    <dt>{t('ui.proposal.output')}</dt>
    <dd><TextBlock text={item.output} mono /></dd>
  {/if}
{/snippet}

<Card title={inlineText(proposal.title, 120)} {level} tone={proposal.tier >= 3 ? 'danger' : proposal.tier === 2 ? 'warning' : undefined}>
  {#snippet actions()}
    <StatusBadge kind="state" value={proposal.status} />
    <StatusBadge kind="tier" value={proposal.tier} />
    {#if (proposal.required_approvals ?? 1) > 1}
      <span class="nv-prop-progress num">{t('ui.proposals.approvals_progress', { count: proposal.approvals_count ?? 0, required: proposal.required_approvals ?? 1 })}</span>
    {/if}
  {/snippet}
  <ul class="nv-prop-meta plain muted">
    <li>{t('ui.proposal.tool', { tool: inlineText(proposal.tool, 64) })}</li>
    {#if proposal.rule_id}<li>{t('ui.proposals.rule', { rule: inlineText(proposal.rule_id, 40) })}</li>{/if}
    {#if proposal.subject_key}<li>{t('ui.proposals.subject', { subject: inlineText(proposal.subject_key, 80) })}</li>{/if}
    <li class="nv-prop-when">
      <Icon name="clock" size="sm" />
      {#if pending && expiry}
        {t(expiry.past ? 'ui.proposals.expired_ago' : 'ui.proposals.expires', { when: expiry.text })}
      {:else}
        {t('ui.proposals.created', { when: dateText(proposal.created_at, i18n.locale) })}
      {/if}
    </li>
  </ul>
  <dl class="nv-prop-facts">
    <dt>{t('ui.proposal.reason')}</dt>
    <dd><TextBlock text={proposal.rationale} /></dd>
    <dt>{t('ui.proposal.action')}</dt>
    <dd><TextBlock text={proposal.suggested_action} /></dd>
    <dt>{t('ui.proposal.impact')}</dt>
    <dd><TextBlock text={proposal.impact} /></dd>
    {#if proposal.action}
      <dt>{t('ui.proposal.prepared')}</dt>
      <dd class="nv-prop-prepared">{actionText(proposal.action)}</dd>
    {/if}
    {#if proposal.dry_run}
      <dt>{t('ui.proposals.dry_run_title')}</dt>
      <dd class="nv-prop-result">
        <p class="muted">{t('ui.proposals.dry_run_state')}</p>
        <p>{inlineText(proposal.dry_run.current_state, 300)}</p>
        {#if proposal.dry_run.steps.length > 0}
          <p class="muted">{t('ui.proposals.dry_run_steps')}</p>
          <ol class="nv-prop-steps">
            {#each proposal.dry_run.steps as step}
              <li>{inlineText(step, 200)}</li>
            {/each}
          </ol>
        {/if}
        {#if proposal.dry_run.diff}
          <p class="muted">{t('ui.proposals.dry_run_diff')}</p>
          <pre class="nv-prop-diff"><code>{proposal.dry_run.diff}</code></pre>
        {/if}
      </dd>
    {/if}
    {#if proposal.execution}{@render outcomeView('ui.proposal.result', proposal.execution, proposal.has_backup)}{/if}
    {#if proposal.rollback}{@render outcomeView('ui.proposals.rollback', proposal.rollback, false)}{/if}
  </dl>
  {#if decidable}
    <div class="nv-prop-decide">
      <p class="muted nv-prop-note">{t('ui.proposals.nothing_runs')}</p>
      <div class="row">
        <Button variant="primary" icon="check" onclick={onapprove} disabled={busy || proposal.user_has_approved === true}>
          {t(proposal.user_has_approved ? 'ui.proposals.already_approved' : 'ui.proposals.approve')}
        </Button>
        <Button variant="secondary" onclick={onreject} disabled={busy}>{t('ui.proposals.reject')}</Button>
        {#if canDryRun}
          <Button variant="secondary" onclick={ondryrun} disabled={busy}>{t('ui.proposals.dry_run')}</Button>
        {/if}
      </div>
    </div>
  {/if}
  {#if approved && proposal.action === null}
    <p class="muted nv-prop-note">{t('ui.proposals.recommendation_only')}</p>
  {:else if runnable || rollbackable}
    {@const hintId = `run-off-${proposal.id}`}
    <div class="nv-prop-decide">
      {#if !executionEnabled}
        <p id={hintId} class="muted nv-prop-note">{t('ui.proposals.run_off')} <code>{ENABLE_COMMAND}</code>.</p>
      {/if}
      <div class="row">
        {#if runnable}
          <Button variant="primary" icon="arrow" onclick={onrun} disabled={busy || !executionEnabled} describedby={executionEnabled ? undefined : hintId}>{t('ui.proposals.run')}</Button>
          {#if canDryRun}
            <Button variant="secondary" onclick={ondryrun} disabled={busy}>{t('ui.proposals.dry_run')}</Button>
          {/if}
        {:else}
          <Button variant="secondary" onclick={onrollback} disabled={busy || !executionEnabled} describedby={executionEnabled ? undefined : hintId}>{t('ui.proposals.rollback')}</Button>
        {/if}
      </div>
    </div>
  {/if}
</Card>
