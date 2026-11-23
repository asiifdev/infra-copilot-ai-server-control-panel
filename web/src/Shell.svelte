<script lang="ts">
  import { t } from './lib/i18n.svelte';
  import { hrefFor, router, type Route, type RouteName } from './lib/router.svelte';
  import { session } from './lib/session.svelte';
  import Toast from './components/Toast.svelte';
  import Assistant from './pages/Assistant.svelte';
  import Findings from './pages/Findings.svelte';
  import Memory from './pages/Memory.svelte';
  import NodeDetail from './pages/NodeDetail.svelte';
  import Nodes from './pages/Nodes.svelte';
  import Overview from './pages/Overview.svelte';
  import PlanDetail from './pages/PlanDetail.svelte';
  import Plans from './pages/Plans.svelte';
  import Settings from './pages/Settings.svelte';
  import Transports from './pages/Transports.svelte';

  // Literal keys on purpose: the key-coverage test finds them.
  const NAV: { route: Route; match: RouteName[]; label: string }[] = [
    { route: { name: 'overview' }, match: ['overview'], label: 'ui.nav.overview' },
    { route: { name: 'nodes' }, match: ['nodes', 'node'], label: 'ui.nav.nodes' },
    { route: { name: 'findings' }, match: ['findings'], label: 'ui.nav.findings' },
    { route: { name: 'assistant', conversation: null }, match: ['assistant'], label: 'ui.nav.assistant' },
    { route: { name: 'plans' }, match: ['plans', 'plan'], label: 'ui.nav.plans' },
    { route: { name: 'transports' }, match: ['transports'], label: 'ui.nav.transports' },
    { route: { name: 'memory' }, match: ['memory'], label: 'ui.nav.memory' },
    { route: { name: 'settings' }, match: ['settings'], label: 'ui.nav.settings' },
  ];

  const current = $derived(NAV.find((item) => item.match.includes(router.route.name)) ?? NAV[0]!);

  // The document title follows the page and the language.
  const title = $derived(`${t(current.label)} - ${t('ui.app.name')}`);
  $effect(() => {
    document.title = title;
  });

  // After a route change, move focus to the page heading so screen-reader and keyboard users
  // land at the start of the new page (not on the first load: that would steal the focus). The
  // router has already updated the route when this runs; a timer waits until the page is swapped.
  function focusHeading() {
    setTimeout(() => document.getElementById('page-title')?.focus(), 0);
  }
</script>

<svelte:window onhashchange={focusHeading} />

<a class="skip" href="#main-content" onclick={(event) => { event.preventDefault(); document.getElementById('main-content')?.focus(); }}>
  {t('ui.app.skip')}
</a>

<header class="top">
  <span class="brand">{t('ui.app.name')}</span>
  <span class="who muted">{session.info?.org.name ?? ''}</span>
</header>

<div class="frame">
  <nav class="nav" aria-label={t('ui.app.navigation')}>
    <ul>
      {#each NAV as item (item.label)}
        <li>
          <a href={hrefFor(item.route)} aria-current={item === current ? 'page' : undefined}>{t(item.label)}</a>
        </li>
      {/each}
    </ul>
  </nav>

  <main id="main-content" tabindex="-1">
    {#key router.route.name}
      {#if router.route.name === 'overview'}
        <Overview />
      {:else if router.route.name === 'nodes'}
        <Nodes />
      {:else if router.route.name === 'node'}
        {#key router.route.node}<NodeDetail name={router.route.node} />{/key}
      {:else if router.route.name === 'findings'}
        <Findings />
      {:else if router.route.name === 'assistant'}
        <Assistant conversation={router.route.conversation} />
      {:else if router.route.name === 'plans'}
        <Plans />
      {:else if router.route.name === 'plan'}
        <PlanDetail name={router.route.plan} />
      {:else if router.route.name === 'transports'}
        <Transports />
      {:else if router.route.name === 'memory'}
        <Memory />
      {:else}
        <Settings />
      {/if}
    {/key}
  </main>
</div>

<Toast />

<style>
  .skip {
    position: absolute;
    inset-inline-start: var(--nv-space-2);
    inset-block-start: calc(-1 * var(--nv-target) * 2);
    padding: var(--nv-space-2) var(--nv-space-4);
    background: var(--nv-surface);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius);
    z-index: 30;
  }
  .skip:focus {
    inset-block-start: var(--nv-space-2);
  }
  .top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--nv-space-3);
    padding: var(--nv-space-3) var(--nv-space-4);
    background: var(--nv-surface);
    border-bottom: var(--nv-border-width) solid var(--nv-border);
  }
  .brand {
    font-weight: 800;
    font-size: var(--nv-text-lg);
  }
  .frame {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    max-width: var(--nv-content-max);
    margin: 0 auto;
  }
  @media (min-width: 52rem) {
    .frame {
      grid-template-columns: var(--nv-sidebar) minmax(0, 1fr);
    }
  }
  .nav ul {
    list-style: none;
    margin: 0;
    padding: var(--nv-space-3);
    display: flex;
    flex-wrap: wrap;
    gap: var(--nv-space-1);
  }
  @media (min-width: 52rem) {
    .nav ul {
      flex-direction: column;
    }
  }
  .nav a {
    display: flex;
    align-items: center;
    min-height: var(--nv-target);
    padding: var(--nv-space-2) var(--nv-space-3);
    border-radius: var(--nv-radius);
    text-decoration: none;
  }
  .nav a[aria-current='page'] {
    background: var(--nv-primary);
    color: var(--nv-on-primary);
    font-weight: 700;
  }
  main {
    padding: var(--nv-space-4);
    min-width: 0;
  }
</style>
