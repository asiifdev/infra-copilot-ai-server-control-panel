<script lang="ts">
  import { api } from '../lib/api';
  import { formatDateTime } from '../lib/format';
  import { i18n, t } from '../lib/i18n.svelte';
  import { useResource } from '../lib/resource.svelte';
  import { inlineText } from '../lib/text';
  import AsyncView from '../components/AsyncView.svelte';
  import Card from '../components/Card.svelte';
  import EmptyState from '../components/EmptyState.svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Select from '../components/Select.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TextBlock from '../components/TextBlock.svelte';

  let minSeverity = $state('');
  const FINDING_LIMIT = 200;

  const findings = useResource((signal) => api.findings(minSeverity || undefined, FINDING_LIMIT, signal));

  const options = $derived([
    { value: '', label: t('ui.findings.all') },
    { value: 'important', label: t('ui.findings.important_up') },
    { value: 'critical', label: t('ui.findings.critical_only') },
  ]);
</script>

<div class="stack">
  <PageHeader title={t('ui.nav.findings')} description={t('ui.findings.intro')} />
  <Select label={t('ui.findings.filter')} id="severity-filter" bind:value={minSeverity} {options} />

  <AsyncView resource={findings}>
    {#snippet children(view)}
      {#if view.comparison}
        <p class="muted">
          {t('ui.findings.comparison', {
            source: inlineText(view.comparison.source, 80),
            target: inlineText(view.comparison.target, 80),
            time: formatDateTime(view.comparison.created_at, i18n.locale) ?? inlineText(view.comparison.created_at, 40),
          })}
        </p>
      {/if}
      {#if view.findings.length === 0}
        <EmptyState title={t('ui.findings.empty')} message={t('ui.findings.empty_hint')} />
      {:else}
        <ul class="list" aria-label={t('ui.findings.list_label')}>
          {#each view.findings as finding, index (`${finding.rule_id}:${finding.subject.kind}:${finding.subject.key}:${index}`)}
            <li>
              <details class="finding">
                <summary>
                  <StatusBadge kind="severity" value={finding.severity} />
                  <span class="title">{inlineText(finding.title, 200)}</span>
                </summary>
                <div class="stack body">
                  <p class="muted">
                    {t('ui.findings.subject', {
                      kind: inlineText(finding.subject.kind, 40),
                      key: inlineText(finding.subject.key, 120),
                    })}
                  </p>
                  <Card title={t('ui.findings.explanation')} level={3}><TextBlock text={finding.explanation} /></Card>
                  <Card title={t('ui.findings.advice')} level={3}><TextBlock text={finding.advice} /></Card>
                  {#if finding.note}
                    <Card title={t('ui.findings.note')} level={3}><TextBlock text={finding.note} /></Card>
                  {/if}
                  {#if finding.evidence.length > 0}
                    <Card title={t('ui.findings.evidence')} level={3}>
                      <ul>
                        {#each finding.evidence as item, i (i)}
                          <li><code>{inlineText(item.kind, 40)}: {inlineText(item.key, 120)}</code></li>
                        {/each}
                      </ul>
                    </Card>
                  {/if}
                </div>
              </details>
            </li>
          {/each}
        </ul>
      {/if}
    {/snippet}
  </AsyncView>
</div>

<style>
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-2);
  }
  .finding {
    background: var(--nv-surface);
    border: var(--nv-border-width) solid var(--nv-border);
    border-radius: var(--nv-radius);
    padding: var(--nv-space-3);
  }
  summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--nv-space-3);
    min-height: var(--nv-target);
    cursor: pointer;
  }
  .title {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .body {
    margin-top: var(--nv-space-3);
  }
</style>
