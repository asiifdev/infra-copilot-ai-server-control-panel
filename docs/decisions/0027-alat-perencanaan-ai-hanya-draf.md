# 0027. Alat perencanaan AI (M3-c): model hanya membuat draf

- Tanggal: 2026-11-20
- Status: diterima
- Pengambil keputusan: pemilik proyek (arah M3 di 0024 dan 0025); rincian oleh pelaksana track M

## Konteks
Model rencana (0024, crate `plan`) dan transport (0025) sudah ada, tetapi asisten AI (0023) belum bisa memakainya. M3-c memberi asisten alat untuk menyusun dan membaca rencana migrasi. Aturan yang tidak boleh dilanggar tetap: tidak ada alat di atas tingkat 1, aksi yang mengubah selalu lewat persetujuan manusia (aturan 3), teks dari server, alat, dan model adalah data (aturan 4), rahasia tidak masuk prompt (aturan 2), setiap query memakai `org_id` (aturan 10).

## Keputusan
1. **Dua seam baru di modul `agent::plans`**, terpisah dari `ReadStore` dan `ProposalSink`, dipasang lewat `ToolContext::with_plans(store, sink)` (bawaan: `NoPlans`, tanpa rencana dan tanpa cara menyimpan).
   - `PlanStore` (baca): daftar transport (hanya nama, jenis, `verified_at`; tidak pernah konfigurasi atau id rahasia), pencarian transport dan node menurut **nama** di dalam org pemanggil, daftar rencana, satu rencana beserta langkah (dibangun ulang dan divalidasi lewat `Plan::from_stored`; data tersimpan tidak dipercaya), dan temuan perbandingan terbaru antara dua node bernama, termasuk mana yang sudah diterima orang.
   - `PlanSink` (tulis): satu metode, `create_draft(org, &Plan)`. Tidak ada metode setujui, jadwalkan, jalankan, ubah, atau hapus, sehingga tidak ada alat atau model yang bisa mencapainya.
   - Setiap metode menerima `OrgId` eksplisit; implementasi database `DbPlanStore` dan `DbPlanSink`.
2. **Lima alat** (modul `plan_tools`, didaftarkan satu baris dari `assistant_registry`): tingkat 1 `draft_migration_plan`; tingkat 0 `show_plan`, `list_plans`, `check_plan_gates`, `list_transports`. Dampak alat tingkat 1 disimpan sebagai kunci katalog (`assistant.plan.impact`).
3. **Hanya draf.** Baris rencana yang ditulis alat berstatus `draft`, selalu (nilai bawaan kolom; tidak ada argumen yang bisa mengubahnya). Skema argumen tertutup menolak `org_id`, `status`, `spec`, `steps`, dan nama lain yang tidak dideklarasikan. Urutan langkah dibuat `plan::draft_plan` dari inventaris terbaru kedua node, bukan oleh model; langkah mematikan server lama (tingkat 4) tetap `pending` dan terkunci oleh gerbang di crate `plan` serta pemicu di migrasi 0008.
4. **Gerbang tidak bisa dilewati lewat data basi.** Perbandingan dipakai hanya bila kedua inventarisnya masih yang terbaru bagi node masing-masing; bila tidak ada atau basi, laporan gerbang ditandai tidak lengkap sehingga `cutover_allowed` salah. Inventaris parsial juga menandai tidak lengkap.
5. **Nama, bukan id.** Node dan transport dicari menurut nama di dalam org; nama milik org lain berperilaku sama persis dengan nama yang tidak ada (teks galat identik; diuji). Node tanpa inventaris, sumber sama dengan tujuan, transport belum terverifikasi, dan transport tidak disebut padahal tidak ada tepat satu yang terverifikasi, semuanya ditolak dengan galat berkode dan teks berbahasa sesuai locale (`ToolError::Explained`, varian baru; teks dari katalog `assistant.plan.err_*`), bukan rencana karangan.
6. **Batas.** Maksimal 50 draf per org (`MAX_DRAFT_PLANS`), nama rencana unik per org tanpa membedakan huruf besar-kecil. Keduanya diperiksa **di dalam transaksi tulis yang sama** dengan penyisipan (`db::create_draft_plan`, `BEGIN IMMEDIATE`), bukan pra-periksa terpisah, jadi pemanggilan kembar paralel hanya membuat satu rencana (diuji dengan empat utas pada berkas database). Nama dan teks bebas diredaksi, karakter kontrol diganti spasi, dipangkas, lalu panjangnya diperiksa **setelah** redaksi (`clean_text`, dipakai bersama `propose_fix`).
7. **Penerimaan temuan hanya dari orang.** Tidak ada alat yang menandai temuan diterima. Status `accepted` pada baris temuan hanya dapat berasal dari tindakan orang di panel; alasan dalam laporan gerbang berupa teks tetap dari katalog (`assistant.plan.accepted_reason`).
8. **Penulisan ulang yang dibutuhkan.** `db`: `create_draft_plan`, `find_plan_by_name`, `latest_comparison_between` (modul `plan_support`); fungsi bersama `plans.rs` dijadikan `pub(crate)`. `agent::testing::contract_suite_for` menjalankan rangkaian kontrak untuk alat bawaan yang tipenya privat, di atas konteks buatan pemanggil.

Tidak ada dependensi pihak ketiga baru; `agent` hanya mendapat dependensi jalur ke crate `plan`. Tidak ada migrasi.

## Alasan
- Alat yang hanya bisa menyisipkan draf membuat jaminan "AI tidak menyetujui apa pun" benar secara tipe, bukan janji prompt.
- Gerbang yang menolak bukti basi menutup celah paling mudah: model menyusun rencana tepat setelah inventaris baru, sebelum perbandingan dijalankan, lalu gerbang tampak terbuka karena daftar temuan kosong.
- Pemeriksaan batas di dalam transaksi menghindari balapan antara "cek lalu sisip".

## Alternatif yang dipertimbangkan
- **Indeks unik `(org_id, lower(name))` di skema.** Lebih kuat, tetapi butuh migrasi, dan migrasi pada putaran ini dipegang track E. Dicatat sebagai usulan lanjutan; sementara itu atomisitas berasal dari transaksi `IMMEDIATE` (SQLite mengizinkan satu penulis pada satu waktu).
- **Alat `approve_plan` atau `run_plan` untuk tingkat 2 ke atas.** Ditolak: aturan 3 dan skill `ai-tool-tiers`. Persetujuan datang di M4 dan hanya dari orang di panel.
- **Menyimpan alasan penerimaan temuan di database.** Ditunda: butuh kolom baru; sementara laporan memakai teks tetap.
- **Satu seam gabungan baca dan tulis.** Ditolak: memisahkan membuat tulis dapat diaudit di satu tempat dan membiarkan konteks baca-saja tetap benar.

## Yang belum diuji
- Alat belum dipakai terhadap penyedia model nyata (tanpa kunci); putaran diuji dengan `ScriptedProvider`.
- `db::create_draft_plan` memakai `Transaction::new_unchecked` pada referensi bersama; bila sambungan sudah berada dalam transaksi, `BEGIN IMMEDIATE` gagal dan alat melapor `plans_unavailable`. Putaran asisten tidak memegang transaksi saat alat berjalan.

## Langkah berikutnya untuk orang
Draf yang disimpan harus ditinjau seseorang: baca lewat `show_plan` atau panel, periksa daftar yang dikecualikan dari backup dan gerbang, terima atau perbaiki temuan kritis. Persetujuan, penjadwalan, dan eksekusi belum ada; semuanya datang di M4 dan hanya bisa dilakukan orang. `infra plan` dan `infra transport` di CLI dikerjakan track lain.

## Dampak
- CHANGELOG, README (en/id), NEXT-STEPS, dan bagian Struktur CLAUDE.md perlu mencatat lima alat dan seam baru (dikerjakan orkestrator).
