<script lang="ts">
  import { api } from '../lib/api';
  import { approvals } from '../lib/approvals.svelte';
  import { toastError } from '../lib/errors';
  import { t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { session } from '../lib/session.svelte';
  import { toasts } from '../lib/toast.svelte';
  import { inlineText } from '../lib/text';
  import type { ProposalView } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Card from '../components/Card.svelte';
  import ConfirmDialog from '../components/ConfirmDialog.svelte';
  import ConfirmTyped from '../components/ConfirmTyped.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import Icon from '../components/Icon.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import ProposalCard from '../components/ProposalCard.svelte';
  import Segmented from '../components/Segmented.svelte';

  const proposals = useResource((signal) => api.proposals(undefined, signal));

  type Filter = 'pending' | 'approved' | 'rejected' | 'expired' | 'all';
  // Which stored states belong to which filter (an approved proposal may later run or fail).
  const GROUPS: Record<Exclude<Filter, 'all'>, string[]> = {
    pending: ['pending'],
    approved: ['approved', 'executed', 'failed'],
    rejected: ['rejected'],
    expired: ['expired'],
  };

  let filter = $state<Filter>('pending');
  let acting = $state<string | null>(null);
  // The decision waiting for a second look: every rejection, and an approval of a risky change.
  let confirming = $state.raw<{ proposal: ProposalView; decision: 'approve' | 'reject' } | null>(null);

  // The approved action waiting for a second look before it runs.
  let running = $state.raw<ProposalView | null>(null);
  let rollingBack = $state.raw<ProposalView | null>(null);
  const executionEnabled = $derived(session.info?.execution_enabled === true);

  function inFilter(list: ProposalView[], which: Filter): ProposalView[] {
    return which === 'all' ? list : list.filter((p) => GROUPS[which].includes(p.status));
  }

  function options(list: ProposalView[]) {
    return [
      { value: 'pending', label: t('ui.proposals.f_pending'), count: inFilter(list, 'pending').length },
      { value: 'approved', label: t('ui.proposals.f_approved'), count: inFilter(list, 'approved').length },
      { value: 'rejected', label: t('ui.proposals.f_rejected'), count: inFilter(list, 'rejected').length },
      { value: 'expired', label: t('ui.proposals.f_expired'), count: inFilter(list, 'expired').length },
      { value: 'all', label: t('ui.proposals.f_all'), count: list.length },
    ];
  }

  function setFilter(value: string) {
    filter = value in GROUPS || value === 'all' ? (value as Filter) : 'pending';
  }

  function approve(proposal: ProposalView) {
    // A risky or destructive change (tier 3 or 4) needs a second, explicit confirmation.
    if (proposal.tier >= 3) confirming = { proposal, decision: 'approve' };
    else void decide(proposal, 'approve');
  }

  async function withProposal(id: string, fn: () => Promise<void>, clear?: () => void) {
    acting = id;
    try {
      await fn();
    } catch (cause) {
      toastError(cause);
    } finally {
      acting = null;
      clear?.();
      proposals.reload();
      void approvals.refresh();
    }
  }

  function decide(proposal: ProposalView, decision: 'approve' | 'reject', typedConfirmation?: string) {
    return withProposal(
      proposal.id,
      async () => {
        await api.decideProposal(proposal.id, decision, typedConfirmation);
        toasts.push('success', t(decision === 'approve' ? 'ui.proposals.approved_toast' : 'ui.proposals.rejected_toast'));
      },
      () => (confirming = null),
    );
  }

  function dryRun(proposal: ProposalView) {
    return withProposal(proposal.id, () => api.dryRunProposal(proposal.id).then(() => {}));
  }

  function run(proposal: ProposalView) {
    return withProposal(
      proposal.id,
      async () => {
        const res = await api.executeProposal(proposal.id);
        const ex = res.execution;
        if (ex?.status === 'succeeded') {
          toasts.push(ex.verified ? 'success' : 'info', t(ex.verified ? 'ui.proposals.run_ok_toast' : 'ui.proposals.run_unverified_toast'));
        } else if (res.status === 'queued' || ex?.status === 'queued' || res.agent_job) {
          toasts.push('info', t('ui.execution.queued'));
        } else {
          toasts.push('error', t('ui.proposals.run_failed_toast'));
        }
      },
      () => (running = null),
    );
  }

  function rollback(proposal: ProposalView) {
    return withProposal(
      proposal.id,
      async () => {
        const result = await api.rollbackProposal(proposal.id);
        if (result.rollback.status === 'succeeded') {
          toasts.push(result.rollback.verified ? 'success' : 'info', t('ui.proposals.rollback_ok_toast'));
        } else {
          toasts.push('error', t('ui.proposals.rollback_failed_toast'));
        }
      },
      () => (rollingBack = null),
    );
  }
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.proposals')} description={t('ui.proposals.intro')} eyebrow={t('ui.nav.group_work')} />

  <Card tone="info">
    <p class="notice"><Icon name="shield" /><span>{t('ui.proposals.nothing_runs')} {t('ui.proposals.later')}</span></p>
  </Card>

  <AsyncView resource={proposals}>
    {#snippet children(list)}
      <Segmented label={t('ui.proposals.filter')} options={options(list)} value={filter} onchange={setFilter} />
      {@const shown = inFilter(list, filter)}
      {#if shown.length === 0}
        {#if filter === 'pending'}
          <EmptyState icon="check" title={t('ui.proposals.empty_pending')} message={t('ui.proposals.empty_pending_hint')} />
        {:else}
          <EmptyState title={t('ui.proposals.empty_other')} />
        {/if}
      {:else}
        <ul class="plain cards" aria-label={t('ui.proposals.list_label')}>
          {#each shown as proposal (proposal.id)}
            <li>
              <ProposalCard
                {proposal}
                level={2}
                busy={acting === proposal.id}
                onapprove={() => approve(proposal)}
                onreject={() => (confirming = { proposal, decision: 'reject' })}
                {executionEnabled}
                onrun={() => (running = proposal)}
                ondryrun={() => dryRun(proposal)}
                onrollback={() => (rollingBack = proposal)}
              />
            </li>
          {/each}
        </ul>
      {/if}
    {/snippet}
  </AsyncView>
</div>

{#if confirming}
  {@const { proposal, decision } = confirming}
  {@const approving = decision === 'approve'}
  {#if approving && proposal.tier === 4}
    <ConfirmTyped
      title={t('ui.proposals.risky_title')}
      message={t('ui.proposals.risky_message', { title: inlineText(proposal.title, 80) })}
      targetLabel={proposal.action?.target_label ?? proposal.title}
      confirmLabel={t('ui.proposals.risky_confirm')}
      cancelLabel={t('ui.common.cancel')}
      tone="primary"
      busy={acting === proposal.id}
      onconfirm={(typed) => decide(proposal, decision, typed)}
      oncancel={() => (confirming = null)}
    />
  {:else}
    <ConfirmDialog
      title={t(approving ? 'ui.proposals.risky_title' : 'ui.proposals.reject_title')}
      message={t(approving ? 'ui.proposals.risky_message' : 'ui.proposals.reject_message', { title: inlineText(proposal.title, 80) })}
      confirmLabel={t(approving ? 'ui.proposals.risky_confirm' : 'ui.proposals.reject')}
      cancelLabel={t('ui.common.cancel')}
      tone={approving ? 'primary' : 'danger'}
      busy={acting === proposal.id}
      onconfirm={() => decide(proposal, decision)}
      oncancel={() => (confirming = null)}
    />
  {/if}
{/if}

{#if running}
  {@const target = running}
  <ConfirmDialog
    title={t('ui.proposals.run_title')}
    message={t('ui.proposals.run_message', {
      service: inlineText(target.action?.kind === 'restart_service' ? target.action.service : target.action?.target_label, 100),
      manager: inlineText(target.action?.kind === 'restart_service' ? target.action.manager : target.action?.kind, 20),
      node: inlineText(target.node, 80) || t('ui.proposal.node_unknown'),
    })}
    confirmLabel={t('ui.proposals.run_confirm')}
    cancelLabel={t('ui.common.cancel')}
    tone="primary"
    busy={acting === target.id}
    onconfirm={() => run(target)}
    oncancel={() => (running = null)}
  />
{/if}

{#if rollingBack}
  {@const target = rollingBack}
  <ConfirmDialog
    title={t('ui.proposals.rollback_title')}
    message={t('ui.proposals.rollback_message')}
    confirmLabel={t('ui.proposals.rollback_confirm')}
    cancelLabel={t('ui.common.cancel')}
    tone="danger"
    busy={acting === target.id}
    onconfirm={() => rollback(target)}
    oncancel={() => (rollingBack = null)}
  />
{/if}

<style>
  .notice {
    display: flex;
    align-items: flex-start;
    gap: var(--nv-space-3);
    max-width: none;
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-4);
  }
</style>
