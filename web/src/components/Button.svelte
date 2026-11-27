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
  <a class={['nv-btn', variant === 'secondary' ? '' : `nv-btn-${variant}`, size === 'sm' && 'nv-btn-sm', block && 'nv-btn-block']} {href}>
    {#if icon}<Icon name={icon} size="sm" />{/if}
    {@render children()}
  </a>
{:else}
  <button class={['nv-btn', variant === 'secondary' ? '' : `nv-btn-${variant}`, size === 'sm' && 'nv-btn-sm', block && 'nv-btn-block']} {type} disabled={disabled || busy} aria-busy={busy} aria-describedby={describedby} {onclick}>
    {#if busy}<Spinner label={busyLabel} small />{:else if icon}<Icon name={icon} size="sm" />{/if}
    {@render children()}
  </button>
{/if}
