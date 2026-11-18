---
name: rust-conventions
description: Konvensi menulis Rust untuk proyek ini (struktur workspace, penanganan galat, rahasia, uji kontrak, lintas OS, pemilihan pustaka). Gunakan saat membuat crate baru, menulis kode inti/agen/adapter, atau memilih dependensi.
---

# Konvensi Rust

Keputusan: `docs/decisions/0004-inti-rust.md`. Inti, agen, dan cangkang desktop (Tauri) memakai Rust dalam satu workspace Cargo.

## Struktur (usulan, dikonfirmasi saat M0)
Satu workspace, crate kecil dengan tanggung jawab sempit: `core` (tipe domain, tingkat risiko, redaksi), `db`, `api`, `agent`, `collectors`, `parity`, `adapters-*`, `i18n`, `ai`, `desktop`. Crate adapter bergantung pada `core`, bukan satu sama lain. Antarmuka adapter (lihat `docs/design.md` bagian 12.4 dan 15.2) didefinisikan sebagai trait di `core`.

## Aturan kode
- **Tanpa `unsafe`** kecuali ada alasan tertulis dan uji; setiap blok `unsafe` memakai komentar `SAFETY:`.
- `#![deny(warnings)]` di CI, `clippy` dengan `-D warnings`, `rustfmt` wajib.
- **Galat:** tipe galat jelas per crate (`thiserror` atau setara), tidak ada `unwrap()`/`expect()` di jalur produksi kecuali invarian terbukti dan dikomentari. Galat yang sampai ke pengguna melewati lapisan redaksi dan katalog i18n.
- **Rahasia:** bungkus dengan tipe khusus yang tidak mencetak nilainya (`Debug`/`Display` disamarkan), dibersihkan dari memori bila memungkinkan. Rahasia tidak pernah masuk log (`tracing`), galat, audit, atau prompt AI.
- **Log:** `tracing` terstruktur dengan lapisan redaksi; tidak ada pencetakan langsung dengan `println!` di produksi.
- **Aksi dan perintah:** tidak membangun perintah shell dari string; gunakan `Command` dengan argumen terpisah dan daftar aksi tertutup.
- **Lintas OS:** perilaku khusus OS berada di balik trait (`ServiceManager`, penyedia jalur/paket/izin); hindari `cfg` tersebar. Jalur memakai `Path`/`PathBuf`, bukan string. Uji di Linux dan macOS (Windows menyusul).
- **Komponen reusable:** sebelum membuat tipe/fungsi baru, cari yang sudah ada. Tidak ada duplikasi antar crate.
- **i18n:** tidak ada teks tampilan tetap di kode; kunci katalog, bahasa bawaan Inggris.

## Uji
- Uji unit per crate, **uji kontrak bersama** untuk setiap trait adapter (satu rangkaian dijalankan ke semua implementasi).
- Fixture inventaris anonim untuk aturan kesetaraan (skill `parity-rules`).
- Uji isolasi tenant: query tanpa `org_id` harus gagal di uji.
- Uji injeksi prompt dan alat berisiko (skill `ai-tool-tiers`, `security-review`).

## Kinerja (prinsip `0005`)
- Teringan dan tercepat yang memenuhi kebutuhan. Ukur dulu, jangan menebak: ukuran biner (`cargo bloat`/ukuran artefak), memori diam, dan waktu mulai dicatat per rilis.
- Anggaran ditetapkan di M0 dari pengukuran prototipe; CI gagal bila melewatinya tanpa alasan tertulis.
- Hindari alokasi dan klon yang tidak perlu di jalur panas; hindari `async` tanpa alasan; pilih fitur crate seminimal mungkin (nonaktifkan fitur bawaan yang tidak dipakai).
- Profil build rilis: optimasi ukuran/kecepatan sesuai pengukuran (mis. LTO, `codegen-units`, `strip`), dicatat di keputusan.

## Dependensi
- **Evaluasi dengan dokumentasi terkini sebelum memakai** (gunakan Context7 atau dokumentasi resmi), jangan dari ingatan: versi dan API cepat berubah.
- Catat alasan dan alternatif untuk pustaka inti (web, basis data, SSH, WebSocket, klien AI, i18n, Tauri) di `docs/decisions/`.
- Minimalkan dependensi; kunci versi (`Cargo.lock` di-commit); jalankan `cargo audit` dan `cargo deny` di CI; tinjau dependensi baru untuk keamanan rantai pasok.

## Build lintas platform
Target: Linux (x86_64, arm64), macOS (arm64, x86_64), Windows menyusul. Biner agen dan inti statis bila memungkinkan. Rilis ditandatangani; notarisasi macOS dan penandatanganan Windows disiapkan di tahap desktop.
