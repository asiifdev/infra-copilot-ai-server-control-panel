<script lang="ts" generics="T">
  import type { Snippet } from 'svelte';
  import type { Resource } from '../lib/resource.svelte';
  import { t } from '../lib/i18n.svelte';
  import ErrorNotice from './ErrorNotice.svelte';
  import Spinner from './Spinner.svelte';

  interface Props {
    resource: Resource<T>;
    children: Snippet<[T]>;
  }

  let { resource, children }: Props = $props();
</script>

{#if resource.status === 'error' && resource.error}
  <ErrorNotice code={resource.error.code} detail={resource.error.message} onretry={() => resource.reload()} />
{:else if resource.status === 'ready' && resource.data !== null}
  {@render children(resource.data)}
{:else}
  <Spinner label={t('ui.common.loading')} />
{/if}
