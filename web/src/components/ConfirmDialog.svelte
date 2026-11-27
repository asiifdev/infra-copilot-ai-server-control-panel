<script lang="ts">
  import type { Snippet } from 'svelte';
  import { dialogFocus, dialogKey } from '../lib/focus';
  import Button from './Button.svelte';

  interface Props {
    title: string;
    message: string;
    confirmLabel: string;
    cancelLabel: string;
    busy?: boolean;
    confirmDisabled?: boolean;
    /** Look of the confirming button: danger for a deletion, primary for a deliberate approval. */
    tone?: 'danger' | 'primary';
    role?: 'alertdialog' | 'dialog';
    children?: Snippet;
    onconfirm: () => void;
    oncancel: () => void;
  }

  let { title, message, confirmLabel, cancelLabel, busy = false, confirmDisabled = false, tone = 'danger', role = 'alertdialog', children, onconfirm, oncancel }: Props = $props();

  // Set once the person confirmed: the action usually removes the row the dialog was opened
  // from (a deleted note or provider), so the opener is no longer a place to return to.
  let confirmed = false;

  const manageFocus = dialogFocus(() => confirmed);

  function accept() {
    confirmed = true;
    onconfirm();
  }

  function cancel() {
    confirmed = false;
    oncancel();
  }
</script>

<div class="nv-backdrop" role="presentation">
  <div
    {@attach manageFocus}
    class="nv-dialog"
    {role}
    aria-modal="true"
    aria-labelledby="confirm-title"
    aria-describedby="confirm-message"
    tabindex="-1"
    onkeydown={(e) => dialogKey(e, cancel)}
  >
    <h2 id="confirm-title">{title}</h2>
    <p id="confirm-message">{message}</p>
    {@render children?.()}
    <div class="row nv-dialog-actions">
      <Button onclick={cancel} disabled={busy}>{cancelLabel}</Button>
      <Button variant={tone} onclick={accept} disabled={confirmDisabled} {busy}>{confirmLabel}</Button>
    </div>
  </div>
</div>
