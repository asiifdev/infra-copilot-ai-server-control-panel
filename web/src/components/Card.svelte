<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** Localized heading; omit for a plain surface. */
    title?: string;
    level?: 2 | 3;
    children: Snippet;
    actions?: Snippet;
  }

  let { title, level = 2, children, actions }: Props = $props();
</script>

<section class="card">
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
    padding: var(--nv-space-4);
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
    min-width: 0;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--nv-space-3);
  }
</style>
