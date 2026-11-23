<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { GATE_LABELS, gateView, SIDE_LABELS, STEP_LABELS, stepRows, type StepRow } from '../lib/plan';
  import { hrefFor } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import AsyncView from '../components/AsyncView.svelte';
  import Card from '../components/Card.svelte';
  import DataTable, { type Column } from '../components/DataTable.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TextBlock from '../components/TextBlock.svelte';

  interface Props {
    name: string;
  }

  let { name }: Props = $props();

  const plan = useResource((signal) => api.plan(name, signal));

  const columns = $derived<Column<StepRow>[]>([
    { id: 'number', header: t('ui.plan.col_number') },
    { id: 'step', header: t('ui.plan.col_step') },
    { id: 'detail', header: t('ui.plan.col_detail') },
    { id: 'after', header: t('ui.plan.col_after') },
  ]);

  function stepName(kind: string): string {
    const key = STEP_LABELS[kind];
    return key ? t(key) : t('ui.plan.step_unknown', { kind: inlineText(kind, 40) });
  }

  function sideName(side: string): string {
    const key = SIDE_LABELS[side];
    return key ? t(key) : inlineText(side, 40);
  }

  function gateName(key: string): string {
    const label = GATE_LABELS[key];
    return label ? t(label) : t('ui.plan.gate_unknown', { key: inlineText(key, 60) });
  }
</script>

<div class="stack">
  <PageHeader title={t('ui.plan.title', { name: inlineText(name, 80) })}>
    <a href={hrefFor({ name: 'plans' })}>{t('ui.plan.back')}</a>
  </PageHeader>
  <p class="draft-note"><StatusBadge kind="state" value="draft" /> {t('ui.plans.draft_note')}</p>

  <AsyncView resource={plan}>
    {#snippet children(detail)}
      {@const rows = stepRows(detail.steps)}
      <p>{t('ui.plan.status')} <StatusBadge kind="state" value={detail.status} /></p>
      {#if detail.source_node && detail.target_node}
        <p>{t('ui.plan.moves', { source: inlineText(detail.source_node, 80), target: inlineText(detail.target_node, 80) })}</p>
      {/if}
      <Card title={t('ui.plan.text')}>
        <TextBlock text={detail.rendered} mono />
      </Card>
      <Card title={t('ui.plan.steps')}>
        {#if rows.length === 0}
          <p class="muted">{t('ui.plan.no_steps')}</p>
        {:else}
          <DataTable
            caption={t('ui.plan.steps_caption')}
            captionId="plan-steps-caption"
            {columns}
            {rows}
            rowKey={(row) => String(row.number)}
            empty={t('ui.plan.no_steps')}
          >
            {#snippet cell(row, column)}
              {#if column.id === 'number'}
                {row.number}
              {:else if column.id === 'step'}
                {stepName(row.kind)}
              {:else if column.id === 'detail'}
                {#if row.side}{sideName(row.side)}{/if}
                {#if row.database}{t('ui.plan.database', { database: inlineText(row.database, 80) })}{/if}
                {#if !row.side && !row.database}<span class="muted">-</span>{/if}
              {:else if row.after.length === 0}
                <span class="muted">{t('ui.plan.first_step')}</span>
              {:else}
                {t('ui.plan.after_steps', { steps: row.after.join(', ') })}
              {/if}
            {/snippet}
          </DataTable>
        {/if}
      </Card>
      <Card title={t('ui.plan.gates')}>
        <dl class="facts">
          {#each Object.entries(detail.gates) as [key, value] (key)}
            {@const view = gateView(value)}
            <dt>{gateName(key)}</dt>
            <dd>
              {#if view.type === 'bool'}
                {view.value ? t('ui.common.yes') : t('ui.common.no')}
              {:else if view.type === 'count'}
                {view.value}
              {:else if view.type === 'findings'}
                {view.items.length}
                {#if view.items.length > 0}
                  <ul class="findings">
                    {#each view.items as item, index (index)}
                      <li>
                        {#if item.reason}
                          {t('ui.plan.gate_finding_reason', {
                            rule: inlineText(item.rule_id, 40),
                            kind: inlineText(item.subject.kind, 40),
                            key: inlineText(item.subject.key, 120),
                            reason: inlineText(item.reason, 200),
                          })}
                        {:else}
                          {t('ui.plan.gate_finding', {
                            rule: inlineText(item.rule_id, 40),
                            kind: inlineText(item.subject.kind, 40),
                            key: inlineText(item.subject.key, 120),
                          })}
                        {/if}
                      </li>
                    {/each}
                  </ul>
                {/if}
              {:else}
                {inlineText(view.value, 120)}
              {/if}
            </dd>
          {/each}
        </dl>
      </Card>
      <p class="muted">{t('ui.plans.no_approval')}</p>
    {/snippet}
  </AsyncView>
</div>

<style>
  .facts {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: var(--nv-space-2) var(--nv-space-4);
    margin: 0;
  }
  dt {
    font-weight: 600;
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .findings {
    margin: var(--nv-space-1) 0 0;
    padding-inline-start: var(--nv-space-5);
    font-size: var(--nv-text-sm);
  }
</style>
