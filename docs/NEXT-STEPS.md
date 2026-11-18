# Langkah berikutnya: serah terima ke sesi implementasi

Terakhir diperbarui: 2026-11-19. Dokumen kerja internal (Indonesia).

## Yang sudah ada

- Repo publik dengan CI hijau (fmt, clippy, tes di Linux dan macOS, `cargo deny`, `cargo audit`, gitleaks, pemeriksaan dokumen) dan perlindungan cabang `main`.
- Crate `core-domain` (tingkat risiko dan penyaring rahasia), 25 tes, tanpa dependensi luar.
- Dokumen: `docs/design.md`, `docs/plan.md`, `docs/decisions/0000-0006`, `docs/market-research.md`, `docs/RELEASE-CHECKLIST.md`.
- Konfigurasi pengembangan AI: `CLAUDE.md`, skill (parity-rules, ai-tool-tiers, migration-planning, security-review, repo-hygiene, rust-conventions, tauri-v2-*, svelte), hook `tauri-guard`, plugin Svelte (MCP dikunci 0.1.31).

## Memulai sesi implementasi

1. Buka sesi Claude Code di folder repo. `CLAUDE.md` dan skill terbaca otomatis. **Buka `/hooks` sekali** (atau mulai ulang) agar hook `tauri-guard` aktif.
2. Baca `CLAUDE.md`, `docs/design.md`, dan keputusan 0002-0006. Jangan memilih sendiri hal yang masih terbuka (di bawah).
3. Untuk setiap pustaka, **verifikasi dengan dokumentasi terkini (Context7)**, bukan ingatan. Untuk Tauri, muat skill `tauri-v2-*` dulu. Untuk Svelte, ikuti skill dan jalankan autofixer.
4. Jalankan `python3 scripts/check-docs.py` dan `./scripts/check-personal-data.sh` sebelum commit yang menyentuh dokumen. Commit: Conventional Commits berbahasa Inggris, `git commit -s`, tanpa atribusi Claude.

## Tugas M0 (urutan usulan)

1. **Evaluasi pustaka inti Rust** (server HTTP, runtime async, akses SQLite/PostgreSQL, serialisasi, logging, klien HTTP, SSH). Catat pilihan dan alasan sebagai keputusan baru di `docs/decisions/`, dengan dampak ukuran biner. Belum ada yang dipilih.
2. **Struktur workspace:** tambah crate sesuai `rust-conventions` (db, api, agent, collectors, parity, i18n, ...). Pertahankan dependensi sesedikit mungkin (prinsip 0005).
3. **Katalog i18n (en, id)** dengan kunci stabil, dan uji yang menolak teks tampilan yang ditulis tetap.
4. **Prototipe pengukuran:** satu halaman tabel di Svelte 5 + Vite (tanpa SvelteKit) ditanam ke satu biner Rust. Ukur ukuran bundel, ukuran biner, memori diam, dan waktu mulai. Jadikan angkanya **anggaran kinerja** yang diperiksa di CI.
5. **Skema database** (`docs/design.md` bagian 3) sebagai migrasi, SQLite lebih dulu, dengan uji isolasi tenant.
6. **M1:** kolektor inventaris Linux read-only, tiga aturan kesetaraan pertama dengan fixture anonim, monitor uptime dasar, mesin alert, dan bot Telegram.
7. Cangkang Tauri tipis menyusul setelah M1-M2 (skill `tauri-v2-*`, command dibatasi `AppManifest::commands`).

## Keputusan yang masih menunggu pemilik

- DCO atau CLA (penting bila ingin dual licensing), konfirmasi `AGPL-3.0-only`, nasihat hukum.
- Cek merek dagang kata "Copilot" dan nama final.
- Kontak pelaporan di Kode Etik (masih penanda). Pelaporan kerentanan privat GitHub sudah aktif.
- Penyedia pembayaran global dan harga edisi berbayar (belum diriset).
- Riset yang belum dikerjakan: DNS dan Cloudflare Tunnel, buildpacks untuk modul deployment, regulasi di luar Indonesia.
