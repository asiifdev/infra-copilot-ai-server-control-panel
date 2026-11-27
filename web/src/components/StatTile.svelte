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
