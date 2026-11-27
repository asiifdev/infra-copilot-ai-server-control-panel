<script lang="ts">
  import { api } from '../lib/api';
  import { toastError } from '../lib/errors';
  import { dateText, formatBytes } from '../lib/format';
  import { i18n, t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import { toasts } from '../lib/toast.svelte';
  import { readCheck } from '../lib/transport';
  import type { TransportSummary } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import ConfirmDialog from '../components/ConfirmDialog.svelte';
  import DataTable, { type Column } from '../components/DataTable.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TextField from '../components/TextField.svelte';

  const transports = useResource((signal) => api.transports(signal));

  const columns = $derived<Column<TransportSummary>[]>([
    { id: 'name', header: t('ui.transports.name'), sortValue: (x) => x.name },
    { id: 'kind', header: t('ui.transports.kind'), sortValue: (x) => x.kind },
    { id: 'verified', header: t('ui.transports.verified'), sortValue: (x) => x.verified_at ?? '' },
    { id: 'check', header: t('ui.transports.check'), sortValue: (x) => checkRank(x) },
    { id: 'actions', header: t('ui.transports.update_credentials') },
  ]);

  let editing = $state.raw<TransportSummary | null>(null);
  let credsJson = $state('');
  let credsError = $state<string | undefined>(undefined);
  let saving = $state(false);

  function openCredentials(item: TransportSummary) {
    editing = item;
    credsJson = '';
    credsError = undefined;
  }

  function closeCredentials() {
    editing = null;
    credsJson = '';
    credsError = undefined;
  }

  function parseCredentials(raw: string): Record<string, string> | null {
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return null;
      const out: Record<string, string> = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof v !== 'string' || k.trim() === '') return null;
        out[k] = v;
      }
      return Object.keys(out).length > 0 ? out : null;
    } catch {
      return null;
    }
  }

  async function submitCredentials() {
    if (!editing || saving) return;
    const parsed = parseCredentials(credsJson);
    if (!parsed) {
      credsError = t('ui.error.invalid_input');
      return;
    }
    const target = editing;
    credsJson = '';
    credsError = undefined;
    saving = true;
    try {
      await api.updateTransportCredentials(target.id, parsed);
      toasts.push('success', t('ui.transports.credentials_updated_toast'));
      editing = null;
      transports.reload();
    } catch (cause) {
      toastError(cause);
    } finally {
      saving = false;
    }
  }

  // Never checked sorts first, then failed, then passed.
  function checkRank(item: TransportSummary): number {
    const check = readCheck(item.last_check);
    return check === null ? 0 : check.ok === true ? 3 : check.ok === false ? 2 : 1;
  }

  function when(value: string | null): string {
    return value ? dateText(value, i18n.locale) : t('ui.transports.never');
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
            {:else if column.id === 'check'}
              {@render check(item)}
            {:else}
              <Button variant="secondary" onclick={() => openCredentials(item)}>{t('ui.transports.update_credentials')}</Button>
            {/if}
          {/snippet}
        </DataTable>
      {/if}
      <p class="muted">{t('ui.transports.no_config')}</p>
    {/snippet}
  </AsyncView>
</div>

{#if editing}
  <ConfirmDialog
    role="dialog"
    title={t('ui.transports.update_credentials')}
    message={inlineText(editing.name, 80)}
    confirmLabel={t('ui.transports.update_credentials')}
    cancelLabel={t('ui.common.cancel')}
    tone="primary"
    busy={saving}
    onconfirm={submitCredentials}
    oncancel={closeCredentials}
  >
    <TextField
      id="transport-credentials-json"
      multiline
      label={t('ui.transports.credentials_json_label')}
      bind:value={credsJson}
      error={credsError}
      disabled={saving}
    />
  </ConfirmDialog>
{/if}

<style>
  .detail {
    display: block;
    font-size: var(--nv-text-sm);
  }
</style>
