<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { approvals } from '../lib/approvals.svelte';
  import { errorKey } from '../lib/errors';
  import { t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { toasts } from '../lib/toast.svelte';
  import { inlineText } from '../lib/text';
  import type { ProposalView } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Card from '../components/Card.svelte';
  import ConfirmDialog from '../components/ConfirmDialog.svelte';
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

  async function decide(proposal: ProposalView, decision: 'approve' | 'reject') {
    acting = proposal.id;
    try {
      await api.decideProposal(proposal.id, decision);
      toasts.push('success', t(decision === 'approve' ? 'ui.proposals.approved_toast' : 'ui.proposals.rejected_toast'));
    } catch (cause) {
      // A 409 means someone else got there first, or the time ran out: say so, then show the truth.
      toasts.push('error', t(errorKey(cause instanceof ApiError ? cause.code : 'internal')));
    } finally {
      acting = null;
      confirming = null;
      proposals.reload();
      void approvals.refresh();
    }
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
