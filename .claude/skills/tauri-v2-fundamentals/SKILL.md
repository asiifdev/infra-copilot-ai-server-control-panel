---
name: tauri-v2-fundamentals
description: Dasar Tauri v2 (arsitektur, tata letak src-tauri, tauri.conf.json, lib.rs vs main.rs, command, invoke, event, Channel, state, capabilities, integrasi Vite + Svelte, beda dari v1). Muat saat membuat atau mengubah crate cangkang desktop Tauri, menulis #[tauri::command], memanggil invoke dari TypeScript, atau mengatur devUrl/frontendDist di infra-copilot.
---

# Tauri v2 fundamentals (untuk infra-copilot)

Diverifikasi dari dokumentasi resmi https://v2.tauri.app pada 2026-11-19 (lihat bagian sumber di akhir). Bagian yang tidak terverifikasi diberi tanda **[TIDAK TERVERIFIKASI]**. Jangan mengarang nama API; cek ulang ke docs (Context7 `/tauri-apps/tauri-docs`) bila ragu.

## 1. Arsitektur

- Aplikasi = proses Rust + webview bawaan OS yang menampilkan HTML/JS/CSS statis. Tidak ada runtime yang dibawa; biner dikompilasi dari Rust.
- Crate `tauri` adalah inti: membaca `tauri.conf.json` saat kompilasi, menyuntikkan skrip, menyediakan API sistem. Tauri memakai langsung **TAO** (pembuatan dan pengelolaan jendela, fork winit) dan **WRY** (lapisan abstraksi webview) untuk memanggil OS.
- Webview per OS: WebKit di macOS dan Linux (WebKitGTK), Microsoft Edge WebView2 di Windows. Akibatnya perilaku CSS/JS bisa berbeda antar OS; uji di ketiganya.
- Tauri bekerja seperti static web host: JS dikompilasi jadi berkas statis dulu, lalu Rust menanamnya.
- Komunikasi frontend-backend lewat message passing: **command** (bertipe, request/response), **event** (dinamis, JSON), **Channel** (streaming berurutan).

## 2. Tata letak proyek

```
package.json, index.html, src/        # proyek JS (SPA)
src-tauri/
  Cargo.toml, build.rs                # build.rs berisi tauri_build::build()
  tauri.conf.json                     # konfigurasi utama; juga penanda bagi CLI
  capabilities/default.json           # izin per jendela (wajib untuk command/plugin)
  icons/
  src/lib.rs                          # kode aplikasi + entry point mobile
  src/main.rs                         # entry desktop tipis: memanggil app_lib::run()
```

**Mengapa `lib.rs` dan bukan `main.rs`:** build mobile mengompilasi aplikasi sebagai pustaka dan memuatnya lewat framework platform, jadi `run()` ada di `lib.rs` dengan `#[cfg_attr(mobile, tauri::mobile_entry_point)]`. `main.rs` hanya memanggil `app_lib::run()` (`app_lib` = `[lib.name]` di Cargo.toml); jangan diubah, ubah `lib.rs`. Walau infra-copilot hanya desktop, pertahankan pola ini agar sesuai scaffold resmi.

Docs menyebut: bila ingin Rust saja, `src-tauri/` boleh dijadikan anggota workspace Cargo. Cocok dengan keputusan workspace di 0004 (crate `desktop`). Penempatan persis untuk infra-copilot (apakah `src-tauri/` di dalam crate `desktop`, di mana `ui/` berada) adalah keputusan M0, bukan fakta docs. `tauri.conf.json` dan `capabilities/` tetap harus ditemukan CLI.

Entry point minimal (pola dari docs):

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![my_cmd])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

## 3. Prasyarat per OS (ringkas)

- **Linux (Debian/Ubuntu):** `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`. Docs juga memberi daftar Fedora, Arch, openSUSE (paket `webkit2gtk4.1-devel` dst.).
- **macOS:** Xcode penuh hanya untuk iOS; untuk desktop saja cukup `xcode-select --install`.
- **Windows:** Microsoft C++ Build Tools (centang "Desktop development with C++"); WebView2 sudah ada di Windows 10 (1803+) dan lebih baru, kalau tidak pasang Evergreen Bootstrapper. Docs juga menyinggung VBSCRIPT untuk installer MSI.
- Semua OS: Rust (rustup) dan Node.js untuk frontend. **[TIDAK TERVERIFIKASI]** versi minimum Rust/Node; halaman prerequisites tidak terbaca utuh di browser, hanya potongan lewat Context7.

## 4. Alur create / dev / build

- Scaffold: `npm create tauri-app@latest` (ada varian yarn/pnpm/deno/bun/cargo `create-tauri-app`). Menambah ke proyek frontend yang ada: `npx tauri init` (menanyakan nama app, judul jendela, lokasi aset web, URL dev server, perintah dev/build).
- Skrip `"tauri": "tauri"` di package.json menjalankan CLI (terlihat di panduan Vite).
- `tauri dev` dan `tauri build` (mis. `npm run tauri dev`): **[TIDAK TERVERIFIKASI]** di sesi ini, halaman CLI reference tidak dibaca; perilaku yang terverifikasi hanya hook `beforeDevCommand`/`beforeBuildCommand` di bagian 8.
- Penandatanganan macOS, notarisasi, updater: belum dikaji (di luar cakupan skill ini).

## 5. Command

```rust
#[tauri::command]
fn greet(name: String) -> String { format!("Hello {name}") }   // tidak pub di lib.rs
```

- Daftarkan **semua** command dalam **satu** `tauri::generate_handler![a, b]`; memanggil `invoke_handler` berkali-kali hanya memakai yang terakhir.
- Di `lib.rs`, command **tidak boleh `pub`** (error `__cmd__...` duplikat). Di modul terpisah harus `pub`, dan didaftarkan dengan `commands::nama`. Nama command tidak ber-scope modul dan harus unik.
- Argumen: objek JSON dengan kunci **camelCase** (`invoke('x', { invokeMessage })` untuk `invoke_message`); `#[tauri::command(rename_all = "snake_case")]` untuk mengubahnya. Tipe argumen harus `serde::Deserialize`, tipe hasil `serde::Serialize`.
- **Sync vs async:** command tanpa `async` jalan di main thread kecuali `#[tauri::command(async)]`. Command `async` dijalankan di task terpisah (`async_runtime::spawn`) dan dianjurkan untuk kerja berat agar UI tidak membeku. Argumen pinjaman (`&str`, `State<'_, T>`) di fungsi async: ubah ke tipe milik (`String`) atau kembalikan `Result<_, _>` (wajib untuk `State<'_, T>`).
- Data biner besar: kembalikan `tauri::ipc::Response::new(bytes)` agar tidak diserialisasi JSON.
- Parameter khusus yang bisa disuntik: `tauri::AppHandle`, `tauri::WebviewWindow`, `tauri::State<'_, T>`, `tauri::ipc::Channel<T>`, `tauri::ipc::Request`.

### Error

Semua yang dikembalikan, termasuk error, harus `Serialize`. `Err` menolak promise di JS. Pola resmi: enum `thiserror` dengan `impl serde::Serialize` manual.

```rust
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error("failed to parse as string: {0}")] Utf8(#[from] std::str::Utf8Error),
}
#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
enum ErrorKind { Io(String), Utf8(String) }
impl serde::Serialize for Error {
    fn serialize<S: serde::ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let msg = self.to_string();
        match self { Self::Io(_) => ErrorKind::Io(msg), Self::Utf8(_) => ErrorKind::Utf8(msg) }.serialize(s)
    }
}
```

Di JS errornya menjadi `{ kind: 'io' | 'utf8', message: string }`. Anotasi tipe di `.catch((e: ErrorKind) => ...)` hanya anotasi, bukan validasi.

## 6. invoke dari TypeScript

```ts
import { invoke } from '@tauri-apps/api/core';
const msg = await invoke<string>('greet', { name: 'x' });   // reject bila Err
```

Alternatif `window.__TAURI__.core.invoke` hanya bila `app.withGlobalTauri` = true; untuk SPA Vite pakai paket npm `@tauri-apps/api`. Jangan bungkus invoke di mana-mana: buat satu modul klien tipis bertipe di `ui/`.

## 7. Event, Channel, State

**Event** (dua arah, JSON, tanpa tipe, tanpa kontrol capabilities, tidak untuk throughput/latensi tinggi):

```rust
use tauri::{AppHandle, Emitter};
app.emit("download-progress", 50).unwrap();          // global
app.emit_to("main", "login-result", "loggedIn").unwrap(); // satu webview
```
```ts
import { listen } from '@tauri-apps/api/event';
const unlisten = await listen<number>('download-progress', e => console.log(e.payload));
unlisten(); // panggil saat komponen dilepas
```
Jebakan: `listen` mengembalikan Promise; `await` dulu sebelum `unlisten`. Event khusus-webview tidak diterima listener global biasa (gunakan `target: { kind: 'Any' }` di JS atau `listen_any` di Rust). Payload event Rust harus `Serialize + Clone`. Listener di router SPA tidak dibersihkan otomatis; bersihkan di `onDestroy`/cleanup efek Svelte.

**Channel** untuk streaming berurutan (dipakai internal Tauri untuk progres unduh, output proses anak, WebSocket):

```rust
use tauri::ipc::Channel;
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
enum Ev { Progress { pct: u8 }, Done }
#[tauri::command]
fn run_job(on_event: Channel<Ev>) { on_event.send(Ev::Progress { pct: 10 }).unwrap(); on_event.send(Ev::Done).unwrap(); }
```
```ts
import { invoke, Channel } from '@tauri-apps/api/core';
const ch = new Channel<Ev>(); ch.onmessage = m => {/* ... */};
await invoke('run_job', { onEvent: ch });
```

**State:** `Builder::manage(x)` (atau `app.manage` di `setup`), diambil dengan `State<'_, T>` atau `app.state::<T>()`.
- Mutasi: bungkus `std::sync::Mutex<T>`. Tidak perlu `Arc`; Tauri sudah mengurusnya. Mutex std sering cukup dan lebih disukai di kode async; pakai async mutex (tokio) hanya bila guard harus ditahan melewati `await` (mis. koneksi DB/IO).
- Tipe harus sama persis: `State<'_, AppState>` padahal yang di-`manage` adalah `Mutex<AppState>` **tidak error saat kompilasi**; `invoke` ditolak saat runtime ("state not managed"), dan `Manager::state()` panic. Gunakan type alias dan pakai apa adanya.

## 8. Window / webview dan wiring Vite + Svelte

- Jendela utama berlabel `main` secara bawaan (label dipakai di `get_webview_window("main")`, capabilities `"windows": ["main"]`, `emit_to`). **[TIDAK TERVERIFIKASI]** definisi jendela di `app.windows` pada tauri.conf.json (halaman config reference tidak dibaca); lihat https://v2.tauri.app/reference/config/.
- Dari JS: `getCurrentWebviewWindow()` dari `@tauri-apps/api/webviewWindow`. `tauri::WebviewWindow` di Rust; `tauri::Window` perilakunya berubah di v2.
- `tauri.conf.json` (terverifikasi di panduan Vite):

```json
{ "build": {
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build",
    "devUrl": "http://localhost:5173",
    "frontendDist": "../dist" } }
```
- `vite.config.ts` (resmi, untuk Vite 8): `clearScreen: false`, `server.port: 5173` dan `strictPort: true` (harus sama dengan `devUrl`), `server.host: process.env.TAURI_DEV_HOST || false`, `server.watch.ignored: ['**/src-tauri/**']`, `envPrefix: ['VITE_', 'TAURI_ENV_*']`, `build.target` = `chrome105` di Windows dan `safari13` selain itu, minify/sourcemap mengikuti `TAURI_ENV_DEBUG`. Target `safari13` konsisten dengan peringatan WebKit lama di 0003.
- Svelte tidak punya halaman panduan khusus yang dibaca di sini; Svelte 5 + Vite polos tanpa SvelteKit mengikuti panduan Vite di atas (SPA statis, tanpa SSR). Halaman "Svelte" di v2.tauri.app/start/frontend/ **[TIDAK TERVERIFIKASI]** (tidak dibuka).

## 9. Capabilities (keamanan)

Izin v2 berbasis ACL: berkas capability di `src-tauri/capabilities/` menentukan jendela/webview mana yang boleh memakai plugin dan API inti. Contoh dari docs:

```json
{ "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main-capability", "windows": ["main"],
  "permissions": ["core:event:default", "core:window:default", "core:window:allow-set-title"] }
```
Izin inti memakai awalan `core:`.

**Command buatan sendiri (terverifikasi 2026-11-19, dokumentasi resmi `security/capabilities`):** semua command yang didaftarkan lewat `tauri::Builder::invoke_handler` **boleh dipanggil oleh semua window dan webview secara bawaan**. Untuk membatasinya, daftarkan hanya command yang diizinkan di `build.rs`:

```rust
fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["your_command"])),
    )
    .unwrap();
}
```

Untuk proyek ini, pembatasan itu **wajib** (lihat skill `tauri-v2-security`): sedikit command, eksplisit, dan tiap command memvalidasi argumennya.

Catatan lint: workspace kita melarang `unwrap()` (`clippy::unwrap_used`). Skrip build memang lazim berhenti dengan panik bila gagal, jadi di `build.rs` saja izinkan secara eksplisit dengan alasan tertulis: `#![allow(clippy::unwrap_used)] // build script: a failed build must stop`.

## 10. Beda dari Tauri v1 (sumber kesalahan pengetahuan usang)

- `tauri.conf.json`: `package` dihapus (`productName`, `version` ke tingkat atas); kunci `tauri` jadi `app`; `build.devPath` jadi `devUrl`, `build.distDir` jadi `frontendDist`; `allowlist` dihapus diganti capabilities; `tauri > bundle` pindah ke tingkat atas; `systemTray` jadi `app.trayIcon`; updater pindah ke `plugins.updater`; `fileDropEnabled` jadi `dragDropEnabled`; perlu `mainBinaryName` bila ingin nama biner = productName.
- Rust: `tauri::Window` vs `tauri::WebviewWindow` dipisah; `tauri::api::path` pindah ke `tauri::path`; `emit`/`listen` kini lewat trait `Emitter` dan `Listener` (`use tauri::{Emitter, Listener, Manager}`); `tauri::command` module pindah ke `tauri::ipc`; fitur `system-tray` jadi `tray-icon`; `FileDropEvent` jadi `DragDropEvent`.
- JS: modul `@tauri-apps/api/tauri` v1 sekarang `@tauri-apps/api/core` (invoke); `getCurrent` jadi `getCurrentWindow`, dan yang biasanya dimaksud adalah `getCurrentWebviewWindow` dari `webviewWindow` (terverifikasi di blog rilis v2 untuk `getAll/getCurrent`; perpindahan `tauri` ke `core` terlihat dari contoh docs, bukan dari daftar perubahan).
- API sistem v1 (fs, shell, dialog, dll.) di v2 menjadi plugin terpisah. **[TIDAK TERVERIFIKASI]** daftar lengkapnya; cek halaman migrasi.
- CLI `migrate` mengubah allowlist v1 menjadi berkas capability.
- Protokol kustom di Windows dan Android memakai `http`, bukan `https`.

## 11. Jebakan umum

1. `pub` pada command di `lib.rs` -> error makro duplikat.
2. `invoke_handler` dipanggil lebih dari sekali -> hanya yang terakhir berlaku.
3. Kunci argumen snake_case dari JS -> argumen tidak terikat; pakai camelCase.
4. Tipe `State` tidak cocok dengan yang di-`manage` -> gagal saat runtime, bukan kompilasi.
5. Command sinkron untuk kerja berat membekukan UI; gunakan `async`.
6. Hasil besar lewat JSON lambat; pakai `ipc::Response` atau Channel.
7. `listen` tanpa `await` sebelum `unlisten`; listener tidak dibersihkan di SPA.
8. Port Vite tidak sama dengan `devUrl` (gunakan `strictPort: true`).
9. Uji di WebKit dan WebView2; jangan mengandalkan fitur Chromium-only.
10. Event bukan jalur untuk data besar atau yang butuh kontrol izin halus.

## 12. Penerapan di infra-copilot

- **Cangkang tipis.** Tauri hanya: membuka jendela, menyalakan/menghubungkan engine lokal (di desktop, engine jalan lokal; di server, SPA yang sama dilayani engine lewat HTTP), dan menyediakan sedikit fungsi OS. Logika bisnis, agen, kolektor, pelaksana aksi, DB, dan tingkat risiko tetap di crate engine, bukan di command Tauri.
- **Command sedikit dan eksplisit.** Daftar kecil (mis. status/alamat engine, mulai/hentikan engine lokal, buka tautan, versi). Tidak ada command generik "jalankan perintah/shell" atau "baca berkas bebas"; itu melanggar aturan 1 dan 3 di CLAUDE.md. Setiap command didaftarkan di `generate_handler!` dan diberi izin eksplisit; argumen bertipe kuat, error enum `Serialize` tanpa membocorkan rahasia (aturan 2). Rahasia tidak lewat `invoke`/event/log.
- **SPA berbicara ke engine lewat API yang sama** dengan mode server, bukan lewat puluhan command Tauri; satu klien API di `ui/` dipakai di kedua edisi. Streaming (log, progres) lewat Channel bila perlu, atau lewat transport API engine.
- Keluaran dari server/log dianggap data (aturan 4): jangan di-`eval` ke webview (`WebviewWindow::eval`) dan jangan dirender sebagai HTML mentah.
- Prinsip teringan-tercepat (0005): tanpa plugin yang tidak diperlukan; pertahankan `build.target` Vite sesuai tabel resmi; ukur biner dan memori di M0.
- Evaluasi tiap plugin (fs, shell, updater) dan versi crate dengan docs terkini sebelum dipakai; plugin tidak dipilih di skill ini.
- Penandatanganan, notarisasi, updater, dan sidecar: belum dikaji; tulis keputusan di `docs/decisions/` saat dikerjakan.

## Sumber dan verifikasi

Dibaca 2026-11-19 dari v2.tauri.app via browser: 
- https://v2.tauri.app/concept/architecture/ (Last updated Jun 30, 2026)
- https://v2.tauri.app/start/project-structure/ (Sep 26, 2025)
- https://v2.tauri.app/start/frontend/vite/ (Sep 3, 2026; berlaku Vite 8)
- https://v2.tauri.app/develop/calling-rust/ (Jun 8, 2026)
- https://v2.tauri.app/develop/calling-frontend/ (Sep 9, 2026)
- https://v2.tauri.app/develop/state-management/ (Oct 6, 2026)

Lewat Context7 (`/tauri-apps/tauri-docs`, sumber repo docs v2: start/prerequisites, start/create-project, start/migrate/from-tauri-1, blog/tauri-2.0, security/capabilities): prasyarat per OS, create-tauri-app, `tauri init`, ringkasan migrasi v1, contoh capability.

**Tidak terverifikasi / tidak dibaca:** halaman prerequisites utuh di browser (isi tab OS tidak terekstrak), CLI reference (`tauri dev/build`), reference/config (skema jendela), halaman security/permissions (izin command kustom), panduan Svelte resmi, plugin (shell, updater), sidecar, penandatanganan, daftar lengkap plugin hasil pecahan API v1, versi minimum Rust/Node, versi crate `tauri` terkini. Rincian tambahan ada di `references/api-cheatsheet.md`.
