// Theme: system (follow prefers-color-scheme), light or dark through data-theme on <html>
// (docs/brand.md; the tokens react to the attribute).

import { readStored, writeStored } from './storage';

export const THEMES = ['system', 'light', 'dark'] as const;
export type Theme = (typeof THEMES)[number];

export function isTheme(value: string | null | undefined): value is Theme {
  return THEMES.includes(value as Theme);
}

class ThemeStore {
  choice = $state<Theme>('system');

  init(): void {
    const stored = readStored('nv.theme');
    this.set(isTheme(stored) ? stored : 'system', false);
  }

  set(theme: Theme, persist = true): void {
    this.choice = theme;
    const root = document.documentElement;
    if (theme === 'system') delete root.dataset.theme;
    else root.dataset.theme = theme;
    if (persist) writeStored('nv.theme', theme);
  }
}

export const theme = new ThemeStore();
