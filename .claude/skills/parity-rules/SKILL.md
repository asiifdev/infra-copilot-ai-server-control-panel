---
name: parity-rules
description: Menulis, mengubah, atau menguji aturan kesetaraan (parity) antara inventaris server sumber dan target. Gunakan saat menambah kolektor inventaris, aturan pembanding, temuan (findings), atau fixture uji dari kegagalan migrasi.
---

# Aturan kesetaraan (parity rules)

Aturan kesetaraan adalah pembeda utama produk. Baca bagian 1, 3.2, dan 6 di `docs/plan.md` dulu.

## Prinsip

- Inventaris berasal dari **apa yang berjalan** (proses, port, unit, container, cron), bukan dari daftar tulisan tangan.
- Uji dari **sudut pandang proses itu sendiri** (misalnya dari dalam container), bukan hanya dari host.
- Bandingkan **jalur publik** (DNS, sertifikat), bukan hanya jalur lokal.
- Status HTTP 200 **tidak** membuktikan kesetaraan. Sediakan uji fungsional.
- Kolektor bersifat **read-only**. Tidak boleh mengubah apa pun di Node.
- Rahasia tidak pernah masuk inventaris. Simpan **nama variabel dan host/port**, bukan nilainya.

## Bentuk sebuah aturan

Setiap aturan punya: `id` stabil (contoh `PAR-007`), judul, kategori, bobot (`critical|important|info`), input yang dibutuhkan dari inventaris, kondisi temuan, penjelasan awam, saran perbaikan, dan apakah perbaikannya bisa diusulkan AI (tingkat berapa, lihat `ai-tool-tiers`).

## Daftar awal (dari migrasi nyata)

1. Proses mendengarkan di sumber tanpa padanan di target.
2. Konfigurasi di container yang menunjuk `localhost` ke layanan di host.
3. Producer dan consumer antrean memakai Redis/DB berbeda.
4. Hostname DNS tidak mengarah ke target, atau ada di akun DNS lain.
5. Database bernama sama dengan jumlah baris berbeda; database kosong di sumber tapi terisi di tempat lain.
6. Aplikasi berjalan di luar supervisor atau dalam mode dev di produksi.
7. Variabel lingkungan merujuk host/port yang sudah tidak ada.
8. Cron/timer/backup terjadwal tanpa padanan, termasuk milik panel hosting.
9. Versi layanan berbeda yang mengubah format data.
10. Port terbuka ke internet di sumber atau target.
11. Uji fungsional: login dengan kredensial uji, koneksi DB dari dalam proses.

## Menambah aturan baru

1. Tulis dulu **fixture** (inventaris sumber + target yang memicu temuan) dan uji gagal.
2. Fixture harus **anonim**: domain `example.test`, IP dokumentasi (`203.0.113.x`), nama generik. Jangan salin data nyata.
3. Implementasikan aturan sampai uji lulus.
4. Tambahkan juga fixture "tidak ada temuan" agar tidak ada positif palsu.
5. Catat asal kasusnya (jenis kegagalan, tanpa nama nyata) di komentar aturan.

## Status implementasi (2026-11-19)

Penomoran: aturan ke-n pada daftar di atas = `PAR-00n` (n 1-9) atau `PAR-0nn`. Sudah ada: **PAR-001, PAR-008, PAR-010** (crate `parity`, kolektor `listeners`/`processes`/`cron` di crate `collectors`, kontrak di crate `inventory`, keputusan 0013). Belum ada: 2-7, 9, 11, dan aturan selisih OS/arsitektur (14.2).

Konvensi yang berlaku untuk aturan baru:
- Aturan mengimplementasikan `inventory::ParityRule` (id, versi, bobot, kategori, tingkat perbaikan AI, `evaluate(source, target, ctx)`), murni dan deterministik, dan hanya bergantung pada `inventory`.
- Teks judul, penjelasan, dan saran **ada di katalog i18n** (`parity.parNNN.title|explanation|advice`, en dan id), bukan di kode. Tambahkan kunci ke `crates/i18n/locales/` dan petakan di `parity/src/render.rs`.
- Inventaris `partial` tidak boleh dianggap bukti ketiadaan: gunakan pembantu `cap_if_partial` (menurunkan ke Info dan menandai `partial=true`).
- Fixture anonim (JSON, `schema_version` 1) di `crates/parity/tests/fixtures/` dipakai bersama uji CLI.
