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

<header class="nv-page-head">
  <div class="nv-page-titles">
    {#if eyebrow}<p class="eyebrow">{eyebrow}</p>{/if}
    <h1 id="page-title" tabindex="-1">{title}</h1>
    {#if description}<p class="muted nv-page-lede">{description}</p>{/if}
  </div>
  {#if children}<div class="row nv-page-actions">{@render children()}</div>{/if}
</header>
