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
  let modal = $state.raw<{ proposal: ProposalView; kind: 'approve' | 'reject' | 'run' | 'rollback' } | null>(null);
  const executionEnabled = $derived(session.info?.execution_enabled === true);

  const FILTERS: [Filter, string][] = [
    ['pending', 'ui.proposals.f_pending'],
    ['approved', 'ui.proposals.f_approved'],
    ['rejected', 'ui.proposals.f_rejected'],
    ['expired', 'ui.proposals.f_expired'],
    ['all', 'ui.proposals.f_all'],
  ];

  function inFilter(list: ProposalView[], which: Filter): ProposalView[] {
    return which === 'all' ? list : list.filter((p) => GROUPS[which].includes(p.status));
  }

  function options(list: ProposalView[]) {
    return FILTERS.map(([value, label]) => ({ value, label: t(label), count: inFilter(list, value).length }));
  }

  function setFilter(value: string) {
    filter = value in GROUPS || value === 'all' ? (value as Filter) : 'pending';
  }

  function approve(proposal: ProposalView) {
    if (proposal.user_has_approved) return;
    // A risky or destructive change (tier 3 or 4) needs a second, explicit confirmation.
    if (proposal.tier >= 3) modal = { proposal, kind: 'approve' };
    else void decide(proposal, 'approve');
  }

  async function withProposal(id: string, fn: () => Promise<void>, closeModal = false) {
    acting = id;
    try {
      await fn();
    } catch (cause) {
      toastError(cause);
    } finally {
      acting = null;
      if (closeModal) modal = null;
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
      true,
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
      true,
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
      true,
    );
  }
  const dialog = $derived.by(() => {
    if (!modal || (modal.kind === 'approve' && modal.proposal.tier === 4)) return null;
    const { proposal, kind } = modal;
    if (kind === 'approve' || kind === 'reject') {
      const approving = kind === 'approve';
      return {
        id: proposal.id,
        title: t(approving ? 'ui.proposals.risky_title' : 'ui.proposals.reject_title'),
        message: t(approving ? 'ui.proposals.risky_message' : 'ui.proposals.reject_message', { title: inlineText(proposal.title, 80) }),
        confirmLabel: t(approving ? 'ui.proposals.risky_confirm' : 'ui.proposals.reject'),
        tone: (approving ? 'primary' : 'danger') as 'primary' | 'danger',
        onconfirm: () => decide(proposal, kind),
      };
    }
    if (kind === 'run') {
      return {
        id: proposal.id,
        title: t('ui.proposals.run_title'),
        message: t('ui.proposals.run_message', {
          service: inlineText(proposal.action?.kind === 'restart_service' ? proposal.action.service : proposal.action?.target_label, 100),
          manager: inlineText(proposal.action?.kind === 'restart_service' ? proposal.action.manager : proposal.action?.kind, 20),
          node: inlineText(proposal.node, 80) || t('ui.proposal.node_unknown'),
        }),
        confirmLabel: t('ui.proposals.run_confirm'),
        tone: 'primary' as const,
        onconfirm: () => run(proposal),
      };
    }
    return {
      id: proposal.id,
      title: t('ui.proposals.rollback_title'),
      message: t('ui.proposals.rollback_message'),
      confirmLabel: t('ui.proposals.rollback_confirm'),
      tone: 'danger' as const,
      onconfirm: () => rollback(proposal),
    };
  });
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.proposals')} description={t('ui.proposals.intro')} eyebrow={t('ui.nav.group_work')} />

  <Card tone="info">
    <p class="nv-notice"><Icon name="shield" /><span>{t('ui.proposals.nothing_runs')} {t('ui.proposals.later')}</span></p>
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
        <ul class="plain stack" aria-label={t('ui.proposals.list_label')}>
          {#each shown as proposal (proposal.id)}
            <li>
              <ProposalCard
                {proposal}
                level={2}
                busy={acting === proposal.id}
                onapprove={() => approve(proposal)}
                onreject={() => (modal = { proposal, kind: 'reject' })}
                {executionEnabled}
                onrun={() => (modal = { proposal, kind: 'run' })}
                ondryrun={() => dryRun(proposal)}
                onrollback={() => (modal = { proposal, kind: 'rollback' })}
              />
            </li>
          {/each}
        </ul>
      {/if}
    {/snippet}
  </AsyncView>
</div>

{#if modal && modal.kind === 'approve' && modal.proposal.tier === 4}
  {@const { proposal } = modal}
  <ConfirmTyped
    title={t('ui.proposals.risky_title')}
    message={t('ui.proposals.risky_message', { title: inlineText(proposal.title, 80) })}
    targetLabel={proposal.action?.target_label ?? proposal.title}
    confirmLabel={t('ui.proposals.risky_confirm')}
    cancelLabel={t('ui.common.cancel')}
    tone="primary"
    busy={acting === proposal.id}
    onconfirm={(typed) => decide(proposal, 'approve', typed)}
    oncancel={() => (modal = null)}
  />
{:else if dialog}
  <ConfirmDialog
    title={dialog.title}
    message={dialog.message}
    confirmLabel={dialog.confirmLabel}
    cancelLabel={t('ui.common.cancel')}
    tone={dialog.tone}
    busy={acting === dialog.id}
    onconfirm={dialog.onconfirm}
    oncancel={() => (modal = null)}
  />
{/if}
