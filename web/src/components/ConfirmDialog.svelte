<script lang="ts">
  import type { Attachment } from 'svelte/attachments';
  import Button from './Button.svelte';

  interface Props {
    title: string;
    message: string;
    confirmLabel: string;
    cancelLabel: string;
    busy?: boolean;
    onconfirm: () => void;
    oncancel: () => void;
  }

  let { title, message, confirmLabel, cancelLabel, busy = false, onconfirm, oncancel }: Props = $props();

  const FOCUSABLE =
    'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  // Moves the focus into the dialog (to the safe choice, cancel, first) and gives it back to the
  // element that opened the dialog when the dialog goes away.
  const manageFocus: Attachment<HTMLElement> = (dialog) => {
    const opener = document.activeElement;
    dialog.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    return () => {
      if (opener instanceof HTMLElement) opener.focus();
    };
  };

  function onkeydown(event: KeyboardEvent & { currentTarget: HTMLElement }) {
    const dialog = event.currentTarget;
    if (event.key === 'Escape') {
      event.preventDefault();
      oncancel();
      return;
    }
    if (event.key !== 'Tab') return;
    const items = [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)];
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    } else if (!dialog.contains(document.activeElement)) {
      event.preventDefault();
      first.focus();
    }
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
    {onkeydown}
  >
    <h2 id="confirm-title">{title}</h2>
    <p id="confirm-message">{message}</p>
    <div class="row actions">
      <Button onclick={oncancel} disabled={busy}>{cancelLabel}</Button>
      <Button variant="danger" onclick={onconfirm} {busy}>{confirmLabel}</Button>
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
    background: var(--nv-bg);
    z-index: 10;
  }
  .dialog {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-4);
    max-width: 32rem;
    width: 100%;
    padding: var(--nv-space-5);
    background: var(--nv-surface);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius);
  }
  .actions {
    justify-content: flex-end;
  }
</style>
