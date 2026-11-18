---
name: migration-planning
description: Merancang model migrasi (sumber, target, lokasi backup, transport) dan konfigurasi otomatis rclone/scp/rsync, cutover DNS, serta rencana tertulis. Gunakan saat membangun wizard migrasi, modul transport, atau perencana migrasi.
---

# Perencanaan migrasi

Baca bagian 3.1 `docs/plan.md`.

## Model

- **Node** = satu titik (VPS atau lokal). Sumber dan target keduanya Node. Kombinasi: VPS→VPS, lokal→VPS, VPS→lokal, lokal→lokal.
- **Akses Node:** agen keluar-saja (disarankan), SSH (self-host), atau lokal.
- **Lokasi backup:** di sumber, di lokal pengguna, atau di perantara.
- **Transport:** rclone (S3, Drive, B2, dll), rsync/scp langsung, pipa SSH, atau penyimpanan terkelola (Cloud).
- **Strategi:** snapshot penuh, lalu delta susulan sebelum cutover. Selalu catat waktu snapshot agar selisih data terlihat.
- **Cutover:** Cloudflare Tunnel atau ganti A record. Snapshot DNS dulu, rollback tersedia.

## Aturan

- Rencana adalah **dokumen tertulis** yang bisa ditinjau sebelum eksekusi: langkah, dampak, urutan, titik batal, waktu perkiraan, dan syarat sebelum mematikan server lama.
- Server lama **tidak boleh dimatikan** sampai laporan kesetaraan tidak punya temuan `critical` yang terbuka.
- Penyimpanan perantara: enkripsi di sisi klien sebelum unggah bila memakai penyimpanan terkelola.
- Cek kapasitas dulu (kuota tujuan, ruang disk). Backup besar tidak boleh dibuat lokal bila disk tidak muat; gunakan streaming.
- Kecualikan yang bisa dibangun ulang (log, cache, `node_modules`) tetapi **tampilkan daftarnya** kepada pengguna.

## Konfigurasi rclone otomatis

Wizard: buat remote → otorisasi OAuth bila perlu (tautan dibuka pengguna, token tidak dicetak) → uji (`rclone about`/`lsd`) → cek kuota → ukur kecepatan → simpan.
- Kredensial dan token disimpan terenkripsi, hanya ditulis ke konfigurasi lewat stdin/berkas berizin ketat, tidak lewat argumen perintah (terlihat di daftar proses).
- Tidak memasang rclone di mesin pengguna tanpa persetujuan; tawarkan, jangan putuskan sendiri.
- Tampilkan kegagalan apa adanya (kuota, token kedaluwarsa, remote salah) dengan saran, bukan error mentah.

## Yang harus dicek sebelum cutover

Kasus nyata yang pernah terlewat: layanan tanpa padanan, `localhost` di dalam container, antrean ke Redis berbeda, DNS yang tidak pindah, database sama nama beda isi, proses jalan di mode dev dari terminal. Jalankan aturan di skill `parity-rules` dan selesaikan semua temuan `critical`.
