<script lang="ts">
  import type { Attachment } from 'svelte/attachments';
  import { startConversation } from '../lib/conversations.svelte';
  import { trapTab } from '../lib/focus';
  import { fuzzyScore } from '../lib/fuzzy';
  import { i18n, LOCALES, t } from '../lib/i18n.svelte';
  import { NAV } from '../lib/nav';
  import { palette } from '../lib/palette.svelte';
  import { router } from '../lib/router.svelte';
  import { THEMES, theme } from '../lib/theme.svelte';
  import Icon, { type IconName } from './Icon.svelte';

  interface Entry {
    id: string;
    group: 'pages' | 'actions';
    label: string;
    icon: IconName;
    run: () => void;
  }

  let query = $state('');
  let active = $state(0);
  let list = $state<HTMLElement>();

  function nextTheme() {
    const at = THEMES.indexOf(theme.choice);
    theme.set(THEMES[(at + 1) % THEMES.length] ?? 'system');
  }

  function nextLocale() {
    const at = LOCALES.indexOf(i18n.locale);
    void i18n.load(LOCALES[(at + 1) % LOCALES.length] ?? 'en', true);
  }

  // Literal keys on purpose: the key-coverage test finds them.
  const entries = $derived<Entry[]>([
    ...NAV.map((item, index) => ({
      id: `page-${index}`,
      group: 'pages' as const,
      label: t(item.label),
      icon: item.icon,
      run: () => router.go(item.route),
    })),
    { id: 'new-conversation', group: 'actions', label: t('ui.palette.new_conversation'), icon: 'plus', run: () => void startConversation() },
    { id: 'theme', group: 'actions', label: t('ui.palette.switch_theme'), icon: 'sun', run: nextTheme },
    { id: 'language', group: 'actions', label: t('ui.palette.switch_language'), icon: 'globe', run: nextLocale },
  ]);

  const view = $derived.by(() => {
    const results = entries
      .map((entry) => ({ entry, score: fuzzyScore(query, entry.label) }))
      .filter((r) => r.score >= 0)
      .sort((a, b) => a.score - b.score)
      .map((r) => r.entry);
    const pages = results.filter((e) => e.group === 'pages');
    const actions = results.filter((e) => e.group === 'actions');
    const shown = [...pages, ...actions];
    return { pages, actions, shown, activeEntry: shown[Math.min(active, shown.length - 1)] };
  });

  // Moves focus to the field and gives it back to what opened the palette when it closes.
  const manageFocus: Attachment<HTMLElement> = (dialog) => {
    const opener = document.activeElement;
    dialog.querySelector<HTMLElement>('input')?.focus();
    return () => {
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
      else document.getElementById('page-title')?.focus();
    };
  };

  function run(entry: Entry | undefined) {
    if (!entry) return;
    palette.hide();
    entry.run();
  }

  function move(delta: number) {
    if (view.shown.length === 0) return;
    active = (Math.min(active, view.shown.length - 1) + delta + view.shown.length) % view.shown.length;
    queueMicrotask(() => list?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' }));
  }

  function onkeydown(event: KeyboardEvent & { currentTarget: HTMLElement }) {
    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        palette.hide();
        return;
      case 'ArrowDown':
        event.preventDefault();
        move(1);
        return;
      case 'ArrowUp':
        event.preventDefault();
        move(-1);
        return;
      case 'Enter':
        if (event.target instanceof HTMLInputElement) {
          event.preventDefault();
          run(view.activeEntry);
        }
        return;
      default:
        trapTab(event, event.currentTarget);
    }
  }

  function onpointerdown(event: PointerEvent & { currentTarget: HTMLElement }) {
    if (event.target === event.currentTarget) palette.hide();
  }
</script>

{#snippet option(entry: Entry)}
  {@const index = view.shown.indexOf(entry)}
  <!-- Options are driven from the field (arrow keys, Enter), as in the combobox pattern; the click is for the mouse. -->
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_interactive_supports_focus -->
  <div
    id="palette-option-{index}"
    class={['option', index === Math.min(active, view.shown.length - 1) && 'active']}
    role="option"
    aria-selected={index === Math.min(active, view.shown.length - 1)}
    onclick={() => run(entry)}
    onpointermove={() => (active = index)}
  >
    <Icon name={entry.icon} />
    <span class="label">{entry.label}</span>
    <span class="go" aria-hidden="true"><Icon name="return" size="sm" /></span>
  </div>
{/snippet}

<div class="nv-palette" role="presentation" {onpointerdown}>
  <div {@attach manageFocus} class="panel" role="dialog" aria-modal="true" aria-labelledby="palette-title" tabindex="-1" {onkeydown}>
    <h2 id="palette-title" class="visually-hidden">{t('ui.palette.title')}</h2>
    <div class="field">
      <Icon name="search" />
      <input
        class="input"
        type="text"
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-list"
        aria-autocomplete="list"
        aria-activedescendant={view.activeEntry ? `palette-option-${view.shown.indexOf(view.activeEntry)}` : undefined}
        aria-label={t('ui.palette.label')}
        placeholder={t('ui.palette.placeholder')}
        autocomplete="off"
        spellcheck="false"
        bind:value={query}
        oninput={() => (active = 0)}
      />
      <button type="button" class="close" onclick={() => palette.hide()} aria-label={t('ui.palette.close')}>
        <Icon name="close" size="sm" />
      </button>
    </div>
    <div id="palette-list" class="list" role="listbox" aria-label={t('ui.palette.label')} bind:this={list}>
      {#if view.pages.length > 0}
        <div role="group" aria-labelledby="palette-pages">
          <div id="palette-pages" class="heading eyebrow">{t('ui.palette.pages')}</div>
          {#each view.pages as entry (entry.id)}{@render option(entry)}{/each}
        </div>
      {/if}
      {#if view.actions.length > 0}
        <div role="group" aria-labelledby="palette-actions">
          <div id="palette-actions" class="heading eyebrow">{t('ui.palette.actions')}</div>
          {#each view.actions as entry (entry.id)}{@render option(entry)}{/each}
        </div>
      {/if}
    </div>
    {#if view.shown.length === 0}<p class="empty muted">{t('ui.palette.empty')}</p>{/if}
    <p class="visually-hidden" role="status" aria-live="polite">{t('ui.palette.results', { count: view.shown.length })}</p>
    <p class="foot muted">{t('ui.palette.hint')}</p>
  </div>
</div>
