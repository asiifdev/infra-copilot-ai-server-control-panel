# 0042. Kelengkapan kolektor read-only (`vhost`, `cert`, `db`, `launchd`) dan seluruh 11 aturan kesetaraan M1 (`PAR-001`..`PAR-011`)

- Tanggal: 2026-11-27
- Status: diterima (melanjutkan 0013, 0014, 0015, dan 0041; menuntaskan daftar aturan `docs/plan.md` §6)
- Pengambil keputusan: pemilik proyek

## Konteks
Sebelumnya, `crates/collectors` baru memiliki kolektor `listeners`, `processes`, `cron`, `systemd`, `docker`, dan `pm2`, serta `crates/parity` baru mengimplementasikan 7 dari 11 aturan kesetaraan awal (`PAR-001`, `PAR-002`, `PAR-006`, `PAR-007`, `PAR-008`, `PAR-009`, `PAR-010`). Empat aturan penting dari pengalaman migrasi nyata di `docs/plan.md` §6 belum tercakup:
- `PAR-003`: producer dan consumer antrean memakai Redis/DB berbeda;
- `PAR-004`: hostname DNS / vhost tidak mengarah ke target (atau di akun berbeda);
- `PAR-005`: database bernama sama dengan jumlah baris berbeda atau kosong di sumber;
- `PAR-011`: pemeriksaan/uji fungsional dari sudut pandang proses, bukan sekadar status HTTP 200.

Selain itu, kolektor untuk virtual host (`vhost`), sertifikat TLS (`cert`), database (`db`), dan pengelola layanan macOS (`launchd`) diperlukan agar inventaris yang dikumpulkan dapat memberi masukan lengkap ke seluruh 11 aturan kesetaraan sekaligus tetap berjalan aman lintas platform (0041).

## Keputusan
1. **Penambahan kolektor read-only berbasis `RootFs` (`crates/collectors`)**:
   - **`vhost`**: membaca berkas konfigurasi virtual host web server (misalnya nginx) melalui `RootFs` (read-only) untuk mengekstrak nama domain/server, root aplikasi, dan target *upstream* tanpa menyimpan rahasia.
   - **`cert`**: membaca metadata sertifikat TLS publik yang dirujuk melalui `RootFs` (subjek/SAN dan masa berlaku), **tanpa pernah membaca atau menyimpan kunci privat**.
   - **`db`**: mengumpulkan metadata database (nama database, mesin, dan statistik/jumlah baris yang tersedia secara read-only) tanpa mengekstrak isi baris data atau kredensial.
   - **`launchd`**: membaca definisi layanan `launchd` (`.plist`) melalui `RootFs` untuk mendukung pengumpulan pada macOS.
   - **Pagar lintas platform & fixture anonim**: seluruh kolektor baru tetap melewati `RootFs` (dengan pagar `#[cfg(unix)]` / `#[cfg(target_os = ...)]` / `#[cfg(not(unix))]` agar kompilasi Windows di 0041 tetap utuh) dan diuji memakai **fixture anonim** (`example.test`, `203.0.113.x`, nama generik; tanpa data pribadi/produksi). Kolektor tetap tidak pernah membaca `/proc/<pid>/environ` atau menyimpan nilai rahasia.

2. **Perluasan `Kind` & Database (migrasi `0013_inventory_kinds.sql`, skema versi 13)**:
   - `inventory::Kind` diperluas dengan varian `Vhost`, `Cert`, `Database`, `Dns`, dan `Check`.
   - Migrasi `0013_inventory_kinds.sql` memperbarui batasan `CHECK` pada `inventory_items.kind` dan `findings.subject_kind` agar menerima kelima jenis baru tersebut, serta membuat ulang pemicu indeks FTS5 (`inventory_items_index`/`unindex` dan `findings_index`/`unindex`) secara aman.

3. **Kelengkapan seluruh 11 aturan kesetaraan (`PAR-001`..`PAR-011`) di `crates/parity`**:
   - Menambahkan **`PAR-003`**, **`PAR-004`**, **`PAR-005`**, dan **`PAR-011`** ke registri default `all_rules()` di `crates/parity`, sehingga seluruh 11 aturan awal di `docs/plan.md` §6 aktif secara bawaan saat `infra compare` maupun pemeriksaan gerbang rencana migrasi.
   - Setiap aturan tetap mengimplementasikan `inventory::ParityRule` secara murni dan deterministik (hanya bergantung pada `crates/inventory`), menggunakan `cap_if_partial` bila inventaris sumber/target berstatus `partial`, serta mengambil seluruh teks judul, penjelasan, dan saran dari katalog `i18n` (`parity.parNNN.title|explanation|advice` dalam bahasa Inggris dan Indonesia).
   - Setiap aturan baru disertai fixture pemicu temuan dan fixture bersih (tanpa positif palsu) di `crates/parity/tests/fixtures/`.

## Belum
- Aturan tambahan khusus perbedaan sistem operasi/arsitektur (`docs/design.md` §14.2) di luar daftar 11 aturan awal dapat ditambahkan sebagai aturan lanjutan di tahap berikutnya.
- Uji end-to-end kolektor `db` / `launchd` terhadap berbagai variasi distribusi server nyata di luar CI tetap bergantung pada lingkungan host tempat `infra collect` dijalankan.

## Alternatif
- **Menunda `PAR-003`/`004`/`005`/`011` hingga M5**: ditolak; justru saat migrasi server (`M3`/`M4`) celah antrean, DNS/vhost, isi database, dan uji fungsional paling sering menjadi penyebab kegagalan setelah cutover.
- **Membiarkan aturan baru tidak aktif di `all_rules()` secara bawaan**: ditolak; pengguna dan gerbang `plan` membutuhkan evaluasi otomatis seluruh 11 aturan tanpa konfigurasi tambahan.
