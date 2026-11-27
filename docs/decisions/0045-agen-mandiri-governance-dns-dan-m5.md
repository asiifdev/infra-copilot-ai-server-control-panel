# 0045. Agen mandiri (`nodivara-agent`), tata kelola dua penyetuju, adapter DNS, dan aksi penuh M5

- Tanggal: 2026-11-28
- Status: diterima (melanjutkan ADR 0038, 0043, dan 0044; migrasi `0015_governance_and_m5.sql`, skema versi 15)
- Pengambil keputusan: pemilik proyek

## Konteks
Setelah eksekutor jarak jauh (`SshExecutor` dan API antrean agen *pull-only* di ADR 0043) serta cangkang desktop tipis (ADR 0044) terpasang, ada empat celah utama yang perlu ditutup untuk mencapai kesiapan pra-produksi M4/M5 dan fondasi open-core M6 tanpa melanggar batasan repositori publik:
1. Server jarak jauh yang memakai mode `agent` belum memiliki biner agen mandiri untuk dipasang di host target.
2. Persetujuan Tier 4 masih membutuhkan penegakan dua penyetuju berbeda di sisi database dan server saat `require_two_approvers` aktif, serta antarmuka *entitlements* open-core di repo publik tanpa mencampurkan kode harga/penagihan komersial.
3. Manajemen DNS (khususnya Cloudflare) belum memiliki abstraksi `DnsProvider` beserta *snapshot* dan *diff* untuk *rollback*.
4. Aksi M5 di `crates/actions` baru mencakup layanan dan kontrak situs lokal `.test`, belum mencakup konfigurasi situs penuh (PHP/TLS), penerbitan sertifikat TLS, dan pencadangan database terstruktur. Selain itu, penambahan fitur di `web/` harus tetap berada di bawah anggaran ukuran bundel (`180,000 B` JS mentah dan `64,000 B` gzip).

## Keputusan
1. **Antarmuka *entitlements* open-core (`crates/core/src/entitlements.rs`)**:
   - Menambahkan kontrak `EntitlementProvider`, `LimitsService`, `EntitlementLimits`, `UsageMetric`, serta implementasi bawaan `LocalCommunityEntitlements`.
   - Repositori publik hanya menyimpan antarmuka dan nilai bawaan komunitas lokal (tanpa pola `ee/`); logika harga, penagihan, dan server lisensi tetap berada di repositori privat.

2. **Biner agen keluar-saja mandiri (`crates/nodivara-agent`)**:
   - Menyediakan biner agen mandiri yang hanya melakukan koneksi keluar (*pull-only*) ke `POST /api/agents/poll` menggunakan token agen node terkait.
   - Agen memverifikasi `args_hash` sebelum mengeksekusi `Action` dari daftar tertutup melalui `LocalExecutor` + `ProcessRunner`, lalu meredaksi (`core_domain::redact`) dan membatasi ukuran keluaran sebelum mengirim hasil ke `POST /api/agents/result`.

3. **Adapter DNS (`crates/dns`)**:
   - Menambahkan trait `DnsProvider` dan adapter `CloudflareDnsProvider` untuk pengelolaan zona dan rekaman DNS, pemeriksaan kepemilikan (*ownership check*), serta pembuatan *snapshot* dan *rollback diff*.

4. **Aksi tertutup M5 (`crates/actions`)**:
   - Menambahkan aksi daftar tertutup M5 (`ConfigureSite`, `IssueTlsCert`, `BackupDatabase`) lengkap dengan dukungan `dry_run`, `backup`, dan `rollback` pada `LocalExecutor` dan `SshExecutor`.

5. **Migrasi skema versi 15 (`crates/db/migrations/0015_governance_and_m5.sql`) & penegakan server**:
   - Menambahkan kolom `organizations.require_two_approvers`, tabel `ai_provider_models`, `licenses`, `usage_counters`, kolom `local_sites.php_version` dan `local_sites.tls_enabled`, serta tabel `dns_snapshots`.
   - Menegakkan aturan dua penyetuju berbeda untuk Tier 4 di `db::decide_proposal_with_details` dan `server::verify_tier_approval`, serta mengekspos status batasan melalui `GET /api/entitlements`.

6. **Pembaruan UI web (`web/`) dalam batas anggaran bundel**:
   - Menambahkan tampilan progres 2 penyetuju pada persetujuan Tier 4, cuplikan perintah pemasangan/jalankan `nodivara-agent`, serta opsi versi PHP dan TLS pada manajemen situs M5.
   - Melakukan konsolidasi *stylesheet* agar total ukuran bundel JS mentah tetap di bawah `180,000 B` (`177,590 B`) dan gzip di bawah `64,000 B` (`~61.2 KB`).

## Alasan
- Memisahkan `EntitlementProvider`/`LimitsService` di `crates/core` menjaga prinsip open-core yang bersih (aturan 7 `CLAUDE.md`) sekaligus memungkinkan penegakan batas pemakaian di `crates/server`.
- Biner `nodivara-agent` yang *pull-only* dengan verifikasi `args_hash` dan daftar aksi tertutup memastikan node jarak jauh tidak memerlukan port masuk atau akses shell bebas.
- Penegakan dua penyetuju di transaksi database (`db::decide_proposal_with_details`) dan pemeriksaan tingkat di `server::verify_tier_approval` menutup celah kondisi balap (*race condition*) dan penyetuju ganda yang sama.

## Alternatif
- **Menyimpan logika lisensi berbayar atau integrasi pembayaran di repo publik**: ditolak; melanggar pemisahan repo publik dan komersial.
- **Membuat biner agen membuka port HTTP/gRPC masuk (*push*)**: ditolak; bertentangan dengan model keamanan *pull-only* di ADR 0043 dan `docs/plan.md` §7.
- **Menambah dependensi *code-splitting* atau bundler baru untuk memangkas ukuran JS**: ditolak untuk tahap ini karena konsolidasi *stylesheet* sudah menurunkan JS mentah ke `177,590 B` dan gzip ke `~61.2 KB` tanpa menambah kompleksitas build.
