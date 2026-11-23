[English](comparison.md) | Bahasa Indonesia

# Perbandingan dengan panel server lain

Terakhir diperbarui: 2026-11-19. Fakta di bawah dibaca dari halaman resmi vendor pada tanggal itu;
harga dan fitur berubah, jadi periksa sumber yang ditautkan sebelum mengandalkannya.

**Nodivara masih pra-alfa: hanya pemeriksaan kesetaraan hanya-baca, baris perintah lokal, dan UI web lokal (hanya loopback, satu pengguna) yang berjalan hari ini, dari kode sumber, sedangkan agen AI dan alur persetujuan di barisnya belum tersedia.** Halaman ini
bukan klaim bahwa ia lebih baik dari produk mana pun yang tercantum. Produk lainnya sudah berjalan
hari ini.

## Gambaran

| Produk | Jenis dan lisensi | Harga (per 2026-11-19, di luar pajak) | Fitur AI menurut vendor | Sumber |
|---|---|---|---|---|
| cPanel & WHM | Panel komersial | Solo $29,99/bulan (ditagih bulanan), $27,46/bulan (tahunan); Admin $35,99; Pro $53,99; Premier $69,99. Harga dijadwalkan berubah pada 2026-12-16 | AI Assistant (bertindak atas nama pelanggan, dengan konfirmasi pelanggan lebih dulu), konektivitas MCP dengan izin terbatas dan jejak audit, AI App Hosting, AI Builder | [harga](https://www.cpanel.net/pricing/), [cPanel AI](https://www.cpanel.net/cpanel-ai/) |
| aaPanel | Edisi gratis plus Pro; lisensi tertulis "Other" di GitHub | Pro: $198/tahun, $699 seumur hidup, $28,80/bulan | Tidak ditemukan di halaman yang dibaca | [harga](https://www.aapanel.com/new/pricing.html) |
| 1Panel | Open core, GPL-3.0 | Pro: $80/tahun atau $200 sekali bayar | Mengelola aplikasi dan agen AI di server (misalnya OpenClaw), menurut daftar fiturnya | [edisi Pro](https://1panel.pro/pro-edition) |
| Coolify | Open source, Apache-2.0 | Self-host gratis dengan semua fitur; Cloud $5/bulan untuk 2 server, ditambah $3 per server tambahan | Tidak ditemukan di halaman yang dibaca | [harga](https://coolify.io/pricing) |
| Dokploy | Open core: Apache-2.0 plus direktori proprietary | Cloud Hobby $4,50/server/bulan; Startup mulai $15/bulan | Asisten AI dengan kunci sendiri; analisis log hanya-baca menurut dokumentasinya | [harga](https://dokploy.com/pricing) |
| **Nodivara** | **Open core, sementara AGPL-3.0-only** | **Belum ditetapkan** | **Tersedia dari kode sumber (2026-11-20):** pemeriksaan kesetaraan migrasi hanya-baca (7 aturan). **Direncanakan:** agen AI dengan persetujuan, analisis error yang menyebut bukti | [README](../README.id.md) |

## Yang ingin dilakukan berbeda oleh Nodivara

Ini tujuan, bukan fitur yang sudah ada.

- **Setiap aksi yang mengubah keadaan melalui usulan tersimpan dan persetujuan manusia**, dengan
  lima tingkat risiko. Aksi merusak butuh nama targetnya diketik.
- **Pemeriksaan kesetaraan sebelum migrasi server dianggap selesai** (versi hanya-baca dengan tujuh aturan sudah berjalan hari ini), yang mendaftar apa yang
  tertinggal.
- **Analisis error yang menyebutkan bukti yang dipakai** dan memisahkan fakta dari dugaan.
- **Rahasia tidak pernah sampai ke model AI**, ditegakkan lewat penyamaran sebelum pencatatan,
  penyimpanan, dan prompt.

## Yang sudah dilakukan dengan baik oleh produk lain

- cPanel, aaPanel, dan 1Panel adalah panel matang dengan basis pengguna besar, dan cPanel kini
  mengirimkan fitur AI dengan konfirmasi sebelum aksi dan jejak audit pada konektivitas MCP-nya.
- Coolify dan Dokploy adalah platform deploy self-hosted yang populer dengan self-hosting gratis.
- Nodivara belum punya pengguna, rilis, maupun rekam jejak.

## Cara fakta dikumpulkan

Halaman harga dan fitur vendor dibaca pada 2026-11-19. Sebagian deskripsi fitur berasal dari
halaman pemasaran vendor, yang mungkin tidak menampilkan semua yang dilakukan produknya. "Tidak
ditemukan di halaman yang dibaca" berarti halaman yang dibaca tidak menyebut fitur AI, bukan
berarti fiturnya tidak ada. Riset lengkap beserta keterbatasannya ada di
[market-research.md](market-research.md).
