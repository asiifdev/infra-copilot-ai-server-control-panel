<script lang="ts">
  import { api } from '../lib/api';
  import { formatDateTime, formatNumber } from '../lib/format';
  import { i18n, t } from '../lib/i18n.svelte';
  import { hrefFor } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import type { NodeSummary } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import DataTable, { type Column } from '../components/DataTable.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';

  const nodes = useResource((signal) => api.nodes(signal));

  const columns = $derived<Column<NodeSummary>[]>([
    { id: 'name', header: t('ui.nodes.name'), sortValue: (n) => n.name },
    { id: 'kind', header: t('ui.nodes.kind'), sortValue: (n) => n.kind },
    { id: 'status', header: t('ui.nodes.status'), sortValue: (n) => n.status },
    { id: 'inventory', header: t('ui.nodes.inventory'), sortValue: (n) => n.latest_inventory?.taken_at ?? '' },
    { id: 'counts', header: t('ui.nodes.counts') },
  ]);
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.nodes')} description={t('ui.nodes.intro')} eyebrow={t('ui.nav.group_monitor')} />
  <AsyncView resource={nodes}>
    {#snippet children(list)}
      {#if list.length === 0}
        <EmptyState icon="nodes" title={t('ui.nodes.empty')} message={t('ui.nodes.empty_hint')} />
      {:else}
        <DataTable
          caption={t('ui.nodes.caption')}
          captionId="nodes-caption"
          {columns}
          rows={list}
          rowKey={(n) => n.name}
          empty={t('ui.nodes.empty')}
        >
          {#snippet cell(node, column)}
            {#if column.id === 'name'}
              <a href={hrefFor({ name: 'node', node: node.name })}>{inlineText(node.name, 120)}</a>
            {:else if column.id === 'kind'}
              {inlineText(node.kind, 60)}
            {:else if column.id === 'status'}
              <StatusBadge kind="node" value={node.status} />
            {:else if column.id === 'inventory'}
              {#if node.latest_inventory}
                {formatDateTime(node.latest_inventory.taken_at, i18n.locale) ?? inlineText(node.latest_inventory.taken_at, 40)}
                <StatusBadge kind="state" value={node.latest_inventory.status} />
              {:else}
                <span class="muted">{t('ui.nodes.never')}</span>
              {/if}
            {:else if node.latest_inventory}
              {@const c = node.latest_inventory.counts}
              {t('ui.nodes.counts_value', {
                listener: formatNumber(c.listener, i18n.locale),
                process: formatNumber(c.process, i18n.locale),
                cron: formatNumber(c.cron, i18n.locale),
                env_ref: formatNumber(c.env_ref, i18n.locale),
              })}
            {:else}
              <span class="muted">{t('ui.common.none')}</span>
            {/if}
          {/snippet}
        </DataTable>
      {/if}
    {/snippet}
  </AsyncView>
</div>
