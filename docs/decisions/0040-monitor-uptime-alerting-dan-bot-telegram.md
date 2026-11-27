# 0040. Monitor uptime, alerting dengan jam tenang, dan bot Telegram interaktif (M1-b)

- Tanggal: 2026-11-27
- Status: diterima (melanjutkan M1-b di atas 0016, 0018, 0021, 0029, dan 0037)
- Pengambil keputusan: pemilik proyek

## Konteks
Pustaka `net`, `monitor`, `notifier`, dan `watch` sudah memiliki logika pemantauan murni, pemeriksaan alamat internal, serta kanal Telegram satu arah, tetapi belum terhubung ke penyimpanan SQLite maupun server lokal `infra serve`. Selain itu, 0018 masih memakai `EnvSecrets` sebagai penambal sementara untuk token bot Telegram, dan interaksi dua arah dari Telegram (perintah status, log, pergantian bahasa, serta persetujuan tombol cepat untuk aksi berisiko rendah) belum tersedia.

Sprint 3 menghubungkan seluruh alur pemantauan uptime, peringatan, penahanan jam tenang (*quiet hours*), dan bot Telegram interaktif ke `infra serve` sambil menjaga batas keamanan persetujuan dan anggaran ukuran biner/UI (`ui_gzip_bytes <= 64,000 B`, 0032).

## Keputusan
1. **Database (migrasi `0012_monitor_and_alerts.sql`, skema versi 12)**:
   - **`monitor_targets`**: menyimpan konfigurasi target pemantauan per organisasi (`org_id`, `node_id` opsional, `name`, `kind`, `target`, `interval_sec`, `timeout_sec`, `allow_private`, `is_active`, status terakhir).
   - **`incidents`**: mencatat siklus hidup insiden (`monitor_id`, `started_at`, `resolved_at`, `cause`, `acknowledged_by`, `severity`).
   - **`notification_channels`**: menyimpan kanal notifikasi per organisasi; token rahasia (misalnya token bot Telegram) **tidak pernah disimpan mentah**, melainkan disegel oleh `vault` ke tabel `secrets` dan dirujuk lewat `secret_id` dengan *composite foreign key* `(org_id, secret_id)` (menutup penambal sementara `EnvSecrets` di 0018).
   - **`alert_rules`**: aturan peringatan dengan jendela jam tenang **`quiet_start`** dan **`quiet_end`** serta tingkat keparahan minimum (`min_severity`).
   - **`pending_alerts`**: antrean peringatan yang ditahan sementara saat jam tenang aktif agar tidak hilang dan tetap terkirim setelah jendela jam tenang berakhir.
   - **`telegram_links`**: memetakan `(org_id, user_id, chat_id)` yang terhubung beserta preferensi bahasa (`language`, `id`/`en`) dan status verifikasi.

2. **Background loop `watch` di `infra serve` & penahanan jam tenang**:
   - `infra serve` menjalankan *background loop* `watch` secara terkelola (berhenti bersih bersama *shutdown* server) menggunakan `watch::NetProber` (`net`) dan `vault` sebagai sumber rahasia.
   - Saat peringatan terpicu di dalam jendela `quiet_start`–`quiet_end` pada `alert_rules`, mesin alert **menahan** pengiriman ke tabel `pending_alerts` (kecuali bila aturan mengizinkan atau di luar jam tenang) alih-alih menjatuhkannya.
   - Begitu waktu keluar dari jam tenang, loop melakukan ***flush* `pending_alerts`** ke kanal tujuan lewat `notifier`, mencatat hasil pengiriman, dan membersihkan antrean tertahan.

3. **Bot Telegram interaktif (`crates/notifier`, `crates/server`, `crates/cli`/UI)**:
   - Menyediakan perintah dasar pada chat terhubung:
     - `/link <code>`: menautkan `chat_id` Telegram ke pengguna melalui kode sekali pakai dan menyimpannya di `telegram_links`.
     - `/status`: menampilkan ringkasan status node, monitor, dan insiden aktif (tingkat 0, baca-saja).
     - `/log`: menampilkan ringkasan temuan/eksekusi terbaru yang sudah ter-redaksi (tingkat 0, baca-saja).
     - `/bahasa <id|en>`: mengubah preferensi bahasa pada `telegram_links` untuk chat terkait.
     - `/restart <service>`: membuat Usulan `RestartService` (`actions` + `proposals`, tingkat 2) dan mengirimkan kartu dengan tombol persetujuan inline.
   - **Batas keamanan persetujuan tombol (ketat tingkat 0–2)**:
     - Persetujuan lewat tombol inline Telegram **dibatasi ketat hanya untuk aksi tingkat 0–2** (dan tetap tunduk pada pengaman eksekusi `--enable-execution` serta validasi `args_hash`).
     - Aksi **tingkat 3 dan 4 ditolak** dieksekusi atau disetujui lewat bot Telegram; bot hanya memberi tahu pengguna dan mengarahkan mereka ke Web UI (tempat `dry_run`, cadangan wajib, dan konfirmasi ketik tingkat 4 ditegakkan sesuai 0038).
     - Semua perintah, penautan, dan persetujuan tombol dari bot dicatat ke `audit_log` berantai-hash dan seluruh pesan keluar diredaksi.

## Belum
- Pengiriman notifikasi untuk kanal selain Telegram (email, WhatsApp, Slack, Discord, webhook) masih menunggu adapter masing-masing.
- Mode webhook Telegram untuk edisi Cloud (multi-tenant publik) menyusul bersama akses jarak jauh dan TLS; server lokal memakai *polling*.

## Alternatif
- **Menjatuhkan peringatan yang terjadi saat jam tenang**: ditolak; insiden penting yang terjadi malam hari akan terlewat tanpa jejak saat jam tenang berakhir, sehingga `pending_alerts` dipakai untuk menahan dan melakukan *flush*.
- **Mengizinkan persetujuan tingkat 3–4 lewat tombol Telegram**: ditolak; layar chat tidak cocok untuk meninjau *diff* panjang, verifikasi cadangan, maupun konfirmasi ketik target tingkat 4 (aturan inti 0038).
- **Menyimpan token bot Telegram di variabel lingkungan atau kolom teks biasa**: ditolak; wajib disegel di `vault` seperti rahasia penyedia AI dan kredensial transport (0021).
