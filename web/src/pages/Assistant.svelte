<script lang="ts">
  import type { Attachment } from 'svelte/attachments';
  import { api } from '../lib/api';
  import { ChatSession } from '../lib/chat.svelte';
  import { errorKey } from '../lib/errors';
  import { t } from '../lib/i18n.svelte';
  import { conversationEvents, startConversation } from '../lib/conversations.svelte';
  import { hrefFor } from '../lib/router.svelte';
  import { Resource, useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import type { ChatMessage as ChatMessageRow } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import ChatMessage from '../components/ChatMessage.svelte';
  import Icon from '../components/Icon.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import TextField from '../components/TextField.svelte';

  interface Props {
    conversation: string | null;
  }

  let { conversation }: Props = $props();

  // Reads the revision, so the list reloads when a conversation is started from elsewhere
  // (the command palette).
  const conversations = useResource((signal) => {
    void conversationEvents.revision;
    return api.conversations(signal);
  });
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
    await startConversation();
    creating = false;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const text = draft.trim();
    if (conversation === null || text === '' || chat.streaming) return;
    draft = '';
    await chat.send(conversation, text);
  }

  function onkeydown(event: KeyboardEvent) {
    // Enter sends, Shift+Enter makes a new line; Ctrl or Cmd+Enter sends too. A key press that
    // only confirms an input-method composition (Enter in some languages) must not send.
    if (event.key === 'Enter' && !event.isComposing && (!event.shiftKey || event.ctrlKey || event.metaKey)) {
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

<div class="stack page nv-chat">
  <PageHeader title={t('ui.nav.assistant')} description={t('ui.assistant.intro')} eyebrow={t('ui.nav.group_work')}>
    <Button variant="primary" icon="plus" onclick={newConversation} busy={creating} busyLabel={t('ui.common.loading')}>
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
            <ul class="plain">
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
        <EmptyState icon="assistant" title={t('ui.assistant.pick')} message={t('ui.assistant.pick_hint')} />
      {:else}
        {#if historyFailed}
          <p class="muted">{t(errorKey(historyFailed.code))}</p>
        {/if}
        <!-- Not a live region: announcing every streamed word would drown a screen reader. The
             status line below speaks only when a reply starts and when it ends. -->
        <ol class="messages plain" aria-label={t('ui.assistant.messages')} aria-busy={chat.streaming} {@attach followNewest}>
          {#each chat.messages as message, index (index)}
            <ChatMessage {message} writing={chat.streaming && index === chat.messages.length - 1} />
          {/each}
        </ol>
        <p class="visually-hidden" role="status" aria-live="polite">{announcement}</p>
        {#if chat.messages.length === 0 && history.status === 'ready'}
          <EmptyState icon="assistant" title={t('ui.assistant.empty')} />
        {/if}
        <form class="composer" onsubmit={submit}>
          <div class="box">
            <TextField id="composer" label={t('ui.assistant.message')} hideLabel placeholder={t('ui.assistant.message')} bind:value={draft} multiline rows={2} {onkeydown} />
            <div class="send row">
              {#if chat.streaming}
                <Button icon="stop" onclick={() => chat.stop()}>{t('ui.assistant.stop')}</Button>
              {/if}
              <Button type="submit" variant="primary" icon="send" disabled={draft.trim() === '' || chat.streaming || history.status !== 'ready'}>
                {t('ui.assistant.send')}
              </Button>
            </div>
          </div>
          <p class="muted hint"><Icon name="return" size="sm" />{t('ui.assistant.send_hint')}</p>
        </form>
      {/if}
    </section>
  </div>
</div>
