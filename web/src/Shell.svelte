<script lang="ts">
  import { approvals } from './lib/approvals.svelte';
  import { t } from './lib/i18n.svelte';
  import { currentItem, NAV_GROUPS } from './lib/nav';
  import { palette } from './lib/palette.svelte';
  import { hrefFor, router } from './lib/router.svelte';
  import { session } from './lib/session.svelte';
  import CommandPalette from './components/CommandPalette.svelte';
  import Icon from './components/Icon.svelte';
  import Toast from './components/Toast.svelte';
  import Assistant from './pages/Assistant.svelte';
  import Findings from './pages/Findings.svelte';
  import Memory from './pages/Memory.svelte';
  import NodeDetail from './pages/NodeDetail.svelte';
  import Nodes from './pages/Nodes.svelte';
  import Overview from './pages/Overview.svelte';
  import PlanDetail from './pages/PlanDetail.svelte';
  import Plans from './pages/Plans.svelte';
  import Proposals from './pages/Proposals.svelte';
  import Settings from './pages/Settings.svelte';
  import Transports from './pages/Transports.svelte';

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

<a class="skip" href="#main-content" onclick={(event) => { event.preventDefault(); document.getElementById('main-content')?.focus(); }}>
  {t('ui.app.skip')}
</a>

<div class="app">
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
      {#key router.route.name}
        <div class="page">
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
          {:else if router.route.name === 'proposals'}
            <Proposals />
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
        </div>
      {/key}
    </div>
  </main>
</div>

{#if palette.open}<CommandPalette />{/if}
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
    box-shadow: var(--nv-shadow-2);
    z-index: 70;
  }
  .skip:focus {
    inset-block-start: var(--nv-space-2);
  }

  .app {
    display: block;
    min-height: 100vh;
  }

  /* Phone: a sticky top bar (brand, search, menu); the navigation drops down under it. */
  .side {
    position: sticky;
    inset-block-start: 0;
    z-index: 30;
    background: var(--nv-surface);
    border-block-end: var(--nv-border-width) solid var(--nv-border);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--nv-space-2);
    min-height: var(--nv-topbar);
    padding: 0 var(--nv-space-3) 0 var(--nv-space-4);
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--nv-space-2);
    margin-inline-end: auto;
    min-height: var(--nv-target);
    color: var(--nv-text);
    text-decoration: none;
  }
  .mark {
    width: 1.875rem;
    height: 1.875rem;
  }
  .tile {
    fill: var(--nv-on-accent);
  }
  .glyph {
    fill: none;
    stroke: var(--nv-accent);
    stroke-width: 3;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .name {
    font-weight: 800;
    font-size: var(--nv-text-title);
    letter-spacing: -0.02em;
  }
  .find,
  .menu {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--nv-space-2);
    min-height: var(--nv-target);
    min-width: var(--nv-target);
    border: var(--nv-border-width) solid transparent;
    border-radius: var(--nv-radius-sm);
    background: none;
    color: var(--nv-text);
    font: inherit;
    cursor: pointer;
    transition: background var(--nv-motion) var(--nv-ease);
  }
  .find:hover,
  .menu:hover {
    background: var(--nv-hover);
  }
  .find-label,
  .find-key {
    display: none;
  }
  .nav {
    display: none;
    position: absolute;
    inset-inline: 0;
    inset-block-start: 100%;
    max-height: calc(100dvh - var(--nv-topbar));
    overflow-y: auto;
    padding: var(--nv-space-2) var(--nv-space-3) var(--nv-space-4);
    background: var(--nv-surface);
    border-block-end: var(--nv-border-width) solid var(--nv-border);
    box-shadow: var(--nv-shadow-2);
  }
  .nav.open {
    display: block;
    animation: drop var(--nv-motion-slow) var(--nv-ease);
  }
  .group {
    padding-block: var(--nv-space-2);
  }
  .group-label {
    padding: var(--nv-space-1) var(--nv-space-3);
  }
  .nav li a {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--nv-space-3);
    min-height: var(--nv-target);
    padding: var(--nv-space-2) var(--nv-space-3);
    border-radius: var(--nv-radius-sm);
    color: var(--nv-text);
    text-decoration: none;
    font-weight: 500;
    transition: background var(--nv-motion) var(--nv-ease);
  }
  .nav li a:hover {
    background: var(--nv-hover);
  }
  .nav li a[aria-current='page'] {
    background: var(--nv-tint-primary);
    font-weight: 700;
  }
  .nav li a[aria-current='page']::before {
    content: '';
    position: absolute;
    inset-block: var(--nv-space-2);
    inset-inline-start: 0;
    width: 3px;
    border-radius: var(--nv-radius-pill);
    background: var(--nv-primary);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .count {
    min-width: 1.5rem;
    padding: 0 var(--nv-space-2);
    border-radius: var(--nv-radius-pill);
    background: var(--nv-primary);
    color: var(--nv-on-primary);
    font-size: var(--nv-text-caption);
    font-weight: 700;
    text-align: center;
  }
  .org {
    margin-block-start: var(--nv-space-3);
    padding: var(--nv-space-3) var(--nv-space-3) 0;
    border-block-start: var(--nv-border-width) solid var(--nv-border);
    font-size: var(--nv-text-sm);
    overflow-wrap: anywhere;
  }

  main {
    min-width: 0;
  }
  .content {
    max-width: var(--nv-content-max);
    margin: 0 auto;
    padding: var(--nv-space-4) var(--nv-space-4) var(--nv-space-6);
  }
  .page {
    animation: nv-enter var(--nv-motion-slow) var(--nv-ease);
  }

  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-0.5rem);
    }
  }

  /* Wide screens: the sidebar is a full-height column of its own. */
  @media (min-width: 52rem) {
    .app {
      display: grid;
      grid-template-columns: var(--nv-sidebar) minmax(0, 1fr);
      align-items: start;
    }
    .side {
      display: flex;
      flex-direction: column;
      height: 100dvh;
      border-block-end: 0;
      border-inline-end: var(--nv-border-width) solid var(--nv-border);
    }
    .top {
      flex-wrap: wrap;
      align-content: center;
      gap: var(--nv-space-2);
      min-height: auto;
      padding: var(--nv-space-4) var(--nv-space-3) var(--nv-space-2);
    }
    .brand {
      flex: 1 0 100%;
      margin: 0 0 var(--nv-space-2) var(--nv-space-2);
    }
    .find {
      flex: 1 0 100%;
      justify-content: flex-start;
      border-color: var(--nv-border-strong);
      color: var(--nv-text-muted);
      padding: 0 var(--nv-space-3);
      background: var(--nv-bg);
    }
    .find-label {
      display: inline;
      flex: 1;
      text-align: start;
    }
    .find-key {
      display: inline;
      padding: 0 var(--nv-space-2);
      border: var(--nv-border-width) solid var(--nv-border-strong);
      border-radius: var(--nv-radius-xs);
      background: var(--nv-surface);
      color: var(--nv-text);
      font: inherit;
      font-size: var(--nv-text-caption);
    }
    .menu {
      display: none;
    }
    .nav,
    .nav.open {
      display: flex;
      flex-direction: column;
      position: static;
      animation: none;
      flex: 1;
      max-height: none;
      padding: var(--nv-space-2) var(--nv-space-3) var(--nv-space-3);
      border: 0;
      box-shadow: none;
    }
    .org {
      margin-block-start: auto;
    }
    .content {
      padding: var(--nv-space-6) var(--nv-space-6) var(--nv-space-7);
    }
  }
</style>
