# 0043. Eksekutor jarak jauh (`SshExecutor` & agen *pull-only*) serta panel dasar M5 (log layanan & situs lokal)

- Tanggal: 2026-11-27
- Status: diterima (melanjutkan M4 jarak jauh dan fondasi M5 di atas 0037, 0038, dan 0039)
- Pengambil keputusan: pemilik proyek

## Konteks
Sampai dengan 0038 dan 0039, eksekusi Usulan (`POST /api/proposals/{id}/execute`) baru mendukung node lokal (`local`), sedangkan node jarak jauh dengan akses `ssh` atau `agent` masih ditolak dengan `unsupported_target`. Sesuai arsitektur keamanan proyek (`docs/plan.md` §7 dan `docs/design.md`), node jarak jauh membutuhkan:
1. Pelaksana SSH tanpa *shell interpolation* yang memakai ulang `transport::Runner`;
2. Pelaksana agen keluar-saja (*pull-only*) sehingga server pengguna tidak perlu membuka port masuk atau menyerahkan akses SSH root ke Cloud; serta
3. Fondasi panel pengelola server (M5) berupa penampil log layanan baca-saja yang aman dan kontrak manajemen situs lokal tervalidasi ketat.

## Keputusan
1. **`SshExecutor` berbasis `transport::Runner` (`crates/actions`)**:
   - Menambahkan `SshExecutor` yang menerjemahkan aksi tertutup (`Action`) menjadi pemanggilan `ssh` berbasis **argv saja** (tanpa *shell interpolation*) lewat `transport::Runner`.
   - Memaksa `BatchMode=yes` agar `ssh` tidak pernah menggantung menunggu masukan interaktif.
   - Host, port, pengguna, dan nama layanan tetap melewati validasi ketat sebelum masuk argumen, dan seluruh keluaran target dianggap tak tepercaya (diredaksi dan dipotong pada batas aman).

2. **Agen keluar-saja (*pull-only*) & Database (migrasi `0014_agent_and_m5.sql`, skema versi 14)**:
   - **`agent_tokens`**: menyimpan token autentikasi agen per node dalam bentuk **hash SHA-256** (`org_id`, `node_id`, `token_hash`, masa berlaku/pencabutan), tidak pernah menyimpan token mentah.
   - **`agent_jobs`**: antrean pekerjaan eksekusi yang terikat `(org_id, proposal_id, execution_id, node_id)` dan membawa aksi tertutup serta `args_hash` yang telah disetujui. Semua kueri dan kunci asing terisolasi ketat per `org_id`.
   - **Endpoint agen (`crates/server`)**:
     - `POST /api/agents/poll`: agen terautentikasi menarik pekerjaan `queued` milik node-nya sendiri dan mengklaimnya secara atomik.
     - `POST /api/agents/result`: agen melaporkan status hasil eksekusi (ter-redaksi dan dibatasi ukuran), memperbarui `agent_jobs`, `executions`, dan `proposals` dalam satu transaksi bersama entri audit berantai-hash.

3. **Perluasan `POST /api/proposals/{id}/execute` untuk node `ssh` dan `agent`**:
   - Selain node `local`, endpoint eksekusi kini menerima node dengan akses `ssh` (dijalankan melalui `SshExecutor`) dan `agent` (mengantrekan `agent_jobs` untuk ditarik oleh agen node terkait).
   - Seluruh pengaman tetap berlaku tanpa pengecualian: bendera `infra serve --enable-execution`, validasi `args_hash`, kesesuaian `tool` dan `tier`, bukti persetujuan di `approvals` (serta konfirmasi ketik pada tingkat 4), cadangan wajib pada tingkat `>= 3`, satu eksekusi per Usulan, dan pencatatan ke `audit_log` berantai-hash.

4. **Panel dasar M5 (log layanan & kontrak situs lokal `.test`)**:
   - **Penampil log layanan read-only (`GET /api/nodes/{id}/logs`)**: membaca log dari daftar layanan yang diizinkan secara baca-saja; keluaran selalu diredaksi (`core_domain::redact`), karakter kendali dibersihkan, dan dipotong ke batas ukuran aman sebelum dikirim ke UI.
   - **Kontrak manajemen situs lokal `.test`**: mendefinisikan model dan validasi ketat untuk pengelolaan situs lokal berakhiran `.test` (nama domain/situs, jalur root, dan konfigurasi terikat allow-list tanpa *shell* atau jalur arbitrer).

## Belum
- Pengelolaan penuh situs produksi (SSL ACME/LetsEncrypt, multi-versi PHP-FPM, manajemen database M5 penuh) serta pemasangan biner agen terpisah pada server jarak jauh masih menunggu tahap lanjutan.
- Penjadwalan dan eksekusi langkah DNS/tunnel Cloudflare tetap ditandai sebagai Usulan tingkat 3 pada tahap berikutnya.

## Alternatif
- **Menerima koneksi masuk (*push*) ke agen atau menyimpan kunci SSH root di database Cloud secara bawaan**: ditolak; model *pull-only* menghindari pembukaan port masuk di server pengguna (selaras dengan `docs/plan.md` §7).
- **Mengirim perintah shell mentah melalui SSH atau `agent_jobs`**: ditolak; bertentangan dengan aturan inti 1 (daftar aksi tertutup dan argumen terstruktur).
