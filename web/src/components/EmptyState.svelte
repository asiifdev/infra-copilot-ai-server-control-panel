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

<div class="empty">
  <span class="badge"><Icon name={icon} size="lg" /></span>
  <p class="title">{title}</p>
  {#if message}<p class="muted">{message}</p>{/if}
  {#if children}<div class="row action">{@render children()}</div>{/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--nv-space-2);
    padding: var(--nv-space-6) var(--nv-space-4);
    border: var(--nv-border-width) dashed var(--nv-border-strong);
    border-radius: var(--nv-radius);
    background: var(--nv-surface);
  }
  .badge {
    display: grid;
    place-items: center;
    width: 3.25rem;
    height: 3.25rem;
    margin-block-end: var(--nv-space-1);
    border-radius: var(--nv-radius-pill);
    background: var(--nv-tint-primary);
    color: var(--nv-primary);
  }
  .title {
    font-weight: 650;
    font-size: var(--nv-text-title);
  }
  .action {
    justify-content: center;
    margin-block-start: var(--nv-space-2);
  }
</style>
