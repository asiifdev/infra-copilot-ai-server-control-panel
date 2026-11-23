// Maps the fixed error codes of the contract to catalog keys. Literal keys on purpose: the
// key-coverage test finds them. A code the UI does not know falls back to a generic message, and
// the text the server sent is only ever shown as an extra detail line, rendered as text.

const MESSAGE_KEYS: Record<string, string> = {
  invalid_token: 'ui.error.invalid_token',
  unauthorized: 'ui.error.unauthorized',
  forbidden: 'ui.error.forbidden',
  not_found: 'ui.error.not_found',
  busy: 'ui.error.busy',
  master_key_missing: 'ui.error.master_key_missing',
  bad_request: 'ui.error.bad_request',
  payload_too_large: 'ui.error.payload_too_large',
  rate_limited: 'ui.error.rate_limited',
  timeout: 'ui.error.timeout',
  network: 'ui.error.network',
  internal: 'ui.error.internal',
  bad_response: 'ui.error.bad_response',
  no_provider: 'ui.error.no_provider',
  provider_unreachable: 'ui.error.provider_unreachable',
  provider_auth: 'ui.error.provider_auth',
  provider_error: 'ui.error.provider_error',
};

const ADVICE_KEYS: Record<string, string> = {
  busy: 'ui.advice.busy',
  master_key_missing: 'ui.advice.master_key_missing',
  no_provider: 'ui.advice.no_provider',
  provider_unreachable: 'ui.advice.provider_unreachable',
  provider_auth: 'ui.advice.provider_auth',
  provider_error: 'ui.advice.provider_error',
  network: 'ui.advice.network',
  unauthorized: 'ui.advice.unauthorized',
};

/** Codes whose advice points to the Settings page. */
const SETTINGS_CODES = new Set(['busy', 'master_key_missing', 'no_provider', 'provider_unreachable', 'provider_auth', 'provider_error']);

export function errorKey(code: string): string {
  return MESSAGE_KEYS[code] ?? 'ui.error.unknown';
}

export function adviceKey(code: string): string | null {
  return ADVICE_KEYS[code] ?? null;
}

export function pointsToSettings(code: string): boolean {
  return SETTINGS_CODES.has(code);
}
