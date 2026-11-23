<script lang="ts">
  interface Props {
    /** Localized text announced to assistive technology. */
    label: string;
    small?: boolean;
  }

  let { label, small = false }: Props = $props();
</script>

<span class={['spinner', small && 'small']} role="status">
  <span class="ring" aria-hidden="true"></span>
  <span class="visually-hidden">{label}</span>
</span>

<style>
  .spinner {
    display: inline-flex;
    align-items: center;
  }
  .ring {
    width: var(--nv-space-5);
    height: var(--nv-space-5);
    border: var(--nv-focus-width) solid var(--nv-border-strong);
    border-top-color: var(--nv-primary);
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
  }
  .small .ring {
    width: var(--nv-space-4);
    height: var(--nv-space-4);
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
