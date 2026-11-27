<script lang="ts">
  import { api } from '../lib/api';
  import { withBusy } from '../lib/errors';
  import { t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { toasts } from '../lib/toast.svelte';
  import { inlineText } from '../lib/text';
  import type { MemoryNote } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import ConfirmDialog from '../components/ConfirmDialog.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import TextBlock from '../components/TextBlock.svelte';

  // Literal keys on purpose: the key-coverage test finds them. Unknown values show as plain text.
  const KINDS: Record<string, string> = {
    org_fact: 'ui.memory.kind_org_fact',
    task: 'ui.memory.kind_task',
    preference: 'ui.memory.kind_preference',
  };
  const SOURCES: Record<string, string> = {
    person: 'ui.memory.source_person',
    assistant: 'ui.memory.source_assistant',
  };
  const kindLabel = (kind: string) => (KINDS[kind] ? t(KINDS[kind]) : inlineText(kind, 40));
  const sourceLabel = (source: string) => (SOURCES[source] ? t(SOURCES[source]) : inlineText(source, 40));

  const pending = useResource((signal) => api.memories('pending', signal));
  const active = useResource((signal) => api.memories('active', signal));

  let toDelete = $state.raw<MemoryNote | null>(null);
  let working = $state(false);

  function runMem(toastKey: string, fn: () => Promise<void>) {
    return withBusy((b) => (working = b), async () => {
      await fn();
      toasts.push('success', t(toastKey));
      pending.reload();
      active.reload();
    });
  }

  const approve = (note: MemoryNote) => runMem('ui.memory.approved', () => api.approveMemory(note.id));

  function confirmDelete() {
    const note = toDelete;
    if (!note) return;
    return runMem('ui.memory.deleted', async () => {
      await api.deleteMemory(note.id);
      toDelete = null;
    });
  }
</script>

{#snippet note(item: MemoryNote, canApprove: boolean)}
  <li class="nv-note">
    <p class="nv-chips"><span class="chip">{kindLabel(item.kind)}</span><span class="chip">{sourceLabel(item.source)}</span></p>
    <TextBlock text={item.content} />
    <div class="row">
      {#if canApprove}
        <Button variant="primary" icon="check" onclick={() => approve(item)} disabled={working}>{t('ui.memory.approve')}</Button>
      {/if}
      <Button variant="danger" onclick={() => (toDelete = item)} disabled={working}>{t('ui.common.delete')}</Button>
    </div>
  </li>
{/snippet}

<div class="stack">
  <PageHeader title={t('ui.nav.memory')} description={t('ui.memory.intro')} eyebrow={t('ui.nav.group_workspace')} />
  <p class="muted">{t('ui.memory.why')}</p>

  <Card title={t('ui.memory.pending')}>
    <AsyncView resource={pending}>
      {#snippet children(list)}
        {#if list.length === 0}
          <EmptyState icon="check" title={t('ui.memory.none_pending')} />
        {:else}
          <ul class="plain stack-sm" aria-label={t('ui.memory.pending')}>
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
          <EmptyState icon="memory" title={t('ui.memory.none_active')} />
        {:else}
          <ul class="plain stack-sm" aria-label={t('ui.memory.active')}>
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
