<script lang="ts">
  import { api } from '../lib/api';
  import { toastError } from '../lib/errors';
  import { t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Select from '../components/Select.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TextField from '../components/TextField.svelte';

  const monitors = useResource((signal) => api.monitors(signal));
  const incidents = useResource((signal) => api.incidents(signal));

  let name = $state('');
  let kind = $state('http');
  let target = $state('');
  let busy = $state(false);

  const KINDS = [
    { value: 'http', label: 'HTTP' },
    { value: 'tcp', label: 'TCP' },
    { value: 'tls', label: 'TLS' },
  ];

  async function runOp(fn: () => Promise<void>) {
    if (busy) return;
    busy = true;
    try {
      await fn();
    } catch (cause) {
      toastError(cause);
    } finally {
      busy = false;
    }
  }

  function addMonitor(event: SubmitEvent) {
    event.preventDefault();
    if (!name.trim() || !target.trim()) return;
    void runOp(async () => {
      await api.createMonitor({ name: name.trim(), kind, target: target.trim() });
      name = '';
      target = '';
      monitors.reload();
    });
  }
</script>

<div class="stack">
  <PageHeader title={t('ui.monitors.title')} eyebrow={t('ui.nav.group_monitor')} />

  <Card title={t('ui.monitors.add')}>
    <form class="row" onsubmit={addMonitor}>
      <TextField id="mon-name" label={t('ui.monitors.name')} bind:value={name} required disabled={busy} />
      <Select id="mon-kind" label={t('ui.monitors.kind')} bind:value={kind} options={KINDS} />
      <TextField id="mon-target" label={t('ui.monitors.target')} bind:value={target} required disabled={busy} />
      <Button type="submit" variant="primary" {busy}>{t('ui.monitors.add')}</Button>
    </form>
  </Card>

  <AsyncView resource={monitors}>
    {#snippet children(list)}
      {#if list.length === 0}
        <EmptyState title={t('ui.monitors.empty')} />
      {:else}
        <ul class="plain stack" aria-label={t('ui.monitors.status')}>
          {#each list as m (m.id)}
            <li class="row">
              <strong>{inlineText(m.name, 80)}</strong>
              <span class="muted">{inlineText(m.kind, 16)}</span>
              <span>{inlineText(m.target, 120)}</span>
              <StatusBadge kind="state" value={m.current_status} />
              <Button variant="secondary" onclick={() => runOp(async () => { await api.deleteMonitor(m.id); monitors.reload(); })} disabled={busy}>{t('ui.common.delete')}</Button>
            </li>
          {/each}
        </ul>
      {/if}
    {/snippet}
  </AsyncView>

  <Card title={t('ui.incidents.title')}>
    {#if incidents.data}
      {#if incidents.data.length === 0}
        <p class="muted">{t('ui.incidents.empty')}</p>
      {:else}
        <ul class="plain stack">
          {#each incidents.data as inc (inc.id)}
            <li class="row">
              <StatusBadge kind="severity" value={inc.severity} />
              <span>{inlineText(inc.cause, 140)}</span>
              <span class="muted">{t(inc.resolved_at ? 'ui.incidents.resolved' : 'ui.incidents.open')}</span>
              {#if !inc.acknowledged_at}
                <Button variant="secondary" onclick={() => runOp(async () => { await api.ackIncident(inc.id); incidents.reload(); })} disabled={busy}>{t('ui.incidents.ack')}</Button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </Card>
</div>
