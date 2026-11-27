# 0038. Eksekusi tingkat 3–4, cadangan wajib, konfirmasi ketik, dan rollback (M4, langkah 3a)

- Tanggal: 2026-11-27
- Status: diterima (melanjutkan M4 di atas 0034 dan 0037)
- Pengambil keputusan: pemilik proyek

## Konteks
0037 hanya mengizinkan aksi tingkat 2 (`restart_service` non-kritis) pada node lokal. Aksi tingkat 3 (seperti restart unit kritis `systemd-*`, `sshd`, `dbus`, atau perubahan konfigurasi yang punya risiko henti layanan) dan tingkat 4 (perubahan destruktif atau berdampak luas seperti `cutover` dan `decommission_source`) masih ditolak oleh `Action::executable_now()`.

Sesuai aturan inti proyek (`docs/design.md` bagian 9 dan skill `ai-tool-tiers`), aksi tingkat 3 dan 4 tidak boleh dijalankan tanpa:
1. Pratinjau (`dry_run`) yang memperlihatkan dampak dan *diff* ter-redaksi sebelum eksekusi;
2. Cadangan (*backup*) otomatis yang terverifikasi integritasnya (`sha256`) sebelum perubahan diterapkan;
3. Konfirmasi ketik eksplisit dari manusia pada tingkat 4 agar klik tak sengaja tidak memicu perubahan besar;
4. Mekanisme `rollback` satu kali per eksekusi yang tercatat dan diaudit; serta
5. Kesiapan skema persetujuan ganda (2 penyetuju berbeda) untuk edisi Cloud.

## Keputusan
1. **Database (migrasi 0011, skema versi 11)**:
   - **`approvals`**: tabel persetujuan terpisah yang mengikat `(org_id, proposal_id, user_id)` dengan `UNIQUE (org_id, proposal_id, user_id)` agar satu pengguna tidak bisa menyetujui dua kali. Pada edisi lokal (pengguna tunggal), 1 baris cukup; pada edisi Cloud, aksi tingkat 4 mensyaratkan 2 baris dari 2 penyetuju berbeda.
   - **`execution_backups`**: menyimpan metadata cadangan yang dibuat sebelum eksekusi (`org_id`, `execution_id`, `proposal_id`, `location`/`path` atau penanda target, `sha256`, `size_bytes`, `created_at`). Untuk aksi tingkat `>= 3`, eksekusi wajib gagal/ditolak bila cadangan belum berhasil dibuat dan dicatat dengan hash `sha256`.
   - **`execution_rollbacks`**: menyimpan catatan rollback dengan `UNIQUE (org_id, execution_id)` sehingga setiap eksekusi maksimal memiliki **1 rollback** (tidak ada pengulangan rollback otomatis atau ganda). Status dan keluaran ter-redaksi dicatat bersama entri audit berantai-hash (`proposal.rollback.start`, `proposal.rollback.finish`).
   - **`proposals.dry_run_json` & `proposals.dry_run_at`**: menyimpan hasil pratinjau `dry_run` terbaru beserta stempel waktunya sehingga UI dan manusia dapat meninjau dampak sebelum menjalankan eksekusi.

2. **Kontrak `Action` (`crates/actions`)**:
   - Menambahkan metode/properti kontrak pada `Action`:
     - `target_label(&self) -> String`: label target kanonik (misalnya nama unit layanan atau pengenal target yang jelas) sebagai acuan tampilan dan validasi konfirmasi ketik.
     - `requires_backup(&self) -> bool`: mengembalikan `true` untuk tingkat `>= 3` (atau aksi yang mengubah keadaan/berkas).
     - `requires_typed_confirmation(&self) -> bool`: mengembalikan `true` untuk tingkat 4.
   - `Action::executable_now()` dibuka untuk tingkat 3 dan 4 sejauh aksinya termasuk dalam daftar tertutup `Action` dan melalui alur prasyarat (`dry_run`, cadangan wajib untuk tingkat `>= 3`, dan konfirmasi ketik untuk tingkat 4).

3. **Kontrak `Executor` (`crates/actions`)**:
   - `dry_run(&self, action: &Action) -> DryRun`: diperkaya dengan *diff* atau ringkasan perubahan yang sudah **ter-redaksi** (`core_domain::redact`) dan terpotong pada batas aman (`MAX_DETAIL`), tanpa mengubah target.
   - `backup(&self, action: &Action) -> Result<BackupRecord, BackupError>`: mengambil cadangan sebelum eksekusi (wajib untuk `requires_backup() == true`), menghitung `sha256` isi cadangan, dan mengembalikan metadata untuk disimpan ke `execution_backups`. Kegagalan cadangan menghentikan eksekusi sebelum mutasi dimulai.
   - `rollback(&self, action: &Action, backup: &BackupRecord) -> Outcome`: memulihkan keadaan menggunakan `BackupRecord` (dengan verifikasi hash/integritas sebelum pemulihan bila relevan) melalui perintah `argv` saja lewat `transport::Runner`, serta menghasilkan `Outcome` ter-redaksi.

4. **Server & Validasi Konfirmasi Ketik (`crates/server`)**:
   - Endpoint pratinjau `POST /api/proposals/{id}/dry-run` menjalankan `Executor::dry_run` dan menyimpan `dry_run_json` serta `dry_run_at` pada `proposals`.
   - Endpoint eksekusi `POST /api/proposals/{id}/execute` menerima `typed_confirmation` opsional pada badan JSON. Untuk aksi tingkat 4 (`action.requires_typed_confirmation()`), server **wajib memvalidasi** `typed_confirmation == action.target_label()` sebelum `begin_execution`; jika kosong atau tidak cocok persis, server menolak dengan galat `409` (`typed_confirmation_mismatch` / `typed_confirmation_required`).
   - Endpoint rollback `POST /api/proposals/{id}/rollback` hanya menerima eksekusi yang sudah selesai, memiliki cadangan valid (bila disyaratkan), dan belum pernah di-rollback (`UNIQUE` pada `execution_rollbacks`).
   - Semua langkah (persetujuan di `approvals`, `dry_run`, pembuatan cadangan, eksekusi, dan rollback) tetap terikat pada `args_hash` aksi yang disetujui dan dicatat ke audit berantai-hash dalam transaksi yang sama.

## Belum
- Eksekusi masih **khusus node lokal** dengan bendera eksplisit `infra serve --enable-execution` (mesin target jarak jauh via agen atau SSH menyusul di Sprint 2+).
- Aturan 2 penyetuju berbeda di `approvals` disiapkan di skema, namun penegakan multi-pengguna penuh menunggu mode Cloud/multi-user aktif.
- Penjadwalan dan eksekusi langkah rencana migrasi (`plan approve` / `plan run` / `migration_steps`) serta resolusi DNS + `is_internal_ip` pada runner transport nyata dikerjakan di Sprint 2 (M4 langkah 3b).

## Alternatif
- **Mengizinkan konfirmasi ketik diverifikasi di UI saja**: ditolak; klien bisa dilewati, sehingga server wajib memvalidasi `typed_confirmation == action.target_label()`.
- **Menyimpan cadangan sebagai teks mentah di SQLite**: ditolak untuk data besar atau sensitif; menyimpan metadata + `sha256` di `execution_backups` menjaga DB tetap ringkas dan memungkinkan verifikasi integritas sebelum rollback.
- **Mengizinkan rollback berulang untuk satu eksekusi**: ditolak; batas 1 rollback per eksekusi (`UNIQUE`) mencegah kondisi balapan dan pemulihan berulang yang tidak terprediksi.
