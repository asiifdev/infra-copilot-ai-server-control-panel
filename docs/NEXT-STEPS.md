# Langkah berikutnya: serah terima ke sesi implementasi

Terakhir diperbarui: 2026-11-19 (setelah langkah 1-5 M0). Dokumen kerja internal (Indonesia).

## Yang sudah ada

- Repo publik dengan CI hijau (fmt, clippy, tes di Linux dan macOS, `cargo deny`, `cargo audit`, gitleaks, pemeriksaan dokumen) dan perlindungan cabang `main`.
- Crate `core-domain` (tingkat risiko dan penyaring rahasia), 25 tes, tanpa dependensi luar.
- Crate `i18n` (katalog en/id), `db` (skema SQLite M0, isolasi tenant, audit berantai-hash), dan prototipe `proto-server` + `ui/` dengan `scripts/measure.sh` (langkah 1-5 M0 selesai; keputusan 0007-0013 diterima; angka anggaran Linux menunggu kalibrasi).
- Dokumen: `docs/design.md`, `docs/plan.md`, `docs/decisions/0000-0012`, `docs/market-research.md`, `docs/RELEASE-CHECKLIST.md`.
- Konfigurasi pengembangan AI: `CLAUDE.md`, skill (parity-rules, ai-tool-tiers, migration-planning, security-review, repo-hygiene, rust-conventions, tauri-v2-*, svelte), hook `tauri-guard`, plugin Svelte (MCP dikunci 0.1.31).

## Memulai sesi implementasi

1. Buka sesi Claude Code di folder repo. `CLAUDE.md` dan skill terbaca otomatis. **Buka `/hooks` sekali** (atau mulai ulang) agar hook `tauri-guard` aktif.
2. Baca `CLAUDE.md`, `docs/design.md`, dan keputusan 0002-0006. Jangan memilih sendiri hal yang masih terbuka (di bawah).
3. Untuk setiap pustaka, **verifikasi dengan dokumentasi terkini (Context7)**, bukan ingatan. Untuk Tauri, muat skill `tauri-v2-*` dulu. Untuk Svelte, ikuti skill dan jalankan autofixer.
4. Jalankan `python3 scripts/check-docs.py` dan `./scripts/check-personal-data.sh` sebelum commit yang menyentuh dokumen. Commit: Conventional Commits berbahasa Inggris, `git commit -s`, tanpa atribusi Claude.

## Tugas M0 (urutan usulan)

1. **Evaluasi pustaka inti Rust** (server HTTP, runtime async, akses SQLite/PostgreSQL, serialisasi, logging, klien HTTP, SSH). Catat pilihan dan alasan sebagai keputusan baru di `docs/decisions/`, dengan dampak ukuran biner. Selesai: 0007 (diterima).
2. **Struktur workspace:** tambah crate sesuai `rust-conventions` (db, api, agent, collectors, parity, i18n, ...). Pertahankan dependensi sesedikit mungkin (prinsip 0005). Selesai: `i18n` dan `db`; crate lain menyusul per tahap.
3. **Katalog i18n (en, id)** dengan kunci stabil, dan uji yang menolak teks tampilan yang ditulis tetap. Selesai (0008).
4. **Prototipe pengukuran:** satu halaman tabel di Svelte 5 + Vite (tanpa SvelteKit) ditanam ke satu biner Rust. Ukur ukuran bundel, ukuran biner, memori diam, dan waktu mulai. Jadikan angkanya **anggaran kinerja** yang diperiksa di CI. Pengukuran selesai (0010); pemeriksaan di CI sudah dipasang (`scripts/check-budget.py`, `docs/perf-budget.json`, job `perf-budget`); angka Linux menunggu kalibrasi dari run CI.
5. **Skema database** (`docs/design.md` bagian 3) sebagai migrasi, SQLite lebih dulu, dengan uji isolasi tenant. Selesai (0009, 0012); pustaka AES-256-GCM untuk enkripsi rahasia belum dievaluasi.
6. **M1** (dipecah; **M1-a selesai**, ditambah PAR-006 beserta deteksi mode dev dan supervisor pada kolektor proses: kontrak `inventory`, kolektor Linux `listeners`/`processes`/`cron`, aturan PAR-001/006/008/010, CLI `infra collect|compare`, ADR 0013. **Modul D (bagian 1) selesai**: crate `net` (klien HTTP/TLS, 0016), `monitor` (mesin insiden murni) dan `notifier` (kontrak channel + Telegram kirim); **perekat `watch` selesai** (mesin probe-insiden-alert, sumber token sementara 0018); **belum**: perintah CLI/server yang menjalankannya, pengiriman ulang alert yang tertahan jam tenang, penyimpanan insiden di DB, perintah dan tombol bot, pengaturan persetujuan Telegram (bawaan tingkat 0-2, tingkat 3-4 tidak pernah lewat bot; dikerjakan bersama Usulan di M2/M4), kenaikan anggaran biner saat klien tertanam. **Modul C selesai** (item `env_ref`, PAR-002/007/009, keputusan 0015). **Modul B selesai**: kolektor systemd, docker, pm2 dan pelaksana perintah terbatas (0014). **Belum**: M1-b monitor uptime, mesin alert, bot Telegram; M1-c agen/SSH; kolektor systemd/docker/pm2/env_ref/database; aturan lain dari 11 aturan; uji di Linux nyata di luar CI). Rencana awal: kolektor inventaris Linux read-only, tiga aturan kesetaraan pertama dengan fixture anonim, monitor uptime dasar, mesin alert, dan bot Telegram.
7. Cangkang Tauri tipis menyusul setelah M1-M2 (skill `tauri-v2-*`, command dibatasi `AppManifest::commands`).

## Keputusan yang masih menunggu pemilik

- DCO atau CLA (penting bila ingin dual licensing), konfirmasi `AGPL-3.0-only`, nasihat hukum.
- Cek merek dagang kata "Copilot" dan nama final.
- Kontak pelaporan di Kode Etik (masih penanda). Pelaporan kerentanan privat GitHub sudah aktif.
- Penyedia pembayaran global dan harga edisi berbayar (belum diriset).
- Riset yang belum dikerjakan: DNS dan Cloudflare Tunnel, buildpacks untuk modul deployment, regulasi di luar Indonesia.
