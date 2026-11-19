# 0007. Pustaka inti Rust (usulan M0)

- Tanggal: 2026-11-19
- Status: diterima oleh pemilik (2026-11-19); angka ukuran biner menyusul dari prototipe M0
- Pengambil keputusan: pemilik proyek

## Konteks
0004 dan 0005 mewajibkan setiap pustaka inti dievaluasi dengan dokumentasi terkini dan dicatat bersama dampak ukurannya. Versi di bawah dicek dengan `cargo search` dan dokumentasi (Context7) pada 2026-11-19. **Ukuran biner belum diukur**; angkanya diisi setelah prototipe M0 (langkah 4 di `docs/NEXT-STEPS.md`).

## Usulan

| Kebutuhan | Pilihan | Versi dicek | Alasan | Alternatif |
|---|---|---|---|---|
| Runtime async | `tokio` (fitur minimal: `rt`, `net`, `time`, `signal`, `sync`, `process`) | 1.54 | Dasar `axum` dan `russh`; tidak ada pilihan lain yang kompatibel dengan keduanya | `smol` (tidak didukung `axum`/`russh`) |
| HTTP server + WebSocket | `axum` | 0.8.9 | Tipis di atas `hyper`/`tower`, WebSocket dan graceful shutdown bawaan (terverifikasi di dokumentasi) | `hyper` mentah (lebih kecil, lebih banyak kode sendiri), `actix-web` (lebih berat) |
| Basis data bawaan | `rusqlite` fitur `bundled` | 0.43 | SQLite tertanam sesuai 0005, tanpa ORM. Migrasi: migrator sendiri berbasis `user_version` (diubah di 0011; `rusqlite_migration` tidak dipakai karena menahan `rusqlite` di 0.41) | `sqlx` (async, tapi lebih berat dan makro kompilasi), `tokio-rusqlite` bila perlu pembungkus async |
| PostgreSQL (cloud) | ditunda ke M6 | - | Tidak dibutuhkan MVP; antarmuka `db` dirancang agar bisa diganti | - |
| SSH klien | `russh` | 0.65 | Rust murni, async, tanpa ikatan ke libssh2 (C); exec dan autentikasi kunci terverifikasi | `ssh2` (ikatan C, sinkron) |
| Klien HTTP (AI, Telegram, Cloudflare) | `ureq` (sinkron, di thread terpisah) dulu, `reqwest` hanya bila perlu | 3.5 / 0.13 | Lebih kecil; `reqwest` menarik banyak dependensi | `reqwest` (nyaman, lebih besar) |
| Hash audit | `sha2` 0.11, tanpa fitur bawaan (0012) | 0.11 | Kripto diaudit, bukan buatan sendiri | tulis sendiri (ditolak) |
| Serialisasi | `serde` + `serde_json` | 1.0 | Standar de facto | - |
| Galat | `thiserror` | 2.0 | Sesuai `rust-conventions` | - |
| Log | `tracing` + lapisan redaksi sendiri | 0.1 | Sesuai `rust-conventions` | `log` (tanpa konteks terstruktur) |
| Menanam UI ke biner | `rust-embed` | 8.14 | Memuat berkas saat kompilasi (rilis), dari disk saat dev | `include_dir` |

## Catatan keamanan yang harus dipatuhi saat implementasi
- Contoh `russh` di dokumentasi memakai `check_server_key` yang selalu `Ok(true)`. **Jangan disalin**: kunci host wajib diverifikasi terhadap `known_hosts` atau sidik jari yang disimpan, dan gagal-tertutup bila berubah.
- Perintah SSH hanya dari daftar aksi tertutup (`ai-tool-tiers`), bukan string bebas.
- Kunci SSH pengguna tidak pernah masuk log atau prompt AI.

## Belum diputuskan (butuh pemilik atau pengukuran)
- Model async untuk akses SQLite (`tokio-rusqlite` vs `spawn_blocking`): putuskan setelah prototipe.
- Pustaka i18n (kemungkinan katalog sendiri, sangat kecil, vs `fluent`): evaluasi terpisah.
- Klien Telegram: tulis sendiri di atas klien HTTP atau pakai `teloxide`; evaluasi terpisah.
- Anggaran ukuran biner, memori diam, dan waktu mulai: dari pengukuran.

## Dampak
Dependensi masuk bertahap per crate sesuai kebutuhan tahap, bukan sekaligus. Setiap penambahan lolos `cargo deny` dan `cargo audit` di CI.
