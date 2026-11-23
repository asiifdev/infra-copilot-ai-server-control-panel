//! The local server (ADR 0029 and 0030): the JSON API and the embedded web UI on a loopback
//! address, for one person on this machine.
//!
//! * It listens on loopback only; any other address is refused ([`ServeError::NotLoopback`]).
//! * Sign-in is a one-time token printed at start (in the URL fragment, so it never reaches the
//!   server log or a `Referer` header), traded for a session cookie.
//! * Every request is checked for `Host`, `Origin`, `X-Nodivara`, content type and size, and every
//!   response carries the security headers (see [`guard`]).
//! * Handlers are thin: they call `db`, `agent`, `plan`, `parity`, `vault` and `bootstrap`.
//!
//! Intentionally missing: remote access and TLS, accounts and passwords, and anything that
//! approves or runs a proposal or a plan (milestone M4).

mod api;
mod app;
mod assets;
mod auth;
mod clock;
mod errors;
mod guard;
mod turn;
mod util;

pub use app::{Config, DEFAULT_PORT};
pub use clock::Clock;

use app::App;
use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post};
use std::fmt;
use std::future::{Future, IntoFuture};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// How long open connections (a stream mid-turn) get to finish once shutdown starts.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Why the server could not start or stopped.
#[derive(Debug)]
pub enum ServeError {
    /// The address is not a loopback address. Remote access needs TLS and real accounts, which
    /// this version does not have.
    NotLoopback(SocketAddr),
    /// Something already listens on the port.
    PortBusy(u16),
    /// Another I/O failure (the reason is the operating system's text).
    Io(String),
    /// The system random source failed, so no sign-in token could be made.
    Random,
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoopback(addr) => write!(f, "not a loopback address: {addr}"),
            Self::PortBusy(port) => write!(f, "port {port} is busy"),
            Self::Io(reason) => write!(f, "i/o error: {reason}"),
            Self::Random => f.write_str("the random source failed"),
        }
    }
}

impl std::error::Error for ServeError {}

/// Makes sign-in tokens for a server that may already be running.
#[derive(Clone)]
pub struct LoginHandle {
    app: Arc<App>,
}

impl LoginHandle {
    /// Makes a one-time sign-in token and returns it (the plaintext, once).
    ///
    /// # Errors
    /// [`ServeError::Random`] when the system random source fails.
    pub fn issue_token(&self) -> Result<String, ServeError> {
        self.app
            .auth()
            .issue_token(self.app.clock.now())
            .ok_or(ServeError::Random)
    }
}

/// A bound server, not yet running.
pub struct Server {
    listener: TcpListener,
    app: Arc<App>,
    local: SocketAddr,
}

fn router(app: &Arc<App>) -> Router {
    Router::new()
        .route("/healthz", get(api::healthz))
        .route("/api/auth/exchange", post(api::exchange))
        .route("/api/auth/logout", post(api::logout))
        .route("/api/session", get(api::session))
        .route("/api/i18n/{locale}", get(api::i18n_catalog))
        .route("/api/nodes", get(api::nodes))
        .route("/api/nodes/{name}/inventory", get(api::inventory))
        .route("/api/findings", get(api::findings))
        .route("/api/plans", get(api::plans))
        .route("/api/plans/{name}", get(api::plan_detail))
        .route("/api/transports", get(api::transports))
        .route(
            "/api/providers",
            get(api::providers_list).post(api::provider_add),
        )
        .route("/api/providers/{name}", delete(api::provider_remove))
        .route("/api/providers/{name}/test", post(api::provider_test))
        .route("/api/memories", get(api::memories_list))
        .route("/api/memories/{id}/approve", post(api::memory_approve))
        .route("/api/memories/{id}", delete(api::memory_delete))
        .route("/api/proposals", get(api::proposals_list))
        .route(
            "/api/conversations",
            get(api::conversations_list).post(api::conversations_create),
        )
        .route(
            "/api/conversations/{id}/messages",
            get(api::conversation_messages),
        )
        .route("/api/conversations/{id}/turn", post(turn::turn))
        .fallback(assets::fallback)
        // After the routes and the fallback, so the layer covers all of them (and 404 and 405).
        .layer(from_fn_with_state(Arc::clone(app), guard::guard))
        .with_state(Arc::clone(app))
}

impl Server {
    /// Binds the listening socket. Use port `0` for an ephemeral port (tests).
    ///
    /// # Errors
    /// [`ServeError::NotLoopback`] for a non-loopback address, [`ServeError::PortBusy`] when the
    /// port is taken, [`ServeError::Io`] otherwise.
    pub async fn bind(config: Config) -> Result<Self, ServeError> {
        if !config.bind.ip().is_loopback() {
            return Err(ServeError::NotLoopback(config.bind));
        }
        let port = config.bind.port();
        let listener = TcpListener::bind(config.bind).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::AddrInUse {
                ServeError::PortBusy(port)
            } else {
                ServeError::Io(error.to_string())
            }
        })?;
        let local = listener
            .local_addr()
            .map_err(|error| ServeError::Io(error.to_string()))?;
        Ok(Self {
            listener,
            app: Arc::new(App::new(config, local)),
            local,
        })
    }

    /// The address the server listens on (with the real port).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Makes a one-time sign-in token and returns the address to open, `http://HOST:PORT/#t=TOKEN`.
    /// The token is valid for ten minutes and works once; only its hash is kept.
    ///
    /// # Errors
    /// [`ServeError::Random`] when the system random source fails.
    pub fn login_url(&self) -> Result<String, ServeError> {
        let token = self.issue_token()?;
        Ok(format!("http://{}/#t={token}", self.local))
    }

    /// Makes a one-time sign-in token and returns it (the plaintext, once).
    ///
    /// # Errors
    /// [`ServeError::Random`] when the system random source fails.
    pub fn issue_token(&self) -> Result<String, ServeError> {
        self.login_handle().issue_token()
    }

    /// A handle that can make sign-in tokens after the server has started running (tests).
    #[must_use]
    pub fn login_handle(&self) -> LoginHandle {
        LoginHandle {
            app: Arc::clone(&self.app),
        }
    }

    /// Serves until `shutdown` completes, then stops accepting, lets requests in flight finish
    /// (up to a short grace period) and returns.
    ///
    /// # Errors
    /// [`ServeError::Io`] when the accept loop fails.
    pub async fn run(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), ServeError> {
        let stop = Arc::new(Notify::new());
        let graceful = {
            let stop = Arc::clone(&stop);
            async move { stop.notified().await }
        };
        let serve = axum::serve(self.listener, router(&self.app))
            .with_graceful_shutdown(graceful)
            .into_future();
        tokio::pin!(serve);
        tokio::select! {
            result = &mut serve => result.map_err(|error| ServeError::Io(error.to_string())),
            () = async {
                shutdown.await;
                stop.notify_one();
                tokio::time::sleep(SHUTDOWN_GRACE).await;
            } => Ok(()),
        }
    }
}

/// Completes on Ctrl-C or, on Unix, SIGTERM.
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
