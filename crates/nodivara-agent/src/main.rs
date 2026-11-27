//! Entry point for `nodivara-agent`, the pull-only remote agent binary.

use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|s| s.to_string_lossy().into_owned())
        .collect();

    let config = match nodivara_agent::AgentConfig::from_args(args) {
        Ok(cfg) => cfg,
        Err(err) => {
            tracing::error!(error = %err, "agent configuration error");
            return ExitCode::from(2);
        }
    };

    let token = match config.resolve_token() {
        Ok(tok) => tok,
        Err(err) => {
            tracing::error!(error = %err, "agent token error");
            return ExitCode::from(2);
        }
    };

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    if let Err(err) = nodivara_agent::run_agent(config, token, shutdown).await {
        tracing::error!(error = %err, "agent exited with error");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
