<script lang="ts">
  import { api } from '../lib/api';
  import { formatNumber } from '../lib/format';
  import { i18n, t } from '../lib/i18n.svelte';
  import { hrefFor, type Route } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { session } from '../lib/session.svelte';
  import type { IconName } from '../components/Icon.svelte';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import Icon from '../components/Icon.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import StatTile from '../components/StatTile.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';

  const nodes = useResource((signal) => api.nodes(signal));
  const findings = useResource((signal) => api.findings(undefined, undefined, signal));
  const providers = useResource((signal) => api.providers(signal));
  const proposals = useResource((signal) => api.proposals('pending', signal));
  const approved = useResource((signal) => api.proposals('approved', signal));

  const SEVERITIES = ['critical', 'important', 'info'] as const;
  const num = (n: number) => formatNumber(n, i18n.locale);

  function countBy<T>(items: T[], pick: (item: T) => string): Record<string, number> {
    const counts: Record<string, number> = {};
    for (const item of items) {
      const key = pick(item);
      counts[key] = (counts[key] ?? 0) + 1;
    }
    return counts;
  }

  interface Attention {
    id: string;
    icon: IconName;
    text: string;
    href: string;
    link: string;
  }

  // What needs a person now, built only from data that has arrived. Pending approvals come first.
  const attention = $derived.by<Attention[] | null>(() => {
    if (!nodes.data || !findings.data || !providers.data || !proposals.data || !approved.data) return null;
    const out: Attention[] = [];
    const add = (id: string, icon: IconName, text: string, route: Route, linkKey: string) =>
      out.push({ id, icon, text, href: hrefFor(route), link: t(linkKey) });
    if (proposals.data.length > 0) {
      add('pending', 'proposals', t('ui.overview.attention_pending', { count: proposals.data.length }), { name: 'proposals' }, 'ui.overview.act_review');
    }
    // Approved and not yet run: only an approved proposal with an action can still be run.
    const waiting = approved.data.filter((p) => p.action !== null).length;
    if (waiting > 0) {
      add('run', 'proposals', t('ui.overview.attention_run', { count: waiting }), { name: 'proposals' }, 'ui.overview.act_review');
    }
    const critical = findings.data.findings.filter((f) => f.severity === 'critical').length;
    if (critical > 0) {
      add('critical', 'findings', t('ui.overview.attention_critical', { count: critical }), { name: 'findings' }, 'ui.overview.see_findings');
    }
    const offline = nodes.data.filter((n) => n.status === 'offline').length;
    if (offline > 0) {
      add('offline', 'nodes', t('ui.overview.attention_offline', { count: offline }), { name: 'nodes' }, 'ui.overview.see_nodes');
    }
    if (providers.data.length === 0) {
      add('provider', 'assistant', t('ui.overview.provider_none'), { name: 'settings' }, 'ui.overview.act_provider');
    }
    if (!session.info?.master_key_configured) {
      add('key', 'shield', t('ui.overview.key_missing'), { name: 'settings' }, 'ui.overview.open_settings');
    }
    return out;
  });
</script>

<div class="stack nv-overview">
  <PageHeader title={t('ui.nav.overview')} description={t('ui.overview.intro')} eyebrow={t('ui.nav.group_monitor')} />

  <div class="tiles">
    <AsyncView resource={proposals}>
      {#snippet children(list)}
        <StatTile
          label={t('ui.nav.proposals')}
          value={num(list.length)}
          icon="proposals"
          tone={list.length > 0 ? 'warning' : 'success'}
          href={hrefFor({ name: 'proposals' })}
          linkLabel={t('ui.overview.act_review')}
        >
          {list.length > 0 ? t('ui.overview.approvals_waiting') : t('ui.overview.approvals_none')}
        </StatTile>
      {/snippet}
    </AsyncView>

    <AsyncView resource={nodes}>
      {#snippet children(list)}
        {@const counts = countBy(list, (n) => n.status)}
        <StatTile
          label={t('ui.nav.nodes')}
          value={num(counts.online ?? 0)}
          icon="nodes"
          tone={(counts.offline ?? 0) > 0 ? 'danger' : undefined}
          href={hrefFor({ name: 'nodes' })}
          linkLabel={t('ui.overview.see_nodes')}
        >
          <span>{t('ui.overview.online_of', { total: num(list.length) })}</span>
          {#if (counts.offline ?? 0) > 0}<span class="pair"><StatusBadge kind="node" value="offline" /><span class="num">{counts.offline}</span></span>{/if}
        </StatTile>
      {/snippet}
    </AsyncView>

    <AsyncView resource={findings}>
      {#snippet children(view)}
        {@const counts = countBy(view.findings, (f) => f.severity)}
        <StatTile
          label={t('ui.nav.findings')}
          value={num(view.findings.length)}
          icon="findings"
          tone={(counts.critical ?? 0) > 0 ? 'danger' : undefined}
          href={hrefFor({ name: 'findings' })}
          linkLabel={t('ui.overview.see_findings')}
        >
          {#if view.findings.length === 0}
            {t('ui.overview.no_findings')}
          {:else}
            {#each SEVERITIES as severity (severity)}
              {#if counts[severity]}<span class="pair"><StatusBadge kind="severity" value={severity} /><span class="num">{counts[severity]}</span></span>{/if}
            {/each}
          {/if}
        </StatTile>
      {/snippet}
    </AsyncView>

    <AsyncView resource={providers}>
      {#snippet children(list)}
        {@const ready = list.length > 0 && session.info?.master_key_configured === true}
        <StatTile
          label={t('ui.overview.ai_title')}
          value={ready ? t('ui.overview.ai_ready') : t('ui.overview.ai_not_ready')}
          icon="assistant"
          tone={ready ? 'success' : 'warning'}
          href={hrefFor({ name: 'settings' })}
          linkLabel={t('ui.overview.open_settings')}
        >
          {list.length > 0 ? t('ui.overview.provider_default', { count: list.length }) : t('ui.overview.provider_none')}
        </StatTile>
      {/snippet}
    </AsyncView>
  </div>

  <div class="lower">
    <Card title={t('ui.overview.attention_title')}>
      {#if attention === null}
        <p class="muted">{t('ui.common.loading')}</p>
      {:else if attention.length === 0}
        <p class="calm"><Icon name="check" />{t('ui.overview.attention_none')}</p>
      {:else}
        <ul class="plain items">
          {#each attention as item (item.id)}
            <li class="item">
              <span class="ico"><Icon name={item.icon} /></span>
              <span class="text">{item.text}</span>
              <a href={item.href}>{item.link}</a>
            </li>
          {/each}
        </ul>
      {/if}
    </Card>

    <Card title={t('ui.overview.quick_title')}>
      <div class="actions">
        <Button variant="primary" icon="assistant" href={hrefFor({ name: 'assistant', conversation: null })}>{t('ui.overview.act_ask')}</Button>
        <Button icon="plus" href={hrefFor({ name: 'settings' })}>{t('ui.overview.act_provider')}</Button>
        <Button icon="proposals" href={hrefFor({ name: 'proposals' })}>{t('ui.overview.act_review')}</Button>
      </div>
    </Card>
  </div>
</div>
