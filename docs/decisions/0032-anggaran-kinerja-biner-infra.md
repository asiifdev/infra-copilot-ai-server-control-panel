# 0032. Anggaran kinerja untuk biner `infra` dengan UI tertanam

- Tanggal: 2026-11-24
- Status: diterima (melaksanakan 0005 dan 0010; angka Linux menunggu kalibrasi CI)
- Pengambil keputusan: pemilik proyek (instruksi "lanjutkan pengerjaan"); angka dapat diubah hanya lewat catatan keputusan

## Konteks
`docs/perf-budget.json` berasal dari probe M0 (`ui/` + `proto-server`). Produk sebenarnya kini `infra serve` dengan `web/dist` tertanam (0030, 0031), sehingga anggaran lama tidak lagi mengawasi apa yang dikirim.

## Keputusan
1. `scripts/measure.sh` menerima `TARGET=infra` (bawaan `proto`): membangun `web/` dan `cargo build --release -p cli`, menyiapkan database sementara dengan `infra init`, menjalankan `infra serve --port 0`, lalu mengukur dengan cara yang sama (ukuran bundel, ukuran biner, RSS diam 5 detik, waktu dari exec sampai `GET /` 200, median).
2. `scripts/check-budget.py` menerima `BUDGET_PROFILE=infra` untuk memilih blok `infra` di `docs/perf-budget.json`. Blok lama tetap untuk probe.
3. Job CI `perf-budget` (Linux dan macOS) mengukur dan memeriksa kedua target.

## Pengukuran (macOS arm64, 2026-11-24, 3 kali jalan) dan anggaran
| Metrik | Terukur | Anggaran |
|---|---|---|
| UI gzip | 44 903 B | 56 000 B |
| JS mentah | 130 878 B | 165 000 B |
| Biner rilis | 5 827 984 B | 7 340 032 B (7 MiB) |
| RSS diam | 10 000 KiB | 16 384 KiB |
| Mulai sampai 200 | 18,65 ms | 150 ms |

Margin sekitar 25-60%, sengaja longgar karena angka Linux belum pernah diukur; bila run CI Linux melampauinya, tulis blok `Linux` di `infra` berdasarkan angka CI beserta alasan, jangan menaikkan bawaan diam-diam.

## Alternatif
Mengganti blok lama: ditolak, probe masih ada sampai dipensiunkan (`ui/` dan `proto-server` dapat dihapus setelah anggaran baru stabil di CI).

## Perubahan
2026-11-24 (0035): UI gzip 56 000 → 64 000 B dan JS mentah 165 000 → 180 000 B setelah UI ops console.
2026-11-24 (kalibrasi Linux dari CI): biner rilis Linux terukur 7 351 152 B (macOS 5 977 840 B; tautan statis dan simbol yang berbeda), melampaui 7 MiB bawaan. Blok `Linux` kini membatasi `binary_bytes` pada 8 388 608 B (8 MiB, margin sekitar 14%); metrik lain tetap memakai bawaan (Linux terukur: RSS 8 192 KiB, mulai 11 ms, UI sama dengan macOS). macOS tetap 7 MiB.
