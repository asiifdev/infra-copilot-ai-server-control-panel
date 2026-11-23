// Sign-in flow (ADR 0029): the one-time token arrives in the URL fragment (#t=...), is removed
// from the address bar immediately, and is exchanged for the HttpOnly session cookie. The token
// lives in one local variable for the duration of one request; it is never put in state, storage
// or the URL again.

import { api, ApiError, onUnauthorized } from './api';
import { detectLocale, i18n } from './i18n.svelte';
import type { SessionInfo } from './types';

export type Phase = 'starting' | 'ready' | 'signed_out' | 'failed';

const TOKEN_FORMAT = /^[A-Za-z0-9._~+/=-]{8,512}$/;

/**
 * Reads the token from the fragment and removes the fragment from the address bar at once, before
 * anything else can run. Returns null when there is no token.
 */
export function takeTokenFromFragment(): string | null {
  const hash = window.location.hash;
  if (!hash.startsWith('#t=')) return null;
  const token = hash.slice(3);
  window.history.replaceState(null, '', window.location.pathname + window.location.search);
  return token;
}

class SessionStore {
  phase = $state<Phase>('starting');
  info = $state.raw<SessionInfo | null>(null);
  /** Fixed error code of the failure that stopped the sign-in, if any. */
  failure = $state<string | null>(null);

  async start(): Promise<void> {
    const token = takeTokenFromFragment();
    const early = i18n.load(detectLocale(window.location.search, navigator.languages ?? []));
    onUnauthorized(() => this.expire());
    await this.#establish(token);
    await early;
    // The catalog may have needed the session: load it again now that there is one.
    if (!i18n.loaded) await i18n.load(i18n.locale);
  }

  /**
   * Watches for a sign-in link opened in an already running tab: the browser then only changes
   * the fragment (no reload), so the token is stripped, exchanged and the session re-fetched here.
   * Returns the function that stops watching.
   */
  watchFragment(): () => void {
    const onHashChange = () => {
      const token = takeTokenFromFragment();
      if (token === null) return;
      this.failure = null;
      void this.#establish(token);
    };
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }

  async #establish(token: string | null): Promise<void> {
    try {
      if (token !== null) {
        if (!TOKEN_FORMAT.test(token)) throw new ApiError(401, 'invalid_token', '');
        await api.exchange(token);
      }
      this.info = await api.session();
      this.phase = 'ready';
    } catch (cause) {
      if (cause instanceof ApiError && cause.code === 'invalid_token') {
        this.failure = 'invalid_token';
        this.phase = 'failed';
      } else if (cause instanceof ApiError && cause.status === 401) {
        this.phase = 'signed_out';
      } else {
        this.failure = cause instanceof ApiError ? cause.code : 'internal';
        this.phase = 'failed';
      }
    }
  }

  expire(): void {
    if (this.phase === 'ready') {
      this.info = null;
      this.phase = 'signed_out';
    }
  }

  async signOut(): Promise<void> {
    try {
      await api.logout();
    } catch {
      // The local state is cleared either way.
    }
    this.info = null;
    this.phase = 'signed_out';
  }
}

export const session = new SessionStore();
