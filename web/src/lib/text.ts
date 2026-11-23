// Text from the server, tools and models is data. It is rendered as text only (never as HTML),
// and this helper removes characters that can make text lie about its own order or hide:
// bidirectional embedding/override/isolate controls and zero-width characters (not the zero-width
// joiner, which emoji need). Very long text is cut so one reply cannot freeze the page.

// eslint-disable-next-line no-misleading-character-class
const HIDDEN = /[\u200B\u200C\u2060\uFEFF\u202A-\u202E\u2066-\u2069\u061C]/g;

export const TEXT_LIMIT = 200_000;

export interface SafeText {
  text: string;
  truncated: boolean;
}

export function safeText(value: unknown): SafeText {
  const raw = typeof value === 'string' ? value : value === null || value === undefined ? '' : String(value);
  const cut = raw.length > TEXT_LIMIT;
  const text = (cut ? raw.slice(0, TEXT_LIMIT) : raw).replace(HIDDEN, '');
  return { text, truncated: cut };
}

/** A short single-line version for table cells and labels. */
export function inlineText(value: unknown, max = 200): string {
  const { text } = safeText(value);
  const flat = text.replace(/\s+/g, ' ').trim();
  return flat.length > max ? `${flat.slice(0, max)}…` : flat;
}
