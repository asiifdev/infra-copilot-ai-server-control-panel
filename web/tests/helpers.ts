import { flushSync, mount, unmount, type Component } from 'svelte';
import { vi } from 'vitest';
import { createMock, MOCK_TOKEN, type Mock, type MockOptions } from '../mock/server';
import { onUnauthorized } from '../src/lib/api';
import { i18n, type Locale } from '../src/lib/i18n.svelte';
import { session } from '../src/lib/session.svelte';
import { toasts } from '../src/lib/toast.svelte';

const mounted: { instance: Record<string, unknown>; target: HTMLElement }[] = [];

/** Mounts a component into the document and returns its container. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function render<P extends Record<string, any>>(component: Component<P>, props?: P): HTMLElement {
  const target = document.createElement('div');
  document.body.appendChild(target);
  const instance = mount(component, { target, props: (props ?? {}) as P });
  flushSync();
  mounted.push({ instance, target });
  return target;
}

export function unmountAll(): void {
  while (mounted.length > 0) {
    const item = mounted.pop();
    if (item) void unmount(item.instance);
  }
  toasts.clear();
}

/** Installs the mock as the global fetch and returns it. */
export function installMock(options?: MockOptions): Mock {
  const mock = createMock(options);
  vi.stubGlobal('fetch', mock.fetch);
  return mock;
}

/** Exchanges the mock token so that the session cookie is in the mock's cookie jar. */
export async function signIn(mock: Mock): Promise<void> {
  const response = await mock.fetch('/api/auth/exchange', {
    method: 'POST',
    headers: { 'X-Nodivara': '1', 'Content-Type': 'application/json' },
    body: JSON.stringify({ token: MOCK_TOKEN }),
  });
  if (response.status !== 204) throw new Error(`sign-in failed: ${response.status}`);
}

/** Signs in, loads a catalog and the session info: a ready app state without mounting the App. */
export async function readyApp(mock: Mock, locale: Locale = 'en'): Promise<void> {
  await signIn(mock);
  await i18n.load(locale);
  session.info = await (await mock.fetch('/api/session')).json();
  session.phase = 'ready';
  onUnauthorized(null);
}

export function resetSession(): void {
  session.info = null;
  session.phase = 'starting';
  session.failure = null;
}

/** Resolves once the assertion holds (polling), failing with its last error after the timeout. */
export async function eventually(assertion: () => void, timeoutMs = 3000): Promise<void> {
  await vi.waitFor(assertion, { timeout: timeoutMs, interval: 10 });
}

export function byText(root: ParentNode, selector: string, text: string): HTMLElement | null {
  for (const el of root.querySelectorAll<HTMLElement>(selector)) {
    if (el.textContent?.trim() === text) return el;
  }
  return null;
}
