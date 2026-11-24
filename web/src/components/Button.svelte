<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';
  import Spinner from './Spinner.svelte';

  interface Props {
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
    size?: 'md' | 'sm';
    type?: 'button' | 'submit';
    disabled?: boolean;
    busy?: boolean;
    /** Text for assistive technology while busy (already localized). */
    busyLabel?: string;
    /** Optional leading icon (decorative). */
    icon?: IconName;
    /** Stretches the button to the width of its container. */
    block?: boolean;
    /** Renders a link that looks like a button (for navigation); the other state props do not apply. */
    href?: string;
    /** Id of the element that explains the button (for example why it is disabled). */
    describedby?: string;
    onclick?: (event: MouseEvent) => void;
    children: Snippet;
  }

  let {
    variant = 'secondary',
    size = 'md',
    type = 'button',
    disabled = false,
    busy = false,
    busyLabel = '',
    icon,
    block = false,
    href,
    describedby,
    onclick,
    children,
  }: Props = $props();
</script>

{#if href}
  <a class={['btn', variant, size, block && 'block']} {href}>
    {#if icon}<Icon name={icon} size="sm" />{/if}
    {@render children()}
  </a>
{:else}
  <button class={['btn', variant, size, block && 'block']} {type} disabled={disabled || busy} aria-busy={busy} aria-describedby={describedby} {onclick}>
    {#if busy}<Spinner label={busyLabel} small />{:else if icon}<Icon name={icon} size="sm" />{/if}
    {@render children()}
  </button>
{/if}

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
    border-radius: var(--nv-radius-sm);
    background: var(--nv-surface);
    color: var(--nv-text);
    font: inherit;
    font-weight: 600;
    line-height: 1.2;
    cursor: pointer;
    text-decoration: none;
    box-shadow: var(--nv-shadow-1);
    transition:
      background var(--nv-motion) var(--nv-ease),
      border-color var(--nv-motion) var(--nv-ease),
      box-shadow var(--nv-motion) var(--nv-ease),
      transform var(--nv-motion) var(--nv-ease);
  }
  .btn:hover:not(:disabled) {
    background: var(--nv-surface-2);
    text-decoration: none;
  }
  .btn:active:not(:disabled) {
    transform: translateY(1px);
    box-shadow: none;
  }
  .sm {
    min-height: 2.25rem;
    padding: var(--nv-space-1) var(--nv-space-3);
    font-size: var(--nv-text-sm);
  }
  .block {
    width: 100%;
  }
  .primary {
    background: var(--nv-primary);
    color: var(--nv-on-primary);
    border-color: var(--nv-primary);
  }
  .primary:hover:not(:disabled) {
    background: var(--nv-primary-hover);
    border-color: var(--nv-primary-hover);
  }
  .ghost {
    background: transparent;
    border-color: transparent;
    box-shadow: none;
  }
  .ghost:hover:not(:disabled) {
    background: var(--nv-hover);
  }
  .danger {
    color: var(--nv-danger);
    border-color: var(--nv-danger);
  }
  .danger:hover:not(:disabled) {
    background: var(--nv-tint-danger);
  }
  .btn:disabled {
    cursor: not-allowed;
    opacity: 0.6;
    box-shadow: none;
  }
</style>
