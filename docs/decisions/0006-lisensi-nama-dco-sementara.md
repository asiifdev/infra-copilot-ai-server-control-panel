# 0006. Lisensi, nama, dan ketentuan kontribusi sementara; kebijakan dokumen dua bahasa

- Tanggal: 2026-11-19
- Status: diusulkan (sementara, menunggu konfirmasi pemilik dan nasihat hukum sebelum rilis publik)
- Pengambil keputusan: pemilik proyek

## Konteks
Repositori akan dibuka sebagai open source. Pemilik meminta README, panduan kontribusi, lisensi, dan
dokumen lain dilengkapi dulu, dengan versi bahasa Indonesia dan Inggris, dan menetapkan nama
sementara "Infra Copilot".

## Keputusan sementara
1. **Nama: Infra Copilot** (sementara).
2. **Lisensi: `AGPL-3.0-only`**, sesuai usulan di 0001 (nomor 2). Teks diambil dari data lisensi
   SPDX. Hanya teks Inggris yang disediakan sebagai `LICENSE`; penjelasan bahasa Indonesia ada di
   `LICENSING.id.md` karena terjemahan lisensi tidak mengikat secara hukum.
3. **Kontribusi: DCO** (`git commit -s`) sementara, karena lebih ringan daripada CLA.
4. **Kebijakan dokumen:** setiap dokumen untuk publik punya versi Inggris (utama, `X.md`) dan
   Indonesia (`X.id.md`), saling menaut di baris pertama, dan diperbarui bersamaan. Pengecualian:
   `LICENSE` (hanya teks asli), `CHANGELOG.md` (hanya Inggris agar tidak melenceng).
5. **Kode Etik:** Contributor Covenant 2.1, memakai terjemahan Indonesia resminya.

## Hal yang belum diputuskan dan harus diselesaikan sebelum rilis publik
- **DCO tidak cukup bila pemilik ingin juga menjual lisensi komersial untuk kode yang sama (dual
  licensing).** DCO tidak memberi hak melisensikan ulang kontribusi orang lain. Untuk itu butuh CLA
  atau penyerahan hak cipta. Ini keputusan hukum dan bisnis yang berdampak ke semua kontributor.
- Konfirmasi `AGPL-3.0-only` dibanding `AGPL-3.0-or-later` atau lisensi source-available, setelah
  nasihat hukum.
- Pemegang hak cipta hukum (sementara "Infra Copilot contributors").
- **Pemeriksaan merek dagang untuk nama.** Kata "Copilot" banyak dipakai produk perusahaan lain;
  risiko konflik belum diperiksa.
- Kontak pelaporan untuk Kode Etik dan kebijakan keamanan (saat ini penanda yang harus diisi).

## Alternatif
- Lisensi permisif (MIT/Apache-2.0): ditolak sementara karena tidak melindungi dari layanan
  terkelola pihak lain yang tidak berbagi kode; keputusan akhir menunggu nasihat hukum.
- CLA sejak awal: lebih fleksibel untuk komersialisasi, tetapi menambah hambatan bagi kontributor.

## Dampak
Daftar periksa rilis: `docs/RELEASE-CHECKLIST.md`.
