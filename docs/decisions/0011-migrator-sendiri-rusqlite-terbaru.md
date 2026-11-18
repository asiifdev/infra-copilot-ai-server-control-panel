# 0011. Migrator sendiri berbasis `user_version`, `rusqlite` versi terbaru

- Tanggal: 2026-11-19
- Status: diterima oleh pemilik ("gas", 2026-11-19)
- Pengambil keputusan: pemilik proyek

## Konteks
Pemilik menetapkan semua dependensi memakai versi stable terbaru. `rusqlite` terbaru adalah 0.43, tetapi `rusqlite_migration` 2.7.0 (juga yang terbaru) hanya cocok dengan `rusqlite` 0.41, dan Cargo menolak dua versi `libsqlite3-sys` sekaligus. Akibatnya `db` tertahan di 0.41 (lihat 0009).

## Keputusan
Hapus `rusqlite_migration`. Tulis migrator kecil sendiri (`crates/db/src/migrate.rs`) dan naik ke `rusqlite` 0.43.

- Migrasi `n` (mulai dari 1) berasal dari `migrations/000n_*.sql`, ditanam dengan `include_str!`.
- Versi disimpan di `PRAGMA user_version`, di dalam transaksi yang sama dengan migrasinya, sehingga migrasi yang gagal dibatalkan seluruhnya.
- Hanya maju. Database yang versinya lebih baru dari biner ditolak (`Error::DatabaseNewer`), tidak diturunkan diam-diam.

## Alasan
- Mengikuti instruksi versi terbaru tanpa menunggu pustaka lain.
- Kebutuhan kita sederhana (SQL berurutan); kodenya ±50 baris, diuji, dan mengurangi satu dependensi (sesuai 0005).

## Alternatif
- Tetap di `rusqlite` 0.41 menunggu `rusqlite_migration`: ditolak pemilik (bukan versi terbaru).
- Pustaka migrasi lain: belum dievaluasi; tidak perlu untuk kebutuhan ini.

## Dampak dan risiko
- Kita memegang kode migrasi sendiri. Tidak ada fitur turun-versi, migrasi berbasis kode Rust, atau pemeriksaan perubahan migrasi yang sudah terapan. Bila kebutuhan itu muncul, evaluasi ulang pustaka.
- Menggantikan baris `rusqlite_migration` di 0007 dan catatan ketidakcocokan versi di 0009.
