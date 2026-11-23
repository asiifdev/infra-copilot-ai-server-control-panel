<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { hrefFor } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import type { PlanSummary } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import DataTable, { type Column } from '../components/DataTable.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import ProposalCard from '../components/ProposalCard.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';

  const plans = useResource((signal) => api.plans(signal));
  const proposals = useResource((signal) => api.proposals(undefined, signal));

  const columns = $derived<Column<PlanSummary>[]>([
    { id: 'name', header: t('ui.plans.name'), sortValue: (p) => p.name },
    { id: 'status', header: t('ui.plans.status'), sortValue: (p) => p.status },
  ]);
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.plans')} description={t('ui.plans.intro')} />
  <p class="draft-note"><StatusBadge kind="state" value="draft" /> {t('ui.plans.draft_note')}</p>
  <AsyncView resource={plans}>
    {#snippet children(list)}
      {#if list.length === 0}
        <EmptyState title={t('ui.plans.empty')} message={t('ui.plans.empty_hint')} />
      {:else}
        <DataTable
          caption={t('ui.plans.caption')}
          captionId="plans-caption"
          {columns}
          rows={list}
          rowKey={(p) => p.name}
          empty={t('ui.plans.empty')}
        >
          {#snippet cell(plan, column)}
            {#if column.id === 'name'}
              <a href={hrefFor({ name: 'plan', plan: plan.name })}>{inlineText(plan.name, 120)}</a>
            {:else}
              <StatusBadge kind="state" value={plan.status} />
            {/if}
          {/snippet}
        </DataTable>
      {/if}
    {/snippet}
  </AsyncView>

  <h2>{t('ui.plans.proposals')}</h2>
  <AsyncView resource={proposals}>
    {#snippet children(list)}
      {#if list.length === 0}
        <EmptyState title={t('ui.plans.no_proposals')} />
      {:else}
        <ul class="plain">
          {#each list as proposal (proposal.id)}
            <li><ProposalCard {proposal} /></li>
          {/each}
        </ul>
      {/if}
    {/snippet}
  </AsyncView>
</div>

<style>
  .plain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
  }
</style>
