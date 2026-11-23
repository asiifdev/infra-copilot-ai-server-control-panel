<script lang="ts">
  import type { DisplayMessage } from '../lib/chat.svelte';
  import { t } from '../lib/i18n.svelte';
  import { inlineText } from '../lib/text';
  import ErrorNotice from './ErrorNotice.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import TextBlock from './TextBlock.svelte';

  interface Props {
    message: DisplayMessage;
    /** True while this message is still being written. */
    writing?: boolean;
  }

  let { message, writing = false }: Props = $props();
</script>

<li class={['message', message.role]}>
  <p class="who">{message.role === 'user' ? t('ui.assistant.you') : t('ui.assistant.assistant')}</p>
  {#if message.tools.length > 0}
    <ul class="chips" aria-label={t('ui.assistant.tools')}>
      {#each message.tools as tool, index (index)}
        <li class="chip">
          <span class="tool-name">{inlineText(tool.name, 60)}</span>
          <StatusBadge kind="tier" value={tool.tier} />
          <StatusBadge kind="state" value={tool.status} />
          {#if tool.durationMs !== undefined}<span class="muted">{t('ui.assistant.duration', { ms: tool.durationMs })}</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if message.text}
    <TextBlock text={message.text} />
  {:else if writing && !message.error}
    <p class="muted">{t('ui.assistant.thinking')}</p>
  {/if}
  {#if message.stopped}<p class="muted">{t('ui.assistant.stopped')}</p>{/if}
  {#if message.error}
    <ErrorNotice code={message.error.code} detail={message.error.detail} />
  {/if}
</li>

<style>
  .message {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
    padding: var(--nv-space-3);
    border: var(--nv-border-width) solid var(--nv-border);
    border-radius: var(--nv-radius);
    background: var(--nv-surface);
    min-width: 0;
  }
  .user {
    background: var(--nv-surface-2);
  }
  .who {
    font-weight: 700;
    font-size: var(--nv-text-sm);
  }
  .chips {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--nv-space-2);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--nv-space-2);
    padding: var(--nv-space-1) var(--nv-space-2);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius);
    font-size: var(--nv-text-sm);
  }
  .tool-name {
    font-family: var(--nv-font-mono);
    overflow-wrap: anywhere;
  }
</style>
