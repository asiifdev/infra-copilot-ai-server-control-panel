# 0014. Pelaksana perintah terbatas untuk kolektor

- Tanggal: 2026-11-19
- Status: diterima (pemilik menyetujui memakai perintah `docker`, 2026-11-19)
- Pengambil keputusan: pemilik proyek

## Konteks
Kolektor M1-a hanya membaca berkas (`/proc`, cron). Docker dan systemd tidak bisa diinventaris dari berkas saja. Larangan di `CLAUDE.md` adalah "shell bebas" untuk agen AI, bukan larangan menjalankan program tetap yang read-only.

## Keputusan
`collectors::exec` menyediakan `CommandRunner` dan daftar tertutup `Cmd` (`docker ps`, `docker inspect <id>`, `systemctl list-units`, `systemctl list-timers`). Tanpa shell, argumen tetap (id kontainer divalidasi heksadesimal), lingkungan dikosongkan kecuali `PATH` dan `HOME`, tanpa stdin, batas waktu 10 detik, keluaran maksimum 2 MiB. Kegagalan menjadi peringatan (inventaris `partial`), bukan "tidak ada". Uji memakai `FakeRunner`; uji tidak pernah menyentuh Docker atau layanan nyata. Menambah perintah baru memerlukan tinjauan dan entri di sini.

## Alasan
Satu tempat yang bisa diaudit untuk semua eksekusi, tidak ada jalur dari teks server ke argumen perintah, dan kolektor tetap dapat diuji tanpa mesin nyata.

## Alternatif
Soket Docker lewat HTTP (dependensi dan kode lebih besar, ditolak sesuai 0005); hanya berkas konfigurasi (tidak melihat kontainer yang berjalan).

## Batas
`pm2 jlist` tidak termasuk: ia dapat menjalankan daemon PM2 bila belum hidup (efek samping). PM2 dibaca dari `dump.pm2`. Belum dicoba pada Docker/systemd nyata.
