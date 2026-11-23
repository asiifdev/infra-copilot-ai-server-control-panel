<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import { session } from '../lib/session.svelte';
  import Card from '../components/Card.svelte';
  import ErrorNotice from '../components/ErrorNotice.svelte';

  const failure = $derived(session.failure);
  const invalid = $derived(failure === 'invalid_token');
</script>

<main class="signin" id="main">
  <Card>
    <h1 id="page-title" tabindex="-1">{t('ui.signin.title')}</h1>
    {#if invalid}
      <p role="alert" class="problem">{t('ui.signin.invalid')}</p>
      <p>{t('ui.signin.invalid_hint')}</p>
    {:else if failure}
      <ErrorNotice code={failure} />
    {:else}
      <p>{t('ui.signin.required')}</p>
      <p class="muted">{t('ui.signin.hint')}</p>
    {/if}
  </Card>
</main>

<style>
  .signin {
    max-width: 36rem;
    margin: var(--nv-space-6) auto;
    padding: 0 var(--nv-space-4);
  }
  .problem {
    font-weight: 700;
    color: var(--nv-danger);
  }
</style>
