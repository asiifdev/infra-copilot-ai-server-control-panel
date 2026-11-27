<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { session } from '../lib/session.svelte';
  import Icon from '../components/Icon.svelte';
  import Card from '../components/Card.svelte';
  import ErrorNotice from '../components/ErrorNotice.svelte';

  const failure = $derived(session.failure);
  const invalid = $derived(failure === 'invalid_token');
</script>

<main class="nv-signin" id="main">
  <Card>
    <div class="nv-signin-lock"><Icon name="shield" size="xl" /></div>
    <h1 id="page-title" tabindex="-1">{t('ui.signin.title')}</h1>
    {#if invalid}
      <p role="alert" class="nv-signin-problem">{t('ui.signin.invalid')}</p>
      <p>{t('ui.signin.invalid_hint')}</p>
    {:else if failure}
      <ErrorNotice code={failure} />
    {:else}
      <p>{t('ui.signin.required')}</p>
      <p class="muted">{t('ui.signin.hint')}</p>
    {/if}
  </Card>
</main>
