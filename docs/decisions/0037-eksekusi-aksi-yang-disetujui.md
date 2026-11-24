# 0037. Menjalankan aksi yang sudah disetujui (M4, langkah 2: tingkat 2, mesin lokal)

- Tanggal: 2026-11-24
- Status: diterima (pemilik: "lanjut ikuti urutan rencana produk"; membangun di atas 0034)
- Pengambil keputusan: pemilik proyek

## Konteks
0034 membuat manusia dapat menyetujui Usulan, tetapi tidak ada yang bisa menjalankannya. Aturan inti: agen hanya menjalankan **daftar aksi yang diizinkan**, tingkat ditentukan kode (bukan AI atau pengguna), persetujuan mengikat pada **hash isi Usulan**, tingkat 3-4 tidak pernah otomatis (design.md bagian 9, skill `ai-tool-tiers`).

## Keputusan
1. **Crate `actions`**: `Action` adalah data tertutup (sekarang satu: `RestartService{manager=systemd, name}`), diparse dari JSON ketat (kunci tepat, tipe tepat), nama layanan divalidasi (`A-Za-z0-9_.@-`, tidak diawali `-` atau `.`, maksimal 100), punya **satu bentuk kanonik** dan hash SHA-256-nya. Tingkat ditetapkan oleh kode (`restart_service` = 2). `executable_now()` menolak tingkat 3-4 sampai dry-run + cadangan, konfirmasi ketik, dan rollback ada.
2. **`Executor`** mengubah aksi menjadi perintah **argv saja** lewat `transport::Runner` (dipakai ulang: `ProcessRunner` di produksi, `FakeRunner` di tes; tidak ada tes yang menyentuh layanan nyata). Restart = `systemctl restart <nama>`, verifikasi = `systemctl is-active <nama>` harus `active`. Keluaran target dianggap tak tepercaya: diredaksi, karakter kendali diganti, dipotong 1000 karakter. Restart tidak punya undo (dinyatakan di dampak).
3. **Database (migrasi 0010, skema versi 10)**: kolom `proposals.action_json` dan `node_id` (diisi sekali saat dibuat; pemicu menolak perubahan `action_json`, `node_id`, `args_hash`, `tool`, `tier` sesudahnya) dan tabel `executions` dengan `UNIQUE (org_id, proposal_id)`: satu Usulan paling banyak satu eksekusi, jadi eksekusi gagal **tidak pernah diulang sendiri** dan dua permintaan serentak tidak bisa sama-sama jalan. `begin_execution` hanya untuk Usulan `approved`, belum kedaluwarsa, punya aksi; `finish_execution` mengubah Usulan menjadi `executed`/`failed`; keduanya dicatat di audit berantai-hash dalam transaksi yang sama (`proposal.execute.start`, `proposal.execute.finish`). Eksekusi yang masih `running` saat server mulai dianggap `failed` (`interrupted`).
4. **Server**: `POST /api/proposals/{id}/execute`. Sebelum apa pun dimulai diperiksa: eksekusi diaktifkan, Usulan disetujui dan belum kedaluwarsa, aksi tersimpan valid dan **hash, tool, dan tingkatnya sama dengan yang disetujui**, dan targetnya node `local` dengan akses `local` (agen/SSH belum ada: `unsupported_target`). Kode galat 409: `execution_disabled`, `proposal_not_approved`, `proposal_no_action`, `proposal_already_run`, `proposal_expired`, `action_invalid`, `unsupported_target`. Daftar Usulan kini memuat `action`, `node`, `execution`, dan kalimat dampak dirender dalam bahasa pembaca (yang tersimpan adalah kunci katalog).
5. **Eksekusi MATI secara bawaan.** Hanya `infra serve --enable-execution` yang menyalakannya (pesan peringatan dicetak saat mulai); `GET /api/session` memuat `execution_enabled`. Alasan: mesin pengembangan bisa menjalankan layanan produksi; tanpa bendera itu menyetujui hanya mencatat.
6. **Alat AI `propose_restart_service`** (node, service, reason): tingkat alat **1** (ia hanya menulis Usulan `pending`, jadi langit-langit tingkat 1 untuk asisten tetap utuh), sedangkan Usulan yang dihasilkan bertingkat 2 dan membawa aksi serta hash-nya. Tidak ada alat yang bisa menyetujui atau menjalankan. Registri asisten kini 15 alat.
7. Menjalankan adalah **tindakan terpisah** dari menyetujui, dilakukan manusia di UI (tombol Jalankan dengan dialog yang menyebut persis apa yang akan terjadi).

## Belum
Tingkat 3-4 (dry-run + cadangan otomatis, konfirmasi ketik, rollback, dua penyetuju di Cloud); pelaksana untuk node jarak jauh (agen dan SSH); aksi selain restart layanan (ubah `.env`, DNS, restore DB); pemblokiran endpoint yang sudah ada di 0033 belum relevan sampai ada pelaksana bersama; tabel `approvals` terpisah (audit berantai-hash cukup untuk satu pengguna lokal). Hasil dari mesin nyata belum diuji: `systemctl` hanya diuji lewat `FakeRunner`.

## Alternatif
Eksekusi otomatis setelah setuju: ditolak (memisahkan dua keputusan manusia, dan melindungi dari klik tak sengaja). Menyimpan perintah shell di Usulan: ditolak (aturan inti 1).
