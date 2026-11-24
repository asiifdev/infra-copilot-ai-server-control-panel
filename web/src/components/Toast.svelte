<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { toasts } from '../lib/toast.svelte';
  import { inlineText } from '../lib/text';
  import Icon, { type IconName } from './Icon.svelte';

  const ICONS: Record<'success' | 'error' | 'info', IconName> = { success: 'check', error: 'cross', info: 'info' };
</script>

<!-- The live region stays in the page so that new notices are announced. -->
<div class="region" role="status" aria-live="polite" aria-relevant="additions text">
  {#each toasts.items as toast (toast.id)}
    <div class={['toast', toast.kind]}>
      <span class="mark"><Icon name={ICONS[toast.kind]} /></span>
      <span class="text">{inlineText(toast.text, 300)}</span>
      <button type="button" class="close" onclick={() => toasts.dismiss(toast.id)} aria-label={t('ui.toast.dismiss')}>
        <Icon name="close" size="sm" />
      </button>
    </div>
  {/each}
</div>

<style>
  .region {
    position: fixed;
    inset-inline: var(--nv-space-3);
    inset-block-start: var(--nv-space-3);
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
    z-index: 40;
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: var(--nv-space-3);
    padding: var(--nv-space-2) var(--nv-space-2) var(--nv-space-2) var(--nv-space-4);
    background: var(--nv-surface);
    color: var(--nv-text);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-inline-start-width: var(--nv-space-1);
    border-radius: var(--nv-radius);
    box-shadow: var(--nv-shadow-2);
    animation: drop-in var(--nv-motion-slow) var(--nv-ease);
  }
  .success {
    border-inline-start-color: var(--nv-success);
  }
  .success .mark {
    color: var(--nv-success);
  }
  .error {
    border-inline-start-color: var(--nv-danger);
  }
  .error .mark {
    color: var(--nv-danger);
  }
  .info {
    border-inline-start-color: var(--nv-info);
  }
  .info .mark {
    color: var(--nv-info);
  }
  .mark {
    display: inline-flex;
  }
  .text {
    flex: 1;
    overflow-wrap: anywhere;
  }
  .close {
    display: inline-grid;
    place-items: center;
    min-width: var(--nv-target);
    min-height: var(--nv-target);
    border: 0;
    border-radius: var(--nv-radius-sm);
    background: none;
    color: var(--nv-text-muted);
    cursor: pointer;
  }
  .close:hover {
    background: var(--nv-hover);
    color: var(--nv-text);
  }
  @keyframes drop-in {
    from {
      opacity: 0;
      transform: translateY(-0.75rem);
    }
  }
  @keyframes slide-in {
    from {
      opacity: 0;
      transform: translateX(1rem);
    }
  }
  @media (min-width: 52rem) {
    .region {
      inset-inline: auto var(--nv-space-5);
      inset-block: auto var(--nv-space-5);
      width: min(28rem, calc(100vw - 2 * var(--nv-space-5)));
    }
    .toast {
      animation-name: slide-in;
    }
  }
</style>
