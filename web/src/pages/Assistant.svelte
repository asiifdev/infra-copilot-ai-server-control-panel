<script lang="ts">
  import type { Attachment } from 'svelte/attachments';
  import { api, ApiError } from '../lib/api';
  import { ChatSession } from '../lib/chat.svelte';
  import { errorKey } from '../lib/errors';
  import { t } from '../lib/i18n.svelte';
  import { hrefFor, router } from '../lib/router.svelte';
  import { Resource, useResource } from '../lib/resource.svelte';
  import { toasts } from '../lib/toast.svelte';
  import { inlineText } from '../lib/text';
  import type { ChatMessage as ChatMessageRow } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import ChatMessage from '../components/ChatMessage.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import TextField from '../components/TextField.svelte';

  interface Props {
    conversation: string | null;
  }

  let { conversation }: Props = $props();

  const conversations = useResource((signal) => api.conversations(signal));
  const history = new Resource<ChatMessageRow[]>();
  const chat = new ChatSession();

  let draft = $state('');
  let creating = $state(false);

  // Keeps the newest text in view while the answer streams in. An attachment runs again whenever
  // the state it reads (the last message and its text) changes.
  const followNewest: Attachment<HTMLOListElement> = (list) => {
    const last = chat.messages[chat.messages.length - 1];
    void last?.text;
    list.lastElementChild?.scrollIntoView?.({ block: 'nearest' });
  };

  // Load the stored messages of the open conversation; leaving it stops a turn in flight.
  $effect(() => {
    const id = conversation;
    chat.reset();
    if (id === null) return;
    history.load(async (signal) => {
      const rows = await api.messages(id, signal);
      if (!chat.streaming) chat.reset(rows);
      return rows;
    });
    return () => {
      history.abort();
      chat.stop();
    };
  });

  async function newConversation() {
    creating = true;
    try {
      const created = await api.createConversation();
      conversations.reload();
      router.go({ name: 'assistant', conversation: String(created.id) });
    } catch (cause) {
      toasts.push('error', t(errorKey(cause instanceof ApiError ? cause.code : 'internal')));
    } finally {
      creating = false;
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const text = draft.trim();
    if (conversation === null || text === '' || chat.streaming) return;
    draft = '';
    await chat.send(conversation, text);
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      (event.currentTarget as HTMLTextAreaElement).form?.requestSubmit();
    }
  }

  // What a screen reader hears about the answer: once when it starts, once when it is over.
  const announcement = $derived.by(() => {
    if (chat.streaming) return t('ui.assistant.thinking');
    if (!chat.live) return ''; // a stored conversation that was just opened is not news
    const last = chat.messages[chat.messages.length - 1];
    if (!last || last.role !== 'assistant') return '';
    if (last.error) return t(errorKey(last.error.code));
    if (last.stopped) return t('ui.assistant.stopped');
    if (last.truncated) return t('ui.assistant.truncated');
    return last.text ? t('ui.assistant.reply_ready') : '';
  });

  const historyFailed = $derived(history.status === 'error' ? history.error : null);
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.assistant')} description={t('ui.assistant.intro')}>
    <Button variant="primary" onclick={newConversation} busy={creating} busyLabel={t('ui.common.loading')}>
      {t('ui.assistant.new')}
    </Button>
  </PageHeader>

  <div class="layout">
    <nav class="list" aria-label={t('ui.assistant.conversations')}>
      <AsyncView resource={conversations}>
        {#snippet children(list)}
          {#if list.length === 0}
            <p class="muted">{t('ui.assistant.no_conversations')}</p>
          {:else}
            <ul>
              {#each list as item (item.id)}
                <li>
                  <a
                    href={hrefFor({ name: 'assistant', conversation: String(item.id) })}
                    aria-current={String(item.id) === conversation ? 'page' : undefined}
                  >
                    {inlineText(item.title, 80) || t('ui.assistant.untitled')}
                  </a>
                </li>
              {/each}
            </ul>
          {/if}
        {/snippet}
      </AsyncView>
    </nav>

    <section class="chat" aria-label={t('ui.assistant.chat')}>
      {#if conversation === null}
        <EmptyState title={t('ui.assistant.pick')} message={t('ui.assistant.pick_hint')} />
      {:else}
        {#if historyFailed}
          <p class="muted">{t(errorKey(historyFailed.code))}</p>
        {/if}
        <!-- Not a live region: announcing every streamed word would drown a screen reader. The
             status line below speaks only when a reply starts and when it ends. -->
        <ol class="messages" aria-label={t('ui.assistant.messages')} aria-busy={chat.streaming} {@attach followNewest}>
          {#each chat.messages as message, index (index)}
            <ChatMessage {message} writing={chat.streaming && index === chat.messages.length - 1} />
          {/each}
        </ol>
        <p class="visually-hidden" role="status" aria-live="polite">{announcement}</p>
        {#if chat.messages.length === 0 && history.status === 'ready'}
          <p class="muted">{t('ui.assistant.empty')}</p>
        {/if}
        <form class="composer" onsubmit={submit}>
          <TextField id="composer" label={t('ui.assistant.message')} bind:value={draft} multiline {onkeydown} />
          <p class="muted hint">{t('ui.assistant.send_hint')}</p>
          <div class="row">
            <Button type="submit" variant="primary" disabled={draft.trim() === '' || chat.streaming || history.status !== 'ready'}>
              {t('ui.assistant.send')}
            </Button>
            {#if chat.streaming}
              <Button onclick={() => chat.stop()}>{t('ui.assistant.stop')}</Button>
            {/if}
          </div>
        </form>
      {/if}
    </section>
  </div>
</div>

<style>
  .layout {
    display: grid;
    gap: var(--nv-space-4);
    grid-template-columns: minmax(0, 1fr);
  }
  @media (min-width: 52rem) {
    .layout {
      grid-template-columns: 16rem minmax(0, 1fr);
    }
  }
  .list ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-1);
  }
  .list a {
    display: block;
    padding: var(--nv-space-2) var(--nv-space-3);
    min-height: var(--nv-target);
    border-radius: var(--nv-radius);
    overflow-wrap: anywhere;
  }
  .list a[aria-current='page'] {
    background: var(--nv-surface-2);
    font-weight: 700;
  }
  .chat {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
    min-width: 0;
  }
  .messages {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
  }
  .composer {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
  }
  .hint {
    font-size: var(--nv-text-sm);
  }
</style>
