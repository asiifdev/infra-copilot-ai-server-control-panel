<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';

  interface Props {
    title: string;
    message?: string;
    icon?: IconName;
    /** An optional next step, such as a button or a link. */
    children?: Snippet;
  }

  let { title, message, icon = 'inbox', children }: Props = $props();
</script>

<div class="nv-empty">
  <span class="nv-empty-badge"><Icon name={icon} size="lg" /></span>
  <p class="nv-empty-title">{title}</p>
  {#if message}<p class="muted">{message}</p>{/if}
  {#if children}<div class="row nv-empty-action">{@render children()}</div>{/if}
</div>
