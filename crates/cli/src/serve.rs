//! `infra serve [--port N] [--db PATH] [--org SLUG]` (ADR 0030).
//!
//! Starts the local web interface on `127.0.0.1` and prints a one-time sign-in address. It never
//! opens a browser, listens on loopback only, and stops cleanly on Ctrl-C and on SIGTERM. The
//! database, the organization and the vault are resolved by the `bootstrap` helpers every other
//! command uses; a missing master key is not an error here (the server then refuses to take
//! provider keys and says so), a malformed one is.

use crate::Ui;
use crate::args::{Globals, parse};
use i18n::MessageKey;
use server::{Config, DEFAULT_PORT, ServeError, Server};

const LISTENING: MessageKey = MessageKey::new("cli.serve.listening");
const OPEN: MessageKey = MessageKey::new("cli.serve.open");
const STOP: MessageKey = MessageKey::new("cli.serve.stop");
const STOPPED: MessageKey = MessageKey::new("cli.serve.stopped");
const NO_KEY: MessageKey = MessageKey::new("cli.serve.no_key");
const PORT_BUSY: MessageKey = MessageKey::new("cli.serve.port_busy");
const EXECUTION_ON: MessageKey = MessageKey::new("cli.serve.execution_on");
const BAD_PORT: MessageKey = MessageKey::new("cli.serve.bad_port");
const FAILED: MessageKey = MessageKey::new("cli.serve.failed");
const RUNTIME: MessageKey = MessageKey::new("cli.serve.runtime");
const TOKEN: MessageKey = MessageKey::new("cli.serve.token");

impl Ui<'_> {
    pub(crate) fn serve(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let usage = Some(crate::USAGE_SERVE);
        let parsed = match parse(rest, &["--port", "--lang"], &["--enable-execution"]) {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&crate::plans::safe_detail(&detail), usage),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", usage);
        }
        if let Some(extra) = parsed.positional.first() {
            return self.usage(extra, usage);
        }
        let port = match parsed.option("--port") {
            None => DEFAULT_PORT,
            Some(text) => match text.parse::<u16>() {
                Ok(port) => port,
                Err(_) => return self.fail(BAD_PORT, &[]),
            },
        };
        let config = match Config::resolve(
            self.env,
            port,
            globals.db.as_deref(),
            globals.org.as_deref(),
        ) {
            Ok(config) => config,
            Err(error) => return self.setup_failure(&error),
        };
        let mut config = config;
        // Off unless asked for: approving a proposal only records the decision (ADR 0037).
        let execution = parsed.flag("--enable-execution");
        if execution {
            config.executor = Some(std::sync::Arc::new(actions::LocalExecutor::new(
                transport::ProcessRunner::new(std::time::Duration::from_secs(30), 64 * 1024),
            )));
        }
        let has_key = config.vault.is_some();
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return self.fail(RUNTIME, &[]);
        };
        let outcome = runtime.block_on(async {
            let server = Server::bind(config).await?;
            // Installed before the address is printed, so a signal right after it is handled.
            let shutdown = server::shutdown_signal();
            let url = server.login_url()?;
            let address = server.local_addr().to_string();
            let text = self.tr(LISTENING, &[("address", &address)]);
            self.note_out(&text);
            if !has_key {
                let note = self.tr(NO_KEY, &[]);
                self.note_err(&note);
            }
            if execution {
                let note = self.tr(EXECUTION_ON, &[]);
                self.note_err(&note);
            }
            let text = self.tr(OPEN, &[]);
            self.note_out(&text);
            // The address is data, not catalog text: printed as is, on a line of its own.
            self.note_out(&url);
            let text = self.tr(STOP, &[]);
            self.note_out(&text);
            server.run(shutdown).await
        });
        match outcome {
            Ok(()) => self.say_out(STOPPED, &[]),
            Err(ServeError::PortBusy(port)) => self.fail(PORT_BUSY, &[("port", &port.to_string())]),
            Err(ServeError::Random) => self.fail(TOKEN, &[]),
            Err(error) => self.fail(FAILED, &[("reason", &error.to_string())]),
        }
    }

    /// A line on stdout that is flushed at once (the sign-in address is read by a person or a
    /// script while the server keeps running).
    fn note_out(&mut self, text: &str) {
        let _ = writeln!(self.out, "{text}");
        let _ = self.out.flush();
    }
}
