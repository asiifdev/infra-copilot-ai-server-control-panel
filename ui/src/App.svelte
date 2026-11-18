<script lang="ts">
  import './app.css';
  import { LOCALES, translate, type Locale, type MessageKey } from './i18n';
  import { makeRows, riskKey, statusKey } from './data';

  let locale = $state<Locale>('en');
  const rows = $derived(makeRows(1000));
  const t = (key: MessageKey): string => translate(locale, key);

  $effect(() => {
    document.documentElement.lang = locale;
  });
</script>

<header>
  <h1>{t('app.title')}</h1>
  <p>{t('app.subtitle')}</p>
  <label>
    {t('lang.label')}
    <select data-testid="lang-switch" bind:value={locale}>
      {#each LOCALES as code (code)}
        <option value={code}>{t(`lang.${code}`)}</option>
      {/each}
    </select>
  </label>
</header>

<main>
  <table data-testid="node-table">
    <caption>{t('table.caption')}</caption>
    <thead>
      <tr>
        <th scope="col">{t('table.col.name')}</th>
        <th scope="col">{t('table.col.status')}</th>
        <th scope="col">{t('table.col.risk')}</th>
        <th scope="col">{t('table.col.lastSeen')}</th>
      </tr>
    </thead>
    <tbody>
      {#each rows as row (row.id)}
        <tr>
          <td>{row.name}</td>
          <td class="status-{row.status}">{t(statusKey(row.status))}</td>
          <td>{t(riskKey(row.risk))}</td>
          <td>{row.lastSeenMinutes} {t('time.minutes')}</td>
        </tr>
      {/each}
    </tbody>
  </table>
</main>

<footer>
  <p>{rows.length} {t('footer.count')}</p>
</footer>
