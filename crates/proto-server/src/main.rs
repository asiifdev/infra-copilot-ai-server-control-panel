//! Temporary prototype server for the M0 performance budget (docs/decisions/0010).
//!
//! Serves the Vite build in `ui/dist`, embedded into the binary, on an
//! ephemeral loopback port. It prints `listening <addr>` to stdout so the
//! measurement script can discover the port, then shuts down gracefully on
//! SIGTERM or Ctrl-C.

use std::io::Write;
use std::net::SocketAddr;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use tokio::net::TcpListener;

/// The UI bundle. `allow_missing` keeps `cargo check`/`clippy` working before
/// the UI has been built; requests then answer 404.
#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
#[allow_missing = true]
struct Assets;

fn mime_for(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

async fn serve_asset(uri: Uri) -> Response {
    let trimmed = uri.path().trim_start_matches('/');
    let path = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };
    match Assets::get(path) {
        Some(file) => {
            let cache = if path == "index.html" {
                "no-cache"
            } else {
                "public, max-age=31536000, immutable"
            };
            (
                [
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_static(mime_for(path)),
                    ),
                    (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
                ],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn app() -> Router {
    Router::new().fallback(serve_asset)
}

async fn shutdown_signal() {
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
            Err(err) => {
                tracing::warn!(%err, "cannot install SIGTERM handler, using Ctrl-C only");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    tracing::info!("shutdown signal received");
}

async fn run() -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, "listening");
    // Machine-readable line for scripts/measure.sh (stdout, flushed at once).
    let mut out = std::io::stdout().lock();
    writeln!(out, "listening {addr}")?;
    out.flush()?;
    drop(out);
    axum::serve(listener, app())
        .with_graceful_shutdown(shutdown_signal())
        .await
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(%err, "server failed");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn mime_types_by_extension() {
        assert_eq!(mime_for("a.js"), "text/javascript; charset=utf-8");
        assert_eq!(mime_for("a.css"), "text/css; charset=utf-8");
        assert_eq!(mime_for("noext"), "application/octet-stream");
    }

    #[tokio::test]
    async fn unknown_path_is_404_and_server_answers() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
        let addr = listener.local_addr()?;
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app())
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
        });
        let mut stream = tokio::net::TcpStream::connect(addr).await?;
        stream
            .write_all(b"GET /does-not-exist.js HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await?;
        let mut buf = String::new();
        stream.read_to_string(&mut buf).await?;
        assert!(buf.starts_with("HTTP/1.1 404"), "got: {buf}");
        let _ = tx.send(());
        server.await??;
        Ok(())
    }
}
