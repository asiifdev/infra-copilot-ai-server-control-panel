<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    description?: string;
    /** A small label above the title, such as the section the page belongs to. */
    eyebrow?: string;
    children?: Snippet;
  }

  let { title, description, eyebrow, children }: Props = $props();
</script>

<header class="page-head">
  <div class="titles">
    {#if eyebrow}<p class="eyebrow">{eyebrow}</p>{/if}
    <h1 id="page-title" tabindex="-1">{title}</h1>
    {#if description}<p class="muted lede">{description}</p>{/if}
  </div>
  {#if children}<div class="row actions">{@render children()}</div>{/if}
</header>

<style>
  .page-head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--nv-space-3) var(--nv-space-5);
    padding-block-end: var(--nv-space-2);
  }
  .titles {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-1);
    min-width: 0;
  }
  .lede {
    font-size: var(--nv-text-md);
  }
  .actions {
    flex: none;
  }
</style>
