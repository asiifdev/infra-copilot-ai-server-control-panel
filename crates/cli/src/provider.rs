//! `infra provider add|list|remove`.
//!
//! The API key never travels on the command line: it is read from standard input
//! (`--key-stdin`) or from `NODIVARA_PROVIDER_KEY`, sealed with the vault (master key from
//! `NODIVARA_MASTER_KEY`) and stored only as ciphertext. Nothing here prints a key.

use crate::Ui;
use crate::args::{Globals, Parsed, parse};
use ai::ProviderConfig;
use db::NewAiProvider;
use i18n::MessageKey;
use std::io::Read;
use vault::Vault;

const ADDED: MessageKey = MessageKey::new("cli.provider.added");
const REMOVED: MessageKey = MessageKey::new("cli.provider.removed");
const UNKNOWN: MessageKey = MessageKey::new("cli.provider.unknown");
const IN_USE: MessageKey = MessageKey::new("cli.provider.in_use");
const EXISTS: MessageKey = MessageKey::new("cli.provider.exists");
const DEFAULT_EXISTS: MessageKey = MessageKey::new("cli.provider.default_exists");
const BAD_CONFIG: MessageKey = MessageKey::new("cli.provider.bad_config");
const BAD_KEY: MessageKey = MessageKey::new("cli.provider.bad_key");
const NO_KEY_NOTE: MessageKey = MessageKey::new("cli.provider.no_key_note");
const LIST_EMPTY: MessageKey = MessageKey::new("cli.provider.list_empty");
const LIST_HEAD: MessageKey = MessageKey::new("cli.provider.list_head");
const LIST_LINE: MessageKey = MessageKey::new("cli.provider.list_line");
const WORD_YES: MessageKey = MessageKey::new("cli.word.yes");
const WORD_NO: MessageKey = MessageKey::new("cli.word.no");
const WORD_KEY_SET: MessageKey = MessageKey::new("cli.provider.key_set");
const WORD_KEY_NONE: MessageKey = MessageKey::new("cli.provider.key_none");
const VAULT_NO_KEY: MessageKey = MessageKey::new("cli.vault.no_key");
const VAULT_BAD_KEY: MessageKey = MessageKey::new("cli.vault.bad_key");

/// Name of the variable that may hold the provider key.
const ENV_PROVIDER_KEY: &str = "NODIVARA_PROVIDER_KEY";
/// Purpose label the key is sealed under (bound into the ciphertext).
const KEY_PURPOSE: &str = "ai_provider_key";
/// Longest key accepted.
const MAX_KEY_BYTES: usize = 1024;

impl Ui<'_> {
    /// The vault from `NODIVARA_MASTER_KEY` (and retired keys), or a localized failure.
    pub(crate) fn vault_from_env(&mut self) -> Result<Vault, u8> {
        let Some(current) = self
            .env
            .get(vault::ENV_KEY)
            .filter(|k| !k.trim().is_empty())
        else {
            return Err(self.fail(VAULT_NO_KEY, &[]));
        };
        let retired = self.env.get(vault::ENV_RETIRED_KEYS).unwrap_or_default();
        let specs: Vec<&str> = retired
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        // The error never carries key material: only the fixed advice is shown.
        Vault::new(current.trim(), &specs).map_err(|_| self.fail(VAULT_BAD_KEY, &[]))
    }

    pub(crate) fn provider_command(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let Some((sub, args)) = rest.split_first() else {
            return self.usage("-", Some(crate::USAGE_PROVIDER));
        };
        let parsed = match sub.as_str() {
            "add" => parse(
                args,
                &["--name", "--base-url", "--model", "--params", "--lang"],
                &["--default", "--key-stdin"],
            ),
            "list" => parse(args, &["--lang"], &[]),
            "remove" => parse(args, &["--lang"], &[]),
            other => return self.usage(other, Some(crate::USAGE_PROVIDER)),
        };
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&detail, Some(crate::USAGE_PROVIDER)),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", Some(crate::USAGE_PROVIDER));
        }
        match sub.as_str() {
            "add" => self.provider_add(globals, &parsed),
            "list" => self.provider_list(globals, &parsed),
            _ => self.provider_remove(globals, &parsed),
        }
    }

    /// The key to seal: from stdin when `--key-stdin`, else from the environment. `None` when
    /// none was supplied; `Err` (already reported) when it is unusable.
    fn read_provider_key(&mut self, from_stdin: bool) -> Result<Option<String>, u8> {
        let raw = if from_stdin {
            let mut bytes = Vec::new();
            let read =
                Read::take(&mut *self.stdin, (MAX_KEY_BYTES as u64) + 1).read_to_end(&mut bytes);
            if read.is_err() {
                return Err(self.fail(BAD_KEY, &[]));
            }
            // Only the first line: a trailing newline (echo, here-string) is not part of the key.
            let text = String::from_utf8(bytes).map_err(|_| self.fail(BAD_KEY, &[]))?;
            Some(text.lines().next().unwrap_or_default().to_owned())
        } else {
            self.env.get(ENV_PROVIDER_KEY)
        };
        let Some(raw) = raw else {
            return Ok(None);
        };
        let key = raw.trim();
        if key.is_empty() {
            return Ok(None);
        }
        if key.len() > MAX_KEY_BYTES || key.chars().any(char::is_control) {
            return Err(self.fail(BAD_KEY, &[]));
        }
        Ok(Some(key.to_owned()))
    }

    fn provider_add(&mut self, globals: &Globals, parsed: &Parsed) -> u8 {
        let (Some(name), Some(base_url), Some(model)) = (
            parsed.option("--name"),
            parsed.option("--base-url"),
            parsed.option("--model"),
        ) else {
            return self.usage("-", Some(crate::USAGE_PROVIDER));
        };
        if let Some(extra) = parsed.positional.first() {
            return self.usage(extra, Some(crate::USAGE_PROVIDER));
        }
        let params = parsed.option("--params").unwrap_or("{}");
        // Refuse unusable settings now rather than at the first question.
        if let Err(error) = ProviderConfig::from_params(base_url, model, params) {
            return self.fail(BAD_CONFIG, &[("why", &error.to_string())]);
        }
        let key = match self.read_provider_key(parsed.flag("--key-stdin")) {
            Ok(key) => key,
            Err(code) => return code,
        };
        // The master key is needed only when there is something to seal.
        let vault = if key.is_some() {
            match self.vault_from_env() {
                Ok(vault) => Some(vault),
                Err(code) => return code,
            }
        } else {
            None
        };
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let is_default = parsed.flag("--default");
        if is_default && db::default_ai_provider(&conn, &scope.org).is_ok() {
            return self.fail(DEFAULT_EXISTS, &[]);
        }
        let secret = match (&vault, &key) {
            (Some(vault), Some(key)) => {
                match vault.store(&conn, &scope.org, KEY_PURPOSE, key.as_bytes()) {
                    Ok(id) => Some(id),
                    Err(error) => {
                        return self.store_reason(error.code());
                    }
                }
            }
            _ => None,
        };
        let created = db::create_ai_provider(
            &conn,
            &scope.org,
            &NewAiProvider {
                name,
                base_url,
                default_model: model,
                secret: secret.as_ref(),
                params_json: params,
                is_default,
            },
        );
        match created {
            Ok(_) => {
                if key.is_none() {
                    // Said on stderr: stdout stays a clean confirmation.
                    let note = self.tr(NO_KEY_NOTE, &[]);
                    self.note_err(&note);
                }
                self.say_out(ADDED, &[("name", name)])
            }
            Err(error) => {
                // Do not leave a sealed key behind for a provider that was not created.
                if let Some(id) = &secret {
                    let _ = db::delete_secret(&conn, &scope.org, id);
                }
                if error.is_constraint_violation() {
                    self.fail(EXISTS, &[("name", name)])
                } else {
                    self.store_error(&error)
                }
            }
        }
    }

    fn provider_list(&mut self, globals: &Globals, parsed: &Parsed) -> u8 {
        if let Some(extra) = parsed.positional.first() {
            return self.usage(extra, Some(crate::USAGE_PROVIDER));
        }
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let rows = match db::list_ai_providers(&conn, &scope.org) {
            Ok(rows) => rows,
            Err(error) => return self.store_error(&error),
        };
        if rows.is_empty() {
            return self.say_out(LIST_EMPTY, &[]);
        }
        let mut text = self.tr(LIST_HEAD, &[]);
        for row in &rows {
            let default = self.tr(if row.is_default { WORD_YES } else { WORD_NO }, &[]);
            let key = self.tr(
                if row.secret_id.is_some() {
                    WORD_KEY_SET
                } else {
                    WORD_KEY_NONE
                },
                &[],
            );
            text.push('\n');
            // Only the stored URL, model and a yes/no for the key: never the key itself.
            text.push_str(&self.tr(
                LIST_LINE,
                &[
                    ("name", &row.name),
                    ("url", &row.base_url),
                    ("model", &row.default_model),
                    ("default", &default),
                    ("key", &key),
                ],
            ));
        }
        self.write_out(&text)
    }

    fn provider_remove(&mut self, globals: &Globals, parsed: &Parsed) -> u8 {
        let [name] = parsed.positional.as_slice() else {
            return self.usage("-", Some(crate::USAGE_PROVIDER));
        };
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let rows = match db::list_ai_providers(&conn, &scope.org) {
            Ok(rows) => rows,
            Err(error) => return self.store_error(&error),
        };
        let Some(row) = rows.iter().find(|r| &r.name == name) else {
            return self.fail(UNKNOWN, &[("name", name)]);
        };
        match db::delete_ai_provider(&conn, &scope.org, &row.id) {
            Ok(()) => {}
            Err(error) if error.is_constraint_violation() => {
                return self.fail(IN_USE, &[("name", name)]);
            }
            Err(error) => return self.store_error(&error),
        }
        if let Some(secret) = &row.secret_id {
            // The provider is gone, so its sealed key goes too.
            let _ = db::delete_secret(&conn, &scope.org, secret);
        }
        self.say_out(REMOVED, &[("name", name)])
    }
}
