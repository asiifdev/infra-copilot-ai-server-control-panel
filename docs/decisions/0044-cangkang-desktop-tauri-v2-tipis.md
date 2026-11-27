# 0044. Cangkang desktop Tauri v2 tipis (`src-tauri/`)

- Tanggal: 2026-11-28
- Status: diterima (mengikuti ADR 0003, 0004, 0029, dan 0030)
- Pengambil keputusan: pemilik proyek

## Konteks
Tahap desktop (Sprint 7) membutuhkan cangkang aplikasi desktop lintas-platform (Linux, macOS, Windows) di atas fondasi `crates/server` dan UI web `web/dist` yang sudah berjalan lewat `infra serve` (ADR 0029, 0030, 0031). Sesuai prinsip keamanan dan kinerja proyek (ADR 0003, 0004, 0005), cangkang desktop tidak boleh menggandakan logika bisnis, tidak boleh membuka port non-loopback, dan tidak boleh memberi akses IPC luas kepada webview.

## Keputusan
1. **Penempatan di root workspace (`src-tauri/`)**:
   - Menambahkan `src-tauri` sebagai anggota workspace Cargo root (`members = ["crates/*", "src-tauri"]`) dengan nama paket `nodivara-desktop` dan pustaka `nodivara_desktop_lib`, mewarisi `[workspace.lints]` (`unsafe_code = "forbid"`, `unwrap_used = "deny"`, `expect_used = "deny"`, `print_stdout = "deny"`, `print_stderr = "deny"`).
2. **Server tertanam pada `127.0.0.1:0` dengan token sekali pakai `#t=<token>`**:
   - Saat aplikasi dimulai (`setup()`), cangkang desktop menjalankan `crates/server` secara tertanam pada alamat loopback efemeral `127.0.0.1:0` (menggunakan `bootstrap::ProcessEnv` / `server::Config::resolve`).
   - Setelah `Server::bind` berhasil, cangkang mengambil URL masuk sekali pakai (`http://127.0.0.1:<port>/#t=<token>`) dan mengarahkan jendela utama `main` ke URL tersebut agar token tetap berada di fragmen URL dan tidak pernah melewati log HTTP atau header `Referer`.
3. **Pembatasan command melalui `AppManifest` di `build.rs`**:
   - Secara bawaan di Tauri v2, command yang didaftarkan di `invoke_handler` dapat dipanggil oleh semua jendela. Untuk menutup celah tersebut, `src-tauri/build.rs` memanggil `tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["app_info"])))`.
   - Command `app_info` ditulis non-`pub` di `src-tauri/src/lib.rs` dan hanya mengembalikan metadata dasar aplikasi serta origin loopback yang terikat.
4. **Keamanan webview dan capability terikat ke jendela `main`**:
   - `src-tauri/tauri.conf.json` menetapkan `withGlobalTauri: false`, `freezePrototype: true`, CSP ketat (`default-src 'self'`, `connect-src ipc: http://ipc.localhost http://127.0.0.1:*`), dan mendaftarkan capability secara eksplisit (`"capabilities": ["main-capability"]`).
   - `src-tauri/capabilities/main.json` terikat ketat ke `"windows": ["main"]` dan hanya mengizinkan permission minimal yang diperlukan (`core:window:allow-set-title`), tanpa `core:default`, tanpa plugin `shell`, tanpa plugin `fs`, dan tanpa `remote.urls`.
5. **Dependensi Linux GTK/WebKit di CI (`.github/workflows/ci.yml`)**:
   - Job `rust` pada `.github/workflows/ci.yml` menambahkan langkah instalasi `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, dan `librsvg2-dev` dengan kondisi `if: runner.os == 'Linux'` agar kompilasi `wry`/`webkit2gtk` di runner `ubuntu-latest` berjalan tanpa galat tautan pustaka sistem.

## Alasan
- Menanam `crates/server` langsung pada `127.0.0.1:0` memastikan satu jalur API, guard keamanan (`Host`, `Origin`, `X-Nodivara`, cookie sesi), dan alur persetujuan yang identik antara `infra serve` dan aplikasi desktop, tanpa konflik port tetap.
- `AppManifest::commands`, `withGlobalTauri: false`, dan capability minimal tanpa `core:default` meminimalkan permukaan serangan jika konten webview pernah terkompromi.

## Alternatif
- **Menjalankan `infra serve` sebagai sidecar terpisah**: ditolak untuk fase awal karena menambah kompleksitas manajemen proses anak, IPC siklus hidup, dan pengemasan biner ganda; menjalankan `crates/server` secara *in-process* lebih ringan dan cepat dimatikan saat jendela ditutup.
- **Memindahkan seluruh endpoint HTTP menjadi puluhan `#[tauri::command]`**: ditolak karena menggandakan kontrak API antara mode server dan mode desktop (melanggar prinsip satu UI dan satu API di ADR 0029).
