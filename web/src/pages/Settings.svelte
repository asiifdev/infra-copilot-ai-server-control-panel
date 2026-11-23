<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { errorKey } from '../lib/errors';
  import { i18n, isLocale, LOCALES, t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { session } from '../lib/session.svelte';
  import { isTheme, theme, THEMES } from '../lib/theme.svelte';
  import { toasts } from '../lib/toast.svelte';
  import { inlineText } from '../lib/text';
  import type { ProviderTestResult, ProviderView } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import ConfirmDialog from '../components/ConfirmDialog.svelte';
  import DataTable, { type Column } from '../components/DataTable.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import ErrorNotice from '../components/ErrorNotice.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import ProviderForm from '../components/ProviderForm.svelte';
  import Select from '../components/Select.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';

  const providers = useResource((signal) => api.providers(signal));
  const masterKey = $derived(session.info?.master_key_configured ?? false);

  // Literal keys on purpose: the key-coverage test finds them.
  const LOCALE_NAMES: Record<string, string> = { en: 'ui.settings.locale_en', id: 'ui.settings.locale_id' };
  const THEME_NAMES: Record<string, string> = {
    system: 'ui.settings.theme_system',
    light: 'ui.settings.theme_light',
    dark: 'ui.settings.theme_dark',
  };

  let localeChoice = $state<string>(i18n.locale);
  let themeChoice = $state<string>(theme.choice);
  const localeOptions = $derived(LOCALES.map((value) => ({ value, label: t(LOCALE_NAMES[value] ?? 'ui.settings.locale_en') })));
  const themeOptions = $derived(THEMES.map((value) => ({ value, label: t(THEME_NAMES[value] ?? 'ui.settings.theme_system') })));

  let toDelete = $state.raw<ProviderView | null>(null);
  let deleting = $state(false);
  let testing = $state<string | null>(null);
  let testResult = $state.raw<{ name: string; result: ProviderTestResult | { ok: false; code: string; message: string } } | null>(null);

  const columns = $derived<Column<ProviderView>[]>([
    { id: 'name', header: t('ui.settings.name'), sortValue: (p) => p.name },
    { id: 'model', header: t('ui.settings.model'), sortValue: (p) => p.model },
    { id: 'url', header: t('ui.settings.base_url') },
    { id: 'key', header: t('ui.settings.key_state') },
    { id: 'actions', header: t('ui.common.actions') },
  ]);

  function changeLocale(value: string) {
    if (isLocale(value)) void i18n.load(value, true);
  }

  function changeTheme(value: string) {
    if (isTheme(value)) theme.set(value);
  }

  async function runTest(provider: ProviderView) {
    testing = provider.name;
    testResult = null;
    try {
      testResult = { name: provider.name, result: await api.testProvider(provider.name) };
    } catch (cause) {
      const code = cause instanceof ApiError ? cause.code : 'internal';
      testResult = { name: provider.name, result: { ok: false, code, message: cause instanceof ApiError ? cause.message : '' } };
    } finally {
      testing = null;
    }
  }

  async function confirmDelete() {
    const provider = toDelete;
    if (!provider) return;
    deleting = true;
    try {
      await api.deleteProvider(provider.name);
      toasts.push('success', t('ui.settings.deleted'));
      toDelete = null;
      providers.reload();
    } catch (cause) {
      toasts.push('error', t(errorKey(cause instanceof ApiError ? cause.code : 'internal')));
    } finally {
      deleting = false;
    }
  }
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.settings')} description={t('ui.settings.intro')} />

  {#if !masterKey}
    <div class="banner" role="alert">
      <p class="title">{t('ui.settings.no_master_key_title')}</p>
      <p>{t('ui.settings.no_master_key')}</p>
    </div>
  {/if}

  <Card title={t('ui.settings.appearance')}>
    <div class="row">
      <Select id="locale" label={t('ui.settings.language')} bind:value={localeChoice} options={localeOptions} onchange={changeLocale} />
      <Select id="theme" label={t('ui.settings.theme')} bind:value={themeChoice} options={themeOptions} onchange={changeTheme} />
    </div>
  </Card>

  <Card title={t('ui.settings.providers')}>
    <p class="muted">{t('ui.settings.providers_intro')}</p>
    <AsyncView resource={providers}>
      {#snippet children(list)}
        {#if list.length === 0}
          <EmptyState title={t('ui.settings.no_providers')} message={t('ui.settings.no_providers_hint')} />
        {:else}
          <DataTable
            caption={t('ui.settings.providers_caption')}
            captionId="providers-caption"
            {columns}
            rows={list}
            rowKey={(p) => p.name}
            empty={t('ui.settings.no_providers')}
          >
            {#snippet cell(provider, column)}
              {#if column.id === 'name'}
                {inlineText(provider.name, 80)}
                {#if provider.is_default}<StatusBadge kind="state" value="active" /> <span class="visually-hidden">{t('ui.settings.default')}</span>{/if}
              {:else if column.id === 'model'}
                {inlineText(provider.model, 80)}
              {:else if column.id === 'url'}
                <code>{inlineText(provider.base_url, 120)}</code>
              {:else if column.id === 'key'}
                {provider.has_key ? t('ui.settings.key_stored') : t('ui.settings.key_none')}
              {:else}
                <div class="row">
                  <Button onclick={() => runTest(provider)} busy={testing === provider.name} busyLabel={t('ui.settings.testing')}>
                    {t('ui.settings.test')}
                  </Button>
                  <Button variant="danger" onclick={() => (toDelete = provider)}>{t('ui.common.delete')}</Button>
                </div>
              {/if}
            {/snippet}
          </DataTable>
        {/if}
      {/snippet}
    </AsyncView>

    <div role="status" aria-live="polite">
      {#if testResult}
        {#if testResult.result.ok}
          <p class="ok">
            <StatusBadge kind="state" value="ok" />
            {t('ui.settings.test_ok', {
              name: inlineText(testResult.name, 60),
              ms: testResult.result.latency_ms,
              model: inlineText(testResult.result.model, 80),
            })}
          </p>
        {:else}
          <ErrorNotice code={testResult.result.code} detail={testResult.result.message} />
        {/if}
      {/if}
    </div>
  </Card>

  <Card title={t('ui.settings.add_provider')}>
    <ProviderForm
      masterKeyConfigured={masterKey}
      isFirst={providers.data?.length === 0}
      hasDefault={providers.data?.some((p) => p.is_default) ?? false}
      onsaved={() => { toasts.push('success', t('ui.settings.saved')); providers.reload(); }} />
  </Card>

  <Card title={t('ui.settings.session')}>
    <p>{t('ui.settings.signed_in', { name: inlineText(session.info?.user.name ?? '', 80), org: inlineText(session.info?.org.name ?? '', 80) })}</p>
    <p class="muted">{t('ui.settings.version', { version: inlineText(session.info?.version ?? '', 40) })}</p>
    <div class="row"><Button onclick={() => session.signOut()}>{t('ui.settings.sign_out')}</Button></div>
  </Card>
</div>

{#if toDelete}
  <ConfirmDialog
    title={t('ui.settings.delete_title')}
    message={t('ui.settings.delete_message', { name: inlineText(toDelete.name, 80) })}
    confirmLabel={t('ui.common.delete')}
    cancelLabel={t('ui.common.cancel')}
    busy={deleting}
    onconfirm={confirmDelete}
    oncancel={() => (toDelete = null)}
  />
{/if}

<style>
  .banner {
    padding: var(--nv-space-4);
    border: var(--nv-border-width) solid var(--nv-warning);
    border-inline-start-width: var(--nv-space-1);
    border-radius: var(--nv-radius);
    background: var(--nv-surface);
  }
  .title {
    font-weight: 700;
    color: var(--nv-warning);
  }
</style>
