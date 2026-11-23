// Toast store: short, localized notices announced through an aria-live region.

export type ToastKind = 'success' | 'error' | 'info';

export interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
}

const AUTO_DISMISS_MS = 7000;
const MAX_VISIBLE = 4;

class ToastStore {
  items = $state<ToastItem[]>([]);
  #next = 1;

  push(kind: ToastKind, text: string): number {
    const id = this.#next++;
    this.items = [...this.items, { id, kind, text }].slice(-MAX_VISIBLE);
    if (kind !== 'error') setTimeout(() => this.dismiss(id), AUTO_DISMISS_MS);
    return id;
  }

  dismiss(id: number): void {
    this.items = this.items.filter((item) => item.id !== id);
  }

  clear(): void {
    this.items = [];
  }
}

export const toasts = new ToastStore();
