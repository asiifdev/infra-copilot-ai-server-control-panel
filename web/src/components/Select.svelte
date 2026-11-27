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

<div class="nv-field">
  <label for={id}>{label}</label>
  <select {id} bind:value onchange={() => onchange?.(value)}>
    {#each options as option (option.value)}
      <option value={option.value}>{option.label}</option>
    {/each}
  </select>
</div>
