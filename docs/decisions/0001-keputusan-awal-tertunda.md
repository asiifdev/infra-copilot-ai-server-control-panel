# 0001. Keputusan awal yang masih tertunda

- Tanggal: 2026-11-19
- Status: diusulkan
- Pengambil keputusan: pemilik proyek

Catatan ini mengumpulkan keputusan bagian 11 `docs/plan.md` yang belum ditetapkan. Setelah ada jawaban, **pecah menjadi satu berkas per keputusan** (0002, 0003, ...) dan ubah status menjadi `diterima`. Sampai saat itu, jangan memilih secara diam-diam.

| # | Keputusan | Usulan | Jawaban |
|---|---|---|---|
| 1 | Nama produk dan lokasi repo (publik + privat komersial) | repo publik untuk inti, repo privat untuk lisensi/pembayaran | nama sementara **Infra Copilot**; lokasi repo belum (0006) |
| 2 | Lisensi kode publik | AGPL (seperti Prospex), perlu nasihat hukum | **sementara** `AGPL-3.0-only`, menunggu nasihat hukum (0006) |
| 3 | Stack inti, UI, dan agen | **usulan direvisi (0003):** satu inti ringan (Go atau Rust) dengan UI SPA tertanam, bukan Next.js; SQLite tertanam untuk desktop/self-host kecil, PostgreSQL untuk cloud | **Rust** untuk inti, agen, dan cangkang desktop Tauri (0003, 0004); UI: **Svelte 5 + Vite + TypeScript, tanpa SvelteKit** (final, 0005) |
| 4 | Cakupan MVP | M0 + M1 + M2 (read-only), eksekusi M4 belakangan | belum; skenario uji: migrasi Mac ke server (lihat 0002) |
| 5 | Pasar awal | Indonesia dulu atau global langsung | **global**, bahasa minimal ID/EN (lihat 0002) |
| 6 | Pengganti FlyEnv (M5) paralel dengan produk | ya, karena kebutuhan lokal tidak menunggu | belum |

Pertanyaan teknis terbuka tambahan ada di bagian 11 `docs/design.md`.
