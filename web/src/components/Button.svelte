<script lang="ts">
  import type { Snippet } from 'svelte';
  import Spinner from './Spinner.svelte';

  interface Props {
    variant?: 'primary' | 'secondary' | 'danger';
    type?: 'button' | 'submit';
    disabled?: boolean;
    busy?: boolean;
    /** Text for assistive technology while busy (already localized). */
    busyLabel?: string;
    onclick?: (event: MouseEvent) => void;
    children: Snippet;
  }

  let {
    variant = 'secondary',
    type = 'button',
    disabled = false,
    busy = false,
    busyLabel = '',
    onclick,
    children,
  }: Props = $props();
</script>

<button class={['btn', variant]} {type} disabled={disabled || busy} aria-busy={busy} {onclick}>
  {#if busy}<Spinner label={busyLabel} small />{/if}
  {@render children()}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--nv-space-2);
    min-height: var(--nv-target);
    min-width: var(--nv-target);
    padding: var(--nv-space-2) var(--nv-space-4);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius);
    background: var(--nv-surface);
    color: var(--nv-text);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    transition: background var(--nv-motion);
  }
  .btn:hover:not(:disabled) {
    background: var(--nv-surface-2);
  }
  .primary {
    background: var(--nv-primary);
    color: var(--nv-on-primary);
    border-color: var(--nv-primary);
  }
  .primary:hover:not(:disabled) {
    background: var(--nv-primary-hover);
  }
  .danger {
    color: var(--nv-danger);
    border-color: var(--nv-danger);
  }
  .btn:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }
</style>
