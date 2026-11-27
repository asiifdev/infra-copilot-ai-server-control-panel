<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import TextField from './TextField.svelte';

  interface Props {
    title: string;
    message: string;
    targetLabel: string;
    confirmLabel: string;
    cancelLabel: string;
    busy?: boolean;
    tone?: 'danger' | 'primary';
    onconfirm: (typed: string) => void;
    oncancel: () => void;
  }

  let { targetLabel, onconfirm, ...rest }: Props = $props();
  let typed = $state('');
  const matches = $derived(typed === targetLabel);
</script>

<ConfirmDialog {...rest} confirmDisabled={!matches} onconfirm={() => onconfirm(typed)}>
  <TextField
    id="confirm-typed-input"
    label={t('ui.proposals.typed_confirm_label', { target: targetLabel })}
    placeholder={t('ui.proposals.typed_confirm_placeholder')}
    bind:value={typed}
    disabled={rest.busy}
    onkeydown={(e) => {
      if (e.key === 'Enter' && matches && !rest.busy) {
        e.preventDefault();
        onconfirm(typed);
      }
    }}
  />
</ConfirmDialog>
