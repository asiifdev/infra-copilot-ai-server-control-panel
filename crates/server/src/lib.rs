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
//! runs a proposal or a plan (milestone M4); a person can approve or reject a proposal, which only records the decision.

mod api;
mod app;
mod assets;
mod auth;
mod clock;
mod errors;
mod guard;
mod stream;
mod turn;
mod util;
pub(crate) mod watch_loop;

pub use app::{Config, DEFAULT_PORT, DEFAULT_PROVIDER_TEST_TIMEOUT};
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
        .route(
            "/api/nodes/{id}/agent-token",
            post(api::node_agent_token_rotate),
        )
        .route("/api/nodes/{id}/logs", get(api::node_logs))
        .route("/api/agents/poll", post(api::agent_poll))
        .route("/api/agents/result", post(api::agent_result))
        .route("/api/sites", get(api::sites_list).post(api::sites_create))
        .route("/api/findings", get(api::findings))
        .route("/api/plans", get(api::plans))
        .route("/api/plans/{name}", get(api::plan_detail))
        .route("/api/plans/{id}/approve", post(api::plan_approve))
        .route("/api/plans/{id}/run", post(api::plan_run))
        .route("/api/transports", get(api::transports))
        .route(
            "/api/transports/{id}/credentials",
            post(api::transport_credentials),
        )
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
        .route("/api/proposals/{id}/decision", post(api::proposal_decide))
        .route("/api/proposals/{id}/dry-run", post(api::proposal_dry_run))
        .route("/api/proposals/{id}/execute", post(api::proposal_execute))
        .route("/api/proposals/{id}/rollback", post(api::proposal_rollback))
        .route(
            "/api/monitors",
            get(api::monitors_list).post(api::monitor_create),
        )
        .route("/api/monitors/{id}", delete(api::monitor_delete))
        .route("/api/incidents", get(api::incidents_list))
        .route("/api/incidents/{id}/ack", post(api::incident_ack))
        .route(
            "/api/channels",
            get(api::channels_list).post(api::channel_create),
        )
        .route("/api/channels/{id}", delete(api::channel_delete))
        .route(
            "/api/channels/telegram/link-code",
            post(api::telegram_link_code),
        )
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
        // An execution still `running` belongs to a process that stopped: fail it, so its
        // proposal does not stay approved and stuck (it is never repeated by itself).
        if let Ok(conn) = db::open(&config.db_path) {
            let _ = db::fail_interrupted_executions(&conn, &config.scope.org);
        }
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
        let (watch_tx, watch_rx) = tokio::sync::watch::channel(false);
        if self.app.enable_watch {
            tokio::spawn(watch_loop::run_background_loop(
                Arc::clone(&self.app),
                watch_rx,
            ));
        }
        let graceful = {
            let stop = Arc::clone(&stop);
            async move { stop.notified().await }
        };
        let serve = axum::serve(self.listener, router(&self.app))
            .with_graceful_shutdown(graceful)
            .into_future();
        tokio::pin!(serve);
        let res = tokio::select! {
            result = &mut serve => result.map_err(|error| ServeError::Io(error.to_string())),
            () = async {
                shutdown.await;
                stop.notify_one();
                tokio::time::sleep(SHUTDOWN_GRACE).await;
            } => Ok(()),
        };
        let _ = watch_tx.send(true);
        res
    }
}

/// A future that completes on Ctrl-C or, on Unix, SIGTERM.
///
/// The signal handlers are installed when this function is **called**, not when the future is
/// first polled: the caller prints the sign-in address in between, and a signal that arrives
/// right after that must stop the server cleanly instead of killing the process. Call it inside
/// the runtime, before announcing that the server is ready.
pub fn shutdown_signal() -> impl Future<Output = ()> + Send + 'static {
    #[cfg(unix)]
    let handlers = {
        use tokio::signal::unix::{SignalKind, signal};
        signal(SignalKind::interrupt())
            .and_then(|int| signal(SignalKind::terminate()).map(|term| (int, term)))
            .ok()
    };
    async move {
        #[cfg(unix)]
        {
            if let Some((mut int, mut term)) = handlers {
                tokio::select! {
                    _ = int.recv() => {}
                    _ = term.recv() => {}
                }
                return;
            }
        }
        let _ = tokio::signal::ctrl_c().await;
    }
}
