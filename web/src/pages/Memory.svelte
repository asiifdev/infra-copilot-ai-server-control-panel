<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { errorKey } from '../lib/errors';
  import { t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { toasts } from '../lib/toast.svelte';
  import type { MemoryNote } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import ConfirmDialog from '../components/ConfirmDialog.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import TextBlock from '../components/TextBlock.svelte';

  const pending = useResource((signal) => api.memories('pending', signal));
  const active = useResource((signal) => api.memories('active', signal));

  let toDelete = $state.raw<MemoryNote | null>(null);
  let working = $state(false);

  function reloadAll() {
    pending.reload();
    active.reload();
  }

  function fail(cause: unknown) {
    toasts.push('error', t(errorKey(cause instanceof ApiError ? cause.code : 'internal')));
  }

  async function approve(note: MemoryNote) {
    working = true;
    try {
      await api.approveMemory(note.id);
      toasts.push('success', t('ui.memory.approved'));
      reloadAll();
    } catch (cause) {
      fail(cause);
    } finally {
      working = false;
    }
  }

  async function confirmDelete() {
    const note = toDelete;
    if (!note) return;
    working = true;
    try {
      await api.deleteMemory(note.id);
      toasts.push('success', t('ui.memory.deleted'));
      toDelete = null;
      reloadAll();
    } catch (cause) {
      fail(cause);
    } finally {
      working = false;
    }
  }
</script>

{#snippet note(item: MemoryNote, canApprove: boolean)}
  <li class="note">
    <TextBlock text={item.content} />
    <div class="row">
      {#if canApprove}
        <Button variant="primary" onclick={() => approve(item)} disabled={working}>{t('ui.memory.approve')}</Button>
      {/if}
      <Button variant="danger" onclick={() => (toDelete = item)} disabled={working}>{t('ui.common.delete')}</Button>
    </div>
  </li>
{/snippet}

<div class="stack">
  <PageHeader title={t('ui.nav.memory')} description={t('ui.memory.intro')} />
  <p class="muted">{t('ui.memory.why')}</p>

  <Card title={t('ui.memory.pending')}>
    <AsyncView resource={pending}>
      {#snippet children(list)}
        {#if list.length === 0}
          <EmptyState title={t('ui.memory.none_pending')} />
        {:else}
          <ul class="notes" aria-label={t('ui.memory.pending')}>
            {#each list as item (item.id)}{@render note(item, true)}{/each}
          </ul>
        {/if}
      {/snippet}
    </AsyncView>
  </Card>

  <Card title={t('ui.memory.active')}>
    <AsyncView resource={active}>
      {#snippet children(list)}
        {#if list.length === 0}
          <EmptyState title={t('ui.memory.none_active')} />
        {:else}
          <ul class="notes" aria-label={t('ui.memory.active')}>
            {#each list as item (item.id)}{@render note(item, false)}{/each}
          </ul>
        {/if}
      {/snippet}
    </AsyncView>
  </Card>
</div>

{#if toDelete}
  <ConfirmDialog
    title={t('ui.memory.delete_title')}
    message={t('ui.memory.delete_message')}
    confirmLabel={t('ui.common.delete')}
    cancelLabel={t('ui.common.cancel')}
    busy={working}
    onconfirm={confirmDelete}
    oncancel={() => (toDelete = null)}
  />
{/if}

<style>
  .notes {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
  }
  .note {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
    padding: var(--nv-space-3);
    border: var(--nv-border-width) solid var(--nv-border);
    border-radius: var(--nv-radius);
  }
</style>
