# 0010. Anggaran kinerja M0 (usulan)

- Tanggal: 2026-11-19
- Status: diusulkan (menunggu persetujuan pemilik)
- Pengambil keputusan: pemilik proyek

## Konteks
0005 mewajibkan anggaran ukuran biner, memori diam, dan waktu mulai ditetapkan dari pengukuran prototipe, lalu diperiksa di CI. Prototipe `crates/proto-server` + `ui/` dibuat hanya untuk itu: server `axum` yang menyajikan UI Svelte 5 (Vite, tanpa SvelteKit) yang ditanam dengan `rust-embed`, dengan satu halaman berisi tabel 1000 baris dummy dan saklar bahasa en/id. Pengukuran dijalankan oleh `scripts/measure.sh`.

## Keterbatasan (baca dulu)
- **Satu mesin, satu OS, satu kali sesi.** Belum ada angka Linux. Angka Linux (target utama produk) wajib diambil di CI sebelum anggaran dikunci.
- **Prototipe tanpa fitur nyata:** tidak ada SQLite, SSH, klien HTTP, agen AI, WebSocket, atau TLS. Biner dan memori produk sesungguhnya akan jauh lebih besar. Angka di sini adalah **lantai**, bukan perkiraan produk.
- RSS diambil setelah 5 detik tanpa beban; belum ada pengukuran di bawah beban.
- Sampel kecil (5 kali). Run pertama setelah build lebih lambat (512 ms, pemeriksaan biner baru oleh macOS); median menyingkirkannya, tetapi ini menunjukkan derau besar pada angka waktu.
- UI diukur sebagai satu halaman; bundel akan tumbuh seiring jumlah layar.

## Angka terukur (2026-11-19)
Mesin: Apple M1 Pro, RAM 32 GiB, macOS 27.0.1 (Darwin 27.0.0, arm64), rustc 1.95.0, Node v24.22.1. Versi UI dikunci: svelte 5.58.0, vite 8.3.8, `@sveltejs/vite-plugin-svelte` 7.3.1. Profil rilis: profil bawaan workspace (`opt-level=3`, LTO thin, `strip`, `panic=abort`, `codegen-units=1`). Fitur `axum` hanya `http1` + `tokio`; `tokio` hanya `rt`, `net`, `signal`, `macros`; `rust-embed` tanpa fitur bawaan, `debug-embed`.

| Metrik | Hasil (median 5 run) |
|---|---|
| JS utama | 38.164 B mentah / 14.971 B gzip |
| CSS | 577 B mentah / 361 B gzip |
| `index.html` | 397 B mentah / 280 B gzip |
| **Total bundel UI** | **39.138 B mentah / 15.612 B gzip** |
| **Biner rilis** (UI sudah tertanam) | **876.128 B (~856 KiB)** |
| **RSS diam setelah 5 detik** | **2.512 KiB (~2,5 MiB)** (sampel 1.392 sampai 2.544 KiB) |
| **Waktu exec sampai `/` menjawab 200** | **11,6 ms** (sampel 10,4 sampai 11,8 ms; run dingin 512 ms) |

Uji fungsional (bukan hanya status): `/` dan aset JS menjawab 200 dengan tipe MIME benar, jalur tak dikenal 404, SIGTERM menghasilkan penutupan halus. Dengan Playwright: 1000 baris tabel tampil, dan saklar bahasa mengubah judul kolom, subjudul, sel, dan atribut `lang` (en ke id). Galat konsol satu-satunya: `favicon.ico` 404 (prototipe tidak punya favicon).

## Usulan anggaran
Anggaran = angka terukur + margin tertulis. Margin besar sengaja, karena prototipe adalah lantai dan derau CI nyata. Anggaran ini berlaku untuk **prototipe/kerangka** dan **dihitung ulang** (dicatat di keputusan baru) saat fitur nyata pertama (SQLite, klien HTTP) masuk.

| Metrik | Terukur | Margin | Anggaran awal |
|---|---|---|---|
| Bundel UI gzip (total) | 15,6 KB | +60% | 25 KB |
| JS utama mentah | 38,2 KB | +60% | 60 KB |
| Biner rilis | 856 KiB | +75% | 1,5 MiB |
| RSS diam (5 detik) | 2,5 MiB | +200% | 8 MiB |
| Waktu sampai 200 (median) | 11,6 ms | +~760% | 100 ms |

Alasan margin: waktu mulai dan RSS berfluktuasi antar run dan antar mesin CI, jadi batas longgar dipilih agar tidak ada alarm palsu; yang dicari adalah regresi besar. Setelah ada riwayat CI beberapa minggu, margin diperketat.

## Rancangan langkah CI (belum dipasang; pemilik yang mengintegrasikan)
1. Job `perf-budget` pada matriks `ubuntu-latest` dan `macos-latest` (angka Linux adalah acuan utama; macOS informatif).
2. Setup Node (versi sama dengan `.nvmrc`/mesin ini) dan Rust stable; `npm ci` memakai `package-lock.json`.
3. Jalankan `./scripts/measure.sh > measure.json` (log build ke stderr, JSON ke stdout).
4. Langkah pembanding (skrip Python stdlib kecil, mis. `scripts/check-budget.py`, belum ditulis): baca `measure.json` dan berkas `docs/perf-budget.json` (anggaran per OS), gagal bila ada metrik melewati batas, dan cetak tabel angka vs anggaran ke ringkasan job.
5. Unggah `measure.json` sebagai artefak dan, pada `main`, simpan riwayatnya agar tren terlihat.
6. Regresi melewati anggaran hanya boleh lolos dengan alasan tertulis di keputusan baru (sesuai 0005), bukan dengan menaikkan angka diam-diam.
7. Kalibrasi: jalankan job ini 10 kali pada `main` sebelum anggaran dikunci, lalu isi `perf-budget.json` per OS dari median + margin di atas.

## Alasan dan alternatif
- Anggaran dari pengukuran, sesuai 0005; angka karangan ditolak.
- Alternatif: memakai `cargo bloat`/`size` per crate untuk menelusuri penyebab; ditunda sampai ada regresi.
- Alternatif untuk waktu mulai: hanya cek relatif terhadap run sebelumnya; ditunda karena butuh riwayat.

## Dampak
- 0007 bisa mengisi baris ukuran: `axum` + `tokio` (fitur minimal) + `rust-embed` + UI 39 KB menghasilkan biner 856 KiB pada macOS arm64.
- `crates/proto-server` bersifat sementara dan dihapus atau diganti `crates/api` saat M1.
