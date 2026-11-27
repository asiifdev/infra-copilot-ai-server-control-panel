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
