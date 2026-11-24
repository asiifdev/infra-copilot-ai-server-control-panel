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
    /** Keeps the label for assistive technology but does not draw it (the placeholder or the layout names the field). */
    hideLabel?: boolean;
    rows?: number;
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
    hideLabel = false,
    rows = 4,
  }: Props = $props();

  const describedBy = $derived([hint ? `${id}-hint` : null, error ? `${id}-error` : null].filter(Boolean).join(' ') || undefined);
</script>

<div class="field">
  <label for={id} class={[hideLabel && 'visually-hidden']}>{label}</label>
  {#if multiline}
    <textarea
      {id}
      bind:value
      {required}
      {disabled}
      {placeholder}
      {autocomplete}
      {rows}
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
    font-size: var(--nv-text-sm);
  }
  input,
  textarea {
    min-height: var(--nv-target);
    padding: var(--nv-space-2) var(--nv-space-3);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius-sm);
    background: var(--nv-surface);
    color: var(--nv-text);
    font: inherit;
    width: 100%;
    transition:
      border-color var(--nv-motion) var(--nv-ease),
      box-shadow var(--nv-motion) var(--nv-ease);
  }
  input:hover:not(:disabled),
  textarea:hover:not(:disabled) {
    border-color: var(--nv-text-muted);
  }
  input:focus-visible,
  textarea:focus-visible {
    border-color: var(--nv-focus);
    box-shadow: 0 0 0 1px var(--nv-focus);
  }
  input:disabled,
  textarea:disabled {
    background: var(--nv-surface-2);
    cursor: not-allowed;
  }
  input[aria-invalid='true'],
  textarea[aria-invalid='true'] {
    border-color: var(--nv-danger);
  }
  textarea {
    font-family: var(--nv-font-mono);
    font-size: var(--nv-text-sm);
    line-height: 1.5;
    resize: vertical;
  }
  .hint {
    font-size: var(--nv-text-caption);
  }
  .error {
    color: var(--nv-danger);
    font-weight: 600;
    font-size: var(--nv-text-sm);
  }
</style>
