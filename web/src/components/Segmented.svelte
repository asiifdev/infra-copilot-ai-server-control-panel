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
<div class="nv-seg-group" role="group" aria-label={label}>
  {#each options as option (option.value)}
    <button type="button" class={['nv-seg', option.value === value && 'on']} aria-pressed={option.value === value} onclick={() => onchange(option.value)}>
      <span>{option.label}</span>
      {#if option.count !== undefined}<span class="nv-seg-count num">{option.count}</span>{/if}
    </button>
  {/each}
</div>
