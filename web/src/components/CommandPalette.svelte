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

  const results = $derived(
    entries
      .map((entry) => ({ entry, score: fuzzyScore(query, entry.label) }))
      .filter((r) => r.score >= 0)
      .sort((a, b) => a.score - b.score)
      .map((r) => r.entry),
  );
  const pages = $derived(results.filter((e) => e.group === 'pages'));
  const actions = $derived(results.filter((e) => e.group === 'actions'));
  // The order on screen: pages, then actions (the headings follow it).
  const shown = $derived([...pages, ...actions]);
  const activeEntry = $derived(shown[Math.min(active, shown.length - 1)]);

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
    if (shown.length === 0) return;
    active = (Math.min(active, shown.length - 1) + delta + shown.length) % shown.length;
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
          run(activeEntry);
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
  {@const index = shown.indexOf(entry)}
  <!-- Options are driven from the field (arrow keys, Enter), as in the combobox pattern; the click is for the mouse. -->
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_interactive_supports_focus -->
  <div
    id="palette-option-{index}"
    class={['option', index === Math.min(active, shown.length - 1) && 'active']}
    role="option"
    aria-selected={index === Math.min(active, shown.length - 1)}
    onclick={() => run(entry)}
    onpointermove={() => (active = index)}
  >
    <Icon name={entry.icon} />
    <span class="label">{entry.label}</span>
    <span class="go" aria-hidden="true"><Icon name="return" size="sm" /></span>
  </div>
{/snippet}

<div class="scrim" role="presentation" {onpointerdown}>
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
        aria-activedescendant={activeEntry ? `palette-option-${shown.indexOf(activeEntry)}` : undefined}
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
      {#if pages.length > 0}
        <div role="group" aria-labelledby="palette-pages">
          <div id="palette-pages" class="heading eyebrow">{t('ui.palette.pages')}</div>
          {#each pages as entry (entry.id)}{@render option(entry)}{/each}
        </div>
      {/if}
      {#if actions.length > 0}
        <div role="group" aria-labelledby="palette-actions">
          <div id="palette-actions" class="heading eyebrow">{t('ui.palette.actions')}</div>
          {#each actions as entry (entry.id)}{@render option(entry)}{/each}
        </div>
      {/if}
    </div>
    {#if shown.length === 0}<p class="empty muted">{t('ui.palette.empty')}</p>{/if}
    <p class="visually-hidden" role="status" aria-live="polite">{t('ui.palette.results', { count: shown.length })}</p>
    <p class="foot muted">{t('ui.palette.hint')}</p>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: var(--nv-space-4);
    padding-block-start: min(14vh, 6rem);
    background: var(--nv-scrim);
    backdrop-filter: blur(6px);
    animation: nv-fade var(--nv-motion-slow) var(--nv-ease);
  }
  .panel {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 36rem;
    max-height: 100%;
    overflow: hidden;
    background: var(--nv-surface);
    border: var(--nv-border-width) solid var(--nv-border-strong);
    border-radius: var(--nv-radius-lg);
    box-shadow: var(--nv-shadow-2);
    animation: nv-pop var(--nv-motion-slow) var(--nv-ease);
  }
  .field {
    display: flex;
    align-items: center;
    gap: var(--nv-space-2);
    padding: 0 var(--nv-space-2) 0 var(--nv-space-4);
    border-block-end: var(--nv-border-width) solid var(--nv-border);
    color: var(--nv-text-muted);
  }
  .input {
    flex: 1;
    min-width: 0;
    min-height: 3.5rem;
    border: 0;
    background: none;
    color: var(--nv-text);
    font: inherit;
    font-size: var(--nv-text-title);
  }
  .input:focus-visible {
    outline: none;
  }
  .field:has(.input:focus-visible) {
    box-shadow: inset 0 calc(-1 * var(--nv-focus-width)) 0 var(--nv-focus);
  }
  .close {
    display: inline-grid;
    place-items: center;
    min-width: var(--nv-target);
    min-height: var(--nv-target);
    border: 0;
    border-radius: var(--nv-radius-sm);
    background: none;
    color: var(--nv-text-muted);
    cursor: pointer;
  }
  .close:hover {
    background: var(--nv-hover);
    color: var(--nv-text);
  }
  .list {
    overflow-y: auto;
    padding: var(--nv-space-2);
    max-height: 22rem;
  }
  .heading {
    padding: var(--nv-space-2) var(--nv-space-3) var(--nv-space-1);
  }
  .option {
    display: flex;
    align-items: center;
    gap: var(--nv-space-3);
    min-height: var(--nv-target);
    padding: var(--nv-space-2) var(--nv-space-3);
    border-radius: var(--nv-radius-sm);
    cursor: pointer;
  }
  .active {
    background: var(--nv-tint-primary);
    box-shadow: inset var(--nv-space-1) 0 0 var(--nv-primary);
  }
  .label {
    flex: 1;
    min-width: 0;
  }
  .go {
    display: none;
    color: var(--nv-text-muted);
  }
  .active .go {
    display: inline-flex;
  }
  .empty {
    padding: var(--nv-space-4);
  }
  .foot {
    padding: var(--nv-space-2) var(--nv-space-4);
    border-block-start: var(--nv-border-width) solid var(--nv-border);
    background: var(--nv-surface-2);
    font-size: var(--nv-text-caption);
    max-width: none;
  }
</style>
