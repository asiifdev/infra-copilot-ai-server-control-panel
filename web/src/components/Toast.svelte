<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { toasts } from '../lib/toast.svelte';
  import { inlineText } from '../lib/text';

  const ICONS = { success: '✓', error: '✕', info: 'ℹ' } as const;
</script>

<!-- The live region stays in the page so that new notices are announced. -->
<div class="region" role="status" aria-live="polite" aria-relevant="additions text">
  {#each toasts.items as toast (toast.id)}
    <div class={['toast', toast.kind]}>
      <span aria-hidden="true">{ICONS[toast.kind]}</span>
      <span class="text">{inlineText(toast.text, 300)}</span>
      <button type="button" class="close" onclick={() => toasts.dismiss(toast.id)} aria-label={t('ui.toast.dismiss')}>
        <span aria-hidden="true">&times;</span>
      </button>
    </div>
  {/each}
</div>

<style>
  .region {
    position: fixed;
    inset-inline-end: var(--nv-space-4);
    inset-block-end: var(--nv-space-4);
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
    max-width: min(28rem, calc(100vw - 2 * var(--nv-space-4)));
    z-index: 20;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: var(--nv-space-3);
    padding: var(--nv-space-2) var(--nv-space-3);
    background: var(--nv-surface);
    color: var(--nv-text);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-inline-start-width: var(--nv-space-1);
    border-radius: var(--nv-radius);
  }
  .success {
    border-inline-start-color: var(--nv-success);
  }
  .error {
    border-inline-start-color: var(--nv-danger);
  }
  .info {
    border-inline-start-color: var(--nv-info);
  }
  .text {
    flex: 1;
    overflow-wrap: anywhere;
  }
  .close {
    min-width: var(--nv-target);
    min-height: var(--nv-target);
    border: 0;
    background: none;
    color: inherit;
    font-size: var(--nv-text-lg);
    cursor: pointer;
  }
</style>
