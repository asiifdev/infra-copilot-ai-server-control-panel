<script lang="ts">
  import { adviceKey, errorKey, pointsToSettings } from '../lib/errors';
  import { hrefFor } from '../lib/router.svelte';
  import { t } from '../lib/i18n.svelte';
  import { inlineText } from '../lib/text';
  import Button from './Button.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    /** Fixed error code from the contract (or from an error event). */
    code: string;
    /** Text the server sent with the code: untrusted, shown only as plain detail. */
    detail?: string;
    onretry?: () => void;
  }

  let { code, detail, onretry }: Props = $props();

  const advice = $derived(adviceKey(code));
  const cleanDetail = $derived(detail ? inlineText(detail, 300) : '');
</script>

<div class="nv-err" role="alert">
  <p class="nv-err-title"><Icon name="alert" />{t(errorKey(code))}</p>
  {#if advice}<p>{t(advice)}</p>{/if}
  {#if cleanDetail}<p class="muted nv-err-detail">{t('ui.error.detail', { detail: cleanDetail })}</p>{/if}
  <div class="row">
    {#if pointsToSettings(code)}<a href={hrefFor({ name: 'settings' })}>{t('ui.error.open_settings')}</a>{/if}
    {#if onretry}<Button onclick={onretry}>{t('ui.common.retry')}</Button>{/if}
  </div>
</div>
