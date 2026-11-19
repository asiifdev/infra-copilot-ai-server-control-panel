# 0013. Model inventaris dan kontrak `Collector`/`ParityRule`

- Tanggal: 2026-11-19
- Status: diterima (rencana M1-a disetujui pemilik, 2026-11-19)
- Pengambil keputusan: pemilik proyek

## Konteks
M1 memerlukan model inventaris, kontrak kolektor, dan kontrak aturan kesetaraan. Sumber kebenaran: `docs/design.md` bagian 3.3 (status inventaris, temuan), 4 (format dan jenis item), 5 (aturan), 12.4 (kontrak adapter), 14 (i18n), skill `parity-rules`, dan `docs/plan.md` bagian 6 (11 aturan awal). Pemilik meminta keluaran sesuai dokumen-dokumen itu.

## Keputusan
Crate baru `inventory` (bergantung pada `serde`, `serde_json`, dan `core-domain`) berisi:
- `Inventory`: `schema_version`, `taken_at`, `collector_version`, `platform {os, arch}`, `status` (`complete|partial`, dari 3.3), `warnings`, dan `items` terurut serta unik per `(kind, key)`. JSON divalidasi saat dibaca (versi skema, duplikat, kunci).
- `Item {kind, key, data, seen_via}` dengan format kunci dan `data` persis 4.1: `listener` (`tcp:<port>`), `process` (`<manager>:<nama>`, `proc:<nama>` tanpa supervisor), `cron` (`<scope>:<owner>:<jadwal>:<petunjuk>`). `seen_via`: `proc|systemd|docker|config`.
- `Finding {rule_id, severity (critical|important|info), subject, evidence, args}`. Teks judul, penjelasan, dan saran **tidak** disimpan di temuan; dicari di katalog i18n (14.1).
- `Collector {kind, run}` dan `ParityRule {id, version, severity, category, fix_tier, evaluate(source, target, ctx)}` sesuai 5, 12.4, dan skill (id stabil, versi, bobot, kategori, tingkat perbaikan yang boleh diusulkan AI). `RuleContext` kosong dan `non_exhaustive`, disediakan agar tanda tangan aturan tidak berubah saat konteks (kepemilikan DNS, uji fungsional) ditambahkan.

## Penyimpangan yang disengaja
- Kontrak ada di `inventory`, bukan di `core-domain` (konvensi `rust-conventions` menyebut `core`), karena `core-domain` sengaja tanpa dependensi luar dan model ini memerlukan serde.
- `warnings` dan `status: partial` ditambahkan: inventaris parsial tidak boleh dianggap bukti bahwa sesuatu tidak ada (prinsip "kejujuran" di plan 13.4 dan skill).
- Item `platform` (14.2) belum ada; `platform {os, arch}` di kepala inventaris mewakili `nodes.os/arch`. Item `platform` ditambah saat aturan selisih OS/arsitektur dikerjakan.
- Penomoran aturan mengikuti urutan daftar di plan bagian 6: aturan ke-n = `PAR-00n`.

## M1-a: yang dikerjakan dan yang tidak
- Kolektor: `listener`, `process`, `cron`. Aturan: `PAR-001` (proses mendengar tanpa padanan), `PAR-008` (cron tanpa padanan), `PAR-010` (port terbuka ke internet).
- Nilai yang belum bisa ditentukan kolektor M1-a ditulis `null`/`unknown`, bukan ditebak: `process.mode = unknown`, `process.supervised = null`, `process.env_refs = []`.
- Belum: systemd/docker/pm2, `env_ref`, DNS, database, uji fungsional, persistensi ke DB, agen/SSH, macOS.

## Dampak
Aturan (`parity`) tidak bergantung pada kolektor, dan kolektor tidak bergantung pada aturan; CLI `infra` merangkai keduanya. Ketergantungan lain dicatat jujur di "Batas yang diketahui".

## Batas yang diketahui
- **Ketergantungan crate.** `inventory` bergantung pada `serde`, `serde_json`, `core-domain`. Kolektor bergantung pada `inventory`, `core-domain` (redaksi rahasia dan tipe bersama; kolektor wajib menyunting nilai sebelum keluar) dan `serde_json` (membangun `Item.data`). `parity` bergantung pada `inventory`, `core-domain` (tingkat perbaikan) dan `i18n` (merender temuan dari katalog). Jadi "hanya bergantung pada `inventory`" berlaku untuk aturan terhadap kolektor, bukan secara mutlak.
- **Peringatan kolektor belum terlokalisasi.** `Inventory.warnings` berisi teks Inggris yang dicetak apa adanya oleh CLI. Perbaikannya butuh perubahan kontrak (kode peringatan terstruktur + argumen, teks dari katalog); belum dikerjakan.
- **Gerbang otomatis harus memeriksa status partial sendiri.** Bila salah satu inventaris `partial`, aturan "tidak ada padanan" (PAR-001, PAR-008) turun ke Info, sehingga kode keluar tetap 0 bila tidak ada temuan Critical, walau catatan partial tercetak di laporan. Skrip/CI yang memakai kode keluar sebagai gerbang wajib memeriksa `status` kedua inventaris. Kode keluar sengaja tidak diubah. PAR-010 tidak diturunkan oleh status partial: listener publik yang terlihat adalah bukti keberadaan.
- **PAR-008 mencocokkan key persis.** Key cron memuat jadwal dan petunjuk perintah; perubahan kecil pada keduanya (mis. jadwal dipindah jam) dianggap tugas hilang. Job dengan jadwal, pemilik, dan skrip yang sama digabung menjadi satu item (`occurrences`), sehingga jumlah salinan yang berbeda antara sumber dan target tidak terdeteksi.
- **PAR-001 mencocokkan nama proses.** Listener target dianggap "port berpindah" hanya bila bernama proses sama, portnya tidak dipakai listener sumber mana pun, dan belum diklaim listener sumber lain (penugasan menurut urutan key). Ini heuristik; proses yang berganti nama tampil Critical.
