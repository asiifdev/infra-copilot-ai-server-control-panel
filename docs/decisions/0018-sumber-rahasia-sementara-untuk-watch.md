# 0018. Sumber rahasia sementara (variabel lingkungan) untuk crate `watch`

- Tanggal: 2026-11-19
- Status: diterima sebagai langkah sementara (pemilik meminta modul D dilanjutkan, 2026-11-19)
- Pengambil keputusan: pemilik proyek

## Konteks
Aturan proyek no. 10: konfigurasi pengguna di database, dan di lingkungan hanya URL DB, kunci enkripsi utama, dan kunci lisensi. Crate `watch` (monitor + alert + Telegram) perlu token bot, tetapi penyimpanan rahasia terenkripsi di DB belum dibangun (M0 baru punya skema; pustaka AES-256-GCM belum dievaluasi).

## Keputusan
`watch` membaca token lewat antarmuka `SecretSource`. Satu-satunya implementasi saat ini, `EnvSecrets`, membaca variabel lingkungan yang **namanya** tertulis di konfigurasi (`token_ref`); token tidak pernah ada di berkas konfigurasi, log, laporan, atau keluaran `Debug`. Nama yang menyerupai token ditolak saat validasi. Ini jalur dogfooding mandiri saja; sumber berbasis DB menggantikannya begitu penyimpanan rahasia ada, tanpa mengubah `Engine`.

## Alasan
Memungkinkan monitor + Telegram berjalan dan diuji sekarang tanpa membangun penyimpanan rahasia lebih dulu, tanpa menaruh token di berkas.

## Batas
Pengecualian terhadap aturan no. 10 ini sengaja sempit dan harus dicabut sebelum rilis publik produk berpenyimpanan DB. Token di lingkungan proses dapat dibaca pengguna yang sama.
