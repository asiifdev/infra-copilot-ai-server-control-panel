//! `infra provider add|list|remove`.
//!
//! The API key never travels on the command line: it is read from standard input
//! (`--key-stdin`) or from `NODIVARA_PROVIDER_KEY`, sealed with the vault (master key from
//! `NODIVARA_MASTER_KEY`) and stored only as ciphertext. Nothing here prints a key.

use crate::Ui;
use crate::args::{Globals, Parsed, parse};
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

/// Name of the variable that may hold the provider key.
const ENV_PROVIDER_KEY: &str = "NODIVARA_PROVIDER_KEY";
/// Longest key accepted.
const MAX_KEY_BYTES: usize = 1024;

impl Ui<'_> {
    /// The vault from `NODIVARA_MASTER_KEY` (and retired keys), or a localized failure.
    pub(crate) fn vault_from_env(&mut self) -> Result<Vault, u8> {
        bootstrap::vault_from_env(self.env).map_err(|error| self.setup_failure(&error))
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
        if let Err(why) = bootstrap::validate_provider_config(base_url, model, params) {
            return self.fail(BAD_CONFIG, &[("why", &why)]);
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
        let created = bootstrap::add_provider(
            &conn,
            &scope.org,
            &bootstrap::NewProvider {
                name,
                base_url,
                model,
                params_json: params,
                is_default: parsed.flag("--default"),
            },
            vault.as_ref().zip(key.as_deref()),
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
            Err(bootstrap::ProviderError::DefaultExists) => self.fail(DEFAULT_EXISTS, &[]),
            Err(bootstrap::ProviderError::Exists) => self.fail(EXISTS, &[("name", name)]),
            Err(bootstrap::ProviderError::Store { reason }) => self.store_reason(&reason),
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
        match bootstrap::remove_provider(&conn, &scope.org, name) {
            Ok(()) => self.say_out(REMOVED, &[("name", name)]),
            Err(bootstrap::RemoveError::Unknown) => self.fail(UNKNOWN, &[("name", name)]),
            Err(bootstrap::RemoveError::InUse) => self.fail(IN_USE, &[("name", name)]),
            Err(bootstrap::RemoveError::Store { reason }) => self.store_reason(&reason),
        }
    }
}
