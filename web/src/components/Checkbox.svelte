<script lang="ts">
  interface Props {
    label: string;
    id: string;
    checked: boolean;
    disabled?: boolean;
    /** Explains the choice (or why it is unavailable); linked to the box for assistive technology. */
    hint?: string;
  }

  let { label, id, checked = $bindable(), disabled = false, hint }: Props = $props();
</script>

<div class="field">
  <div class="check">
    <input {id} type="checkbox" bind:checked {disabled} aria-describedby={hint ? `${id}-hint` : undefined} />
    <label for={id}>{label}</label>
  </div>
  {#if hint}<p class="muted hint" id="{id}-hint">{hint}</p>{/if}
</div>

<style>
  .hint {
    margin: 0;
    font-size: var(--nv-text-sm);
  }
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
