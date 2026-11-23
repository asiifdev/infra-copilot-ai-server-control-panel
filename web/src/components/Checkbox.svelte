<script lang="ts">
  interface Props {
    label: string;
    id: string;
    checked: boolean;
  }

  let { label, id, checked = $bindable() }: Props = $props();
</script>

<div class="check">
  <input {id} type="checkbox" bind:checked />
  <label for={id}>{label}</label>
</div>

<style>
  .check {
    display: flex;
    align-items: center;
    gap: var(--nv-space-2);
    min-height: var(--nv-target);
  }
  input {
    width: var(--nv-space-5);
    height: var(--nv-space-5);
    accent-color: var(--nv-primary);
  }
</style>
