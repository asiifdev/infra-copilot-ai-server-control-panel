<script lang="ts">
  import { api } from '../lib/api';
  import { toastError, withBusy } from '../lib/errors';
  import { dateText, describeData } from '../lib/format';
  import { i18n, t } from '../lib/i18n.svelte';
  import { hrefFor } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import { toasts } from '../lib/toast.svelte';
  import type { InventoryItem, NodeLogsView } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import DataTable, { type Column } from '../components/DataTable.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Select from '../components/Select.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TextBlock from '../components/TextBlock.svelte';
  import TextField from '../components/TextField.svelte';

  interface Props {
    name: string;
  }

  let { name }: Props = $props();

  let kind = $state('');
  let logService = $state('');
  let logBusy = $state(false);
  let logs = $state.raw<NodeLogsView | null>(null);
  let tokenBusy = $state(false);
  let rotatedToken = $state<string | null>(null);

  function loadLogs(event: SubmitEvent) {
    event.preventDefault();
    void withBusy((b) => (logBusy = b), async () => {
      logs = await api.nodeLogs(name, logService.trim());
    });
  }

  function rotateToken() {
    void withBusy((b) => (tokenBusy = b), async () => {
      rotatedToken = (await api.rotateAgentToken(name)).token;
    });
  }

  const inventory = useResource((signal) => api.inventory(name, kind || undefined, 500, signal));
  const nodes = useResource((signal) => api.nodes(signal));
  const isAgent = $derived(
    inventory.data?.access === 'agent' || nodes.data?.some((n) => n.name === name && n.access === 'agent') === true,
  );
  const agentCommand = $derived(
    `nodivara-agent --server ${globalThis.location?.origin ?? ''} --node ${inlineText(name, 80)} --token ${rotatedToken ?? '<TOKEN>'}`,
  );

  async function copyCommand() {
    try {
      await navigator.clipboard?.writeText(agentCommand);
      toasts.push('success', t('ui.nodes.agent_command_copied'));
    } catch (cause) {
      toastError(cause);
    }
  }

  const kindOptions = $derived([
    { value: '', label: t('ui.node.kind_all') },
    { value: 'listener', label: t('ui.node.kind_listener') },
    { value: 'process', label: t('ui.node.kind_process') },
    { value: 'cron', label: t('ui.node.kind_cron') },
    { value: 'env_ref', label: t('ui.node.kind_env_ref') },
  ]);

  const columns = $derived<Column<InventoryItem>[]>([
    { id: 'key', header: t('ui.node.item_key'), sortValue: (i) => i.key },
    { id: 'kind', header: t('ui.node.item_kind'), sortValue: (i) => i.kind },
    { id: 'data', header: t('ui.node.item_data') },
  ]);
</script>

<div class="stack">
  <PageHeader title={t('ui.node.title', { name: inlineText(name, 80) })} eyebrow={t('ui.nav.group_monitor')}>
    <Button variant="ghost" icon="return" href={hrefFor({ name: 'nodes' })}>{t('ui.node.back')}</Button>
  </PageHeader>

  <Select label={t('ui.node.filter')} id="inventory-kind" bind:value={kind} options={kindOptions} />

  <AsyncView resource={inventory}>
    {#snippet children(view)}
      <Card>
        <p>
          <StatusBadge kind="state" value={view.status} />
          {t('ui.node.taken_at', { time: dateText(view.taken_at, i18n.locale) })}
        </p>
        {#if view.warnings.length > 0}
          <h2>{t('ui.node.warnings')}</h2>
          <ul>
            {#each view.warnings as warning, index (index)}
              <li>{inlineText(warning, 300)}</li>
            {/each}
          </ul>
        {/if}
      </Card>
      <DataTable
        caption={t('ui.node.caption')}
        captionId="inventory-caption"
        {columns}
        rows={view.items}
        rowKey={(i) => `${i.kind}\u0000${i.key}`}
        empty={t('ui.node.empty')}
      >
        {#snippet cell(item, column)}
          {#if column.id === 'key'}
            <code>{inlineText(item.key, 200)}</code>
          {:else if column.id === 'kind'}
            {inlineText(item.kind, 40)}
          {:else}
            <details>
              <summary>{t('ui.node.show_data')}</summary>
              <TextBlock text={describeData(item.data)} mono />
            </details>
          {/if}
        {/snippet}
      </DataTable>
    {/snippet}
  </AsyncView>

  <Card title={t('ui.logs.title')}>
    <form class="row" onsubmit={loadLogs}>
      <TextField id="node-log-svc" label={t('ui.logs.title')} bind:value={logService} required disabled={logBusy} />
      <Button type="submit" variant="secondary" busy={logBusy}>{t('ui.logs.title')}</Button>
    </form>
    {#if logs}
      <pre>{logs.content}</pre>
    {/if}
  </Card>

  <Card title={t('ui.node.agent_token')}>
    <div class="row">
      <Button variant="secondary" onclick={rotateToken} busy={tokenBusy}>{t('ui.node.rotate_token')}</Button>
    </div>
    {#if rotatedToken}
      <p class="muted">{t('ui.node.token_once')}</p>
      <TextBlock text={rotatedToken} mono />
    {/if}
    {#if isAgent}
      <p class="muted">{t('ui.nodes.agent_command_hint')}</p>
      <div class="row">
        <TextBlock text={agentCommand} mono />
        <Button variant="secondary" onclick={copyCommand}>{t('ui.nodes.copy_command')}</Button>
      </div>
    {/if}
  </Card>
</div>
