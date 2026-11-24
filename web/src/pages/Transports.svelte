<script lang="ts">
  import { api } from '../lib/api';
  import { formatBytes, formatDateTime } from '../lib/format';
  import { i18n, t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import { readCheck } from '../lib/transport';
  import type { TransportSummary } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import DataTable, { type Column } from '../components/DataTable.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import PageHeader from '../components/PageHeader.svelte';

  const transports = useResource((signal) => api.transports(signal));

  const columns = $derived<Column<TransportSummary>[]>([
    { id: 'name', header: t('ui.transports.name'), sortValue: (x) => x.name },
    { id: 'kind', header: t('ui.transports.kind'), sortValue: (x) => x.kind },
    { id: 'verified', header: t('ui.transports.verified'), sortValue: (x) => x.verified_at ?? '' },
    { id: 'check', header: t('ui.transports.check'), sortValue: (x) => checkRank(x) },
  ]);

  // Never checked sorts first, then failed, then passed.
  function checkRank(item: TransportSummary): number {
    const check = readCheck(item.last_check);
    return check === null ? 0 : check.ok === true ? 3 : check.ok === false ? 2 : 1;
  }

  function when(value: string | null): string {
    if (!value) return t('ui.transports.never');
    return formatDateTime(value, i18n.locale) ?? inlineText(value, 40);
  }
</script>

{#snippet check(item: TransportSummary)}
  {@const view = readCheck(item.last_check)}
  {#if view === null}
    {t('ui.transports.never')}
  {:else}
    {#if view.ok === null}
      {t('ui.transports.check_unknown')}
    {:else}
      <StatusBadge kind="state" value={view.ok ? 'passed' : 'failed'} />
    {/if}
    {#if view.freeBytes !== null && view.totalBytes !== null}
      <span class="muted detail">{t('ui.transports.check_free', { free: formatBytes(view.freeBytes, i18n.locale), total: formatBytes(view.totalBytes, i18n.locale) })}</span>
    {:else if view.freeBytes !== null}
      <span class="muted detail">{t('ui.transports.check_free_only', { free: formatBytes(view.freeBytes, i18n.locale) })}</span>
    {/if}
    {#if view.speedBytesPerSec !== null}
      <span class="muted detail">{t('ui.transports.check_speed', { speed: formatBytes(view.speedBytesPerSec, i18n.locale) })}</span>
    {/if}
  {/if}
{/snippet}

<div class="stack">
  <PageHeader title={t('ui.nav.transports')} description={t('ui.transports.intro')} eyebrow={t('ui.nav.group_work')} />
  <AsyncView resource={transports}>
    {#snippet children(list)}
      {#if list.length === 0}
        <EmptyState icon="transports" title={t('ui.transports.empty')} message={t('ui.transports.empty_hint')} />
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
              {@render check(item)}
            {/if}
          {/snippet}
        </DataTable>
      {/if}
      <p class="muted">{t('ui.transports.no_config')}</p>
    {/snippet}
  </AsyncView>
</div>

<style>
  .detail {
    display: block;
    font-size: var(--nv-text-sm);
  }
</style>
