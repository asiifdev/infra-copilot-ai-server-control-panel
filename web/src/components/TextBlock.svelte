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

<div class={['nv-text-block', mono && 'mono']} dir="auto">{safe.text}</div>
{#if safe.truncated}<p class="muted">{t('ui.common.truncated')}</p>{/if}
