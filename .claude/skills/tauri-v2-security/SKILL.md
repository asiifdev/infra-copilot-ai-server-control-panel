---
name: tauri-v2-security
description: Panduan keamanan Tauri v2 (batas kepercayaan webview vs Rust core, capabilities, permissions, scopes, CSP, isolation pattern, izin plugin shell/fs/http/opener, updater bertanda tangan) beserta prosedur audit. Muat sebelum membuat atau mengubah src-tauri/capabilities/*, src-tauri/permissions/*, tauri.conf.json (bagian app.security, plugins), perintah #[tauri::command], menambah plugin Tauri, atau menggabungkan perubahan cangkang desktop infra-copilot.
---

# Keamanan Tauri v2

Dibuat dari dokumentasi resmi v2.tauri.app dan Context7 (dibaca 2026-11-19). Setiap klaim di bawah berasal dari halaman yang tercantum di "Sumber dan verifikasi"; yang tidak terverifikasi ditandai **[BELUM TERVERIFIKASI]**. Berlaku bersama skill `security-review`. Detail tambahan: `references/audit-capability.md`.

## 1. Model keamanan: dua zona kepercayaan

- Kode Rust inti aplikasi dan kode plugin punya **akses penuh** ke sumber daya sistem, tanpa batasan.
- Kode di webview (framework apa pun, termasuk Svelte) hanya bisa mencapai sumber daya yang **diekspos lewat lapisan IPC**. Akses ke command dibatasi oleh **capabilities**; implementasi command menegakkan scope granular yang ditetapkan di capabilities.
- Dokumentasi resmi mengasumsikan webview **tidak aman** (runtime threats). Pertahanan: CSP (membatasi komunikasi webview) dan capabilities (membatasi API untuk konten/skrip tak tepercaya).
- Runtime authority (bagian Tauri Core): tiap `invoke` dari webview dicek asalnya; bila origin/window tidak masuk capability untuk command itu, request ditolak dan command **tidak pernah dipanggil**. Bila ada scope, scope disuntikkan ke request.
- Tauri memakai webview bawaan OS, tidak dibundel; alasan resmi: patch keamanan webview sampai ke pengguna lebih cepat lewat OS.
- Capabilities **tidak** melindungi dari: kode Rust yang jahat/rawan, scope terlalu longgar, pengecekan scope yang salah di implementasi command, bypass yang disengaja dari Rust, 0-day webview, serangan rantai pasok.

Implikasi: semua data yang melintasi batas webview -> Rust adalah **input tak tepercaya**. Validasi di Rust, bukan di UI.

## 2. Capabilities, permissions, scopes

**Permission** = deskripsi hak eksplisit atas command (boleh/tolak command, sertakan scope). **Capability** = menautkan permission ke window/webview. **Scope** = batasan granular perilaku command (allow/deny; **deny selalu menang** atas allow).

Identifier permission: `<plugin>:default` (set bawaan), `<plugin>:allow-<command>`, `<plugin>:deny-<command>`. Nama plugin tanpa awalan `tauri-plugin-`; hanya huruf kecil ASCII `[a-z]` (dan pemisah). Izin core diawali `core:` (mis. `core:window:allow-set-title`); `core:default` adalah set yang memuat default semua plugin core, jadi **luas**. Pilih izin core spesifik bila memungkinkan.

File capability: JSON/JSON5/TOML di `src-tauri/capabilities/`. **Semua file di folder itu aktif otomatis**; bila `app.security.capabilities` di `tauri.conf.json` diisi eksplisit, **hanya yang tercantum** yang dipakai (dan boleh inline). Izin aplikasi sendiri ditulis di `src-tauri/permissions/*.toml` (hanya TOML).

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main-capability",
  "description": "Capability for the main window",
  "windows": ["main"],
  "platforms": ["linux", "macOS", "windows"],
  "permissions": [
    "core:event:default",
    "core:window:allow-set-title",
    { "identifier": "fs:allow-read-text-file",
      "allow": [{ "path": "$APPDATA/**" }],
      "deny":  [{ "path": "$APPDATA/secrets/**" }] }
  ]
}
```

Field capability (referensi konfigurasi): `identifier` (wajib), `permissions` (wajib), `description`, `windows`, `webviews` (glob boleh), `platforms` (`linux|macOS|windows|iOS|android`), `local` (default `true`), `remote.urls`.

Fakta penting:
- Window/webview yang masuk **lebih dari satu** capability: izinnya **digabung (union)**.
- Window/webview yang tidak cocok capability mana pun **tidak punya akses IPC sama sekali**.
- Batas keamanan bergantung pada **label** window (bukan judul). Batasi pembuatan window baru ke window berprivilese tinggi.
- Pada multiwebview, dokumentasi menyarankan memakai `webviews` dan mengosongkan `windows` untuk kontrol halus.
- Scope bisa per-permission (bersama command yang diaktifkan) atau global plugin (mis. `fs:scope`).
- Skema JSON dibuat otomatis di `gen/schemas/` untuk autocompletion; `$schema` menunjuk ke sana.

## 3. Command dan ACL: apa yang bisa dipanggil frontend

- **Bawaan berbahaya**: semua command yang didaftarkan lewat `tauri::Builder::invoke_handler` boleh dipakai **semua window dan webview**. Untuk membatasi, pakai `AppManifest::commands` di `build.rs`:

```rust
fn main() {
  tauri_build::try_build(
    tauri_build::Attributes::new()
      .app_manifest(tauri_build::AppManifest::new().commands(&["your_command"])),
  ).unwrap();
}
```
  Setelah itu command aplikasi diatur lewat izin (`allow-your_command`) di capability; detail pola penamaan izin aplikasi **[BELUM TERVERIFIKASI]** (halaman "Calling Rust" dan "Using plugin permissions" tidak dibaca penuh; periksa saat menerapkan).
- `build.removeUnusedCommands` (bawaan `false`): saat `tauri build` mencoba membuang command plugin yang tidak diizinkan ACL; tidak memperhitungkan ACL dinamis (fitur `dynamic-acl`); butuh tauri 2.4 / tauri-plugin 2.1.
- Scope diteruskan ke command, **penegakannya tanggung jawab command**. Command buatan sendiri yang menerima scope harus diaudit agar tidak bisa di-bypass.

## 4. Content Security Policy (`app.security.csp`)

- CSP **hanya aktif bila diisi** di konfigurasi; contoh bawaan proyek baru memakai `"csp": null` (artinya mati). Isi sebisa mungkin ketat.
- Saat build Tauri menambahkan hash/nonce untuk aset bundel, jadi isi hanya yang khas aplikasi. Skrip lokal di-hash; style dan skrip eksternal pakai nonce.
- Hindari konten remote (CDN): vektor serangan.
- Bentuk: string tunggal, atau objek direktif -> daftar sumber.
- Agar `invoke` jalan, `connect-src` perlu `ipc: http://ipc.localhost` (contoh resmi).
- Frontend WebAssembly perlu `'wasm-unsafe-eval'` di `script-src`.
- Terkait: `app.security.headers` (header respons seperti COOP/COEP; CSP **tidak** diatur di sini), `dangerousDisableAssetCspModification` (bool atau daftar direktif; `false` bawaan; **jangan diaktifkan**, mematikan injeksi nonce/hash), `freezePrototype` (bawaan `false`), `assetProtocol` (`enable` bawaan `false`, plus `scope`).

```json
"app": { "security": {
  "csp": {
    "default-src": "'self'",
    "connect-src": "ipc: http://ipc.localhost http://127.0.0.1:PORT",
    "img-src": "'self' data:",
    "style-src": "'self' 'unsafe-inline'"
  },
  "freezePrototype": true
} }
```
Catatan: `PORT`, `style-src 'unsafe-inline'`, dan `freezePrototype: true` adalah usulan proyek, bukan salinan contoh resmi; uji bahwa UI Svelte tetap jalan. Kegunaan persis `freezePrototype` **[BELUM TERVERIFIKASI]** dari halaman yang dibaca (hanya nilai bawaan terlihat).

## 5. Isolation pattern

- Menyisipkan "aplikasi isolasi" JS dalam `<iframe>` ter-sandbox di antara frontend dan Tauri Core; **semua pesan IPC** (termasuk Events) lewat hook `window.__TAURI_ISOLATION_HOOK__` yang boleh memeriksa/mengubah payload, lalu dienkripsi AES-GCM dengan kunci baru tiap aplikasi dijalankan.
- Tauri "sangat menyarankan" memakainya bila bisa; tujuan utama: ancaman rantai pasok dari dependensi frontend yang banyak.
- Konfigurasi:
```json
"app": { "security": { "pattern": { "use": "isolation", "options": { "dir": "../dist-isolation" } } } }
```
- Batasan: skrip eksternal tidak dimuat benar dalam iframe sandbox di Windows, jadi skrip diinline saat build; **ES Modules tidak jalan**. Jaga aplikasi isolasi sangat kecil, tanpa dependensi/build rumit. Ada overhead enkripsi kecil.
- Isolation memvalidasi input (mis. path di luar lokasi yang diharapkan); ia pelengkap, bukan pengganti scope dan validasi di Rust.

## 6. Izin plugin (shell, fs, http, opener, updater)

Prinsip resmi: perintah dan scope berbahaya plugin **diblokir secara bawaan** sampai Anda mengizinkannya di capability.

- **shell**: `shell:allow-execute|spawn|kill|stdin-write|open` (dan `deny-*`), tanpa scope bawaan. Default permission hanya `allow-open` (membuka `http(s)://`, `tel:`, `mailto:`). Eksekusi harus ditautkan ke entri scope bernama:
```json
{ "identifier": "shell:allow-execute",
  "allow": [{ "name": "binaries/my-sidecar", "sidecar": true,
              "args": ["--flag", { "validator": "\\S+" }] }] }
```
  Argumen statis berurutan atau objek `validator` (regex). **Contoh resmi `sh -c` + `\S+` justru contoh buruk** untuk produk ini: memberi shell bebas. Dokumentasi menyatakan `shell.open` sudah dipindah ke plugin **opener**.
- **opener**: default mengizinkan `openUrl` untuk `mailto:`/`tel:`/`http(s)://` dan reveal-item-in-dir. Scope `opener:allow-open-url` (`url` glob) dan `opener:allow-open-path` (`path` glob).
- **fs**: izin per command (`fs:allow-read-text-file` dst.), scope `path` glob dengan variabel `$APPDATA`, `$HOME`, dll. `fs:scope` global. Deny menang. Dokumentasi memperingatkan folder data webview (`$APPLOCALDATA` di Linux, `EBWebView` di Windows) sensitif; ada set `deny-default`. `fs:default` mengizinkan baca rekursif direktori app (dari catatan `appDirectoriesOverride`).
- **http**: `http:default` dengan `allow`/`deny` berisi `{ "url": "https://*.tauri.app" }`.
- **updater**: `updater:default` memuat check, download, install, download-and-install; boleh dipersempit ke `updater:allow-check`.
- Beberapa API hanya tersedia dari Rust "demi keamanan" (mis. mengubah endpoint/pubkey updater saat runtime).

## 7. Konten remote dan bahaya lain

- Secara bawaan API **hanya** untuk kode bundel lokal. Memberi sumber remote akses IPC memakai `remote.urls` (pola URLPattern, mis. `https://*.domain.dev`, `https://domain.dev/api/*`) di capability; dokumentasi: "make sure you understand the security implications". Di **Linux dan Android Tauri tidak bisa membedakan request dari `<iframe>` tertanam dan window itu sendiri**.
- `local` (default `true`) mengatur apakah capability berlaku untuk URL lokal aplikasi.
- `dangerousRemoteDomainIpcAccess` adalah opsi **v1**; di konfigurasi v2 yang dibaca **tidak ditemukan** (bagian `security` hanya: `assetProtocol`, `capabilities`, `csp`, `dangerousDisableAssetCspModification`, `freezePrototype`, `headers`, `pattern`, dll). Penggantinya `remote.urls` pada capability. Halaman migrasi v1->v2 tidak dibaca.
- `build.frontendDist` bisa berupa URL remote: aplikasi memuat situs itu tanpa aset bundel. Hindari.
- Server dev (`devUrl`) tidak terenkripsi/terautentikasi; jangan bekerja di jaringan tak tepercaya.
- `appDirectoriesOverride` dengan root yang bukan folder khusus aplikasi memperluas scope `$APP*`; webview yang terkompromi + izin tulis bisa menanam DLL. Selalu subfolder khusus.
- `withGlobalTauri: true` menyuntik `window.__TAURI__` ke semua skrip halaman (bawaan `false`); biarkan `false`.
- Peringatan resmi: lindungi kunci penandatanganan; jangan pakai rahasia produksi di mesin dev; pakai `npm audit`/`cargo audit`, kunci versi dependensi.

## 8. Updater bertanda tangan sebagai kontrol keamanan

- Tanda tangan updater **tidak bisa dimatikan**. Kunci publik di `plugins.updater.pubkey` (isi kunci, **bukan path**), kunci privat hanya di lingkungan build (`TAURI_SIGNING_PRIVATE_KEY`, opsional `..._PASSWORD`; file `.env` tidak bekerja). Hilang kunci privat = tidak bisa merilis update ke instalasi lama.
- `endpoints` wajib array URL; TLS diwajibkan di produksi; `dangerousInsecureTransportProtocol` ada tetapi jangan dipakai. Lanjut ke URL berikutnya hanya bila status non-2xx.
- Respons: `version`, `url`, `signature` (isi file `.sig`, bukan path). `createUpdaterArtifacts: true`.
- Dokumentasi risiko distribusi: bila server manifest/build/hosting biner hilang kendali, semuanya gagal. Menurunkan versi harus lewat `version_comparator` di Rust (sengaja); jangan aktifkan tanpa alasan.
- Tanda tangan updater **tidak menggantikan** penandatanganan kode OS (notarisasi macOS, Authenticode); detail Apple/Windows **[BELUM TERVERIFIKASI]** (halaman distribusi tidak dibaca penuh).

## 9. Daftar periksa hardening

1. `capabilities` eksplisit di `tauri.conf.json` (satu file per window), tanpa `windows: ["*"]`.
2. `app.security.csp` terisi ketat; tanpa CDN; `dangerousDisableAssetCspModification` tetap `false`.
3. `pattern.use = "isolation"` dengan aplikasi isolasi minimal.
4. `AppManifest::commands` membatasi command aplikasi; tiap command memvalidasi argumen.
5. Plugin ditambah hanya bila perlu; izin `allow-*` spesifik, bukan `*:default` luas; scope sempit; `deny` untuk jalur sensitif.
6. Tidak ada `remote.urls`, tidak ada `frontendDist` remote.
7. `withGlobalTauri` false; `assetProtocol.enable` false kecuali perlu.
8. Updater: pubkey terisi, endpoint HTTPS, kunci privat di brankas CI.
9. Dependensi: `cargo audit`, `npm audit`, versi dikunci, Tauri selalu diperbarui.

## 10. Cara mengaudit file capability

Ringkas (versi lengkap dan templat laporan di `references/audit-capability.md`):
1. Daftar semua file di `src-tauri/capabilities/` dan `capabilities` di `tauri.conf.json`; cari file yatim yang aktif otomatis.
2. Per file: `windows`/`webviews` (glob? `*`?), `platforms`, `remote`, `local`.
3. Per permission: apa yang diaktifkan? Ada `core:default`/`<plugin>:default` luas? Ada scope? Apakah scope dapat dilewati (symlink, `..`, glob rekursif, `$HOME`)?
4. Hitung gabungan izin tiap window (union lintas capability).
5. Cocokkan dengan command nyata di `invoke_handler` dan `AppManifest::commands`.
6. Bandingkan dengan diff sebelumnya; setiap izin baru butuh alasan tertulis.
7. Laporkan: diperiksa, bukti, **belum diuji** (aturan proyek).

## 11. Penerapan pada infra-copilot

Konteks: cangkang Tauri membungkus SPA Svelte 5 (ADR 0003/0004); produk bisa mengubah server orang lain; keamanan mengalahkan kinerja (ADR 0005). Ini **usulan kebijakan proyek**, bukan fakta Tauri.

- **Sesedikit mungkin command.** Cangkang cukup: ambil alamat/token sesi mesin lokal, status, buka URL. Seluruh logika berbahaya tetap di engine di balik daftar aksi yang diizinkan dan alur Usulan + persetujuan; **frontend tidak mendapat command "jalankan aksi" langsung** yang melewati persetujuan.
- **Tanpa plugin shell dengan scope terbuka.** Jangan `shell:allow-execute` tanpa `name`/`cmd`/`args` tetap; tidak ada `sh -c`, tidak ada validator `\S+`. Memulai engine sebagai sidecar dilakukan dari Rust (bukan JS) bila memungkinkan; bila lewat izin, satu entri `sidecar: true` dengan argumen statis.
- **Tanpa fs luas.** Tidak ada `$HOME/**`; maksimal `$APPDATA` khusus aplikasi, tanpa folder kredensial/kunci git/SSH. Tidak ada `fs:scope` global.
- **Frontend tidak pernah memegang rahasia.** Kunci utama, token agen, kredensial server tidak melintasi IPC; Rust/engine menyuntikkannya di sisi server. Hasil command harus sudah diredaksi. Jangan menyimpan rahasia di localStorage webview.
- **Engine hanya loopback.** Bind `127.0.0.1`; `connect-src` hanya `ipc: http://ipc.localhost` dan origin loopback tepat (port spesifik). Autentikasi per sesi (token acak per peluncuran) dan cek Host/Origin tetap diperlukan karena proses lokal lain bisa menjangkau loopback. Tidak ada `remote.urls`; webview tidak memuat situs eksternal.
- **Tautan eksternal** lewat `opener:allow-open-url` dengan `url` terbatas (bukan default luas) dan teks dari server diperlakukan sebagai data tak tepercaya (tautan di hasil log jangan dibuka otomatis).
- **Capability per window**, tanpa `core:default` kecuali dibenarkan; tanpa izin pembuatan window dari webview.
- **Updater**: tanda tangan wajib; kunci privat di CI privat; pubkey di repo boleh (publik).
- **Sebelum merge perubahan yang menyentuh capabilities/permissions/tauri.conf.json/command:** (a) jalankan prosedur bagian 10; (b) tulis tabel diff izin (ditambah/dihapus, alasan); (c) tidak ada izin shell/fs/http/remote baru tanpa catatan keputusan di `docs/decisions/`; (d) uji bahwa window tanpa capability menolak `invoke`; (e) uji CSP tidak memblokir UI dan memblokir sumber luar; (f) laporkan yang belum diuji. Gunakan juga `security-review`.

## Sumber dan verifikasi

Dibaca 2026-11-19 lewat Chrome (halaman resmi) dan Context7 (`/tauri-apps/tauri-docs`):
- https://v2.tauri.app/security/ (trust boundaries, webview tidak dibundel; diperbarui 5 Nov 2026)
- https://v2.tauri.app/security/capabilities/
- https://v2.tauri.app/security/permissions/
- https://v2.tauri.app/security/scope/
- https://v2.tauri.app/security/runtime-authority/
- https://v2.tauri.app/security/csp/
- https://v2.tauri.app/security/lifecycle/
- https://v2.tauri.app/concept/inter-process-communication/isolation/
- https://v2.tauri.app/reference/config/ (hanya 50.000 karakter pertama; bagian `SecurityConfig`/`WindowConfig` terpotong)
- https://v2.tauri.app/plugin/shell/, /plugin/opener/, /plugin/updater/
- Context7: contoh fs, http, shell, HTTP headers, isolation.

**Tidak terverifikasi / tidak dibaca:** halaman Calling Rust dan Using plugin permissions (pola izin command aplikasi), halaman Core Permissions (daftar identifier `core:*` lengkap; hanya yang muncul di contoh dipakai), halaman plugin fs dan http versi penuh (hanya cuplikan Context7), halaman Asset protocol scope, Ecosystem Security, Future Work, migrasi v1->v2 (status `dangerousRemoteDomainIpcAccess`), penandatanganan kode macOS/Windows, kegunaan `freezePrototype`, versi Tauri terbaru yang dipakai proyek. Bagian 11 adalah rekomendasi proyek, bukan klaim dokumentasi.
