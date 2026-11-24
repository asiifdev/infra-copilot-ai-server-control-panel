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
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';

  const plans = useResource((signal) => api.plans(signal));

  const columns = $derived<Column<PlanSummary>[]>([
    { id: 'name', header: t('ui.plans.name'), sortValue: (p) => p.name },
    { id: 'status', header: t('ui.plans.status'), sortValue: (p) => p.status },
  ]);
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.plans')} description={t('ui.plans.intro')} eyebrow={t('ui.nav.group_work')} />
  <p class="row"><StatusBadge kind="state" value="draft" /> {t('ui.plans.draft_note')}</p>
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

  <Card title={t('ui.nav.proposals')}>
    <p class="muted">{t('ui.plans.proposals_moved')}</p>
    <div class="row">
      <Button icon="proposals" href={hrefFor({ name: 'proposals' })}>{t('ui.plans.open_approvals')}</Button>
    </div>
  </Card>
</div>
