<script lang="ts">
  import type { DisplayMessage } from '../lib/chat.svelte';
  import { t } from '../lib/i18n.svelte';
  import { inlineText } from '../lib/text';
  import ErrorNotice from './ErrorNotice.svelte';
  import Icon from './Icon.svelte';
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
  <p class="who">
    {#if message.role === 'assistant'}<span class="avatar"><Icon name="assistant" size="sm" /></span>{/if}
    {message.role === 'user' ? t('ui.assistant.you') : t('ui.assistant.assistant')}
  </p>
  <div class="bubble">
    {#if message.tools.length > 0}
      <ul class="chips plain" aria-label={t('ui.assistant.tools')}>
        {#each message.tools as tool, index (index)}
          <li class="chip">
            <Icon name="settings" size="sm" />
            <span class="tool-name">{inlineText(tool.name, 60)}</span>
            {#if tool.tier !== undefined}<StatusBadge kind="tier" value={tool.tier} />{/if}
            {#if tool.status}<StatusBadge kind="state" value={tool.status} />{/if}
            {#if tool.durationMs !== undefined}<span class="muted num">{t('ui.assistant.duration', { ms: tool.durationMs })}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if message.text}
      <TextBlock text={message.text} />
      {#if writing && !message.error}<span class="caret" aria-hidden="true"></span>{/if}
    {:else if writing && !message.error}
      <p class="muted thinking">{t('ui.assistant.thinking')}<span class="caret" aria-hidden="true"></span></p>
    {/if}
    {#if message.truncated}<p class="muted">{t('ui.assistant.truncated')}</p>{/if}
    {#if message.stopped}<p class="muted">{t('ui.assistant.stopped')}</p>{/if}
    {#if message.error}
      <ErrorNotice code={message.error.code} detail={message.error.detail} />
    {/if}
  </div>
</li>

<style>
  .message {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-1);
    min-width: 0;
  }
  .user {
    align-items: flex-end;
  }
  .who {
    display: flex;
    align-items: center;
    gap: var(--nv-space-2);
    padding-inline: var(--nv-space-1);
    font-weight: 700;
    font-size: var(--nv-text-caption);
    color: var(--nv-text-muted);
  }
  .avatar {
    display: inline-grid;
    place-items: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--nv-radius-pill);
    background: var(--nv-tint-primary);
    color: var(--nv-primary);
  }
  .bubble {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
    min-width: 0;
    max-width: min(100%, 46rem);
    padding: var(--nv-space-3) var(--nv-space-4);
    border: var(--nv-border-width) solid var(--nv-border);
    border-radius: var(--nv-radius-lg);
    background: var(--nv-surface);
    box-shadow: var(--nv-shadow-1);
  }
  .user .bubble {
    background: var(--nv-tint-primary);
    border-color: color-mix(in srgb, var(--nv-primary) 35%, var(--nv-border));
    border-end-end-radius: var(--nv-radius-xs);
  }
  .assistant .bubble {
    border-start-start-radius: var(--nv-radius-xs);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nv-space-2);
  }
  .chip {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--nv-space-2);
    padding: var(--nv-space-1) var(--nv-space-3);
    border: var(--nv-border-width) solid var(--nv-border);
    border-radius: var(--nv-radius-pill);
    background: var(--nv-surface-2);
    font-size: var(--nv-text-sm);
  }
  .tool-name {
    font-family: var(--nv-font-mono);
    font-size: var(--nv-text-caption);
    overflow-wrap: anywhere;
  }
  .caret {
    display: inline-block;
    width: 0.5rem;
    height: 1.1em;
    margin-inline-start: var(--nv-space-1);
    vertical-align: text-bottom;
    border-radius: 1px;
    background: var(--nv-primary);
    animation: blink 1s steps(2, start) infinite;
  }
  @keyframes blink {
    to {
      visibility: hidden;
    }
  }
</style>
