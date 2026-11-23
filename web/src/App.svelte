<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from './lib/i18n.svelte';
  import { router } from './lib/router.svelte';
  import { session } from './lib/session.svelte';
  import { theme } from './lib/theme.svelte';
  import Spinner from './components/Spinner.svelte';
  import SignIn from './pages/SignIn.svelte';
  import Shell from './Shell.svelte';

  onMount(() => {
    theme.init();
    const stop = router.start();
    const stopWatching = session.watchFragment();
    void session.start();
    return () => {
      stop();
      stopWatching();
    };
  });
</script>

{#if session.phase === 'starting'}
  <div class="boot"><Spinner label={t('ui.common.loading')} /></div>
{:else if session.phase === 'ready'}
  <Shell />
{:else}
  <SignIn />
{/if}

<style>
  .boot {
    display: grid;
    place-items: center;
    min-height: 100vh;
  }
</style>
