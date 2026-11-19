# 0004. Inti, agen, dan cangkang desktop ditulis dengan Rust

- Tanggal: 2026-11-19
- Status: diterima
- Pengambil keputusan: pemilik proyek

## Konteks
0003 memilih Tauri untuk cangkang desktop (backend Rust). Bahasa inti masih terbuka: Rust, Go, atau TypeScript. Pemilik memilih Rust, dan implementasi dikerjakan oleh asisten AI atas arahan pemilik.

## Keputusan
**Inti (API, penjadwal, kolektor, pelaksana aksi), agen, dan cangkang desktop (Tauri) ditulis dengan Rust**, dalam satu workspace Cargo. UI berupa SPA (kerangka belum diputuskan) yang ditanam ke biner inti.

## Alasan
- Satu bahasa untuk server, agen, dan desktop; biner statis kecil dan ringan di server sesuai keinginan pemilik.
- Keamanan memori dan tipe yang ketat cocok untuk produk yang berhak mengubah server orang lain.
- Kompilasi lintas OS dan arsitektur dalam satu rantai alat.

## Risiko dan cara menanganinya
- Waktu kompilasi dan iterasi awal lebih lambat dibanding Go atau TypeScript. Tangani dengan workspace yang dipecah menjadi crate kecil dan pembangunan inkremental.
- Kurva belajar bagi kontributor luar. Tangani dengan konvensi tertulis (skill `rust-conventions`) dan uji kontrak untuk setiap adapter.
- Ekosistem dan versi pustaka bergerak cepat. **Setiap pustaka dievaluasi dengan dokumentasi terkini sebelum dipakai** (bukan dari ingatan), dan keputusannya dicatat.
- Pustaka untuk hal tertentu (WebSocket, SQLite/PostgreSQL, SSH, rclone, i18n, klien OpenAI-compatible) perlu dipilih dan diuji; belum dipilih. (Catatan 2026-11-20: SQLite dan klien HTTP diputuskan di 0007 dan 0016, i18n di 0008, enkripsi rahasia di 0021, klien OpenAI-compatible di 0022; WebSocket, SSH, dan rclone sebagai pustaka belum.)

## Alternatif
- Go untuk inti dan Tauri tipis: lebih cepat ditulis tetapi dua bahasa.
- TypeScript untuk inti: sejalan dengan Prospex tetapi berat untuk server dan agen.

## Dampak
- Struktur awal yang diusulkan (dikonfirmasi saat M0): workspace Cargo dengan crate `core` (tipe domain, tingkat risiko, redaksi), `db`, `api`, `agent`, `collectors`, `parity`, `adapters-*` (notifikasi, DNS, transport), `i18n`, `ai`, `desktop` (Tauri), dan direktori `ui/` untuk SPA.
- Kerangka UI, pustaka web, akses basis data, dan pustaka SSH masih menunggu keputusan dan evaluasi.
