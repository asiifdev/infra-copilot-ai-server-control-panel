# 0039. Persetujuan & eksekusi rencana migrasi, pemeriksaan DNS runner transport, dan pembaruan kredensial transport (M4, langkah 3b)

- Tanggal: 2026-11-27
- Status: diterima (melanjutkan M4 di atas 0028, 0033, 0037, dan 0038)
- Pengambil keputusan: pemilik proyek

## Konteks
Setelah 0037 dan 0038 membuka eksekusi Usulan tingkat 2–4 pada node lokal, ada tiga celah yang masih menahan alur migrasi dan kesiapan infrastruktur bersama:
1. Rencana migrasi (`migration_plans` dan `migration_steps`, 0024 & 0028) baru bisa dibuat sebagai `draft` dan dilihat daftarnya; belum ada persetujuan rencana maupun eksekusi langkah berurutan yang menegakkan gerbang (`GateReport`) dan pencatatan status langkah secara atomik.
2. Pada 0033 butir 4, pemeriksaan endpoint `SharedInfrastructure` baru menilai teks host/IP literal di `TransportSpec`; runner transport nyata (`ProcessRunner`) belum me-resolve nama DNS dan memeriksa setiap alamat hasilnya dengan `is_internal_ip` sebelum koneksi, sehingga nama DNS publik yang menunjuk ke IP privat masih bisa lolos (SSRF / DNS rebinding).
3. Pembaruan kredensial transport di 0028 mengharuskan pengguna menghapus transport lalu menambahkannya ulang, padahal rencana migrasi dapat merujuk pada nama/ID transport yang sama.

## Keputusan
1. **Persetujuan dan eksekusi rencana migrasi (`db`, `plan`, `server`, `cli`, `web`)**:
   - **Persetujuan terpisah (`draft -> approved`)**: `approve_plan` (CLI `infra plan approve <id|name>` dan `POST /api/plans/{id}/approve`) hanya memindahkan status rencana dari `draft`/`reviewed` menjadi `approved` dalam satu transaksi bersama entri audit berantai-hash (`plan.approve`). Menyetujui tidak menjalankan langkah apa pun.
   - **Eksekusi rencana & pelacakan status atomik (`approved -> running -> done|failed`)**: `infra plan run <id|name>` dan `POST /api/plans/{id}/run` (serta eksekusi per langkah bila diperlukan) wajib melewati pemeriksaan gerbang `plan::gate_report` dan `Plan::check_schedulable`: langkah `cutover` dan `decommission_source` ditolak bila ada temuan kritis terbuka atau bukti inventaris tidak lengkap, dan `decommission_source` hanya boleh maju setelah semua langkah lain `succeeded` (atau `delta` yang `skipped`, dijaga juga oleh pemicu SQLite di 0008).
   - Setiap langkah pada `migration_steps` bertransisi secara atomik di DB (`pending -> running -> succeeded|failed|skipped`) dan dicatat pada audit berantai-hash (`plan.run.start`, `plan.step.start`, `plan.step.finish`, `plan.run.finish`). Langkah yang gagal menghentikan rangkaian dan tidak diulang otomatis.
   - **Kewajiban `--enable-execution`**: konsisten dengan 0037 dan 0038, eksekusi rencana di server tetap **mati secara bawaan** dan hanya aktif bila `infra serve --enable-execution` dijalankan.

2. **Penegakan resolusi DNS + `is_internal_ip` di `Runner` transport (`crates/transport`)**:
   - Melanjutkan 0033 butir 4, `ProcessRunner` menerima `EndpointPolicy` (`OwnNetwork` untuk CLI/server lokal, `SharedInfrastructure` untuk infrastruktur bersama).
   - Saat kebijakan `SharedInfrastructure` aktif, sebelum proses anak dijalankan untuk perintah yang membawa endpoint jaringan, runner me-resolve hostname target menjadi daftar alamat IP dan memeriksa **setiap** alamat dengan `transport::policy::is_internal_ip`. Jika ada satu saja alamat internal/terlarang atau resolusi gagal/kosong, eksekusi ditolak sebelum koneksi dibuka.
   - Untuk mencegah *DNS rebinding*, runner menyematkan (*pin*) alamat IP yang sudah tervalidasi ke rencana koneksi alih-alih membiarkan proses anak melakukan resolusi DNS ulang secara terpisah. Perilaku `OwnNetwork` tetap tidak dibatasi.

3. **Pembaruan kredensial transport *write-only* (`db`, `transport`, `server`, `cli`, `web`)**:
   - Menambahkan jalur pembaruan kredensial tanpa harus menghapus dan membuat ulang baris transport:
     - **CLI**: `infra transport update-credentials <name> --credentials-stdin` membaca pasangan `name=value` hanya dari `stdin` (tidak pernah dari argumen perintah).
     - **Server**: `POST /api/transports/{id}/credentials` menerima kredensial melalui badan JSON (dibatasi ukuran pesan server, tanpa pencatatan isi ke log).
     - **Penyimpanan**: kredensial baru disegel oleh `vault` dengan tujuan `transport_credentials`, referensi `secret_id` pada `transports` diperbarui secara atomik, status verifikasi (`verified_at`, `last_check`) di-reset karena kredensial berubah, dan kejadian dicatat di audit log tanpa rahasia.
   - Operasi ini bersifat ***write-only***: tidak ada endpoint API maupun perintah CLI yang mengembalikan nilai kredensial yang tersimpan.

## Belum
- Eksekusi langkah migrasi masih berjalan pada konteks lokal/proses terkelola dari `infra serve --enable-execution` dan belum mencakup agen jarak jauh terdistribusi.
- Pemantauan uptime berkelanjutan (`watch` loop), insiden, jam tenang, serta bot Telegram dua arah dijadwalkan pada Sprint 3 (M1-b).

## Alternatif
- **Menjalankan langkah rencana otomatis setelah `plan approve`**: ditolak; mempertahankan pemisahan persetujuan manusia dan eksekusi (konsisten dengan 0034 dan 0037).
- **Mengandalkan pemeriksaan string host saat `transport add` saja di Cloud**: ditolak; tidak melindungi dari *DNS rebinding* atau nama publik yang menunjuk ke IP privat (0033 butir 4).
- **Mempertahankan alur hapus-dan-tambah-ulang untuk rotasi kredensial transport**: ditolak; memutus referensi ID/nama transport pada rencana migrasi yang sudah ada.
