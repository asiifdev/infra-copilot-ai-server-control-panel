// Locale-aware formatting of values from the API. Invalid input is shown as plain text, never
// guessed at.

export function formatDateTime(value: string | null | undefined, locale: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  try {
    return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(date);
  } catch {
    return null;
  }
}

export function formatNumber(value: number, locale: string): string {
  try {
    return new Intl.NumberFormat(locale).format(value);
  } catch {
    return String(value);
  }
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];

/** Bytes in the largest sensible unit (steps of 1024), numbers in the locale. */
export function formatBytes(value: number, locale: string): string {
  let size = Math.max(0, value);
  let unit = 0;
  while (size >= 1024 && unit < UNITS.length - 1) {
    size /= 1024;
    unit += 1;
  }
  try {
    const text = new Intl.NumberFormat(locale, { maximumFractionDigits: unit === 0 ? 0 : 1 }).format(size);
    return `${text} ${UNITS[unit]}`;
  } catch {
    return `${Math.round(size)} ${UNITS[unit]}`;
  }
}

/** Pretty-prints inventory item data (untrusted JSON) as text, bounded in size. */
export function describeData(data: unknown, max = 2000): string {
  let text: string;
  try {
    text = typeof data === 'string' ? data : (JSON.stringify(data, null, 2) ?? '');
  } catch {
    text = '';
  }
  return text.length > max ? `${text.slice(0, max)}…` : text;
}
