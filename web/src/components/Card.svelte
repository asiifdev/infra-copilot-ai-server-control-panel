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

<section class={['nv-card', hover && 'hover', tone && `tone-${tone}`]}>
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
