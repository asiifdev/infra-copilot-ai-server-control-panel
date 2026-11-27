//! Thin Tauri v2 desktop shell for Nodivara (ADR 0044).
//!
//! Responsibilities are intentionally minimal:
//! - Start the embedded loopback `server` on `127.0.0.1:0` with a one-time `#t=<token>` URL.
//! - Navigate the `main` window to that loopback URL.
//! - Expose a single, non-privileged `app_info` command gated by `AppManifest::commands`.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

/// Metadata returned by the `app_info` command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    name: &'static str,
    version: &'static str,
    loopback_origin: Option<String>,
}

/// Shared desktop shell state managed by Tauri.
#[derive(Default)]
struct DesktopState {
    loopback_origin: Mutex<Option<String>>,
    embedded: Mutex<Option<EmbeddedServer>>,
}

/// Errors produced while starting the embedded loopback server.
#[derive(Debug)]
enum DesktopError {
    Setup(bootstrap::SetupError),
    Serve(server::ServeError),
    Runtime(std::io::Error),
}

impl fmt::Display for DesktopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Setup(err) => write!(f, "setup failed: {err}"),
            Self::Serve(err) => write!(f, "server failed: {err}"),
            Self::Runtime(err) => write!(f, "tokio runtime failed: {err}"),
        }
    }
}

impl std::error::Error for DesktopError {}

impl From<bootstrap::SetupError> for DesktopError {
    fn from(err: bootstrap::SetupError) -> Self {
        Self::Setup(err)
    }
}

impl From<server::ServeError> for DesktopError {
    fn from(err: server::ServeError) -> Self {
        Self::Serve(err)
    }
}

/// Running handle for the embedded loopback `server`.
struct EmbeddedServer {
    local_addr: SocketAddr,
    login_url: String,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    runtime: Option<tokio::runtime::Runtime>,
}

impl EmbeddedServer {
    fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

impl Drop for EmbeddedServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Validates that a generated sign-in URL targets `127.0.0.1:<port>` and carries `#t=<token>`.
fn is_loopback_login_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let Some((port_part, frag)) = rest.split_once("/#t=") else {
        return false;
    };
    !port_part.is_empty() && port_part.bytes().all(|b| b.is_ascii_digit()) && !frag.is_empty()
}

/// Binds and starts the embedded `server` on `127.0.0.1:0` using the supplied environment.
fn start_embedded_server(env: &dyn bootstrap::Environment) -> Result<EmbeddedServer, DesktopError> {
    let mut config = server::Config::resolve(env, 0, None, None)?;
    config.enable_watch = true;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(DesktopError::Runtime)?;

    let bound = runtime.block_on(server::Server::bind(config))?;
    let local_addr = bound.local_addr();
    let login_url = bound.login_url()?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    runtime.spawn(async move {
        if let Err(err) = bound
            .run(async move {
                let _ = shutdown_rx.await;
            })
            .await
        {
            tracing::error!(error = %err, "embedded server stopped with error");
        }
    });

    Ok(EmbeddedServer {
        local_addr,
        login_url,
        shutdown_tx: Some(shutdown_tx),
        runtime: Some(runtime),
    })
}

fn build_app_info(state: &DesktopState) -> AppInfo {
    let origin = state.loopback_origin.lock().ok().and_then(|g| g.clone());
    AppInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        loopback_origin: origin,
    }
}

#[tauri::command]
fn app_info(state: tauri::State<'_, DesktopState>) -> AppInfo {
    build_app_info(&state)
}

/// Starts the Tauri v2 desktop application.
///
/// # Errors
/// Returns an error if Tauri fails to initialize or run the event loop.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), tauri::Error> {
    tauri::Builder::default()
        .manage(DesktopState::default())
        .invoke_handler(tauri::generate_handler![app_info])
        .setup(|app| {
            match start_embedded_server(&bootstrap::ProcessEnv) {
                Ok(embedded) => {
                    let origin = format!("http://{}", embedded.local_addr);
                    let login_url = embedded.login_url.clone();
                    let state = app.state::<DesktopState>();
                    if let Ok(mut guard) = state.loopback_origin.lock() {
                        *guard = Some(origin);
                    }
                    if let Ok(mut guard) = state.embedded.lock() {
                        *guard = Some(embedded);
                    }
                    if is_loopback_login_url(&login_url)
                        && let Some(window) = app.get_webview_window("main")
                        && let Ok(parsed) = login_url.parse::<tauri::Url>()
                        && let Err(err) = window.navigate(parsed)
                    {
                        tracing::warn!(error = %err, "failed to navigate main window to loopback");
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "embedded server did not start; keeping bundled frontend"
                    );
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MapEnv(HashMap<&'static str, String>);

    impl bootstrap::Environment for MapEnv {
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    fn unique_temp_db() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "nodivara-desktop-test-{}-{nanos}.db",
            std::process::id()
        ))
    }

    #[test]
    fn is_loopback_login_url_validates_strictly() {
        assert!(is_loopback_login_url("http://127.0.0.1:43210/#t=abc123"));
        assert!(!is_loopback_login_url("http://0.0.0.0:43210/#t=abc123"));
        assert!(!is_loopback_login_url("http://localhost:43210/#t=abc123"));
        assert!(!is_loopback_login_url("https://example.com/#t=abc123"));
        assert!(!is_loopback_login_url("http://127.0.0.1:43210/"));
        assert!(!is_loopback_login_url("http://127.0.0.1:43210/#t="));
        assert!(!is_loopback_login_url("http://127.0.0.1:/#t=abc123"));
    }

    #[test]
    fn build_app_info_reflects_state() {
        let state = DesktopState::default();
        if let Ok(mut guard) = state.loopback_origin.lock() {
            *guard = Some("http://127.0.0.1:50000".to_owned());
        }
        let info = build_app_info(&state);
        assert_eq!(info.name, "nodivara-desktop");
        assert_eq!(
            info.loopback_origin.as_deref(),
            Some("http://127.0.0.1:50000")
        );
    }

    #[test]
    fn start_embedded_server_binds_ephemeral_loopback_and_issues_token() {
        let db_path = unique_temp_db();
        let mut conn = db::open(&db_path).expect_ok_or_fail();
        db::migrate(&mut conn).expect_ok_or_fail();
        let org = db::create_org(&conn, "Test Org", "test-org", "free").expect_ok_or_fail();
        let user = db::create_user(&conn, "owner@example.test", "Owner", None).expect_ok_or_fail();
        let _ = db::add_member(&conn, &org.id, &user.id, db::Role::Owner).expect_ok_or_fail();
        drop(conn);

        let mut vars = HashMap::new();
        vars.insert("NODIVARA_DB", db_path.display().to_string());
        let env = MapEnv(vars);

        let mut srv = start_embedded_server(&env).expect_ok_or_fail();
        assert!(srv.local_addr.ip().is_loopback());
        assert_ne!(srv.local_addr.port(), 0);
        assert!(is_loopback_login_url(&srv.login_url));

        srv.shutdown();
        let _ = std::fs::remove_file(&db_path);
    }

    trait TestUnwrap<T> {
        fn expect_ok_or_fail(self) -> T;
    }

    impl<T, E: fmt::Debug> TestUnwrap<T> for Result<T, E> {
        #[allow(clippy::panic)]
        fn expect_ok_or_fail(self) -> T {
            match self {
                Ok(v) => v,
                Err(e) => panic!("unexpected error in test: {e:?}"),
            }
        }
    }
}
