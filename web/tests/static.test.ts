// Source scans: i18n key coverage and the security rules of ADR 0029 and 0031, enforced on the
// code itself so that a later change cannot quietly break them.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = join(import.meta.dirname, '..');
const src = join(root, 'src');
const locales = join(root, '..', 'crates', 'i18n', 'locales');

function files(dir: string, test: (name: string) => boolean): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...files(path, test));
    else if (test(entry)) out.push(path);
  }
  return out;
}

const sources = files(src, (n) => /\.(svelte|ts|css)$/.test(n));
const code = files(src, (n) => /\.(svelte|ts)$/.test(n));

function catalogKeys(locale: string): Map<string, string> {
  const map = new Map<string, string>();
  for (const raw of readFileSync(join(locales, `${locale}.txt`), 'utf8').split('\n')) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const eq = line.indexOf('=');
    map.set(line.slice(0, eq).trim(), line.slice(eq + 1).trim());
  }
  return map;
}

function placeholders(value: string): string[] {
  return [...value.replaceAll('{{', '').replaceAll('}}', '').matchAll(/\{([a-z_]+)\}/g)].map((m) => m[1] ?? '').sort();
}

const KEY = /['"`](ui\.[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*)['"`]/g;

describe('i18n key coverage', () => {
  const used = new Set<string>();
  for (const file of code) for (const m of readFileSync(file, 'utf8').matchAll(KEY)) used.add(m[1] ?? '');
  const en = catalogKeys('en');
  const id = catalogKeys('id');

  it('finds the keys the sources use', () => {
    expect(used.size).toBeGreaterThan(100);
  });

  it('has every used key in both catalog files', () => {
    expect([...used].filter((k) => !en.has(k))).toEqual([]);
    expect([...used].filter((k) => !id.has(k))).toEqual([]);
  });

  it('has no unused ui.* key in either catalog file', () => {
    const reserved = new Set(['ui.sites.title', 'ui.sites.proposed', 'ui.sites.active']);
    const uiEn = [...en.keys()].filter((k) => k.startsWith('ui.') && !reserved.has(k));
    const uiId = [...id.keys()].filter((k) => k.startsWith('ui.') && !reserved.has(k));
    expect(uiEn.filter((k) => !used.has(k))).toEqual([]);
    expect(uiId.filter((k) => !used.has(k))).toEqual([]);
  });

  it('has identical ui.* keys and placeholders in en and id, and keys of three segments', () => {
    const uiEn = [...en.keys()].filter((k) => k.startsWith('ui.')).sort();
    const uiId = [...id.keys()].filter((k) => k.startsWith('ui.')).sort();
    expect(uiEn).toEqual(uiId);
    for (const key of uiEn) {
      expect(key.split('.').length, key).toBe(3);
      expect(placeholders(en.get(key) ?? ''), key).toEqual(placeholders(id.get(key) ?? ''));
    }
  });

  it('uses every placeholder the templates give (no t() call passes an unknown one)', () => {
    // Each t('key', { name: ... }) call must pass exactly the placeholders of its key.
    for (const file of code) {
      const text = readFileSync(file, 'utf8');
      for (const m of text.matchAll(/t\('(ui\.[a-z0-9_.]+)',\s*\{([^}]*)\}/g)) {
        const key = m[1] ?? '';
        const passed = [...(m[2] ?? '').matchAll(/(?:^|,)\s*([a-z_]+)\s*(?::|,|$)/g)].map((x) => x[1] ?? '').sort();
        expect(passed, `${relative(root, file)} ${key}`).toEqual(placeholders(en.get(key) ?? ''));
      }
    }
  });

  it('has Indonesian text that is not just a copy of the English text (apart from names)', () => {
    const same = [...en.keys()].filter((k) => k.startsWith('ui.') && en.get(k) === id.get(k));
    const allowed = new Set(['ui.app.name', 'ui.settings.locale_en', 'ui.settings.locale_id', 'ui.state.ok', 'ui.severity.info', 'ui.settings.model', 'ui.assistant.duration', 'ui.settings.params_placeholder', 'ui.nodes.status', 'ui.plans.status', 'ui.plan.status', 'ui.node.title', 'ui.node.item_key', 'ui.proposals.dry_run', 'ui.monitors.target', 'ui.monitors.status']);
    expect(same.filter((k) => !allowed.has(k))).toEqual([]);
  });
});

describe('security rules of the UI sources', () => {
  const svelte = files(src, (n) => n.endsWith('.svelte'));

  it('has no {@html}, innerHTML, outerHTML, insertAdjacentHTML, document.write, eval or new Function', () => {
    const forbidden = [/\{@html/, /\binnerHTML\b/, /\bouterHTML\b/, /insertAdjacentHTML/, /document\.write/, /\beval\s*\(/, /new\s+Function\b/, /\bsetTimeout\s*\(\s*['"`]/, /\bsrcdoc\b/];
    for (const file of code) {
      const text = readFileSync(file, 'utf8');
      for (const pattern of forbidden) expect(pattern.test(text), `${relative(root, file)} ${pattern}`).toBe(false);
    }
  });

  it('has no inline <script> and no style="..." attribute strings (the CSP forbids both)', () => {
    for (const file of svelte) {
      const text = readFileSync(file, 'utf8');
      const markup = text.replace(/<script[^>]*>[\s\S]*?<\/script>/g, '').replace(/<style[^>]*>[\s\S]*?<\/style>/g, '');
      expect(/<script\b/i.test(markup), `${relative(root, file)} script`).toBe(false);
      expect(/\sstyle\s*=\s*["'{]/.test(markup), `${relative(root, file)} style attribute`).toBe(false);
    }
    expect(/<script(?![^>]*\bsrc=)[^>]*>[^<]/.test(readFileSync(join(root, 'index.html'), 'utf8'))).toBe(false);
  });

  it('has no hex or rgb colours outside the shared tokens (components use var(--nv-*))', () => {
    for (const file of sources) {
      // The shared token file is the one place where colour values live.
      if (file.endsWith('tokens.css')) continue;
      const text = readFileSync(file, 'utf8');
      expect(/#[0-9a-fA-F]{3,8}\b/.test(text.replace(/&#\d+;/g, '')), `${relative(root, file)} hex`).toBe(false);
      expect(/\brgba?\(|\bhsla?\(/.test(text), `${relative(root, file)} rgb/hsl`).toBe(false);
    }
  });

  it('touches the network only in lib/api.ts, and only same-origin', () => {
    for (const file of code) {
      const rel = relative(src, file);
      const text = readFileSync(file, 'utf8');
      if (rel !== join('lib', 'api.ts')) {
        expect(/\bfetch\s*\(|XMLHttpRequest|WebSocket|EventSource|sendBeacon/.test(text), rel).toBe(false);
      }
      expect(/https?:\/\/(?!127\.0\.0\.1:PORT)[a-z0-9.-]+\.[a-z]{2,}/i.test(text.replace(/https?:\/\/(www\.)?w3\.org[^\s'"]*/g, '')), `${rel} external URL`).toBe(false);
    }
  });

  it('keeps only the locale and the theme in browser storage', () => {
    for (const file of code) {
      const rel = relative(src, file);
      const text = readFileSync(file, 'utf8');
      const uses = /localStorage|sessionStorage|indexedDB|document\.cookie/.test(text);
      if (rel === join('lib', 'storage.ts')) {
        expect(text).toContain("'nv.locale' | 'nv.theme'");
      } else {
        expect(uses, rel).toBe(false);
      }
    }
  });

  it('has no literal text nodes or labels hard-coded in the markup', () => {
    for (const file of svelte) {
      const text = readFileSync(file, 'utf8');
      const markup = text.replace(/<script[^>]*>[\s\S]*?<\/script>/g, '').replace(/<style[^>]*>[\s\S]*?<\/style>/g, '').replace(/<!--[\s\S]*?-->/g, '');
      // Remove every {...} expression (balanced for one level of nesting), then look for words.
      let flat = markup;
      for (let i = 0; i < 4; i += 1) flat = flat.replace(/\{[^{}]*\}/g, '');
      const nodes = [...flat.matchAll(/>([^<>]+)</g)].map((m) => (m[1] ?? '').replace(/&[a-z#0-9]+;/g, '').trim()).filter((s) => /\p{L}/u.test(s));
      expect(nodes, relative(root, file)).toEqual([]);
      for (const attr of flat.matchAll(/\b(title|aria-label|placeholder|alt)="([^"]*)"/g)) {
        expect(/\p{L}/u.test(attr[2] ?? ''), `${relative(root, file)} ${attr[1]}`).toBe(false);
      }
    }
  });

  it('keeps node_modules and dist out of git', () => {
    const ignore = readFileSync(join(root, '..', '.gitignore'), 'utf8');
    expect(ignore).toMatch(/^node_modules\/$/m);
    expect(ignore).toMatch(/^dist\/$/m);
  });
});
