import { createRawSnippet, flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ConfirmDialog from '../src/components/ConfirmDialog.svelte';
import DataTable, { type Column } from '../src/components/DataTable.svelte';
import ProposalCard from '../src/components/ProposalCard.svelte';
import StatusBadge from '../src/components/StatusBadge.svelte';
import Toast from '../src/components/Toast.svelte';
import { i18n } from '../src/lib/i18n.svelte';
import { toasts } from '../src/lib/toast.svelte';
import { HOSTILE } from '../mock/fixtures';
import { installMock, render, signIn } from './helpers';

beforeEach(async () => {
  const mock = installMock();
  await signIn(mock);
  await i18n.load('en');
});

afterEach(() => {
  vi.unstubAllGlobals();
});

interface Row {
  name: string;
  size: number;
}

const cell = createRawSnippet<[Row, Column<Row>]>((row, column) => ({
  render: () => `<span>${column().id === 'name' ? 'N' : 'S'}</span>`,
  setup: (element) => {
    element.textContent = column().id === 'name' ? row().name : String(row().size);
  },
}));

const columns: Column<Row>[] = [
  { id: 'name', header: 'Name', sortValue: (r) => r.name },
  { id: 'size', header: 'Size', sortValue: (r) => r.size },
  { id: 'note', header: 'Note' },
];

const rows: Row[] = [
  { name: 'beta', size: 10 },
  { name: 'alpha', size: 2 },
  { name: 'gamma', size: 33 },
];

function names(root: HTMLElement): string[] {
  return [...root.querySelectorAll('tbody tr')].map((tr) => tr.querySelector('th')?.textContent ?? '');
}

describe('DataTable', () => {
  const props = { caption: 'Things', captionId: 'cap', columns, rows, rowKey: (r: Row) => r.name, cell, empty: 'Nothing here' };

  it('has a caption, column headers and row headers', () => {
    const root = render(DataTable<Row>, props);
    expect(root.querySelector('caption')?.textContent).toBe('Things');
    expect(root.querySelectorAll('thead th[scope="col"]').length).toBe(3);
    expect(root.querySelectorAll('tbody th[scope="row"]').length).toBe(3);
    expect(root.querySelector('[role="region"]')?.getAttribute('aria-labelledby')).toBe('cap');
  });

  it('sorts by keyboard-operable header buttons and reports aria-sort', () => {
    const root = render(DataTable<Row>, props);
    const th = (i: number) => root.querySelectorAll('thead th')[i] as HTMLElement;
    expect(th(0).getAttribute('aria-sort')).toBe('none');
    expect(th(2).hasAttribute('aria-sort')).toBe(false); // not sortable
    const button = th(0).querySelector('button') as HTMLButtonElement;
    expect(button.tagName).toBe('BUTTON'); // a real button: focusable, Enter and Space work
    button.click();
    flushSync();
    expect(names(root)).toEqual(['alpha', 'beta', 'gamma']);
    expect(th(0).getAttribute('aria-sort')).toBe('ascending');
    button.click();
    flushSync();
    expect(names(root)).toEqual(['gamma', 'beta', 'alpha']);
    expect(th(0).getAttribute('aria-sort')).toBe('descending');
    (th(1).querySelector('button') as HTMLButtonElement).click();
    flushSync();
    expect(names(root)).toEqual(['alpha', 'beta', 'gamma']); // numeric: 2, 10, 33
    expect(th(0).getAttribute('aria-sort')).toBe('none');
    expect(th(1).getAttribute('aria-sort')).toBe('ascending');
  });

  it('shows the empty state in one row that spans every column', () => {
    const root = render(DataTable<Row>, { ...props, rows: [] });
    const td = root.querySelector('tbody td');
    expect(td?.textContent).toBe('Nothing here');
    expect(td?.getAttribute('colspan')).toBe('3');
  });
});

describe('StatusBadge', () => {
  it('shows text and an icon, never colour alone', () => {
    const root = render(StatusBadge, { kind: 'node', value: 'offline' });
    expect(root.textContent).toContain('Offline');
    expect(root.querySelector('[aria-hidden="true"]')?.textContent?.length).toBeGreaterThan(0);
  });

  it('maps severities and risk tiers (number or name) to their own text', () => {
    expect(render(StatusBadge, { kind: 'severity', value: 'critical' }).textContent).toContain('Critical');
    expect(render(StatusBadge, { kind: 'tier', value: 2 }).textContent).toContain('Tier 2');
    expect(render(StatusBadge, { kind: 'tier', value: 'destructive' }).textContent).toContain('Tier 4');
  });

  it('shows an unknown value as plain text, even a hostile one', () => {
    const root = render(StatusBadge, { kind: 'state', value: HOSTILE[1] ?? '' });
    expect(root.querySelector('img')).toBeNull();
    expect(root.textContent).toContain('<img');
  });
});

describe('Toast', () => {
  it('is a polite live region that announces additions and can be dismissed', () => {
    const root = render(Toast);
    const region = root.querySelector('[aria-live]');
    expect(region?.getAttribute('aria-live')).toBe('polite');
    expect(region?.getAttribute('role')).toBe('status');
    toasts.push('success', 'Saved it');
    flushSync();
    expect(region?.textContent).toContain('Saved it');
    (root.querySelector('button') as HTMLButtonElement).click();
    flushSync();
    expect(region?.textContent).not.toContain('Saved it');
    expect(region?.isConnected).toBe(true); // the region itself stays
  });

  it('renders hostile text inert', () => {
    const root = render(Toast);
    toasts.push('error', HOSTILE[0] ?? '');
    flushSync();
    expect(root.querySelector('script')).toBeNull();
    expect(root.textContent).toContain('<script>');
  });
});

describe('ConfirmDialog', () => {
  function open() {
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    outside.focus();
    const onconfirm = vi.fn();
    const oncancel = vi.fn();
    const root = render(ConfirmDialog, { title: 'Delete?', message: 'Sure?', confirmLabel: 'Yes delete', cancelLabel: 'Keep', onconfirm, oncancel });
    return { root, outside, onconfirm, oncancel };
  }

  it('is a labelled modal alert dialog and focuses the safe choice first', () => {
    const { root } = open();
    const dialog = root.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.getAttribute('aria-labelledby')).toBe('confirm-title');
    expect(document.getElementById('confirm-title')?.textContent).toBe('Delete?');
    expect(document.activeElement?.textContent).toContain('Keep');
  });

  it('closes on Escape without confirming', () => {
    const { root, onconfirm, oncancel } = open();
    const dialog = root.querySelector('[role="alertdialog"]') as HTMLElement;
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(oncancel).toHaveBeenCalledTimes(1);
    expect(onconfirm).not.toHaveBeenCalled();
  });

  it('traps Tab and Shift+Tab inside the dialog', () => {
    const { root } = open();
    const dialog = root.querySelector('[role="alertdialog"]') as HTMLElement;
    const buttons = dialog.querySelectorAll('button');
    const first = buttons[0] as HTMLButtonElement;
    const last = buttons[buttons.length - 1] as HTMLButtonElement;
    last.focus();
    const forward = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    last.dispatchEvent(forward);
    expect(forward.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(first);
    const back = new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true });
    first.dispatchEvent(back);
    expect(back.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(last);
  });

  it('confirms only through the confirm button', () => {
    const { root, onconfirm } = open();
    const confirm = [...root.querySelectorAll('button')].find((b) => b.textContent?.includes('Yes delete')) as HTMLButtonElement;
    confirm.click();
    expect(onconfirm).toHaveBeenCalledTimes(1);
  });
});

describe('ProposalCard', () => {
  const proposal = { id: 1, action: 'Restart the web service', reason: 'It stopped answering.', impact: 'Ten seconds of downtime.', status: 'pending', tier: 2 };

  it('shows action, reason, impact and state, and has no approve or run control', () => {
    const root = render(ProposalCard, { proposal });
    expect(root.textContent).toContain('Restart the web service');
    expect(root.textContent).toContain('It stopped answering.');
    expect(root.textContent).toContain('Ten seconds of downtime.');
    expect(root.textContent).toContain('Pending');
    expect(root.textContent).toContain('milestone M4');
    expect(root.querySelectorAll('button, [role="button"], input, a[href]').length).toBe(0);
  });

  it('renders every hostile string as text', () => {
    for (const text of HOSTILE.slice(0, 8)) {
      const root = render(ProposalCard, { proposal: { ...proposal, action: text, reason: text, impact: text } });
      expect(root.querySelector('script, img, svg, iframe, a')).toBeNull();
    }
  });
});
