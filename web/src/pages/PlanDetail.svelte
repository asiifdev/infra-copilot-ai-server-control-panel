<script lang="ts">
  import { api } from '../lib/api';
  import { withBusy } from '../lib/errors';
  import { t } from '../lib/i18n.svelte';
  import { GATE_LABELS, gateView, SIDE_LABELS, STEP_LABELS, stepRows } from '../lib/plan';
  import { hrefFor } from '../lib/router.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { session } from '../lib/session.svelte';
  import { inlineText } from '../lib/text';
  import { toasts } from '../lib/toast.svelte';
  import type { PlanDetail as PlanDetailType } from '../lib/types';
  import AsyncView from '../components/AsyncView.svelte';
  import Button from '../components/Button.svelte';
  import Card from '../components/Card.svelte';
  import ConfirmDialog from '../components/ConfirmDialog.svelte';
  import Icon from '../components/Icon.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TextBlock from '../components/TextBlock.svelte';

  interface Props {
    name: string;
  }

  let { name }: Props = $props();

  const plan = useResource((signal) => api.plan(name, signal));
  const executionEnabled = $derived(session.info?.execution_enabled === true);
  const ENABLE_COMMAND = 'infra serve --enable-execution';
  let busy = $state(false);
  let confirming = $state<'approve' | 'run' | null>(null);

  async function planOp(fn: () => Promise<void>) {
    await withBusy((b) => (busy = b), async () => {
      await fn();
      plan.reload();
    });
    confirming = null;
  }

  function approve(detail: PlanDetailType) {
    return planOp(async () => {
      await api.approvePlan(detail.id);
      toasts.push('success', t('ui.plans.approved_toast'));
    });
  }

  function run(detail: PlanDetailType) {
    return planOp(async () => {
      const res = await api.runPlan(detail.id);
      toasts.push(res.status === 'paused' ? 'info' : 'success', t(res.status === 'paused' ? 'ui.plans.gate_blocked_toast' : 'ui.plans.run_ok_toast'));
    });
  }

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

<div class="stack nv-plan">
  <PageHeader title={t('ui.plan.title', { name: inlineText(name, 80) })} eyebrow={t('ui.nav.group_work')}>
    <Button variant="ghost" icon="return" href={hrefFor({ name: 'plans' })}>{t('ui.plan.back')}</Button>
  </PageHeader>
  <p class="row"><StatusBadge kind="state" value="draft" /> {t('ui.plans.draft_note')}</p>

  <AsyncView resource={plan}>
    {#snippet children(detail)}
      {@const rows = stepRows(detail.steps)}
      {@const canApprove = detail.status === 'draft' || detail.status === 'reviewed'}
      {@const canRun = detail.status === 'approved' || detail.status === 'paused'}
      <div class="row">
        <p>{t('ui.plan.status')} <StatusBadge kind="state" value={detail.status} /></p>
        {#if canApprove}
          <Button variant="primary" icon="check" onclick={() => (confirming = 'approve')} disabled={busy}>{t('ui.plans.approve')}</Button>
        {/if}
        {#if canRun}
          <Button variant="primary" icon="arrow" onclick={() => (confirming = 'run')} disabled={busy || !executionEnabled}>{t('ui.plans.run')}</Button>
        {/if}
      </div>
      {#if canRun && !executionEnabled}
        <p class="muted">{t('ui.proposals.run_off')} <code>{ENABLE_COMMAND}</code>.</p>
      {/if}
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
                  <p class="step-title">
                    <span class="visually-hidden">{t('ui.plan.col_number')} {row.number}: </span>{stepName(row.kind)}
                    <StatusBadge kind="state" value={row.status} />
                  </p>
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
                          {@const base = { rule: inlineText(item.rule_id, 40), kind: inlineText(item.subject.kind, 40), key: inlineText(item.subject.key, 120) }}
                          <li>
                            {item.reason
                              ? t('ui.plan.gate_finding_reason', { rule: base.rule, kind: base.kind, key: base.key, reason: inlineText(item.reason, 200) })
                              : t('ui.plan.gate_finding', base)}
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
      {#if confirming}
        {@const isApprove = confirming === 'approve'}
        <ConfirmDialog
          title={t(isApprove ? 'ui.plans.approve' : 'ui.plans.run')}
          message={t(isApprove ? 'ui.plans.approve_confirm' : 'ui.plans.run_confirm')}
          confirmLabel={t(isApprove ? 'ui.plans.approve' : 'ui.plans.run')}
          cancelLabel={t('ui.common.cancel')}
          tone="primary"
          {busy}
          onconfirm={() => (isApprove ? approve(detail) : run(detail))}
          oncancel={() => (confirming = null)}
        />
      {/if}
    {/snippet}
  </AsyncView>
</div>
