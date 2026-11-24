<script lang="ts">
  interface Option {
    value: string;
    label: string;
  }

  interface Props {
    label: string;
    id: string;
    value: string;
    options: Option[];
    onchange?: (value: string) => void;
  }

  let { label, id, value = $bindable(), options, onchange }: Props = $props();
</script>

<div class="field">
  <label for={id}>{label}</label>
  <select {id} bind:value onchange={() => onchange?.(value)}>
    {#each options as option (option.value)}
      <option value={option.value}>{option.label}</option>
    {/each}
  </select>
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-1);
  }
  label {
    font-weight: 600;
    font-size: var(--nv-text-sm);
  }
  select {
    min-height: var(--nv-target);
    padding: var(--nv-space-2) var(--nv-space-3);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius-sm);
    background: var(--nv-surface);
    color: var(--nv-text);
    font: inherit;
    cursor: pointer;
    transition: border-color var(--nv-motion) var(--nv-ease);
  }
  select:hover {
    border-color: var(--nv-text-muted);
  }
  select:focus-visible {
    border-color: var(--nv-focus);
  }
</style>
