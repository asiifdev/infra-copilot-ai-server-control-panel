// Maps the fixed error codes of the server to catalog keys. Literal keys on purpose: the
// key-coverage test finds them. Every code of the server's table (crates/server/src/errors.rs)
// has a line here and a test reads that table, so a new server code fails the build until it is
// mapped. Several server codes share one message. A code the UI does not know falls back to a
// generic message, and the text the server sent is only ever shown as an extra detail line,
// rendered as text.

const MESSAGE_KEYS: Record<string, string> = {
  // Guards and sign-in.
  invalid_token: 'ui.error.invalid_token',
  unauthenticated: 'ui.error.unauthorized',
  unauthorized: 'ui.error.unauthorized',
  forbidden: 'ui.error.forbidden',
  bad_host: 'ui.error.forbidden',
  bad_origin: 'ui.error.forbidden',
  csrf_header: 'ui.error.forbidden',
  bad_content_type: 'ui.error.bad_request',
  body_too_large: 'ui.error.payload_too_large',
  payload_too_large: 'ui.error.payload_too_large',
  rate_limited: 'ui.error.rate_limited',
  // Requests.
  not_found: 'ui.error.not_found',
  method_not_allowed: 'ui.error.bad_request',
  bad_request: 'ui.error.bad_request',
  invalid_input: 'ui.error.invalid_input',
  invalid_provider: 'ui.error.invalid_input',
  master_key_missing: 'ui.error.master_key_missing',
  busy: 'ui.error.busy',
  turn_busy: 'ui.error.busy',
  already_exists: 'ui.error.already_exists',
  default_exists: 'ui.error.default_exists',
  provider_in_use: 'ui.error.provider_in_use',
  no_provider: 'ui.error.no_provider',
  conversation_archived: 'ui.error.conversation_archived',
  conversation_not_found: 'ui.error.not_found',
  provider_not_found: 'ui.error.not_found',
  message_empty: 'ui.error.message_empty',
  message_too_large: 'ui.error.message_too_large',
  message_invalid: 'ui.error.bad_request',
  timeout: 'ui.error.timeout',
  network: 'ui.error.network',
  internal: 'ui.error.internal',
  store_failed: 'ui.error.internal',
  bad_response: 'ui.error.bad_response',
  // The AI provider and the turn.
  provider_unreachable: 'ui.error.provider_unreachable',
  provider_auth: 'ui.error.provider_auth',
  provider_error: 'ui.error.provider_error',
  ai_auth: 'ui.error.provider_auth',
  ai_transport: 'ui.error.provider_unreachable',
  provider_timeout: 'ui.error.provider_unreachable',
  provider_client: 'ui.error.provider_unreachable',
  ai_quota: 'ui.error.ai_quota',
  ai_rate_limited: 'ui.error.ai_rate_limited',
  ai_circuit_open: 'ui.error.ai_circuit_open',
  ai_unsupported: 'ui.error.provider_error',
  ai_empty: 'ui.error.provider_error',
  ai_config: 'ui.error.provider_error',
  ai_server: 'ui.error.provider_error',
  ai_rejected: 'ui.error.provider_error',
  ai_malformed: 'ui.error.provider_error',
  ai_other: 'ui.error.provider_error',
  provider_config: 'ui.error.provider_error',
  provider_key_missing: 'ui.error.provider_auth',
  provider_key_mismatch: 'ui.error.provider_auth',
  turn_claim_lost: 'ui.error.busy',
  token_budget_exceeded: 'ui.error.provider_error',
  no_answer: 'ui.error.provider_error',
};

const ADVICE_KEYS: Record<string, string> = {
  busy: 'ui.advice.busy',
  turn_busy: 'ui.advice.busy',
  turn_claim_lost: 'ui.advice.busy',
  master_key_missing: 'ui.advice.master_key_missing',
  provider_key_mismatch: 'ui.advice.master_key_missing',
  no_provider: 'ui.advice.no_provider',
  provider_unreachable: 'ui.advice.provider_unreachable',
  ai_transport: 'ui.advice.provider_unreachable',
  provider_timeout: 'ui.advice.provider_unreachable',
  provider_auth: 'ui.advice.provider_auth',
  ai_auth: 'ui.advice.provider_auth',
  provider_key_missing: 'ui.advice.provider_auth',
  provider_error: 'ui.advice.provider_error',
  ai_unsupported: 'ui.advice.provider_error',
  ai_rejected: 'ui.advice.provider_error',
  ai_malformed: 'ui.advice.provider_error',
  ai_config: 'ui.advice.provider_error',
  provider_config: 'ui.advice.provider_error',
  network: 'ui.advice.network',
  unauthenticated: 'ui.advice.unauthorized',
  unauthorized: 'ui.advice.unauthorized',
};

/** Codes whose advice points to the Settings page. */
const SETTINGS_CODES = new Set([
  'busy',
  'master_key_missing',
  'provider_key_mismatch',
  'no_provider',
  'provider_unreachable',
  'provider_auth',
  'provider_error',
  'provider_timeout',
  'provider_key_missing',
  'provider_config',
  'ai_auth',
  'ai_transport',
  'ai_unsupported',
  'ai_rejected',
  'ai_malformed',
  'ai_config',
]);

/** Every code the UI knows (the drift test compares it with the server's table). */
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
