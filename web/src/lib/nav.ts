// The pages of the app as the navigation and the command palette list them. Literal keys on
// purpose: the key-coverage test finds them.

import type { IconName } from '../components/Icon.svelte';
import type { Route, RouteName } from './router.svelte';

export interface NavItem {
  route: Route;
  /** Route names that count as "this page" (a detail page keeps its list highlighted). */
  match: RouteName[];
  label: string;
  icon: IconName;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const NAV_GROUPS: NavGroup[] = [
  {
    label: 'ui.nav.group_monitor',
    items: [
      { route: { name: 'overview' }, match: ['overview'], label: 'ui.nav.overview', icon: 'overview' },
      { route: { name: 'nodes' }, match: ['nodes', 'node'], label: 'ui.nav.nodes', icon: 'nodes' },
      { route: { name: 'findings' }, match: ['findings'], label: 'ui.nav.findings', icon: 'findings' },
    ],
  },
  {
    label: 'ui.nav.group_work',
    items: [
      { route: { name: 'assistant', conversation: null }, match: ['assistant'], label: 'ui.nav.assistant', icon: 'assistant' },
      { route: { name: 'proposals' }, match: ['proposals'], label: 'ui.nav.proposals', icon: 'proposals' },
      { route: { name: 'plans' }, match: ['plans', 'plan'], label: 'ui.nav.plans', icon: 'plans' },
      { route: { name: 'transports' }, match: ['transports'], label: 'ui.nav.transports', icon: 'transports' },
    ],
  },
  {
    label: 'ui.nav.group_workspace',
    items: [
      { route: { name: 'memory' }, match: ['memory'], label: 'ui.nav.memory', icon: 'memory' },
      { route: { name: 'settings' }, match: ['settings'], label: 'ui.nav.settings', icon: 'settings' },
    ],
  },
];

export const NAV: NavItem[] = NAV_GROUPS.flatMap((group) => group.items);

export function currentItem(name: RouteName): NavItem {
  return NAV.find((item) => item.match.includes(name)) ?? NAV[0]!;
}
