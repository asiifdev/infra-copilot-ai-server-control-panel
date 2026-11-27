<script lang="ts">
  import { t } from '../lib/i18n.svelte';
</script>

<!-- Placeholder shapes are drawn with classes only (the CSP forbids inline styles). One polite
     status announces the loading once; the shapes themselves are hidden from assistive technology. -->
<div class="skeleton" data-loading role="status">
  <span class="visually-hidden">{t('ui.common.loading')}</span>
  <div class="nv-skel-shapes" aria-hidden="true">
    <div class="nv-skel-bar nv-w-40"></div>
    <div class="nv-skel-bar nv-w-100"></div>
    <div class="nv-skel-bar nv-w-100"></div>
    <div class="nv-skel-bar nv-w-80"></div>
  </div>
</div>
