<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { safeText } from '../lib/text';

  interface Props {
    /** Untrusted text (server, tool or model). Always rendered as text, line breaks kept. */
    text: unknown;
    mono?: boolean;
  }

  let { text, mono = false }: Props = $props();

  const safe = $derived(safeText(text));
</script>

<div class={['block', mono && 'mono']} dir="auto">{safe.text}</div>
{#if safe.truncated}<p class="muted">{t('ui.common.truncated')}</p>{/if}

<style>
  .block {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: plaintext;
    max-width: var(--nv-measure);
  }
  .mono {
    font-family: var(--nv-font-mono);
    font-size: var(--nv-text-sm);
    max-width: none;
  }
</style>
