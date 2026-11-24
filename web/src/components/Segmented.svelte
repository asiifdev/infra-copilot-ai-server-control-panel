<script lang="ts">
  interface Option {
    value: string;
    /** Localized text. */
    label: string;
    /** Optional count shown after the label. */
    count?: number;
  }

  interface Props {
    /** Localized name of the group, for assistive technology. */
    label: string;
    options: Option[];
    value: string;
    onchange: (value: string) => void;
  }

  let { label, options, value, onchange }: Props = $props();
</script>

<!-- A row of toggle buttons where exactly one is pressed: a filter, not a navigation. -->
<div class="segmented" role="group" aria-label={label}>
  {#each options as option (option.value)}
    <button type="button" class={['seg', option.value === value && 'on']} aria-pressed={option.value === value} onclick={() => onchange(option.value)}>
      <span>{option.label}</span>
      {#if option.count !== undefined}<span class="count num">{option.count}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: flex;
    flex-wrap: wrap;
    gap: var(--nv-space-2);
  }
  .seg {
    display: inline-flex;
    align-items: center;
    gap: var(--nv-space-2);
    min-height: var(--nv-target);
    padding: var(--nv-space-1) var(--nv-space-4);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius-pill);
    background: var(--nv-surface);
    color: var(--nv-text);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    transition:
      background var(--nv-motion) var(--nv-ease),
      border-color var(--nv-motion) var(--nv-ease);
  }
  .seg:hover {
    background: var(--nv-hover);
  }
  .on {
    background: var(--nv-primary);
    border-color: var(--nv-primary);
    color: var(--nv-on-primary);
  }
  .on:hover {
    background: var(--nv-primary-hover);
  }
  .count {
    min-width: 1.5rem;
    padding: 0 var(--nv-space-2);
    border-radius: var(--nv-radius-pill);
    background: var(--nv-surface-2);
    color: var(--nv-text);
    font-size: var(--nv-text-caption);
    text-align: center;
  }
  .on .count {
    background: var(--nv-surface);
    color: var(--nv-text);
  }
</style>
