<script lang="ts" generics="T">
  import type { Snippet } from 'svelte';
  import type { Resource } from '../lib/resource.svelte';
  import ErrorNotice from './ErrorNotice.svelte';
  import Skeleton from './Skeleton.svelte';

  interface Props {
    resource: Resource<T>;
    children: Snippet<[T]>;
  }

  let { resource, children }: Props = $props();
</script>

{#if resource.status === 'error' && resource.error}
  <ErrorNotice code={resource.error.code} detail={resource.error.message} onretry={() => resource.reload()} />
{:else if resource.data !== null}
  <!-- While it reloads the previous data stays in place: no flicker after a filter or a decision. -->
  {@render children(resource.data)}
{:else}
  <Skeleton />
{/if}
