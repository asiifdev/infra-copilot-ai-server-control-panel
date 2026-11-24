<script lang="ts">
  import { api } from '../lib/api';
  import { t } from '../lib/i18n.svelte';
  import { GATE_LABELS, gateView, SIDE_LABELS, STEP_LABELS, stepRows } from '../lib/plan';
  import { hrefFor } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import Icon from '../components/Icon.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TextBlock from '../components/TextBlock.svelte';

  interface Props {
    name: string;
  }

  let { name }: Props = $props();

  const plan = useResource((signal) => api.plan(name, signal));

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
  <PageHeader title={t('ui.plan.title', { name: inlineText(name, 80) })} eyebrow={t('ui.nav.group_work')}>
    <Button variant="ghost" icon="return" href={hrefFor({ name: 'plans' })}>{t('ui.plan.back')}</Button>
  </PageHeader>
  <p class="row"><StatusBadge kind="state" value="draft" /> {t('ui.plans.draft_note')}</p>

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
          <ol class="timeline plain" aria-label={t('ui.plan.steps_caption')}>
            {#each rows as row (row.number)}
              <li class="step">
                <span class="marker num" aria-hidden="true">{row.number}</span>
                <div class="step-body">
                  <p class="step-title"><span class="visually-hidden">{t('ui.plan.col_number')} {row.number}: </span>{stepName(row.kind)}</p>
                  <p class="step-meta muted">
                    {#if row.side}<span class="chip">{sideName(row.side)}</span>{/if}
                    {#if row.database}<span class="chip">{t('ui.plan.database', { database: inlineText(row.database, 80) })}</span>{/if}
                    <span>
                      {#if row.after.length === 0}{t('ui.plan.first_step')}{:else}{t('ui.plan.after_steps', { steps: row.after.join(', ') })}{/if}
                    </span>
                  </p>
                </div>
              </li>
            {/each}
          </ol>
        {/if}
      </Card>
      <Card title={t('ui.plan.gates')}>
        <ul class="gates plain">
          {#each Object.entries(detail.gates) as [key, value] (key)}
            {@const view = gateView(value)}
            <li class="gate">
              <Icon name="shield" />
              <div class="gate-body">
                <p class="gate-name">{gateName(key)}</p>
                <div class="gate-value">
                  {#if view.type === 'bool'}
                    <span class="chip">{view.value ? t('ui.common.yes') : t('ui.common.no')}</span>
                  {:else if view.type === 'count'}
                    <span class="num">{view.value}</span>
                  {:else if view.type === 'findings'}
                    <span class="num">{view.items.length}</span>
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
                </div>
              </div>
            </li>
          {/each}
        </ul>
      </Card>
      <p class="muted">{t('ui.plans.no_approval')}</p>
    {/snippet}
  </AsyncView>
</div>

<style>
  .timeline {
    display: flex;
    flex-direction: column;
  }
  .step {
    position: relative;
    display: flex;
    gap: var(--nv-space-4);
    padding-block-end: var(--nv-space-4);
  }
  /* The line that joins one step to the next. */
  .step:not(:last-child)::before {
    content: '';
    position: absolute;
    inset-block: 2rem 0;
    inset-inline-start: calc(1rem - 1px);
    width: 2px;
    background: var(--nv-border-strong);
  }
  .marker {
    flex: none;
    display: grid;
    place-items: center;
    width: 2rem;
    height: 2rem;
    border: 2px solid var(--nv-primary);
    border-radius: var(--nv-radius-pill);
    background: var(--nv-surface);
    color: var(--nv-text);
    font-size: var(--nv-text-sm);
    font-weight: 700;
  }
  .step-body {
    min-width: 0;
    padding-block-start: 0.2rem;
  }
  .step-title {
    font-weight: 650;
  }
  .step-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--nv-space-1) var(--nv-space-2);
    font-size: var(--nv-text-sm);
  }
  .chip {
    padding: 0 var(--nv-space-2);
    border-radius: var(--nv-radius-pill);
    background: var(--nv-surface-2);
    color: var(--nv-text);
    font-weight: 600;
  }
  .gates {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
  }
  .gate {
    display: flex;
    gap: var(--nv-space-3);
    padding: var(--nv-space-3) var(--nv-space-4);
    border: var(--nv-border-width) solid var(--nv-border);
    border-inline-start: var(--nv-space-1) solid var(--nv-info);
    border-radius: var(--nv-radius-sm);
    background: var(--nv-tint-info);
    color: var(--nv-text);
  }
  .gate-body {
    min-width: 0;
  }
  .gate-name {
    font-weight: 650;
  }
  .gate-value {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--nv-space-2);
    overflow-wrap: anywhere;
  }
  .findings {
    flex-basis: 100%;
    margin: var(--nv-space-1) 0 0;
    padding-inline-start: var(--nv-space-5);
    font-size: var(--nv-text-sm);
  }
</style>
