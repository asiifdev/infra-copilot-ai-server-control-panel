<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';

  interface Props {
    /** Localized name of the figure. */
    label: string;
    /** The big number or word. */
    value: string | number;
    icon: IconName;
    /** Colours the icon and the edge when the figure needs attention (the text says it too). */
    tone?: 'success' | 'warning' | 'danger' | 'info';
    /** Where the tile leads, and the localized text of that link. */
    href: string;
    linkLabel: string;
    /** Details under the figure: badges, a short sentence. */
    children?: Snippet;
  }

  let { label, value, icon, tone, href, linkLabel, children }: Props = $props();
</script>

<section class={['tile', tone && `tone-${tone}`]}>
  <header class="head">
    <span class="ico"><Icon name={icon} /></span>
    <h2 class="label">{label}</h2>
  </header>
  <p class="value num">{value}</p>
  {#if children}<div class="sub">{@render children()}</div>{/if}
  <a class="more" href={href}>{linkLabel}<Icon name="arrow" size="sm" /></a>
</section>

<style>
  .tile {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
    min-width: 0;
    padding: var(--nv-space-4);
    background: var(--nv-surface);
    border: var(--nv-border-width) solid var(--nv-border);
    border-radius: var(--nv-radius);
    box-shadow: var(--nv-shadow-1);
    transition:
      box-shadow var(--nv-motion-slow) var(--nv-ease),
      transform var(--nv-motion-slow) var(--nv-ease);
  }
  .tile:hover {
    box-shadow: var(--nv-shadow-2);
    transform: translateY(-1px);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--nv-space-2);
  }
  .ico {
    display: inline-grid;
    place-items: center;
    width: 2.25rem;
    height: 2.25rem;
    border-radius: var(--nv-radius-sm);
    background: var(--nv-tint-primary);
    color: var(--nv-primary);
  }
  .label {
    font-size: var(--nv-text-sm);
    font-weight: 650;
    color: var(--nv-text-muted);
  }
  .value {
    font-size: var(--nv-text-display);
    font-weight: 800;
    line-height: 1.1;
    letter-spacing: -0.03em;
  }
  .sub {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--nv-space-2);
    color: var(--nv-text-muted);
    font-size: var(--nv-text-sm);
    min-height: 1.75rem;
  }
  .more {
    display: inline-flex;
    align-items: center;
    gap: var(--nv-space-1);
    min-height: var(--nv-target);
    margin-block-start: auto;
    font-weight: 600;
    font-size: var(--nv-text-sm);
  }
  .tone-warning {
    border-inline-start: var(--nv-space-1) solid var(--nv-warning);
  }
  .tone-warning .ico {
    background: var(--nv-tint-warning);
    color: var(--nv-warning);
  }
  .tone-danger {
    border-inline-start: var(--nv-space-1) solid var(--nv-danger);
  }
  .tone-danger .ico {
    background: var(--nv-tint-danger);
    color: var(--nv-danger);
  }
  .tone-success .ico {
    background: var(--nv-tint-success);
    color: var(--nv-success);
  }
  .tone-info .ico {
    background: var(--nv-tint-info);
    color: var(--nv-info);
  }
</style>
