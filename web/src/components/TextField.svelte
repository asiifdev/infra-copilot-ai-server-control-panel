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

<div class="nv-field">
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
      {onkeydown}
      spellcheck="false"
      autocapitalize="off"
      aria-invalid={error ? true : undefined}
      aria-describedby={describedBy}
    />
  {/if}
  {#if hint}<p id="{id}-hint" class="muted nv-field-hint">{hint}</p>{/if}
  {#if error}<p id="{id}-error" class="nv-field-error">{error}</p>{/if}
</div>
