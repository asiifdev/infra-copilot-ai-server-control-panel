// The only data kept in the browser: the chosen locale and theme (ADR 0031). Nothing else, and
// never a token, a key or any server text. Every access can fail (private window, blocked data).

export type StoredKey = 'nv.locale' | 'nv.theme';

export function readStored(key: StoredKey): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

export function writeStored(key: StoredKey, value: string): void {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch {
    // Storage is a convenience; the page works without it.
  }
}
