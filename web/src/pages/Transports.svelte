<script lang="ts">
  import { api } from '../lib/api';
  import { formatDateTime } from '../lib/format';
  import { i18n, t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import type { TransportSummary } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import DataTable, { type Column } from '../components/DataTable.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';

  const transports = useResource((signal) => api.transports(signal));

  const columns = $derived<Column<TransportSummary>[]>([
    { id: 'name', header: t('ui.transports.name'), sortValue: (x) => x.name },
    { id: 'kind', header: t('ui.transports.kind'), sortValue: (x) => x.kind },
    { id: 'verified', header: t('ui.transports.verified'), sortValue: (x) => x.verified_at ?? '' },
    { id: 'check', header: t('ui.transports.check'), sortValue: (x) => x.last_check ?? '' },
  ]);

  function when(value: string | null): string {
    if (!value) return t('ui.transports.never');
    return formatDateTime(value, i18n.locale) ?? inlineText(value, 40);
  }
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.transports')} description={t('ui.transports.intro')} />
  <AsyncView resource={transports}>
    {#snippet children(list)}
      {#if list.length === 0}
        <EmptyState title={t('ui.transports.empty')} message={t('ui.transports.empty_hint')} />
      {:else}
        <DataTable
          caption={t('ui.transports.caption')}
          captionId="transports-caption"
          {columns}
          rows={list}
          rowKey={(x) => x.name}
          empty={t('ui.transports.empty')}
        >
          {#snippet cell(item, column)}
            {#if column.id === 'name'}
              {inlineText(item.name, 120)}
            {:else if column.id === 'kind'}
              {inlineText(item.kind, 60)}
            {:else if column.id === 'verified'}
              {when(item.verified_at)}
            {:else}
              {when(item.last_check)}
            {/if}
          {/snippet}
        </DataTable>
      {/if}
      <p class="muted">{t('ui.transports.no_config')}</p>
    {/snippet}
  </AsyncView>
</div>
