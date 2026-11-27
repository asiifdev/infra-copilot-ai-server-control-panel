// Build script: a failed build must stop, so panicking on `try_build` is intentional.
#![allow(clippy::unwrap_used)]

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["app_info"])),
    )
    .unwrap();
}
