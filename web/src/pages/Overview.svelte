<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { hrefFor } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { session } from '../lib/session.svelte';
  import AsyncView from '../components/AsyncView.svelte';
  import Card from '../components/Card.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';

  const nodes = useResource((signal) => api.nodes(signal));
  const findings = useResource((signal) => api.findings(undefined, undefined, signal));
  const providers = useResource((signal) => api.providers(signal));

  const SEVERITIES = ['critical', 'important', 'info'] as const;

  function countBy<T>(items: T[], pick: (item: T) => string): Record<string, number> {
    const counts: Record<string, number> = {};
    for (const item of items) {
      const key = pick(item);
      counts[key] = (counts[key] ?? 0) + 1;
    }
    return counts;
  }
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.overview')} description={t('ui.overview.intro')} />

  <div class="grid">
    <Card title={t('ui.nav.nodes')}>
      <AsyncView resource={nodes}>
        {#snippet children(list)}
          <p class="big">{t('ui.overview.nodes_count', { count: list.length })}</p>
          <ul class="plain">
            {#each Object.entries(countBy(list, (n) => n.status)) as [status, count] (status)}
              <li><StatusBadge kind="node" value={status} /> <span>{count}</span></li>
            {/each}
          </ul>
          <a href={hrefFor({ name: 'nodes' })}>{t('ui.overview.see_nodes')}</a>
        {/snippet}
      </AsyncView>
    </Card>

    <Card title={t('ui.nav.findings')}>
      <AsyncView resource={findings}>
        {#snippet children(view)}
          {#if view.findings.length === 0}
            <p>{t('ui.overview.no_findings')}</p>
          {:else}
            {@const counts = countBy(view.findings, (f) => f.severity)}
            <p class="big">{t('ui.overview.findings_count', { count: view.findings.length })}</p>
            <ul class="plain">
              {#each SEVERITIES as severity (severity)}
                <li><StatusBadge kind="severity" value={severity} /> <span>{counts[severity] ?? 0}</span></li>
              {/each}
            </ul>
          {/if}
          <a href={hrefFor({ name: 'findings' })}>{t('ui.overview.see_findings')}</a>
        {/snippet}
      </AsyncView>
    </Card>

    <Card title={t('ui.overview.ai_title')}>
      <AsyncView resource={providers}>
        {#snippet children(list)}
          {@const chosen = list.find((p) => p.is_default) ?? list[0]}
          {#if chosen}
            <p>{t('ui.overview.provider_default', { count: list.length })}</p>
          {:else}
            <p>{t('ui.overview.provider_none')}</p>
          {/if}
        {/snippet}
      </AsyncView>
      <p>
        <StatusBadge kind="state" value={session.info?.master_key_configured ? 'ok' : 'error'} />
        {session.info?.master_key_configured ? t('ui.overview.key_ok') : t('ui.overview.key_missing')}
      </p>
      <a href={hrefFor({ name: 'settings' })}>{t('ui.overview.open_settings')}</a>
    </Card>
  </div>
</div>

<style>
  .big {
    font-size: var(--nv-text-lg);
    font-weight: 700;
  }
  .plain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
  }
</style>
