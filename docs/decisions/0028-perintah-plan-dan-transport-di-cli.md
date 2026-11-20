# 0028. Perintah `infra plan` dan `infra transport` di CLI (M3-d)

- Tanggal: 2026-11-20
- Status: diterima
- Pengambil keputusan: pemilik proyek (arah M3 di 0024, 0025, 0027); rincian oleh pelaksana track CLI

## Konteks
Model rencana (0024), transport (0025), dan alat perencanaan AI (0027) sudah ada, tetapi orang belum bisa memakainya dari baris perintah. M3-d menambah `infra plan ...` dan `infra transport ...` dengan gaya yang sama seperti perintah lain: penguraian argumen buatan sendiri (tanpa crate pengurai argumen), satu modul per perintah, semua teks lewat katalog i18n (EN dan ID), dan kode keluar yang sama (0 berhasil, 1 temuan kritis, 2 penggunaan/konfigurasi/IO, 3 asisten atau penyedia gagal).

Aturan yang tidak boleh dilanggar tetap: rahasia tidak pernah di argumen, stdout, stderr, log, atau database tanpa segel (aturan 2); teks dari server dan alat adalah data (aturan 4); setiap query memakai `org_id` (aturan 10); mesin pengembangan menjalankan layanan produksi, jadi tidak ada uji yang boleh menjalankan rclone, rsync, scp, ssh, atau menyentuh jaringan.

## Keputusan

### Rencana
1. **Satu jalur kode dengan alat AI.** Logika menyusun draf dan membaca gerbang dipindahkan dari `plan_tools.rs` ke modul baru `agent::plan_flow`: `draft_migration_plan(&DraftContext, &DraftRequest)`, `collect_evidence`, `gates_for_stored`, dan tipe galat `DraftError`. Alat `draft_migration_plan`, `show_plan`, dan `check_plan_gates` sekarang memanggil fungsi yang sama dan memetakan `DraftError` ke kode dan kunci katalog yang sama seperti sebelumnya (`DraftError::into_tool_error`); perilaku dan uji alat tidak berubah. CLI memetakan `DraftError` ke kunci `cli.plan.*` sendiri, karena teks alat menyebut `list_nodes` dan `list_transports` yang tidak ada di CLI.
2. **Perintah**: `plan draft --source NODE --target NODE [--transport NAMA] [--backup-location source|local|intermediary] [--cutover tunnel|dns] [--name NAMA]`, `plan list`, `plan show NAMA` (dirender `plan::render_plan` dalam bahasa `--lang`), `plan gates NAMA`. Semua memakai nama, bukan id, di dalam org yang dipilih (`--org`).
3. **Hanya draf.** Tidak ada perintah setujui, jadwalkan, atau jalankan; `plan approve|run|schedule` adalah galat penggunaan. Baris yang ditulis berstatus `draft` (tulis lewat `PlanSink` yang hanya punya `create_draft`). Persetujuan dan eksekusi milik M4 dan hanya untuk orang di panel.
4. **Kode keluar `plan gates`.** 1 bila `!cutover_allowed()`: ada temuan kritis yang masih terbuka, atau buktinya hilang, basi, atau tidak lengkap (perbandingan tidak mencakup inventaris terbaru kedua node, atau salah satu inventaris parsial). Alasannya sama dengan 0027 butir 4: daftar temuan kosong tanpa perbandingan terkini bukan bukti, jadi skrip yang membaca kode keluar tidak boleh mengira gerbang terbuka. Ini sedikit lebih ketat dari "hanya temuan kritis" pada `compare`; `plan draft`, `list`, dan `show` keluar 0 bila berhasil.
5. **Galat** (kode 2, teks sesuai bahasa, tidak ada yang disimpan): node tidak ada (nama milik org lain sama persis dengan nama yang tidak ada), node tanpa inventaris, sumber sama dengan tujuan, transport tidak ada, transport belum terverifikasi, transport tidak disebut padahal tidak ada tepat satu yang terverifikasi, nama rencana kosong atau terlalu panjang, nama sudah dipakai (tanpa membedakan huruf besar-kecil), batas 50 draf.

### Transport
6. **Perintah**: `transport add --name N --kind rclone|rsync|scp|ssh_pipe ...`, `list`, `show NAMA`, `remove NAMA`, `check NAMA [--run]`. Bendera per jenis: rclone memakai `--remote-kind s3|b2|drive|sftp|dropbox|onedrive|webdav`, `--remote`, `--path`, `--options kunci=nilai,kunci=nilai`; rsync, scp, dan ssh_pipe memakai `--host`, `--user`, `--port`, `--path`. Semua nilai melewati konstruktor crate `transport` (`RcloneSpec::new`, `SshTarget::new`, `validate::transport_name`). Bendera milik jenis lain adalah galat penggunaan; jenis `managed` ditolak. Opsi non-rahasia dipisah koma karena pengurai tidak menerima bendera berulang; nilai yang memuat koma tidak didukung (galat penggunaan).
7. **Kredensial hanya dari stdin.** `--credentials-stdin` membaca satu `nama=nilai` per baris (maksimum 64 KiB, UTF-8). Tidak ada bendera kredensial di argumen: `--secret-access-key`, `--token`, dan sejenisnya adalah bendera tak dikenal, dan pesan penggunaan hanya memuat nama bendera, dipotong sebelum `=`, tidak pernah nilainya atau kata liar sesudah sakelar. Galat stdin menyebut nomor baris, tidak isinya. Nama diperiksa terhadap daftar izin backend dengan membangun rencana `Verify` (tanpa menjalankan apa pun) sebelum menyimpan, jadi nama tak dikenal atau opsi ganda ditolak saat `add`, bukan saat pemeriksaan pertama. Jenis SSH tidak menerima kredensial (0025 butir 5).
8. **Penyimpanan.** Kredensial disegel `vault::Vault` (kunci utama dari `NODIVARA_MASTER_KEY`, saran berbahasa lokal bila kosong atau cacat) lewat `transport::store::save`. Tanpa kredensial tidak ada yang disegel, jadi kunci utama tidak diperlukan dan baris disimpan langsung lewat `db::create_transport`. Nama ganda diperiksa lebih dulu (`StoreError::Database` tidak membedakan sebab).
9. **`show` dan `list` minim.** Hanya nama, jenis, `verified_at`, dan pemeriksaan terakhir (JSON angka, boolean, dan kode masalah tetap). Tidak pernah konfigurasi, id rahasia, atau nilai kredensial. `remove` menghapus baris beserta rahasia tersegelnya (`store::delete`).

### Mengapa `check` secara bawaan adalah uji coba
10. **`transport check NAMA` tidak menjalankan apa pun.** Ia menjalankan `Wizard` (langkah buat/uji/kuota/kecepatan) terhadap `DryRunner` yang hanya mencetak setiap rencana perintah dan menjawab sukses, sehingga yang ditampilkan adalah urutan perintah yang persis sama dengan yang akan dijalankan. Nilai rahasia tampil `***`. Hanya `--run` yang menjalankannya, lewat `transport::ProcessRunner` di mesin ini.
    - Alasan: pemeriksaan sungguhan menghubungi layanan luar (rclone mengunggah 4 MiB uji kecepatan, ssh membuka sambungan), memakai kredensial yang dibuka dari segel, dan bisa tersandung batas kuota atau memicu peringatan di akun orang. Itu harus keputusan yang diucapkan, bukan efek samping dari mengetik nama transport. Uji coba juga membuat orang bisa memeriksa apa yang akan terjadi sebelum percaya (aturan 1: daftar aksi yang diizinkan, bukan shell bebas), dan memungkinkan semua uji berjalan tanpa proses.
    - Uji coba tetap membuka kredensial tersegel (perlu kunci utama) agar nama variabel lingkungan yang akan dipakai dapat ditampilkan; nilainya tidak pernah dicetak.
    - `--run` tidak pernah memasang apa pun. Bila programnya tidak ada (`RunError::NotInstalled`), orang melihat saran `transport.not_installed.*` ditambah kalimat yang menawarkan bantuan pemasangan "hanya bila Anda meminta dan menyetujui" dan menegaskan perintah ini tidak memasang.
    - Remote OAuth tanpa token: wizard berhenti di langkah otorisasi. Uji coba mengatakannya dan keluar 0; `--run` mencetak petunjuk `rclone authorize` dan keluar 2 (belum ada perintah untuk memperbarui kredensial; cara sementara: hapus lalu tambahkan lagi dengan baris `token=...`).
    - Jenis rsync, scp, dan ssh_pipe tidak punya wizard: dua perintah (`Probe`, `Verify`).
11. **Hasil.** Berhasil di bawah `--run`: `verified_at` dan `last_check` dicatat (`db::record_transport_check`), kode 0. Gagal di bawah `--run`: saran berbahasa lokal di stderr (kuota penuh, token kedaluwarsa, remote salah, jaringan, timeout, rclone tidak terpasang, tak dikenal; potongan pesan alat sudah diredaksi, nilai kredensial dihapus), kode 2, `last_check` dicatat dengan `{"ok":false,"problem":"<kode>"}` dan `verified_at` tidak berubah. Uji coba tidak mencatat apa pun.
12. **Rencana perintah dicetak oleh CLI, bukan `Display` crate transport.** `Display` melewatkan seluruh baris lewat penyamar (redactor) yang juga merusak bendera tak berbahaya seperti `--ask-password=false` (jadi `[REDACTED]`). CLI mencetak sendiri: lingkungan biasa, rahasia `***`, program, dan argumen dikutip; rahasia hanya ada di lingkungan dan stdin rencana dan stdin tidak pernah dicetak. Sebagai pagar kedua, nilai kredensial yang dikenal diganti `***` di baris itu.
13. **Celah uji.** `cli::Env` mendapat `with_runner(Arc<dyn Runner>)`; tanpa itu `ProcessRunner` (batas waktu 120 detik per perintah, 1 MiB keluaran per aliran) dipakai. Semua uji memasang `FakeRunner` sehingga satu bug pun tidak dapat menjalankan proses; uji uji-coba menegaskan runner tidak menerima panggilan.

## Alasan
- Satu jalur kode untuk alat AI dan CLI membuat perilaku yang sama (urutan langkah, gerbang, batas) tanpa salinan, dan alat tidak berubah.
- Kredensial dari stdin dan bendera tak dikenal untuk yang lain menutup jalur argumen (terlihat di `ps` dan riwayat shell) secara struktural, bukan lewat larangan.
- Uji coba sebagai bawaan membuat tindakan yang menyentuh dunia luar selalu sengaja.

## Alternatif yang dipertimbangkan
- **`check` menjalankan bawaan, `--dry-run` untuk mencetak.** Ditolak: kelalaian mengetik menjadi efek di luar mesin.
- **Kredensial lewat variabel lingkungan** (seperti `NODIVARA_PROVIDER_KEY`). Ditolak untuk transport: ada banyak nilai bernama per backend; stdin `nama=nilai` memuat semuanya.
- **Menjalankan `Wizard` dengan `FakeRunner` dari pustaka untuk uji coba.** Ditolak: `FakeRunner` merekam argumen dan nilai rahasia asli; `DryRunner` hanya menyimpan baris bertopeng.
- **Mengubah `Display` untuk `CommandPlan`.** Ditolak: diuji kontrak 0025 dan dipakai untuk log; perbaikan penyamar di sana adalah pekerjaan terpisah.
- **Kode keluar 1 hanya untuk temuan kritis (seperti `compare`).** Ditolak: lihat butir 4.

## Yang belum diuji atau belum ada
- rclone, rsync, scp, dan ssh nyata tidak pernah dijalankan; seluruh jalur `--run` diuji dengan `FakeRunner`. Frasa galat rclone masih heuristik (lihat 0025).
- Belum ada perintah untuk memperbarui kredensial atau token transport, atau mengubah transport; sementara hapus lalu tambah lagi.
- `transport remove` tidak memeriksa apakah sebuah draf rencana masih menunjuk ke transport itu (rencana menyimpan id transport tanpa kunci asing); rencana yang menunjuk transport yang sudah dihapus akan gagal divalidasi saat dijalankan di M4.
- Transport yang pernah lolos tetap `verified_at` terisi walau pemeriksaan berikutnya gagal (hanya `last_check` yang berubah).

## Dampak
- Crate `cli`: modul `plans` dan `transports`, kunci katalog `cli.plan.*` dan `cli.transport.*` (EN dan ID), ketergantungan jalur ke `plan` dan `transport`, `Env::with_runner`.
- Crate `agent`: modul `plan_flow` dan ekspor publiknya; alat tidak berubah perilakunya.
- CHANGELOG, README (en/id), NEXT-STEPS, dan bagian Struktur CLAUDE.md perlu mencatatnya (dikerjakan orkestrator).
