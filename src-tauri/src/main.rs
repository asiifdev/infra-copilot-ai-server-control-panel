// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(err) = nodivara_desktop_lib::run() {
        tracing::error!(error = %err, "desktop application exited with error");
        std::process::exit(1);
    }
}
