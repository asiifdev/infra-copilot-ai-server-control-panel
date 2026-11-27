<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { toasts } from '../lib/toast.svelte';
  import { inlineText } from '../lib/text';
  import Icon, { type IconName } from './Icon.svelte';

  const ICONS: Record<'success' | 'error' | 'info', IconName> = { success: 'check', error: 'cross', info: 'info' };
</script>

<!-- The live region stays in the page so that new notices are announced. -->
<div class="nv-toast-region" role="status" aria-live="polite" aria-relevant="additions text">
  {#each toasts.items as toast (toast.id)}
    <div class={['nv-toast', toast.kind]}>
      <span class="mark"><Icon name={ICONS[toast.kind]} /></span>
      <span class="text">{inlineText(toast.text, 300)}</span>
      <button type="button" class="close" onclick={() => toasts.dismiss(toast.id)} aria-label={t('ui.toast.dismiss')}>
        <Icon name="close" size="sm" />
      </button>
    </div>
  {/each}
</div>
