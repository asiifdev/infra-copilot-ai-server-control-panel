<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** Localized heading; omit for a plain surface. */
    title?: string;
    level?: 2 | 3;
    /** Lifts the card slightly on hover: for cards that are as a whole a target or a summary. */
    hover?: boolean;
    /** A coloured edge that tells the state at a glance (never the only signal). */
    tone?: 'success' | 'warning' | 'danger' | 'info';
    children: Snippet;
    actions?: Snippet;
  }

  let { title, level = 2, hover = false, tone, children, actions }: Props = $props();
</script>

<section class={['card', hover && 'hover', tone && `tone-${tone}`]}>
  {#if title || actions}
    <header class="head">
      {#if title}
        <svelte:element this={level === 2 ? 'h2' : 'h3'}>{title}</svelte:element>
      {/if}
      {#if actions}<div class="row">{@render actions()}</div>{/if}
    </header>
  {/if}
  {@render children()}
</section>

<style>
  .card {
    background: var(--nv-surface);
    border: var(--nv-border-width) solid var(--nv-border);
    border-radius: var(--nv-radius);
    box-shadow: var(--nv-shadow-1);
    padding: var(--nv-space-4);
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
    min-width: 0;
    transition:
      box-shadow var(--nv-motion-slow) var(--nv-ease),
      transform var(--nv-motion-slow) var(--nv-ease),
      border-color var(--nv-motion-slow) var(--nv-ease);
  }
  .hover:hover {
    box-shadow: var(--nv-shadow-2);
    transform: translateY(-1px);
    border-color: var(--nv-border-strong);
  }
  .tone-success {
    --tone: var(--nv-success);
  }
  .tone-warning {
    --tone: var(--nv-warning);
  }
  .tone-danger {
    --tone: var(--nv-danger);
  }
  .tone-info {
    --tone: var(--nv-info);
  }
  [class*='tone-'] {
    border-inline-start: var(--nv-space-1) solid var(--tone);
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--nv-space-3);
  }
  @media (min-width: 40rem) {
    .card {
      padding: var(--nv-space-5);
    }
  }
</style>
