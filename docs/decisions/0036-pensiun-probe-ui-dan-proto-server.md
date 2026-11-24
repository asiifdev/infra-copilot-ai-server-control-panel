# 0036. Pensiun prototipe `ui/` dan `proto-server`

- Tanggal: 2026-11-24
- Status: diterima (pemilik: "gas kerjakan aja"; syarat di 0032 terpenuhi: angka anggaran biner `infra` lulus di CI Linux dan macOS)
- Pengambil keputusan: pemilik proyek

## Keputusan
1. Direktori `ui/` dan crate `proto-server` dihapus. Keduanya hanya probe anggaran kinerja M0 (0010).
2. `ui/src/tokens.css` pindah ke `web/src/tokens.css` (satu-satunya sumber token warna; riwayatnya terjaga lewat `git mv`). Alias `$tokens` di `web/vite.config.ts` tetap, kini menunjuk `web/src`. `scripts/check-palette.py` membaca berkas baru.
3. `docs/perf-budget.json` kini hanya berisi anggaran biner `infra` (blok `default`, `Linux`, `Darwin`); `scripts/measure.sh` hanya mengukur `infra serve`; `BUDGET_PROFILE` dihapus; job CI `perf-budget` mengukur dan memeriksa satu target.
4. Angka M0 tetap terdokumentasi di 0010 sebagai sejarah.
