//! API errors: a fixed code, a status and a localized message from the catalog. Nothing here
//! ever carries text from a server, a model, a URL or a key.
//!
//! A handler returns an [`ApiError`], which becomes an empty response marked with [`ErrorMark`];
//! the guard layer (the one place that knows the request language) renders the JSON body. Codes
//! that describe a provider failure are also used as advice in the provider test and in the
//! `error` event of a turn, through [`advice`].

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use i18n::{Catalog, Locale, MessageKey};

/// One row of the table of known codes.
struct Entry {
    code: &'static str,
    status: StatusCode,
    key: MessageKey,
}

macro_rules! entry {
    ($code:literal, $status:expr, $key:literal) => {
        Entry {
            code: $code,
            status: $status,
            key: MessageKey::new($key),
        }
    };
}

/// Every code the API can return, with its HTTP status (used when it is an HTTP error) and the
/// catalog key of its message.
const TABLE: &[Entry] = &[
    entry!("bad_host", StatusCode::FORBIDDEN, "api.error.bad_host"),
    entry!("bad_origin", StatusCode::FORBIDDEN, "api.error.bad_origin"),
    entry!(
        "csrf_header",
        StatusCode::FORBIDDEN,
        "api.error.csrf_header"
    ),
    entry!(
        "bad_content_type",
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "api.error.bad_content_type"
    ),
    entry!(
        "body_too_large",
        StatusCode::PAYLOAD_TOO_LARGE,
        "api.error.body_too_large"
    ),
    entry!(
        "unauthenticated",
        StatusCode::UNAUTHORIZED,
        "api.error.unauthenticated"
    ),
    entry!(
        "invalid_token",
        StatusCode::UNAUTHORIZED,
        "api.error.invalid_token"
    ),
    entry!(
        "rate_limited",
        StatusCode::TOO_MANY_REQUESTS,
        "api.error.rate_limited"
    ),
    entry!("not_found", StatusCode::NOT_FOUND, "api.error.not_found"),
    entry!(
        "method_not_allowed",
        StatusCode::METHOD_NOT_ALLOWED,
        "api.error.method_not_allowed"
    ),
    entry!(
        "bad_request",
        StatusCode::BAD_REQUEST,
        "api.error.bad_request"
    ),
    entry!(
        "invalid_input",
        StatusCode::BAD_REQUEST,
        "api.error.invalid_input"
    ),
    entry!(
        "invalid_provider",
        StatusCode::BAD_REQUEST,
        "api.error.invalid_provider"
    ),
    entry!(
        "master_key_missing",
        StatusCode::CONFLICT,
        "api.error.master_key_missing"
    ),
    entry!("busy", StatusCode::CONFLICT, "api.error.busy"),
    entry!(
        "already_exists",
        StatusCode::CONFLICT,
        "api.error.already_exists"
    ),
    entry!(
        "default_exists",
        StatusCode::CONFLICT,
        "api.error.default_exists"
    ),
    entry!(
        "provider_in_use",
        StatusCode::CONFLICT,
        "api.error.provider_in_use"
    ),
    entry!("no_provider", StatusCode::CONFLICT, "api.error.no_provider"),
    entry!(
        "conversation_archived",
        StatusCode::CONFLICT,
        "api.error.conversation_archived"
    ),
    entry!(
        "message_empty",
        StatusCode::BAD_REQUEST,
        "api.error.message_empty"
    ),
    entry!(
        "message_too_large",
        StatusCode::BAD_REQUEST,
        "api.error.message_too_large"
    ),
    entry!(
        "message_invalid",
        StatusCode::BAD_REQUEST,
        "api.error.message_invalid"
    ),
    entry!("timeout", StatusCode::GATEWAY_TIMEOUT, "api.error.timeout"),
    entry!(
        "internal",
        StatusCode::INTERNAL_SERVER_ERROR,
        "api.error.internal"
    ),
    entry!(
        "store_failed",
        StatusCode::INTERNAL_SERVER_ERROR,
        "api.error.store_failed"
    ),
    // Provider and turn failures (advice in the provider test and in `error` events).
    entry!("ai_auth", StatusCode::BAD_GATEWAY, "api.error.ai_auth"),
    entry!("ai_quota", StatusCode::BAD_GATEWAY, "api.error.ai_quota"),
    entry!(
        "ai_rate_limited",
        StatusCode::BAD_GATEWAY,
        "api.error.ai_rate_limited"
    ),
    entry!(
        "ai_circuit_open",
        StatusCode::BAD_GATEWAY,
        "api.error.ai_circuit_open"
    ),
    entry!(
        "ai_unsupported",
        StatusCode::BAD_GATEWAY,
        "api.error.ai_unsupported"
    ),
    entry!("ai_empty", StatusCode::BAD_GATEWAY, "api.error.ai_empty"),
    entry!("ai_config", StatusCode::BAD_GATEWAY, "api.error.ai_config"),
    entry!(
        "ai_transport",
        StatusCode::BAD_GATEWAY,
        "api.error.ai_transport"
    ),
    entry!("ai_server", StatusCode::BAD_GATEWAY, "api.error.ai_server"),
    entry!(
        "ai_rejected",
        StatusCode::BAD_GATEWAY,
        "api.error.ai_rejected"
    ),
    entry!(
        "ai_malformed",
        StatusCode::BAD_GATEWAY,
        "api.error.ai_malformed"
    ),
    entry!("ai_other", StatusCode::BAD_GATEWAY, "api.error.ai_other"),
    entry!(
        "provider_not_found",
        StatusCode::NOT_FOUND,
        "api.error.provider_not_found"
    ),
    entry!(
        "provider_config",
        StatusCode::BAD_GATEWAY,
        "api.error.provider_config"
    ),
    entry!(
        "provider_key_missing",
        StatusCode::BAD_GATEWAY,
        "api.error.provider_key_missing"
    ),
    entry!(
        "provider_key_mismatch",
        StatusCode::BAD_GATEWAY,
        "api.error.provider_key_mismatch"
    ),
    entry!(
        "provider_client",
        StatusCode::BAD_GATEWAY,
        "api.error.provider_client"
    ),
    entry!(
        "provider_timeout",
        StatusCode::BAD_GATEWAY,
        "api.error.provider_timeout"
    ),
    entry!(
        "conversation_not_found",
        StatusCode::NOT_FOUND,
        "api.error.conversation_not_found"
    ),
    entry!(
        "proposal_decided",
        StatusCode::CONFLICT,
        "api.error.proposal_decided"
    ),
    entry!(
        "proposal_expired",
        StatusCode::CONFLICT,
        "api.error.proposal_expired"
    ),
    entry!(
        "execution_disabled",
        StatusCode::CONFLICT,
        "api.error.execution_disabled"
    ),
    entry!(
        "proposal_not_approved",
        StatusCode::CONFLICT,
        "api.error.proposal_not_approved"
    ),
    entry!(
        "proposal_no_action",
        StatusCode::CONFLICT,
        "api.error.proposal_no_action"
    ),
    entry!(
        "proposal_already_run",
        StatusCode::CONFLICT,
        "api.error.proposal_already_run"
    ),
    entry!(
        "action_invalid",
        StatusCode::CONFLICT,
        "api.error.action_invalid"
    ),
    entry!(
        "unsupported_target",
        StatusCode::CONFLICT,
        "api.error.unsupported_target"
    ),
    entry!(
        "invalid_service",
        StatusCode::BAD_REQUEST,
        "api.error.invalid_service"
    ),
    entry!(
        "logs_unavailable",
        StatusCode::BAD_REQUEST,
        "api.error.logs_unavailable"
    ),
    entry!(
        "invalid_test_domain",
        StatusCode::BAD_REQUEST,
        "api.error.invalid_test_domain"
    ),
    entry!(
        "invalid_upstream_port",
        StatusCode::BAD_REQUEST,
        "api.error.invalid_upstream_port"
    ),
    entry!(
        "invalid_ssh_target",
        StatusCode::BAD_REQUEST,
        "api.error.invalid_ssh_target"
    ),
    entry!(
        "duplicate_domain",
        StatusCode::CONFLICT,
        "api.error.duplicate_domain"
    ),
    entry!(
        "typed_confirmation_mismatch",
        StatusCode::BAD_REQUEST,
        "api.error.typed_confirmation_mismatch"
    ),
    entry!(
        "typed_confirmation_required",
        StatusCode::CONFLICT,
        "api.error.typed_confirmation_required"
    ),
    entry!(
        "approval_required",
        StatusCode::CONFLICT,
        "api.error.approval_required"
    ),
    entry!(
        "proposal_not_executed",
        StatusCode::CONFLICT,
        "api.error.proposal_not_executed"
    ),
    entry!(
        "execution_running",
        StatusCode::CONFLICT,
        "api.error.execution_running"
    ),
    entry!(
        "backup_missing",
        StatusCode::CONFLICT,
        "api.error.backup_missing"
    ),
    entry!(
        "already_rolled_back",
        StatusCode::CONFLICT,
        "api.error.already_rolled_back"
    ),
    entry!(
        "plan_not_approvable",
        StatusCode::CONFLICT,
        "api.error.plan_not_approvable"
    ),
    entry!(
        "plan_not_approved",
        StatusCode::CONFLICT,
        "api.error.plan_not_approved"
    ),
    entry!(
        "plan_gate_blocked",
        StatusCode::CONFLICT,
        "api.error.plan_gate_blocked"
    ),
    entry!(
        "credentials_not_accepted",
        StatusCode::BAD_REQUEST,
        "api.error.credentials_not_accepted"
    ),
    entry!(
        "bad_credentials",
        StatusCode::BAD_REQUEST,
        "api.error.bad_credentials"
    ),
    entry!("turn_busy", StatusCode::CONFLICT, "api.error.turn_busy"),
    entry!(
        "turn_claim_lost",
        StatusCode::CONFLICT,
        "api.error.turn_claim_lost"
    ),
    entry!(
        "token_budget_exceeded",
        StatusCode::BAD_GATEWAY,
        "api.error.token_budget_exceeded"
    ),
    entry!("no_answer", StatusCode::BAD_GATEWAY, "api.error.no_answer"),
];

fn lookup(code: &str) -> Option<&'static Entry> {
    TABLE.iter().find(|e| e.code == code)
}

/// An error of an API call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ApiError {
    code: &'static str,
}

/// Marks a response as an API error; the guard layer renders its body.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ErrorMark(pub(crate) &'static str);

impl ApiError {
    /// An error with this code; an unknown code is an internal error.
    pub(crate) const fn new(code: &'static str) -> Self {
        Self { code }
    }

    /// The HTTP status of the code.
    pub(crate) fn status(self) -> StatusCode {
        lookup(self.code).map_or(StatusCode::INTERNAL_SERVER_ERROR, |e| e.status)
    }
}

pub(crate) const NOT_FOUND: ApiError = ApiError::new("not_found");
pub(crate) const INVALID_INPUT: ApiError = ApiError::new("invalid_input");
pub(crate) const INTERNAL: ApiError = ApiError::new("internal");
pub(crate) const STORE_FAILED: ApiError = ApiError::new("store_failed");

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = self.status();
        response.extensions_mut().insert(ErrorMark(self.code));
        response
    }
}

/// A database error as an API error: a missing row (or one of another org) is `not_found`,
/// everything else is `store_failed`. The database text is never passed on.
pub(crate) fn from_db(error: &db::Error) -> ApiError {
    match error {
        db::Error::NotFound => NOT_FOUND,
        _ => STORE_FAILED,
    }
}

/// The localized message of `code`; an unknown code reads as an internal error.
pub(crate) fn message(catalog: &Catalog, locale: Locale, code: &str) -> String {
    let key = lookup(code).map_or(MessageKey::new("api.error.internal"), |e| e.key);
    catalog.get(locale, key, &[]).text
}

/// The code to show for a provider call failure.
pub(crate) fn ai_code(error: &ai::AiError) -> &'static str {
    match error.code() {
        "ai_auth" => "ai_auth",
        "ai_quota" => "ai_quota",
        "ai_rate_limited" => "ai_rate_limited",
        "ai_circuit_open" => "ai_circuit_open",
        "ai_unsupported" => "ai_unsupported",
        "ai_empty" => "ai_empty",
        "ai_config" => "ai_config",
        "ai_transport" => "ai_transport",
        "ai_server" => "ai_server",
        "ai_rejected" => "ai_rejected",
        "ai_malformed" => "ai_malformed",
        _ => "ai_other",
    }
}

/// The code to show for a turn failure.
pub(crate) fn turn_code(error: &agent::TurnError) -> &'static str {
    match error {
        agent::TurnError::NotFound => "conversation_not_found",
        agent::TurnError::Busy => "turn_busy",
        agent::TurnError::Archived => "conversation_archived",
        agent::TurnError::EmptyMessage => "message_empty",
        agent::TurnError::MessageTooLarge => "message_too_large",
        agent::TurnError::BadMessage => "message_invalid",
        agent::TurnError::ClaimLost => "turn_claim_lost",
        agent::TurnError::Provider(ai) => ai_code(ai),
        agent::TurnError::BudgetExceeded { .. } => "token_budget_exceeded",
        agent::TurnError::NoAnswer => "no_answer",
        agent::TurnError::Store => "store_failed",
    }
}

/// The code to show when a provider could not be built.
pub(crate) fn load_code(error: &agent::LoadError) -> &'static str {
    match error {
        agent::LoadError::NotFound => "provider_not_found",
        agent::LoadError::Config(_) => "provider_config",
        agent::LoadError::Key("vault_not_found") => "provider_key_missing",
        agent::LoadError::Key(_) => "provider_key_mismatch",
        agent::LoadError::Client => "provider_client",
        agent::LoadError::Store => "store_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_every_key_exists_in_both_locales() -> Result<(), i18n::CatalogError> {
        let catalog = Catalog::builtin()?;
        let mut seen = std::collections::BTreeSet::new();
        for entry in TABLE {
            assert!(seen.insert(entry.code), "duplicate code {}", entry.code);
            for locale in Locale::ALL {
                assert!(
                    catalog.raw(locale, entry.key.as_str()).is_some(),
                    "{} missing in {}",
                    entry.key.as_str(),
                    locale.tag()
                );
            }
        }
        Ok(())
    }

    #[test]
    fn unknown_codes_read_as_internal() -> Result<(), i18n::CatalogError> {
        let catalog = Catalog::builtin()?;
        assert_eq!(
            message(&catalog, Locale::En, "who_knows"),
            message(&catalog, Locale::En, "internal")
        );
        assert_eq!(
            ApiError::new("who_knows").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        Ok(())
    }
}
