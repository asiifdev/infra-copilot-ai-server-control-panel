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

<div class="nv-field">
  <div class="nv-check">
    <input {id} type="checkbox" bind:checked {disabled} aria-describedby={hint ? `${id}-hint` : undefined} />
    <label for={id}>{label}</label>
  </div>
  {#if hint}<p class="muted nv-check-hint" id="{id}-hint">{hint}</p>{/if}
</div>
