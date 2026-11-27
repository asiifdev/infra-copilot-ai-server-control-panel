// Keyboard focus helpers shared by the modal dialogs (confirmation, command palette).

export const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Keeps Tab and Shift+Tab inside the container: wraps at both ends and pulls stray focus back in. */
export function trapTab(event: KeyboardEvent, container: HTMLElement): void {
  if (event.key !== 'Tab') return;
  const items = [...container.querySelectorAll<HTMLElement>(FOCUSABLE)];
  const first = items[0];
  const last = items[items.length - 1];
  if (!first || !last) return;
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  } else if (!container.contains(document.activeElement)) {
    event.preventDefault();
    first.focus();
  }
}

export function dialogFocus(skipReturn?: () => boolean) {
  return (dialog: HTMLElement) => {
    const opener = document.activeElement;
    dialog.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    return () => {
      if (!skipReturn?.() && opener instanceof HTMLElement && opener.isConnected) opener.focus();
      else document.getElementById('page-title')?.focus();
    };
  };
}

export function dialogKey(event: KeyboardEvent & { currentTarget: HTMLElement }, onCancel: () => void): void {
  if (event.key === 'Escape') {
    event.preventDefault();
    onCancel();
    return;
  }
  trapTab(event, event.currentTarget);
}
