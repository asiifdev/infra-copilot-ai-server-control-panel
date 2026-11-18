---
name: tauri-v2-build-release
description: Panduan build, bundling, penandatanganan (macOS notarisasi, Windows, Linux), updater plugin, CI GitHub Actions (tauri-action), kompilasi lintas OS, penyetelan ukuran/kinerja, pengujian (mockIPC, WebDriver) dan debugging untuk cangkang Tauri v2 proyek ini. Muat saat menyentuh src-tauri/tauri.conf.json, Cargo.toml profil release, workflow rilis, kunci updater, anggaran kinerja M0, atau menyiapkan rilis tertanda tangan.
---

# Tauri v2: build, rilis, penandatanganan, pembaruan

Dibaca dari dokumentasi resmi v2.tauri.app pada 2026-11-19 (lihat "Sumber dan verifikasi" di bawah). Konteks proyek: `docs/decisions/0003`, `0004`, `0005`. Fakta bertanda **[tidak terverifikasi]** tidak ada di dokumentasi resmi yang dibaca; jangan dijadikan dasar tanpa memeriksa ulang. Jangan menebak nama kunci konfigurasi; baca https://v2.tauri.app/reference/config/ untuk kunci yang tidak tercantum di sini.

## 1. Format bundel per OS dan perintah build

| OS | Format | Catatan resmi |
|---|---|---|
| macOS | `.app`, `.dmg` | `tauri build --bundles app` / `--bundles dmg`; harus dibangun di Mac |
| Windows | `.msi` (WiX v3), `-setup.exe` (NSIS) | MSI hanya bisa dibuat di Windows; NSIS bisa lintas-kompilasi (lihat bagian 6) |
| Linux | `.deb`, `.rpm`, `.AppImage` (juga Snap, Flatpak, AUR sebagai jalur distribusi) | bangun di sistem dasar tertua yang didukung |

Perintah (opsi CLI dari sumber `tauri-cli` build.rs): `tauri build` dengan `--target <triple>` (atau `universal-apple-darwin`, butuh target `aarch64-apple-darwin` dan `x86_64-apple-darwin` terpasang), `--bundles app,dmg` (daftar dipisah koma/spasi), `--no-bundle`, `--debug`, `--features`, `--config <json|berkas>` (digabung berurutan; berkas `tauri.macos.conf.json`, `tauri.linux.conf.json`, `tauri.windows.conf.json` digabung otomatis), `--runner`, `--ci` (atau env `CI`), `--no-sign`, `--skip-stapling`, `--ignore-version-mismatches`, `--no-binary-patching`. Ada juga `tauri bundle` untuk memisah langkah:

```sh
npm run tauri build -- --no-bundle
npm run tauri bundle -- --bundles app,dmg   # di luar App Store
```

Versi aplikasi: `tauri.conf.json > version` (jika kosong, memakai `Cargo.toml`). Konfigurasi dasar yang dirujuk dokumen: `bundle.macOS.{signingIdentity, entitlements, minimumSystemVersion, frameworks, files, dmg}`, `bundle.windows.{certificateThumbprint, digestAlgorithm, timestampUrl, signCommand, webviewInstallMode, minimumWebview2Version, wix, nsis}`, `bundle.linux.{deb, appimage}`. Isi lengkap: referensi konfigurasi.

## 2. macOS: tanda tangan dan notarisasi

Prasyarat (halaman signing macOS): akun Apple Developer, berbayar "99$ per year" menurut dokumen, atau paket gratis hanya untuk uji dan **tidak bisa notarisasi**. Penandatanganan harus dari perangkat Apple. Untuk distribusi di luar App Store: sertifikat **Developer ID Application** (hanya Account Holder yang bisa membuatnya); notarisasi **wajib** dengan sertifikat itu. "Apple Distribution" untuk App Store.

- Lokal: `security find-identity -v -p codesigning`; isi `bundle.macOS.signingIdentity` atau env `APPLE_SIGNING_IDENTITY` (env menimpa konfigurasi; bila kosong dan `APPLE_CERTIFICATE` ada, identitas disimpulkan dari sertifikat).
- CI: ekspor `.p12`, ubah ke base64 (`openssl base64 -A -in cert.p12 -out cert-base64.txt`), isi `APPLE_CERTIFICATE` dan `APPLE_CERTIFICATE_PASSWORD`.
- Notarisasi, pilih satu cara kredensial:
  - App Store Connect API (disarankan untuk CI): `APPLE_API_ISSUER`, `APPLE_API_KEY` (Key ID), `APPLE_API_KEY_PATH` (berkas `.p8`, hanya bisa diunduh sekali).
  - Apple ID: `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), `APPLE_TEAM_ID`.
  - Multi-tim: `APPLE_PROVIDER_SHORT_NAME`.
- Notarisasi pertama bisa lama (docstring CLI: "multiple hours"); pakai `--skip-stapling` untuk lulus pertama tanpa menunggu, lalu jalankan tanpa flag itu.
- Tanpa kredensial notarisasi, bundler hanya memberi peringatan dan lanjut (dari sumber bundler); tetapi bila `APPLE_TEAM_ID` hilang, galat.
- Ad-hoc: `"signingIdentity": "-"`. Berguna di Apple Silicon, **tidak** menghilangkan keharusan pengguna mengizinkan di Privacy & Security, bukan pengganti notarisasi.
- Entitlements: `src-tauri/Entitlements.plist` + `bundle.macOS.entitlements: "./Entitlements.plist"`. Hardened runtime: sumber bundler memanggil `codesign --options runtime` bila pengaturan `hardened_runtime` aktif; **nama kunci JSON-nya dan nilai bawaannya [tidak terverifikasi]** di halaman yang dibaca. Periksa `reference/config` sebelum memakainya.
- Info.plist tambahan: berkas `src-tauri/Info.plist` digabung otomatis. Minimum macOS bawaan 10.13; ubah via `minimumSystemVersion`.
- Penanda tangan dalam-keluar (framework dan sidecar dulu) ditangani bundler. Untuk sidecar, env `TAURI_SKIP_SIDECAR_SIGNATURE_CHECK` ada (arti persisnya di luar yang dibaca).
- DMG: ukuran/posisi jendela di `bundle.macOS.dmg`. Peringatan resmi: ukuran dan posisi ikon **tidak diterapkan di CI** (issue tauri#1731).

## 3. Windows: tanda tangan

- Tanda tangan tidak wajib untuk berjalan, tetapi tanpanya SmartScreen memperingatkan unduhan browser; Windows 11 dengan Smart App Control memblokir aplikasi tak dikenal tanpa tanda tangan.
- Sejak 2024 sertifikat EV **tidak** lagi memberi reputasi SmartScreen instan; EV dan OV membangun reputasi dengan cara sama (dokumen resmi). Tanda tangani semua rilis dengan sertifikat yang sama agar reputasi terbawa.
- Panduan OV dalam dokumen hanya untuk sertifikat OV yang dibeli **sebelum 1 Juni 2023**; selain itu ikuti dokumentasi penerbit sertifikat dan pakai `signCommand`.
- Opsi yang didokumentasikan: (a) `certificateThumbprint` + `digestAlgorithm` + `timestampUrl` (sertifikat di penyimpanan Windows, pakai `signtool`; path `signtool` lewat `TAURI_WINDOWS_SIGNTOOL_PATH`); (b) Azure Key Vault via `relic` dan `signCommand` (`"relic sign --file %1 --key azure --config relic.conf"`); (c) **Azure Artifact Signing** (dulu Azure Trusted Signing) via `artifact-signing-cli` dan `signCommand`; (d) perintah kustom apa pun lewat `bundle.windows.signCommand` (`%1` = berkas).
- Lintas-kompilasi dari Linux/macOS **wajib** `signCommand` kustom (implementasi bawaan hanya jalan di Windows).
- Biaya sertifikat OV/EV, Azure Artifact Signing: **[tidak terverifikasi]**, dokumen Tauri tidak menyebut harga.
- WebView2: `webviewInstallMode` `downloadBootstrapper` (bawaan, +0MB), `embedBootstrapper` (~1.8MB), `offlineInstaller` (~127MB), `fixedRuntime` (~180MB), `skip`. Windows 10 (April 2018+) dan 11 membawa runtime WebView2.
- Pemasangan NSIS bawaan per-pengguna (tanpa admin, di `%LOCALAPPDATA%`); `installMode` `perMachine` atau `both` butuh admin. MSI butuh fitur Windows opsional VBSCRIPT.

## 4. Linux

- Bangun di sistem dasar **tertua** yang didukung dan punya WebKitGTK 4.1 (contoh resmi: Ubuntu 22.04, Debian 12), karena glibc baru menaikkan syarat minimum (galat `GLIBC_2.33 not found`). Disarankan Docker atau GitHub Actions.
- `.deb` bergantung pada `libwebkit2gtk-4.1-0`, `libgtk-3-0` (plus `libappindicator3-1` bila tray). Tambah dependensi/berkas lewat `bundle.linux.deb`.
- AppImage: membengkak dari kisaran 2-6 MB menjadi 70+ MB (dokumen resmi); `bundleMediaFramework` menambah GStreamer (hanya penuh didukung di Ubuntu). AppImage ARM tidak bisa lintas-kompilasi (linuxdeploy), hanya di perangkat ARM atau emulator; runner publik `ubuntu-22.04-arm` tersedia (repo publik).
- Penandatanganan: RPM dengan GPG (env `TAURI_SIGNING_RPM_KEY`, `TAURI_SIGNING_RPM_KEY_PASSPHRASE`); AppImage dengan `SIGN=1`, `SIGN_KEY`, `APPIMAGETOOL_SIGN_PASSPHRASE`, `APPIMAGETOOL_FORCE_SIGN`. Peringatan resmi: **AppImage tidak memverifikasi tanda tangan itu sendiri**; pengguna harus memverifikasi manual dan ID kunci dipublikasikan lewat kanal terautentikasi.
- Aplikasi GUI macOS/Linux tidak mewarisi `$PATH` shell; dokumen menyarankan crate `fix-path-env-rs`.
- Edisi server (biner inti Rust tanpa Tauri) bukan cakupan skill ini.

## 5. Updater plugin

Instal: `npm run tauri add updater`. Perlu Rust minimal 1.90 (halaman plugin). Tanda tangan pembaruan **tidak bisa dimatikan**.

```sh
npm run tauri signer generate -- -w ~/.tauri/myapp.key   # buat pasangan kunci
export TAURI_SIGNING_PRIVATE_KEY="<path atau isi>"        # .env TIDAK bekerja
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
```

```json
{
  "bundle": { "createUpdaterArtifacts": true },
  "plugins": { "updater": {
    "pubkey": "ISI PUBLIC KEY (bukan path)",
    "endpoints": ["https://releases.example.com/{{target}}/{{arch}}/{{current_version}}"]
  } }
}
```

- `createUpdaterArtifacts`: `true` untuk v2; `"v1Compatible"` hanya untuk migrasi; akan dihapus di v3.
- `endpoints`: TLS dipaksa di produksi; berpindah ke URL berikutnya hanya bila status non-2xx; `dangerousInsecureTransportProtocol` memperbolehkan non-HTTPS (hindari). Variabel: `{{current_version}}`, `{{target}}` (`linux|windows|darwin`), `{{arch}}` (`x86_64|i686|aarch64|armv7`). Variabel kustom tidak didukung, tetapi `target` kustom bisa (mis. `macos-universal`).
- Artefak: Linux `.AppImage` + `.sig`; macOS `.app.tar.gz` + `.sig`; Windows `-setup.exe`/`.msi` + `.sig`.
- **Format `latest.json` statis**: wajib `version` (SemVer), `platforms.<os-arch>.url`, `platforms.<os-arch>.signature` (isi berkas `.sig`, bukan path/URL); opsional `notes`, `pub_date` (RFC 3339). Kunci platform `linux-x86_64`, `darwin-aarch64`, `windows-x86_64`, dst. Seluruh berkas divalidasi sebelum cek versi: semua platform harus lengkap dan valid. `tauri-action` membuat berkas ini untuk GitHub Releases.
- Server dinamis: balas 204 bila tidak ada pembaruan, 200 + `{version, url, signature, notes?, pub_date?}` bila ada.
- Windows: `plugins.updater.windows.installMode` = `passive` (bawaan), `basicUi`, `quiet`; aplikasi **otomatis keluar** saat memasang (batasan installer), hook `on_before_exit`.
- Izin: tambahkan `"updater:default"` ke `src-tauri/capabilities/*.json` (bawaan: check, download, install, download-and-install). Beri hanya izin yang dipakai.
- Keamanan (dokumen resmi): kunci privat tidak boleh dibagikan; **bila hilang, pengguna yang sudah memasang tidak bisa menerima pembaruan lagi**. Rotasi kunci dimungkinkan lewat `pubkey` runtime (Rust), tetapi prosedur rotasi aman **[tidak terverifikasi]**. Simpan kunci di rahasia CI dan cadangan offline.
- Rollback/downgrade: `version_comparator` di Rust. Saluran rilis (stable/beta): `updater_builder().endpoints(...)`.
- Pemeriksaan JS: `check()` lalu `update.downloadAndInstall(cb)` lalu `relaunch()` dari `@tauri-apps/plugin-process`.

## 6. CI GitHub Actions dan lintas-kompilasi

Alur resmi (halaman pipeline GitHub, 2026-11-19): `tauri-apps/tauri-action@v1`, `actions/checkout@v7`, `actions/setup-node@v6`, `dtolnay/rust-toolchain@stable`, `swatinem/rust-cache@v2` (`workspaces: './src-tauri -> target'`). Matriks resmi: `macos-latest` dengan `--target aarch64-apple-darwin` dan `--target x86_64-apple-darwin`, `ubuntu-22.04`, `ubuntu-22.04-arm` (hanya repo publik), `windows-latest`. Paket Linux: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf xdg-utils`. Perlu `permissions: contents: write`. Input `tagName` (`__VERSION__` diganti otomatis), `releaseName`, `releaseBody`, `releaseDraft`, `prerelease`, `args`, `projectPath`. Opsi lain action: baca README `tauri-apps/tauri-action` **[tidak dibaca, bukan halaman v2.tauri.app]**. Workflow lengkap dan penempatan rahasia: `references/ci-and-measurement.md`.

Batas lintas-kompilasi (dokumen resmi):
- macOS: harus di Mac (tanda tangan membutuhkan perangkat Apple). x86_64 dari runner arm64 dengan `--target x86_64-apple-darwin` dan `rustup target add`.
- Windows: MSI hanya di Windows. NSIS dari Linux/macOS "possible with caveats", kurang diuji, "last resort", butuh NSIS, LLVM/lld, `cargo-xwin` (`--runner cargo-xwin --target x86_64-pc-windows-msvc`) dan `signCommand` eksternal.
- Linux: lintas-arsitektur `.deb` secara manual didokumentasikan; AppImage ARM tidak bisa lintas-kompilasi.
- Rekomendasi resmi: gunakan runner asli per OS.

## 7. Ukuran dan kinerja

Profil resmi (halaman App Size), `src-tauri/Cargo.toml` (untuk workspace, profil harus di Cargo.toml **akar workspace**; ini aturan Cargo umum, bukan dari halaman Tauri):

```toml
[profile.release]
codegen-units = 1   # optimasi lebih baik
lto = true
opt-level = "s"     # "3" bila memilih kecepatan; "z" paling kecil
panic = "abort"
strip = true
```

Catatan: `opt-level` `s` vs `3` vs `z` adalah pertukaran ukuran/kecepatan; **ukur** (anggaran M0). `panic = "abort"` mematikan unwinding; pastikan kode tidak bergantung pada `catch_unwind`. Opsi lain yang disebut halaman: `trim-paths`, `rpath`, `rustflags`, `-Cdebuginfo=0`; opsi `[profile.dev] incremental = true`.

Fitur Tauri: `build.removeUnusedCommands: true` di `tauri.conf.json` menghapus perintah yang tidak diizinkan di capability (butuh tauri 2.4, tauri-build 2.1, tauri-plugin 2.1, tauri-cli 2.4); tidak menghitung ACL dinamis saat runtime; manfaatnya maksimal bila capability tidak memakai `default`. Matikan fitur Cargo `tauri` dan plugin yang tidak dipakai (daftar fitur per crate: baca docs.rs, **tidak dibaca**). Ukuran frontend (Svelte+Vite): praktik umum (code-splitting, hindari dependensi besar, analisis bundel) **[tidak dari dokumen Tauri, tidak terverifikasi di sini]**. Tauri bergantung pada webview OS sehingga biner kecil; Windows `offlineInstaller`/`fixedRuntime` menambah 127-180MB, hindari untuk anggaran ukuran.

Cara mengukur (cara umum, **bukan dari dokumen Tauri, tidak terverifikasi** sebagai praktik resmi; perintah standar OS): ukuran biner dan bundel dengan `ls -l`/`stat` pada `src-tauri/target/<triple>/release/` dan `bundle/`; waktu mulai sampai jendela siap dengan penanda waktu di aplikasi (log `tracing`) atau `hyperfine` pada mode headless; memori diam dengan `ps -o rss` (macOS/Linux) atau Task Manager/Get-Process (Windows) setelah N detik idle, termasuk **proses webview anakan** (di macOS proses WebKit terpisah dari proses aplikasi, jadi jumlahkan semuanya). Anggaran angka ditetapkan di M0 dari pengukuran prototipe, sesuai 0005.

## 8. Pengujian dan debugging

- Unit Rust: `cargo test`; Tauri menyediakan mock runtime untuk uji unit/integrasi tanpa webview (halaman Tests).
- Frontend: `@tauri-apps/api/mocks`: `mockIPC((cmd, args) => ...)` mencegat `invoke`; `mockIPC(fn, { shouldMockEvents: true })` (sejak 2.7.0; `emitTo`/`emit_filter` belum didukung); `mockWindows('main','second')`; **panggil `clearMocks()` setelah tiap tes**. Contoh Vitest memakai jsdom + isi `window.crypto.getRandomValues` (lihat halaman mocking).
- E2E (halaman WebDriver): yang direkomendasikan WebdriverIO + `@wdio/tauri-service`, mendukung Windows, Linux, macOS. Bawaan: **server WebDriver tertanam** di aplikasi lewat plugin `tauri-plugin-wdio-webdriver` (itulah cara macOS didukung); `tauri-plugin-wdio` untuk `browser.tauri.execute()`, mock IPC, log. Dapat memakai `tauri-driver` native hanya di **Windows dan Linux** (macOS tidak punya driver WKWebView; fork CrabNebula lintas-platform, kunci API berbayar untuk macOS). Ada mode browser (Chrome + Vite dev server, tanpa biner Tauri) untuk uji renderer cepat. Panduan CI WebDriver ada di dokumen resmi (tidak dibaca). Memasang plugin wdio menambah kode ke biner: batasi ke build uji lewat fitur Cargo atau `--config` **[saran, tidak terverifikasi]**.
- Debug: `tauri build --debug` (bundel di `target/debug/bundle`, konsol dev aktif). Devtools di rilis hanya dengan fitur Cargo `devtools` pada crate `tauri`; **API devtools privat di macOS dan mencegah penerimaan App Store**. `WebviewWindow::open_devtools()` dalam `#[cfg(debug_assertions)]`. Log: plugin `tauri-plugin-log` (`tauri_plugin_log::Builder::new().build()`); lokasi berkas log per OS dan konfigurasi target: baca halaman plugin logging (hanya potongan Context7 yang dibaca). Proyek ini mewajibkan `tracing` dengan redaksi rahasia (rust-conventions): log webview/plugin tidak boleh memuat rahasia.

## 9. Reprodusibilitas

Dokumen resmi yang dibaca **tidak** membahas build reproducible. Hal yang diketahui: `trim-paths` dihitung sebagai opsi profil untuk menghapus jalur dari biner; versi runner dan toolchain harus dipin (`rust-toolchain.toml`, `npm ci`, `Cargo.lock` di-commit, `--locked`) **[praktik umum, tidak terverifikasi di dokumen Tauri]**. Tanda tangan, stempel waktu, dan notarisasi membuat artefak akhir tidak identik bit demi bit; yang dibandingkan adalah biner sebelum tanda tangan. Peringatan resmi: runner `*-latest` dapat berubah; matriks resmi memakai `ubuntu-22.04` tetap untuk Linux.

## 10. Bagaimana ini berlaku untuk infra-copilot

Keputusan relevan: 0003 (Tauri, macOS arm64 + x86_64, Linux, Windows terakhir; penandatanganan dan pembaruan "biaya belum diverifikasi"), 0005 (anggaran ukuran/memori/mulai diukur dan diperiksa di CI), CLAUDE.md (rahasia tidak masuk repo; tidak ada data pribadi: ID tim Apple, jalur kunci, domain nyata dilarang di repo).

Yang diketahui mahal atau belum pasti:
- Apple Developer Program: dokumen Tauri menyebut "99$ per year"; syarat dan harga lain **[tidak terverifikasi]** (cek developer.apple.com, bukan skill ini). Tanpa akun berbayar tidak ada notarisasi.
- Sertifikat tanda tangan Windows (OV/EV/Azure Artifact Signing): harga dan syarat identitas **[tidak terverifikasi]**. Windows ditunda (0003), jadi tidak diblokir sekarang.
- Waktu notarisasi pertama bisa berjam-jam (docstring CLI).
- Runner macOS di GitHub untuk repo privat: biaya menit **[tidak terverifikasi]**; dokumen hanya memperingatkan membaca halaman billing Actions.
- Mengelola kunci updater: hilang = tidak bisa memperbarui pemasangan lama.
- Fitur updater bukan "gratis" dalam ukuran: plugin menambah dependensi (ukur dampaknya).

Daftar periksa rilis (urut; tiap butir diverifikasi sebelum klaim "siap"):
1. Anggaran kinerja M0 terbaca dari CI (ukuran biner dan bundel, memori diam, waktu mulai) dan lolos; regresi dijelaskan.
2. `Cargo.lock` dan `package-lock.json` terkomit; toolchain dipin; `cargo test`, `clippy -D warnings`, tes frontend (`mockIPC`) hijau.
3. Versi dinaikkan di satu tempat (`tauri.conf.json > version`); cocok dengan tag rilis.
4. `createUpdaterArtifacts: true`, `pubkey` terisi (bukan path), `endpoints` HTTPS, capability `updater:*` minimal.
5. Rahasia CI terisi: `TAURI_SIGNING_PRIVATE_KEY` (+password), `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_API_ISSUER`, `APPLE_API_KEY`, kunci `.p8` (isi lewat file sementara), tanpa mencetaknya ke log.
6. Build matriks: macOS arm64 dan x86_64, Linux (runner tetap lama, mis. `ubuntu-22.04`), Windows bila sudah masuk jadwal.
7. Verifikasi artefak macOS: `codesign --verify --deep --strict`, `spctl -a -vv`, `xcrun stapler validate` **[perintah standar Apple, tidak dari dokumen Tauri]**; jalankan di Mac bersih (unduhan lewat browser).
8. Verifikasi `latest.json`: semua platform lengkap, `signature` berisi isi `.sig`.
9. Uji pembaruan nyata dari versi N-1 ke N di tiap OS sebelum menandai rilis (draft dulu, `releaseDraft: true`).
10. Uji E2E dasar: WebdriverIO dengan server tertanam di macOS; `tauri-driver` di Linux (bukan macOS).
11. Jalankan `scripts/check-personal-data.sh`; pastikan tidak ada ID akun/kunci di repo.
12. Setelah semua lulus, pemilik yang commit, tag, dan push (aturan CLAUDE.md).

## Sumber dan verifikasi

Dibaca penuh lewat browser pada 2026-11-19: v2.tauri.app/distribute/ (diperbarui 2026-11-05), /distribute/sign/macos/ (2026-05-17), /distribute/sign/windows/ (2026-10-27), /distribute/macos-application-bundle/, /distribute/dmg/, /distribute/windows-installer/ (2026-11-05), /distribute/appimage/, /distribute/debian/, /distribute/pipelines/github/ (2026-06-29), /plugin/updater/ (2025-11-28), /concept/size/ (2025-11-04), /develop/tests/, /develop/tests/mocking/, /develop/tests/webdriver/ (2026-06-29).
Dibaca lewat Context7 (`/tauri-apps/tauri-docs`, potongan, termasuk sumber Rust `tauri-cli` build.rs dan bundler app.rs): referensi variabel lingkungan, opsi `tauri build`, halaman Debug, Linux signing, RPM signing, plugin logging.

**Tidak diverifikasi / tidak dibaca:** kunci JSON `hardenedRuntime` dan nilai bawaan; README `tauri-action` (input lengkap, pembuatan `latest.json`); halaman reference/config, /distribute/rpm/, /distribute/sign/linux/ (hanya potongan), /plugin/logging/ (lokasi log), panduan CI WebDriver, halaman prerequisites; harga Apple/Windows selain "99$ per year" dari dokumen Tauri; cara mengukur startup/memori/ukuran (praktik umum, bukan dokumen); reprodusibilitas; praktik ukuran frontend Svelte/Vite; arti persis `TAURI_SKIP_SIDECAR_SIGNATURE_CHECK`; Snap/Flatpak/AUR.
