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

<li class={['nv-msg', message.role]}>
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
