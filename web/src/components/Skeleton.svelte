<script lang="ts">
  import { t } from '../lib/i18n.svelte';
</script>

<!-- Placeholder shapes are drawn with classes only (the CSP forbids inline styles). One polite
     status announces the loading once; the shapes themselves are hidden from assistive technology. -->
<div class="skeleton" data-loading role="status">
  <span class="visually-hidden">{t('ui.common.loading')}</span>
  <div class="shapes" aria-hidden="true">
    <div class="bar w-40"></div>
    <div class="bar w-100"></div>
    <div class="bar w-100"></div>
    <div class="bar w-80"></div>
  </div>
</div>

<style>
  .shapes {
    display: flex;
    flex-direction: column;
    gap: var(--nv-space-3);
  }
  .bar {
    height: 1rem;
    border-radius: var(--nv-radius-xs);
    background: linear-gradient(90deg, var(--nv-surface-2) 0%, var(--nv-border) 50%, var(--nv-surface-2) 100%);
    background-size: 200% 100%;
    animation: shimmer 1.4s ease-in-out infinite;
  }
  .w-40 {
    width: 40%;
  }
  .w-80 {
    width: 80%;
  }
  .w-100 {
    width: 100%;
  }
  @keyframes shimmer {
    from {
      background-position: 100% 0;
    }
    to {
      background-position: -100% 0;
    }
  }
</style>
