<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    description?: string;
    children?: Snippet;
  }

  let { title, description, children }: Props = $props();
</script>

<header class="page-head">
  <div class="stack titles">
    <h1 id="page-title" tabindex="-1">{title}</h1>
    {#if description}<p class="muted">{description}</p>{/if}
  </div>
  {#if children}<div class="row">{@render children()}</div>{/if}
</header>

<style>
  .page-head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--nv-space-3);
  }
  .titles {
    gap: var(--nv-space-1);
  }
</style>
