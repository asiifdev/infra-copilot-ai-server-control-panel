<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import type { ProposalView } from '../lib/types';
  import { inlineText } from '../lib/text';
  import Card from './Card.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import TextBlock from './TextBlock.svelte';

  interface Props {
    proposal: ProposalView;
  }

  let { proposal }: Props = $props();
</script>

<!-- Read-only by design: approving or running a proposal arrives with milestone M4. -->
<Card title={inlineText(proposal.action, 120)} level={3}>
  {#snippet actions()}
    <StatusBadge kind="state" value={proposal.status} />
    {#if proposal.tier !== undefined}<StatusBadge kind="tier" value={proposal.tier} />{/if}
  {/snippet}
  <dl class="facts">
    <dt>{t('ui.proposal.reason')}</dt>
    <dd><TextBlock text={proposal.reason} /></dd>
    <dt>{t('ui.proposal.impact')}</dt>
    <dd><TextBlock text={proposal.impact} /></dd>
  </dl>
  <p class="muted note">{t('ui.proposal.readonly')}</p>
</Card>

<style>
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--nv-space-2) var(--nv-space-4);
    margin: 0;
  }
  dt {
    font-weight: 600;
  }
  dd {
    margin: 0;
    min-width: 0;
  }
  .note {
    font-size: var(--nv-text-sm);
  }
</style>
