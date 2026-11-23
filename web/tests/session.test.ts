import { afterEach, describe, expect, it, vi } from 'vitest';
import { onUnauthorized } from '../src/lib/api';
import { session, takeTokenFromFragment } from '../src/lib/session.svelte';
import { MOCK_EXPIRED_TOKEN, MOCK_TOKEN } from '../mock/server';
import { eventually, installMock, resetSession, signIn } from './helpers';

afterEach(() => {
  vi.unstubAllGlobals();
  onUnauthorized(null);
  resetSession();
});

describe('fragment token', () => {
  it('reads the token and removes the fragment from the address bar at once', () => {
    history.replaceState(null, '', `/?lang=en#t=${MOCK_TOKEN}`);
    const spy = vi.spyOn(history, 'replaceState');
    const token = takeTokenFromFragment();
    expect(token).toBe(MOCK_TOKEN);
    expect(window.location.hash).toBe('');
    expect(window.location.search).toBe('?lang=en');
    expect(spy).toHaveBeenCalledTimes(1);
    spy.mockRestore();
  });

  it('leaves ordinary route fragments alone', () => {
    history.replaceState(null, '', '/#/nodes');
    expect(takeTokenFromFragment()).toBeNull();
    expect(window.location.hash).toBe('#/nodes');
  });
});

describe('session start', () => {
  it('exchanges the token, loads the session and the catalog, and leaves no trace of the token', async () => {
    const mock = installMock();
    history.replaceState(null, '', `/#t=${MOCK_TOKEN}`);
    await session.start();
    expect(session.phase).toBe('ready');
    expect(session.info?.org.slug).toBe('example');
    expect(window.location.hash).toBe('');
    expect(window.location.href).not.toContain(MOCK_TOKEN);
    expect(localStorage.length).toBe(0);
    const exchange = mock.state.requests.find((r) => r.path === '/api/auth/exchange');
    expect(exchange?.body).toEqual({ token: MOCK_TOKEN });
    // The token is in no store of the session object.
    expect(JSON.stringify({ phase: session.phase, info: session.info, failure: session.failure })).not.toContain(MOCK_TOKEN);
  });

  it('reports an expired or used token as invalid_token without calling it a session timeout', async () => {
    installMock();
    history.replaceState(null, '', `/#t=${MOCK_EXPIRED_TOKEN}`);
    await session.start();
    expect(session.phase).toBe('failed');
    expect(session.failure).toBe('invalid_token');
  });

  it('refuses a token with an impossible format without sending it', async () => {
    const mock = installMock();
    history.replaceState(null, '', '/#t=<script>');
    await session.start();
    expect(session.failure).toBe('invalid_token');
    expect(mock.state.requests.some((r) => r.path === '/api/auth/exchange')).toBe(false);
  });

  it('is signed out (not failed) when there is no token and no session', async () => {
    installMock();
    await session.start();
    expect(session.phase).toBe('signed_out');
  });

  it('an existing session starts without a token', async () => {
    const mock = installMock();
    await signIn(mock);
    await session.start();
    expect(session.phase).toBe('ready');
  });

  it('a 401 after sign-in ends the session; sign out clears it', async () => {
    const mock = installMock();
    await signIn(mock);
    await session.start();
    mock.cookies.clear();
    const { api } = await import('../src/lib/api');
    await api.nodes().catch(() => undefined);
    await eventually(() => expect(session.phase).toBe('signed_out'));
    expect(session.info).toBeNull();
  });
});

describe('fragment token in a running tab', () => {
  it('strips, exchanges and signs in when a sign-in link only changes the fragment', async () => {
    const mock = installMock();
    history.replaceState(null, '', '/');
    await session.start();
    expect(session.phase).toBe('signed_out');
    const stop = session.watchFragment();
    window.location.hash = `#t=${MOCK_TOKEN}`;
    await eventually(() => expect(session.phase).toBe('ready'));
    stop();
    expect(window.location.hash).toBe('');
    expect(window.location.href).not.toContain(MOCK_TOKEN);
    expect(mock.state.requests.some((r) => r.path === '/api/auth/exchange')).toBe(true);
  });

  it('strips an invalid token too and reports it', async () => {
    installMock();
    history.replaceState(null, '', '/');
    await session.start();
    const stop = session.watchFragment();
    window.location.hash = '#t=short';
    await eventually(() => expect(session.phase).toBe('failed'));
    stop();
    expect(window.location.hash).toBe('');
    expect(session.failure).toBe('invalid_token');
  });
});
