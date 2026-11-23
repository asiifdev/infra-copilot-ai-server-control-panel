<script lang="ts">
  interface Props {
    label: string;
    value: string;
    id: string;
    type?: 'text' | 'password' | 'url';
    autocomplete?: 'off' | 'on';
    onkeydown?: (event: KeyboardEvent) => void;
    hint?: string;
    error?: string;
    required?: boolean;
    multiline?: boolean;
    placeholder?: string;
    disabled?: boolean;
  }

  let {
    label,
    value = $bindable(),
    id,
    type = 'text',
    autocomplete = 'off',
    onkeydown,
    hint,
    error,
    required = false,
    multiline = false,
    placeholder,
    disabled = false,
  }: Props = $props();

  const describedBy = $derived([hint ? `${id}-hint` : null, error ? `${id}-error` : null].filter(Boolean).join(' ') || undefined);
</script>

<div class="field">
  <label for={id}>{label}</label>
  {#if multiline}
    <textarea
      {id}
      bind:value
      {required}
      {disabled}
      {placeholder}
      {autocomplete}
      rows="4"
      {onkeydown}
      spellcheck="false"
      aria-invalid={error ? true : undefined}
      aria-describedby={describedBy}
    ></textarea>
  {:else}
    <input
      {id}
      {type}
      bind:value
      {required}
      {disabled}
      {placeholder}
      {autocomplete}
      spellcheck="false"
      autocapitalize="off"
      aria-invalid={error ? true : undefined}
      aria-describedby={describedBy}
    />
  {/if}
  {#if hint}<p id="{id}-hint" class="muted hint">{hint}</p>{/if}
  {#if error}<p id="{id}-error" class="error">{error}</p>{/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-1);
  }
  label {
    font-weight: 600;
  }
  input,
  textarea {
    min-height: var(--nv-target);
    padding: var(--nv-space-2) var(--nv-space-3);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius);
    background: var(--nv-surface);
    color: var(--nv-text);
    font: inherit;
    width: 100%;
  }
  textarea {
    font-family: var(--nv-font-mono);
    resize: vertical;
  }
  .hint {
    font-size: var(--nv-text-sm);
  }
  .error {
    color: var(--nv-danger);
    font-weight: 600;
  }
</style>
