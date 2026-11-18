---
name: repo-hygiene
description: Memastikan tidak ada data pribadi, rahasia, atau nama klien di repo sebelum commit atau sebelum repo dibuka ke publik. Gunakan sebelum rilis, saat menyalin kode dari proyek lain (misalnya vps-restore-tools), atau saat membuat fixture dan dokumentasi.
---

# Kebersihan repo

Repo ini berencana dibuka sebagai open source. Data pribadi yang bocor tidak bisa ditarik kembali.

## Yang dilarang masuk repo
- Nama domain nyata, subdomain klien, alamat IP server nyata.
- ID akun, ID tunnel, ID zona (Cloudflare dan sejenisnya), nama proyek internal.
- Jalur ke kunci, nama berkas kunci, alamat surel pribadi, nama klien.
- Token, password, kunci API, isi `.env`, dump database, cuplikan log nyata.

## Pengganti
- Domain: `example.test`, `app.example.test`. IP: `203.0.113.x`, `198.51.100.x`. Nama: `acme`, `client-a`.
- Fixture dan contoh dibuat dari nol, bukan dari data nyata yang diedit sebagian.

## Pemeriksaan
`scripts/check-personal-data.sh` mencari pola dari berkas lokal `.claude/hygiene-patterns.local` (tidak ikut git, karena berisi data yang justru ingin disembunyikan).
- `scripts/check-personal-data.sh` memeriksa berkas yang dilacak git.
- `scripts/check-personal-data.sh --staged` memeriksa yang di-stage.
- Jalankan sebelum setiap commit ke remote publik dan wajib bersih sebelum rilis (tahap M7).
- Tambahkan pola baru ke berkas lokal setiap kali muncul data pribadi baru dalam pekerjaan.

## Catatan
- Repo ini privat sampai tahap M7. `docs/plan.md` masih menyebut asal kasus; bagian itu harus ditulis ulang generik sebelum repo dibuka.
- Riwayat git ikut terbaca. Kalau data pribadi pernah ter-commit, menghapus berkas saja tidak cukup; bersihkan riwayatnya atau mulai repo baru.
- Kode dari proyek lain disalin hanya setelah ditulis ulang generik dan diperiksa.
