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

  let { title, message, targetLabel, confirmLabel, cancelLabel, busy = false, tone = 'danger', onconfirm, oncancel }: Props = $props();
  let typed = $state('');
  const matches = $derived(typed === targetLabel);
</script>

<ConfirmDialog {title} {message} {confirmLabel} {cancelLabel} {busy} {tone} {oncancel} confirmDisabled={!matches} onconfirm={() => onconfirm(typed)}>
  <TextField
    id="confirm-typed-input"
    label={t('ui.proposals.typed_confirm_label', { target: targetLabel })}
    placeholder={t('ui.proposals.typed_confirm_placeholder')}
    bind:value={typed}
    disabled={busy}
    onkeydown={(e) => {
      if (e.key === 'Enter' && matches && !busy) {
        e.preventDefault();
        onconfirm(typed);
      }
    }}
  />
</ConfirmDialog>
