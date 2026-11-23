<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    message?: string;
    children?: Snippet;
  }

  let { title, message, children }: Props = $props();
</script>

<div class="empty">
  <p class="title">{title}</p>
  {#if message}<p class="muted">{message}</p>{/if}
  {@render children?.()}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--nv-space-2);
    padding: var(--nv-space-5);
    border: var(--nv-border-width) dashed var(--nv-border-strong);
    border-radius: var(--nv-radius);
    background: var(--nv-surface);
  }
  .title {
    font-weight: 600;
  }
</style>
