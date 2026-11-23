<script lang="ts">
  import { api, ApiError } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import type { ProviderInput } from '../lib/types';
  import Button from './Button.svelte';
  import Checkbox from './Checkbox.svelte';
  import ErrorNotice from './ErrorNotice.svelte';
  import TextField from './TextField.svelte';

  interface Props {
    /** False when the server has no master key: a key cannot be stored then. */
    masterKeyConfigured: boolean;
    /** No provider exists yet: the first one becomes the default, so the box starts checked. */
    isFirst: boolean;
    /** A default exists already: the server refuses a second one, so the box is off and locked. */
    hasDefault: boolean;
    onsaved: () => void;
  }

  let { masterKeyConfigured, isFirst, hasDefault, onsaved }: Props = $props();

  let name = $state('');
  let baseUrl = $state('');
  let model = $state('');
  // The API key lives here only until the request is sent; it is cleared after every submit
  // and nothing ever reads it back from the server.
  let key = $state('');
  let params = $state('');
  // null = the person has not touched the box: it follows whether this is the first provider.
  let defaultChoice = $state<boolean | null>(null);
  const isDefault = $derived(!hasDefault && (defaultChoice ?? isFirst));
  let saving = $state(false);
  let failure = $state.raw<{ code: string; detail: string } | null>(null);
  let errors = $state<{ name?: string; baseUrl?: string; model?: string; params?: string; key?: string }>({});

  const NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;

  function validate(): { input: ProviderInput | null; errors: typeof errors } {
    const found: typeof errors = {};
    const trimmedName = name.trim();
    if (!NAME_PATTERN.test(trimmedName)) found.name = t('ui.settings.err_name');
    let url: URL | null = null;
    try {
      url = new URL(baseUrl.trim());
    } catch {
      url = null;
    }
    if (!url || (url.protocol !== 'https:' && url.protocol !== 'http:')) found.baseUrl = t('ui.settings.err_url');
    if (model.trim() === '') found.model = t('ui.settings.err_model');
    let parsed: Record<string, unknown> | undefined;
    if (params.trim() !== '') {
      try {
        const value: unknown = JSON.parse(params);
        if (typeof value !== 'object' || value === null || Array.isArray(value)) throw new Error('shape');
        parsed = value as Record<string, unknown>;
      } catch {
        found.params = t('ui.settings.err_params');
      }
    }
    if (key !== '' && !masterKeyConfigured) found.key = t('ui.settings.err_key_blocked');
    if (Object.keys(found).length > 0) return { input: null, errors: found };
    const input: ProviderInput = {
      name: trimmedName,
      base_url: baseUrl.trim(),
      model: model.trim(),
      is_default: isDefault,
    };
    if (parsed) input.params = parsed;
    if (key !== '') input.key = key;
    return { input, errors: found };
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    failure = null;
    const result = validate();
    errors = result.errors;
    if (!result.input) return;
    saving = true;
    try {
      await api.saveProvider(result.input);
      name = '';
      baseUrl = '';
      model = '';
      params = '';
      defaultChoice = null;
      onsaved();
    } catch (cause) {
      failure =
        cause instanceof ApiError
          ? { code: cause.code, detail: cause.message }
          : { code: 'internal', detail: '' };
    } finally {
      // Success or failure: the secret does not stay in the page.
      key = '';
      result.input.key = undefined;
      saving = false;
    }
  }
</script>

<form class="stack" onsubmit={submit} novalidate>
  <p class="muted">{t('ui.settings.add_hint')}</p>
  <TextField id="provider-name" label={t('ui.settings.name')} bind:value={name} required error={errors.name} />
  <TextField
    id="provider-url"
    label={t('ui.settings.base_url')}
    bind:value={baseUrl}
    type="url"
    required
    hint={t('ui.settings.base_url_hint')}
    error={errors.baseUrl}
  />
  <TextField id="provider-model" label={t('ui.settings.model')} bind:value={model} required error={errors.model} />
  <TextField
    id="provider-key"
    label={t('ui.settings.api_key')}
    bind:value={key}
    type="password"
    autocomplete="off"
    hint={masterKeyConfigured ? t('ui.settings.api_key_hint') : t('ui.settings.api_key_blocked')}
    error={errors.key}
    disabled={!masterKeyConfigured}
  />
  <TextField
    id="provider-params"
    label={t('ui.settings.params')}
    bind:value={params}
    multiline
    placeholder={t('ui.settings.params_placeholder')}
    hint={t('ui.settings.params_hint')}
    error={errors.params}
  />
  <Checkbox
    id="provider-default"
    label={t('ui.settings.is_default')}
    bind:checked={() => isDefault, (value) => (defaultChoice = value)}
    disabled={hasDefault}
    hint={hasDefault ? t('ui.settings.default_exists_hint') : isFirst ? t('ui.settings.default_first_hint') : undefined}
  />
  {#if failure}<ErrorNotice code={failure.code} detail={failure.detail} />{/if}
  <div class="row">
    <Button type="submit" variant="primary" busy={saving} busyLabel={t('ui.common.saving')}>{t('ui.settings.save')}</Button>
  </div>
</form>
