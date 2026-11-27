// Maps the fixed error codes of the server to catalog keys. Literal keys on purpose: the
// key-coverage test finds them. Every code of the server's table (crates/server/src/errors.rs)
// has an entry here and a test reads that table.

import { t } from './i18n.svelte';
import { toasts } from './toast.svelte';

function expand(groups: Record<string, string[]>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, codes] of Object.entries(groups)) {
    for (const code of codes) out[code] = key;
  }
  return out;
}

const MESSAGE_KEYS = expand({
  'ui.error.invalid_token': ['invalid_token'],
  'ui.error.unauthorized': ['unauthenticated', 'unauthorized'],
  'ui.error.forbidden': ['forbidden', 'bad_host', 'bad_origin', 'csrf_header'],
  'ui.error.bad_request': ['bad_content_type', 'method_not_allowed', 'bad_request', 'message_invalid', 'plan_not_approvable', 'plan_not_approved', 'credentials_not_accepted', 'logs_unavailable'],
  'ui.error.payload_too_large': ['body_too_large', 'payload_too_large'],
  'ui.error.rate_limited': ['rate_limited'],
  'ui.error.not_found': ['not_found', 'conversation_not_found', 'provider_not_found'],
  'ui.error.invalid_input': ['invalid_input', 'invalid_provider', 'bad_credentials', 'invalid_service', 'invalid_test_domain', 'invalid_upstream_port', 'invalid_ssh_target'],
  'ui.error.master_key_missing': ['master_key_missing'],
  'ui.error.busy': ['busy', 'turn_busy', 'turn_claim_lost', 'execution_running'],
  'ui.error.already_exists': ['already_exists', 'duplicate_domain'],
  'ui.error.default_exists': ['default_exists'],
  'ui.error.provider_in_use': ['provider_in_use'],
  'ui.error.proposal_decided': ['proposal_decided'],
  'ui.error.proposal_expired': ['proposal_expired'],
  'ui.error.execution_disabled': ['execution_disabled'],
  'ui.error.proposal_not_approved': ['proposal_not_approved', 'approval_required', 'proposal_not_executed'],
  'ui.error.proposal_no_action': ['proposal_no_action'],
  'ui.error.proposal_already_run': ['proposal_already_run'],
  'ui.error.typed_confirmation_mismatch': ['typed_confirmation_mismatch', 'typed_confirmation_required'],
  'ui.error.backup_missing': ['backup_missing'],
  'ui.error.already_rolled_back': ['already_rolled_back'],
  'ui.plans.gate_blocked_toast': ['plan_gate_blocked'],
  'ui.error.action_invalid': ['action_invalid'],
  'ui.error.unsupported_target': ['unsupported_target'],
  'ui.error.no_provider': ['no_provider'],
  'ui.error.conversation_archived': ['conversation_archived'],
  'ui.error.message_empty': ['message_empty'],
  'ui.error.message_too_large': ['message_too_large'],
  'ui.error.timeout': ['timeout'],
  'ui.error.network': ['network'],
  'ui.error.internal': ['internal', 'store_failed'],
  'ui.error.bad_response': ['bad_response'],
  'ui.error.provider_unreachable': ['provider_unreachable', 'ai_transport', 'provider_timeout', 'provider_client'],
  'ui.error.provider_auth': ['provider_auth', 'ai_auth', 'provider_key_missing', 'provider_key_mismatch'],
  'ui.error.provider_error': ['provider_error', 'ai_unsupported', 'ai_empty', 'ai_config', 'ai_server', 'ai_rejected', 'ai_malformed', 'ai_other', 'provider_config', 'token_budget_exceeded', 'no_answer'],
  'ui.error.ai_quota': ['ai_quota'],
  'ui.error.ai_rate_limited': ['ai_rate_limited'],
  'ui.error.ai_circuit_open': ['ai_circuit_open'],
});

const ADVICE_KEYS = expand({
  'ui.advice.busy': ['busy', 'turn_busy', 'turn_claim_lost'],
  'ui.advice.master_key_missing': ['master_key_missing', 'provider_key_mismatch'],
  'ui.advice.no_provider': ['no_provider'],
  'ui.advice.provider_unreachable': ['provider_unreachable', 'ai_transport', 'provider_timeout'],
  'ui.advice.provider_auth': ['provider_auth', 'ai_auth', 'provider_key_missing'],
  'ui.advice.provider_error': ['provider_error', 'ai_unsupported', 'ai_rejected', 'ai_malformed', 'ai_config', 'provider_config'],
  'ui.advice.network': ['network'],
  'ui.advice.unauthorized': ['unauthenticated', 'unauthorized'],
});

const SETTINGS_CODES = new Set([
  'busy', 'master_key_missing', 'provider_key_mismatch', 'no_provider',
  'provider_unreachable', 'provider_auth', 'provider_error', 'provider_timeout',
  'provider_key_missing', 'provider_config', 'ai_auth', 'ai_transport',
  'ai_unsupported', 'ai_rejected', 'ai_malformed', 'ai_config',
]);

export const KNOWN_ERROR_CODES: readonly string[] = Object.keys(MESSAGE_KEYS);

export function errorKey(code: string): string {
  return MESSAGE_KEYS[code] ?? 'ui.error.unknown';
}

export function adviceKey(code: string): string | null {
  return ADVICE_KEYS[code] ?? null;
}

export function pointsToSettings(code: string): boolean {
  return SETTINGS_CODES.has(code);
}

export function toastError(cause: unknown): void {
  const code = cause && typeof cause === 'object' && 'code' in cause && typeof cause.code === 'string' ? cause.code : 'internal';
  toasts.push('error', t(errorKey(code)));
}
