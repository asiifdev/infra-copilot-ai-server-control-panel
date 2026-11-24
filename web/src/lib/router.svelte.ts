// Hash router, no library. Routes live after "#/"; the sign-in token "#t=..." is removed from the
// address bar before the router starts, and anything it cannot parse falls back to the overview.

export type Route =
  | { name: 'overview' }
  | { name: 'nodes' }
  | { name: 'node'; node: string }
  | { name: 'findings' }
  | { name: 'assistant'; conversation: string | null }
  | { name: 'proposals' }
  | { name: 'plans' }
  | { name: 'plan'; plan: string }
  | { name: 'transports' }
  | { name: 'memory' }
  | { name: 'settings' };

export type RouteName = Route['name'];

const PARAM_MAX = 200;

function decode(segment: string | undefined): string | null {
  if (!segment || segment.length > PARAM_MAX * 3) return null;
  try {
    const value = decodeURIComponent(segment);
    return value.length > 0 && value.length <= PARAM_MAX ? value : null;
  } catch {
    return null;
  }
}

export function parseHash(hash: string): Route {
  const path = hash.startsWith('#') ? hash.slice(1) : hash;
  if (!path.startsWith('/')) return { name: 'overview' };
  const [head, param, ...rest] = path.split('?')[0]!.slice(1).split('/');
  if (rest.length > 0) return { name: 'overview' };
  switch (head) {
    case 'nodes': {
      if (param === undefined || param === '') return { name: 'nodes' };
      const node = decode(param);
      return node ? { name: 'node', node } : { name: 'nodes' };
    }
    case 'findings':
      return { name: 'findings' };
    case 'assistant':
      return { name: 'assistant', conversation: decode(param) };
    case 'plans': {
      if (param === undefined || param === '') return { name: 'plans' };
      const plan = decode(param);
      return plan ? { name: 'plan', plan } : { name: 'plans' };
    }
    case 'proposals':
      return { name: 'proposals' };
    case 'transports':
      return { name: 'transports' };
    case 'memory':
      return { name: 'memory' };
    case 'settings':
      return { name: 'settings' };
    default:
      return { name: 'overview' };
  }
}

export function hrefFor(route: Route): string {
  const e = encodeURIComponent;
  switch (route.name) {
    case 'overview':
      return '#/';
    case 'node':
      return `#/nodes/${e(route.node)}`;
    case 'assistant':
      return route.conversation === null ? '#/assistant' : `#/assistant/${e(route.conversation)}`;
    case 'plan':
      return `#/plans/${e(route.plan)}`;
    default:
      return `#/${route.name}`;
  }
}

class Router {
  route = $state<Route>({ name: 'overview' });

  start(): () => void {
    const sync = () => {
      this.route = parseHash(window.location.hash);
    };
    sync();
    window.addEventListener('hashchange', sync);
    return () => window.removeEventListener('hashchange', sync);
  }

  go(route: Route): void {
    window.location.hash = hrefFor(route);
  }
}

export const router = new Router();
