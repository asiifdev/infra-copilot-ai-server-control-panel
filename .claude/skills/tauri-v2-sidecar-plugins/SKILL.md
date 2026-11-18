---
name: tauri-v2-sidecar-plugins
description: Muat skill ini saat mengerjakan cangkang desktop Tauri v2 untuk infra-copilot - sidecar/externalBin, plugin shell dan scope capabilities, sistem plugin (tauri add, urutan init), plugin resmi (single-instance, autostart, updater, store, log, window-state, deep-link, fs, http, opener), tray/menu-bar, jalan tanpa jendela, siklus hidup dan exit, serta memilih engine tertanam vs sidecar dan komunikasi loopback. Bukan untuk UI Svelte atau logika inti.
---

# Tauri v2: sidecar, plugin, tray, integrasi desktop

Dibaca dari dokumentasi resmi pada 2026-11-19 (lihat "Sumber dan verifikasi"). Hal yang tidak terverifikasi ditandai **[BELUM TERVERIFIKASI]**. Jangan menebak nama kunci konfigurasi; cek halaman resmi atau Context7 (`/tauri-apps/tauri-docs`) dulu.

## 1. Sidecar (external binary)

Konfigurasi di `src-tauri/tauri.conf.json`, path relatif terhadap berkas itu:

```json
{ "bundle": { "externalBin": ["binaries/engine"] } }
```

- Wajib ada berkas dengan **akhiran target triple**: `src-tauri/binaries/engine-aarch64-apple-darwin`, `engine-x86_64-unknown-linux-gnu`, dst. (Windows: ditambah `.exe`.) Triple host: `rustc --print host-tuple` (Rust >= 1.84; sebelumnya parse `rustc -Vv`).
- Skrip penamaan ulang dari dokumen tidak berfungsi untuk cross-compile; di CI, build engine per target lalu salin dengan nama triple yang sesuai.
- Di Rust, `sidecar("engine")` memakai **nama berkas saja**, bukan `binaries/engine`. Di JS, `Command.sidecar('binaries/engine')` harus **sama persis** dengan string di `externalBin`.
- Plugin shell harus sudah terpasang dan diinisialisasi.

### Spawn dari Rust

```rust
use tauri_plugin_shell::{ShellExt, process::CommandEvent};

let (mut rx, child) = app.shell().sidecar("engine")?
    .args(["--listen", "127.0.0.1:0"])
    .env("ENGINE_DATA_DIR", data_dir)
    .set_process_group(true)   // kill ikut membunuh proses anak
    .spawn()?;
tauri::async_runtime::spawn(async move {
    while let Some(ev) = rx.recv().await {
        match ev {
            CommandEvent::Stdout(b) => { /* baris log / port */ }
            CommandEvent::Stderr(b) => { /* log error */ }
            CommandEvent::Error(e) => { /* gagal baca/wait */ }
            CommandEvent::Terminated(p) => { /* p.code, p.signal -> crash restart */ break; }
            _ => {}
        }
    }
});
```

Fakta (kode sumber plugin shell, via Context7): `spawn()` mengembalikan `(Receiver<CommandEvent>, CommandChild)`; `Terminated` selalu event terakhir (setelah stdout/stderr habis); `CommandChild::write()` menulis ke stdin; `CommandChild::kill()` mengirim **SIGKILL** (POSIX) / terminate (Windows), bukan sinyal halus; `kill_on_drop` default `false` (child yang di-drop tetap berjalan); `set_process_group(true)` membunuh seluruh grup/job object; `kill_on_drop` tidak berlaku untuk proses yang dijalankan dari JS API, plugin membunuhnya saat app keluar. Method lain: `env`, `envs`, `env_clear`, `current_dir`, `set_raw_out`.

### Spawn dari JS (hindari untuk engine)

`Command.sidecar(...).execute()` butuh `shell:allow-execute`; `.spawn()` butuh `shell:allow-spawn`. Spawn dari Rust tidak melewati capabilities webview, jadi **lebih aman**: webview tidak perlu hak menjalankan apa pun.

### Argumen dan scope

Di `capabilities/*.json`, entri `allow` berisi `name`, `sidecar: true`, dan `args`: `true` (bebas), `false` (tak ada), atau daftar berurutan dengan argumen statis dan `{ "validator": "\\S+" }` untuk dinamis. Argumen JS harus cocok persis dengan daftar itu.

### Lifecycle yang disarankan (pola, bukan API resmi)

1. **Start**: spawn dari `setup`/`RunEvent::Ready`; port `0` atau port acak, engine mencetak port aktual di stdout (satu baris JSON); simpan `CommandChild` di state (`Mutex<Option<CommandChild>>`) via `app.manage`.
2. **Health check**: poll `GET /healthz` loopback dengan timeout dan backoff sebelum membuka webview/UI.
3. **Stop halus**: dokumentasi hanya menyediakan `kill()` (SIGKILL). Untuk stop halus, kirim perintah lewat stdin (`child.write`) atau endpoint admin loopback, tunggu `Terminated` dengan batas waktu, baru `kill()`. **[BELUM TERVERIFIKASI]** apakah ada API sinyal SIGTERM di plugin shell; tidak ditemukan.
4. **Crash restart**: pada `Terminated` yang tak diminta, restart dengan backoff eksponensial dan batas percobaan; tampilkan status di tray.
5. **Stdout/stderr**: kuras terus (channel berkapasitas 1 di sumber; jangan biarkan buffer penuh), teruskan ke log, **redaksi rahasia** sebelum log/UI.
6. **Exit app**: tangani `RunEvent::Exit`/`ExitRequested` untuk menghentikan engine; jangan mengandalkan drop.

## 2. Plugin shell dan keamanan

- Izin default plugin shell hanya `allow-open` (http(s), tel, mailto). `shell.open` kini dipindah ke plugin **opener**.
- Izin tersedia: `shell:allow-execute|spawn|kill|stdin-write|open` (dan `deny-*`), semuanya tanpa scope bawaan. Semua perintah berbahaya diblokir sampai capability menyalakannya.
- Prinsip: beri `shell:allow-*` hanya untuk sidecar bernama dengan `args` ketat; jangan pernah `cmd` bebas atau `args: true` untuk perintah yang dapat menjalankan shell (`sh -c`). Ini selaras aturan produk "bukan shell bebas".

## 3. Sistem plugin

- Tambah: `npm run tauri add <nama>` (mengubah Cargo, JS, dan menambah izin default; **detail perubahan persis [BELUM TERVERIFIKASI]**). Manual: `cargo add tauri-plugin-<nama>` + `.plugin(tauri_plugin_<nama>::init())` + paket npm `@tauri-apps/plugin-<nama>` + izin di capabilities.
- Plugin desktop-only didaftarkan di `setup` dengan `#[cfg(desktop)] app.handle().plugin(...)`, atau di Builder: `builder.plugin(...)`.
- **Urutan**: `single-instance` harus **plugin pertama** didaftarkan. Bila memakai deep-link bersama single-instance: aktifkan fitur `deep-link` pada single-instance dan daftarkan sebelum `tauri_plugin_deep_link`.
- Menulis plugin sendiri: `tauri plugin new <nama>`; crate `tauri-plugin-<nama>`, paket JS `@scope/plugin-<nama>`; hook `setup`, `on_navigation`, `on_webview_ready`, `on_event`, `on_drop`; perintah baru tidak bisa diakses frontend sampai ada permission (`permissions/*.toml`, atau autogenerate lewat `COMMANDS` di `build.rs` menghasilkan `allow-<cmd>`/`deny-<cmd>`). Identifier: huruf kecil, angka, tanda hubung.
- Pustaka plugin resmi mensyaratkan Rust >= 1.90 (tercantum di halaman plugin).

## 4. Plugin resmi relevan (satu baris)

- **single-instance**: satu instans; callback `(app, args, cwd)` untuk memfokuskan jendela. Tanpa API JS, tanpa capability. Linux memakai DBus (`org.{id}.SingleInstance`); snap/flatpak butuh izin DBus di manifest.
- **autostart**: `tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![..args]))`. Hanya desktop. **[BELUM TERVERIFIKASI]** izin JS dan perilaku per OS selain contoh init.
- **updater**: tanda tangan wajib dan tidak bisa dimatikan; `bundle.createUpdaterArtifacts: true`; `plugins.updater.pubkey` (isi kunci, bukan path) dan `endpoints` (TLS dipaksa); kunci privat lewat env `TAURI_SIGNING_PRIVATE_KEY` (`.env` tidak bekerja); endpoint dinamis balas 204 bila tak ada update; di Windows app otomatis keluar saat instalasi (`on_before_exit`). Izin `updater:default`. Kunci hilang berarti tidak bisa merilis update lagi. Bila engine sidecar, hentikan sebelum install.
- **notification**: notifikasi OS. **[BELUM TERVERIFIKASI]** (tidak dibaca halamannya).
- **store**: penyimpanan key-value persisten; izin `store:default`. Untuk preferensi UI, bukan data inti (inti memakai SQLite sendiri).
- **log**: logging; izin `log:default`. **[BELUM TERVERIFIKASI]** target dan rotasi.
- **window-state**: simpan/pulihkan posisi dan ukuran jendela; init `tauri_plugin_window_state::Builder::default().build()`.
- **deep-link**: skema URL kustom; di desktop teruskan argumen ke instans berjalan lewat single-instance. Bila skema didefinisikan saat runtime, cek juga `argv` di callback.
- **fs**: akses berkas dari webview, dibatasi scope (`fs:scope` dengan `allow` `$APPDATA/**/*`). Lebih baik berkas dibaca engine di Rust, bukan webview.
- **http**: klien HTTP dari webview dengan scope URL. **[BELUM TERVERIFIKASI]** format scope. Panggilan ke engine loopback tidak butuh plugin ini bila memakai `fetch` biasa (lihat bagian 8) tetapi CSP harus mengizinkan.
- **opener**: membuka URL/berkas dengan aplikasi default (`opener:allow-open-url` dengan scope `url`, mis. `https://tauri.app`, `custom:*`). Dipakai untuk "Buka di browser".
- **localhost**: melayani aset lewat server localhost; dokumentasi memberi peringatan keras soal risiko keamanan. Jangan dipakai tanpa alasan kuat.

## 5. Tray dan menu bar

- Cargo: `tauri = { version = "2", features = ["tray-icon"] }`. API Rust `tauri::tray::TrayIconBuilder`, JS `TrayIcon.new` dari `@tauri-apps/api/tray`.
- Menu: `.menu(&menu)`; secara default tampil pada klik kiri dan kanan; `show_menu_on_left_click(false)` / `menuOnLeftClick: false` untuk mematikannya. Event menu: `.on_menu_event(|app, e| match e.id.as_ref() { "quit" => app.exit(0), _ => {} })`.
- Event ikon: `Click`, `DoubleClick`, `Enter`, `Move`, `Leave`. **Linux: event ikon tidak didukung** (ikon tampil dan menu konteks klik kanan tetap muncul). Karena itu, di Linux semua aksi penting harus ada di menu, bukan hanya klik ikon. Detail daemon/AppIndicator Linux **[BELUM TERVERIFIKASI]**.
- Contoh klik kiri menampilkan jendela:

```rust
.on_tray_icon_event(|tray, ev| {
  if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
    if let Some(w) = tray.app_handle().get_webview_window("main") {
      let _ = w.unminimize(); let _ = w.show(); let _ = w.set_focus();
    }
  }
})
```

- **macOS menu-bar-only**: di `setup`, `#[cfg(target_os = "macos")] app.set_activation_policy(tauri::ActivationPolicy::Accessory);` (dari referensi API; hilang dari Dock). Ada juga `set_dock_visibility(false)`. `RunEvent::Reopen { has_visible_windows }` (macOS) muncul saat ikon Dock diklik; gunakan untuk menampilkan jendela. Ikon template/gelap-terang macOS dan perilaku Windows overflow ikon **[BELUM TERVERIFIKASI]**.

## 6. Tanpa jendela utama, sembunyi ke tray, exit

- Tutup jendela terakhir memicu `RunEvent::ExitRequested`; `api.prevent_exit()` menjaga proses hidup (tidak berlaku untuk `AppHandle::restart`). `RunEvent::Exit` untuk pembersihan akhir.

```rust
tauri::Builder::default()
  .build(tauri::generate_context!())?
  .run(|app, ev| match ev {
    tauri::RunEvent::ExitRequested { api, code, .. } if code.is_none() => api.prevent_exit(), // tutup jendela = sembunyi
    tauri::RunEvent::Exit => { /* hentikan engine */ }
    _ => {}
  });
```

  Bentuk field `code` pada `ExitRequested` (`Option<i32>`) tersirat dari sumber `ExitRequestApi`; **cek di docs.rs sebelum memakai**. Menu "Quit" harus memanggil `app.exit(0)` (itu memicu `ExitRequested` dengan kode, jadi tangani agar tidak dicegat).
- Pola alternatif: intercept `WindowEvent::CloseRequested` lalu `window.hide()` + `api.prevent_close()` **[BELUM TERVERIFIKASI di halaman resmi pada sesi ini]**.
- Tanpa jendela sama sekali: konfigurasi `app.windows` kosong dan buat jendela sesuai permintaan dengan `WebviewWindowBuilder` **[BELUM TERVERIFIKASI]**.

## 7. Engine tertanam vs sidecar untuk infra-copilot

| Aspek | Tertanam (crate library dalam proses Tauri) | Sidecar (biner terpisah) |
|---|---|---|
| Satu bahasa/workspace | Ya, engine dipanggil sebagai crate | Ya (Rust), tetapi dua biner |
| Biner yang sama di server | Perlu tetap ada biner `engine` mandiri untuk edisi server, jadi crate engine tetap dipakai dua tempat | Biner yang sama persis dengan edisi server |
| Isolasi crash/memori | Crash engine = app jatuh | Crash terisolasi; bisa restart |
| Update engine | Satu paket | `externalBin` ikut bundle dan updater; versi engine dan shell terkunci bersama |
| Penandatanganan/notarisasi macOS | Satu biner | Sidecar juga harus ditandatangani; **[BELUM TERVERIFIKASI]** detail notarisasi sidecar |
| Kompleksitas | Rendah, tanpa penamaan triple | Naming triple, CI, lifecycle (bagian 1) |
| Keamanan | Surface IPC Tauri besar jika salah scope | Proses terpisah, bisa diberi hak lebih sempit (kemudian) |
| Tray tanpa jendela | Engine hidup selama app hidup | Perlu supervisor |

Rekomendasi (usulan, perlu keputusan pemilik): **MVP = engine lokal + UI di browser + tray** (tanpa webview). Tahap Tauri: **sidecar**, karena biner engine yang sama dipakai edisi server (satu artefak, satu set uji), isolasi crash, dan cangkang tipis. Embedding dipertimbangkan hanya jika ukuran/start-up terukur lebih baik (ukur di M0, ADR 0005).

## 8. Komunikasi loopback dan keamanan

- **IPC Tauri** (`invoke`, protokol `ipc:`): dikendalikan capabilities per jendela; paling terkontrol. CSP `connect-src` harus memuat `ipc: http://ipc.localhost`.
- **Protokol kustom** (default pelayanan aset): aset UI dilayani tanpa membuka port. Plugin **localhost** menggantinya dengan server HTTP dan dokumentasi memperingatkan risikonya.
- **Server HTTP loopback milik engine**: UI memanggil `fetch('http://127.0.0.1:PORT')`; tambahkan ke `connect-src` CSP. Risiko: proses lokal lain, halaman web di browser (DNS rebinding/CSRF), dan pengguna lain di mesin yang sama dapat menjangkau port. Mitigasi (rekomendasi, bukan dari dokumen): bind hanya `127.0.0.1`, port acak, **token acak per sesi** (header `Authorization`) yang diberikan lewat stdout/IPC, periksa `Host` dan `Origin`, CORS ketat, tidak ada endpoint tanpa autentikasi, tidak ada aksi tingkat 3-4 tanpa Usulan+persetujuan (aturan proyek).
- Unix domain socket / named pipe untuk engine **[BELUM TERVERIFIKASI]** di dokumentasi Tauri; hanya opsi desain.

## 9. Bagaimana ini berlaku untuk infra-copilot

1. **MVP termurah**: biner `engine` jalan lokal, UI dibuka di browser default (via `opener` atau perintah OS), ikon tray/menu-bar menampilkan status dan menu (Open, Status, Quit). Tidak perlu webview; tray butuh proses ber-event-loop (Tauri tanpa jendela, `Accessory` di macOS) **atau** crate tray terpisah **[BELUM TERVERIFIKASI]**.
2. **Cangkang Tauri tahap berikutnya**: `externalBin: ["binaries/engine"]`, spawn dari Rust (bukan JS), `single-instance` pertama, `window-state`, `updater` (pasang kunci tanda tangan), `autostart` opsional. Webview memuat UI SPA yang sama; token sesi diberikan lewat IPC command, bukan URL.
3. **Capabilities minimal**: jangan beri `shell:*` ke webview sama sekali; spawn engine hanya dari Rust. Jangan plugin `fs`/`http` ke webview bila engine yang melakukannya.
4. **Rahasia** tidak boleh lewat argumen CLI sidecar (terlihat di daftar proses); gunakan stdin atau env, dan redaksi stdout/stderr sebelum log (aturan 2 proyek).
5. **Uji**: WebKit (macOS, Linux) vs WebView2 (Windows); uji stop/crash-restart engine, exit lewat tray, dan update Windows (app keluar otomatis).
6. Jangan menjalankan atau mengarahkan uji ke stack produksi di mesin pemilik (lihat CLAUDE.md proyek).

## Sumber dan verifikasi

Dibaca penuh via browser pada 2026-11-19:
- https://v2.tauri.app/develop/sidecar/ (update terakhir halaman: 15 Jun 2026)
- https://v2.tauri.app/learn/system-tray/ (20 Apr 2026)
- https://v2.tauri.app/plugin/shell/ (22 Feb 2025)
- https://v2.tauri.app/plugin/single-instance/ (28 Aug 2026)
- https://v2.tauri.app/develop/plugins/ (20 Aug 2026)
- https://v2.tauri.app/plugin/localhost/ (22 Feb 2025)
- https://v2.tauri.app/plugin/updater/ (28 Nov 2025)

Via Context7 (`/tauri-apps/tauri-docs`, `/tauri-apps/tauri`, `/tauri-apps/plugins-workspace`), kutipan sumber docs/kode, 2026-11-19: autostart, window-state, deep-link, opener, fs, store, log, updater config, CSP/IPC, localhost, ActivationPolicy/Reopen/ExitRequested, perilaku `CommandChild`/`CommandEvent`.

**Tidak diverifikasi** (tanda di teks): halaman notification, http, autostart, deep-link, window-state, store, log, opener, fs dibaca hanya sebagai cuplikan Context7; `tauri add` detail; halaman security/capabilities dan lifecycle tidak dibuka; WindowEvent::CloseRequested; tanpa jendela awal; Linux tray prasyarat; notarisasi sidecar; API stop halus. Rekomendasi bagian 7-9 adalah penilaian saya, bukan klaim dokumentasi.
