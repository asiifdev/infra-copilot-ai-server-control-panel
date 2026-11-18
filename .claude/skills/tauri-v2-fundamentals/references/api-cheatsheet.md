# Tauri v2: cheatsheet API (terverifikasi 2026-11-19)

Semua dari https://v2.tauri.app/develop/calling-rust/, /develop/calling-frontend/, /develop/state-management/, /start/frontend/vite/, /start/project-structure/.

## Rust
| Kebutuhan | API |
|---|---|
| Definisikan command | `#[tauri::command]` |
| Daftarkan | `Builder::invoke_handler(tauri::generate_handler![a, b])` |
| State terkelola | `Builder::manage(T)` / `app.manage(T)` di `setup`; `tauri::State<'_, T>`; `app.state::<T>()` |
| Emit | `use tauri::Emitter`: `emit`, `emit_to`, `emit_filter` |
| Listen di Rust | `use tauri::Listener`: `listen`, `once`, `unlisten`, `listen_any` (untuk event khusus-webview) |
| Ambil jendela | `use tauri::Manager`; `app.get_webview_window("main")` |
| Channel | `tauri::ipc::Channel<T>`; `.send(x)` |
| Respons biner | `tauri::ipc::Response::new(Vec<u8>)` |
| Request mentah | `tauri::ipc::Request` (`body()`, `headers()`) |
| Generik runtime (uji mock) | `AppHandle<R: Runtime>`, `WebviewWindow<R>` |

## TypeScript (`@tauri-apps/api`)
| Kebutuhan | Impor |
|---|---|
| invoke, Channel | `@tauri-apps/api/core` |
| emit, emitTo, listen, once | `@tauri-apps/api/event` |
| getCurrentWebviewWindow | `@tauri-apps/api/webviewWindow` |

## Contoh cleanup Svelte 5 (pola umum; docs menyediakan tab Svelte yang tidak terekstrak, jadi ini pola sendiri berdasarkan aturan "await listen lalu unlisten")
```ts
import { onMount } from 'svelte';
import { listen } from '@tauri-apps/api/event';
onMount(() => {
  const p = listen<number>('x', e => { /* ... */ });
  return () => { p.then(un => un()); };
});
```
Jalankan `svelte-autofixer` pada kode `.svelte` sebelum dipakai (aturan proyek).

## Berkas konfigurasi kunci (v2)
`build.devUrl`, `build.frontendDist`, `build.beforeDevCommand`, `build.beforeBuildCommand`, `app.withGlobalTauri`, `app.security.pattern`, `app.security.assetProtocol.scope`, `app.trayIcon`, `app.windows[].dragDropEnabled`, `bundle.*`, `bundle.createUpdaterArtifacts`, `mainBinaryName`, `plugins.updater`. Skema lengkap: https://v2.tauri.app/reference/config/ (tidak dibaca di sesi ini).
