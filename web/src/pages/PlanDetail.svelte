<script lang="ts">
  import { api } from '../lib/api';
  import { describeData } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { hrefFor } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import AsyncView from '../components/AsyncView.svelte';
  import Card from '../components/Card.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TextBlock from '../components/TextBlock.svelte';

  interface Props {
    name: string;
  }

  let { name }: Props = $props();

  const plan = useResource((signal) => api.plan(name, signal));

  // Literal keys on purpose: the key-coverage test finds them.
  const GATE_LABELS: Record<string, string> = {
    open_critical: 'ui.plan.gate_open_critical',
    accepted_critical: 'ui.plan.gate_accepted_critical',
    important: 'ui.plan.gate_important',
    info: 'ui.plan.gate_info',
    inventory_incomplete: 'ui.plan.gate_inventory_incomplete',
    cutover_allowed: 'ui.plan.gate_cutover_allowed',
  };

  function gateValue(value: unknown): string {
    if (typeof value === 'boolean') return value ? t('ui.common.yes') : t('ui.common.no');
    if (Array.isArray(value)) return String(value.length);
    if (typeof value === 'number' || typeof value === 'string') return inlineText(value, 80);
    return inlineText(describeData(value, 200), 200);
  }
</script>

<div class="stack">
  <PageHeader title={t('ui.plan.title', { name: inlineText(name, 80) })}>
    <a href={hrefFor({ name: 'plans' })}>{t('ui.plan.back')}</a>
  </PageHeader>
  <p class="draft-note"><StatusBadge kind="state" value="draft" /> {t('ui.plans.draft_note')}</p>

  <AsyncView resource={plan}>
    {#snippet children(detail)}
      <p>{t('ui.plan.status')} <StatusBadge kind="state" value={detail.status} /></p>
      <Card title={t('ui.plan.text')}>
        <TextBlock text={detail.rendered} mono />
      </Card>
      <Card title={t('ui.plan.steps')}>
        {#if detail.steps.length === 0}
          <p class="muted">{t('ui.plan.no_steps')}</p>
        {:else}
          <ol class="steps">
            {#each detail.steps as step, index (index)}
              <li><TextBlock text={describeData(step, 600)} mono /></li>
            {/each}
          </ol>
        {/if}
      </Card>
      <Card title={t('ui.plan.gates')}>
        <dl class="facts">
          {#each Object.entries(detail.gates) as [key, value] (key)}
            <dt>{GATE_LABELS[key] ? t(GATE_LABELS[key]) : inlineText(key, 60)}</dt>
            <dd>{gateValue(value)}</dd>
          {/each}
        </dl>
      </Card>
      <p class="muted">{t('ui.plans.no_approval')}</p>
    {/snippet}
  </AsyncView>
</div>

<style>
  .steps {
    margin: 0;
    padding-inline-start: var(--nv-space-5);
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
  }
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
  }
</style>
