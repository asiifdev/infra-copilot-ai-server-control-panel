<script lang="ts">
  import type { Component } from 'svelte';
  import { approvals } from './lib/approvals.svelte';
  import { t } from './lib/i18n.svelte';
  import { currentItem, NAV_GROUPS } from './lib/nav';
  import { palette } from './lib/palette.svelte';
  import { hrefFor, router, type RouteName } from './lib/router.svelte';
  import { session } from './lib/session.svelte';
  import CommandPalette from './components/CommandPalette.svelte';
  import Icon from './components/Icon.svelte';
  import Toast from './components/Toast.svelte';
  import Assistant from './pages/Assistant.svelte';
  import Findings from './pages/Findings.svelte';
  import Memory from './pages/Memory.svelte';
  import Monitors from './pages/Monitors.svelte';
  import NodeDetail from './pages/NodeDetail.svelte';
  import Nodes from './pages/Nodes.svelte';
  import Overview from './pages/Overview.svelte';
  import PlanDetail from './pages/PlanDetail.svelte';
  import Plans from './pages/Plans.svelte';
  import Proposals from './pages/Proposals.svelte';
  import Settings from './pages/Settings.svelte';
  import Transports from './pages/Transports.svelte';

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const PAGES: Record<RouteName, Component<any>> = {
    overview: Overview,
    nodes: Nodes,
    node: NodeDetail,
    findings: Findings,
    monitors: Monitors,
    assistant: Assistant,
    proposals: Proposals,
    plans: Plans,
    plan: PlanDetail,
    transports: Transports,
    memory: Memory,
    settings: Settings,
  };

  const current = $derived(currentItem(router.route.name));

  // The document title follows the page and the language.
  const title = $derived(`${t(current.label)} - ${t('ui.app.name')}`);
  $effect(() => {
    document.title = title;
  });

  // The badge on "Approvals": how many proposals wait for a decision.
  $effect(() => {
    const controller = new AbortController();
    void approvals.refresh(controller.signal);
    return () => controller.abort();
  });

  // On a phone the navigation is a disclosure under the top bar; on a wide screen it is the
  // sidebar and the button is hidden.
  let menuOpen = $state(false);

  const shortcut = /Mac|iPhone|iPad/.test(navigator.userAgent) ? '⌘K' : 'Ctrl K';

  // After a route change, move focus to the page heading so screen-reader and keyboard users
  // land at the start of the new page (not on the first load: that would steal the focus). The
  // router has already updated the route when this runs; a timer waits until the page is swapped.
  function focusHeading() {
    menuOpen = false;
    setTimeout(() => document.getElementById('page-title')?.focus(), 0);
  }

  function onkeydown(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      palette.toggle();
    } else if (event.key === 'Escape' && menuOpen && !palette.open) {
      menuOpen = false;
      document.getElementById('menu-button')?.focus();
    }
  }
</script>

<svelte:window onhashchange={focusHeading} {onkeydown} />

<a class="nv-skip" href="#main-content" onclick={(event) => { event.preventDefault(); document.getElementById('main-content')?.focus(); }}>
  {t('ui.app.skip')}
</a>

<div class="nv-shell">
  <div class="side">
    <header class="top">
      <a class="brand" href={hrefFor({ name: 'overview' })}>
        <svg class="mark" data-icon="brand" viewBox="0 0 32 32" aria-hidden="true" focusable="false">
          <rect class="tile" width="32" height="32" rx="9" />
          <path class="glyph" d="M9 23V9l14 14V9" />
        </svg>
        <span class="name">{t('ui.app.name')}</span>
      </a>
      <button type="button" class="find" onclick={() => palette.show()}>
        <Icon name="search" />
        <span class="find-label">{t('ui.palette.open')}</span>
        <kbd class="find-key">{shortcut}</kbd>
      </button>
      <button
        type="button"
        id="menu-button"
        class="menu"
        aria-expanded={menuOpen}
        aria-controls="primary-nav"
        aria-label={menuOpen ? t('ui.nav.menu_close') : t('ui.nav.menu_open')}
        onclick={() => (menuOpen = !menuOpen)}
      >
        <Icon name={menuOpen ? 'close' : 'menu'} />
      </button>
    </header>

    <nav id="primary-nav" class={['nav', menuOpen && 'open']} aria-label={t('ui.app.navigation')}>
      {#each NAV_GROUPS as group (group.label)}
        <div class="group">
          <p class="eyebrow group-label">{t(group.label)}</p>
          <ul class="plain">
            {#each group.items as item (item.label)}
              <li>
                <a href={hrefFor(item.route)} aria-current={item === current ? 'page' : undefined} onclick={() => (menuOpen = false)}>
                  <Icon name={item.icon} /><span class="text">{t(item.label)}</span>{#if item.route.name === 'proposals' && approvals.pending > 0}
                    <span class="count num" aria-hidden="true">{approvals.pending}</span>
                    <span class="visually-hidden">{t('ui.nav.pending_count', { count: approvals.pending })}</span>
                  {/if}
                </a>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
      <p class="org muted">{session.info?.org.name ?? ''}</p>
    </nav>
  </div>

  <main id="main-content" tabindex="-1">
    <div class="content">
      {#key router.route.name === 'node' ? router.route.node : router.route.name}
        {@const Page = PAGES[router.route.name]}
        <div class="page">
          <Page
            name={router.route.name === 'node' ? router.route.node : router.route.name === 'plan' ? router.route.plan : undefined}
            conversation={router.route.name === 'assistant' ? router.route.conversation : undefined}
          />
        </div>
      {/key}
    </div>
  </main>
</div>

{#if palette.open}<CommandPalette />{/if}
<Toast />
