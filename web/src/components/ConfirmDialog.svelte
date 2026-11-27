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
    children?: Snippet;
    onconfirm: () => void;
    oncancel: () => void;
  }

  let { title, message, confirmLabel, cancelLabel, busy = false, confirmDisabled = false, tone = 'danger', children, onconfirm, oncancel }: Props = $props();

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

<div class="backdrop" role="presentation">
  <div
    {@attach manageFocus}
    class="dialog"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="confirm-title"
    aria-describedby="confirm-message"
    tabindex="-1"
    onkeydown={(e) => dialogKey(e, cancel)}
  >
    <h2 id="confirm-title">{title}</h2>
    <p id="confirm-message">{message}</p>
    {@render children?.()}
    <div class="row actions">
      <Button onclick={cancel} disabled={busy}>{cancelLabel}</Button>
      <Button variant={tone} onclick={accept} disabled={confirmDisabled} {busy}>{confirmLabel}</Button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    padding: var(--nv-space-4);
    background: var(--nv-scrim);
    backdrop-filter: blur(6px);
    z-index: 50;
    animation: nv-fade var(--nv-motion-slow) var(--nv-ease);
  }
  .dialog {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
    max-width: 30rem;
    width: 100%;
    padding: var(--nv-space-5);
    background: var(--nv-surface);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius-lg);
    box-shadow: var(--nv-shadow-2);
    animation: nv-pop var(--nv-motion-slow) var(--nv-ease);
  }
  .actions {
    justify-content: flex-end;
    margin-block-start: var(--nv-space-2);
  }
</style>
