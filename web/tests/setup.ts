import { afterEach } from 'vitest';
import { unmountAll } from './helpers';

afterEach(() => {
  unmountAll();
  document.body.replaceChildren();
  document.documentElement.removeAttribute('data-theme');
  try {
    localStorage.clear();
  } catch {
    // not available
  }
  history.replaceState(null, '', '/');
});
