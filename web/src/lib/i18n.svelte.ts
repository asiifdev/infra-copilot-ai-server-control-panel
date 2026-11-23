// Translation store. The catalog is the Rust one (crates/i18n/locales), served by
// GET /api/i18n/{locale}; the UI has no catalog of its own (ADR 0029). A missing key renders as
// the key itself, which the key-coverage test makes impossible for keys used in src/.

import { api, setApiLocale } from './api';
import { readStored, writeStored } from './storage';

export const LOCALES = ['en', 'id'] as const;
export type Locale = (typeof LOCALES)[number];

export function isLocale(value: string | null | undefined): value is Locale {
  return LOCALES.includes(value as Locale);
}

/** Substitutes {name} placeholders; {{ and }} are literal braces (same rules as the Rust catalog). */
export function format(template: string, args?: Record<string, string | number>): string {
  let out = '';
  for (let i = 0; i < template.length; i += 1) {
    const c = template[i];
    if ((c === '{' || c === '}') && template[i + 1] === c) {
      out += c;
      i += 1;
    } else if (c === '{') {
      const end = template.indexOf('}', i);
      if (end < 0) {
        out += template.slice(i);
        break;
      }
      const name = template.slice(i + 1, end);
      const value = args?.[name];
      out += value === undefined ? `{${name}}` : String(value);
      i = end;
    } else {
      out += c;
    }
  }
  return out;
}

/** The locale to start in: ?lang, then the stored choice, then the browser, then English. */
export function detectLocale(search: string, navigatorLanguages: readonly string[]): Locale {
  const fromQuery = new URLSearchParams(search).get('lang')?.slice(0, 2).toLowerCase();
  if (isLocale(fromQuery)) return fromQuery;
  const stored = readStored('nv.locale');
  if (isLocale(stored)) return stored;
  for (const language of navigatorLanguages) {
    const base = language.slice(0, 2).toLowerCase();
    if (isLocale(base)) return base;
  }
  return 'en';
}

class I18n {
  locale = $state<Locale>('en');
  messages = $state.raw<Record<string, string>>({});
  loaded = $state(false);
  #controller: AbortController | null = null;

  has(key: string): boolean {
    return key in this.messages;
  }

  t(key: string, args?: Record<string, string | number>): string {
    const template = this.messages[key];
    return template === undefined ? key : format(template, args);
  }

  /** Loads the catalog of a locale. When it fails the page keeps working with the keys. */
  async load(locale: Locale, persist = false): Promise<void> {
    this.#controller?.abort();
    const controller = new AbortController();
    this.#controller = controller;
    setApiLocale(locale);
    try {
      const map = await api.i18n(locale, controller.signal);
      if (controller.signal.aborted) return;
      this.messages = Object.fromEntries(
        Object.entries(map).filter(([, value]) => typeof value === 'string'),
      );
      this.locale = locale;
      this.loaded = true;
      if (persist) writeStored('nv.locale', locale);
      if (typeof document !== 'undefined') document.documentElement.lang = locale;
    } catch {
      if (controller.signal.aborted) return;
      this.locale = locale;
      if (typeof document !== 'undefined') document.documentElement.lang = locale;
    }
  }
}

export const i18n = new I18n();

/** Translates a key in the current locale; reactive when read in a template or $derived. */
export function t(key: string, args?: Record<string, string | number>): string {
  return i18n.t(key, args);
}
