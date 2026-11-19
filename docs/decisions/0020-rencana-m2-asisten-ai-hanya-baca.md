# 0020. Rencana M2: asisten AI hanya-baca

- Tanggal: 2026-11-19
- Status: diterima (pemilik, 2026-11-19: kinerja dan kecepatan maksimal, tanpa solusi sementara, semua irisan termasuk M2-e dikerjakan)
- Pengambil keputusan: pemilik proyek

## Konteks
M0 dan M1-a selesai. `docs/plan.md` bagian 9 menetapkan M2 sebagai pembeda utama: penyedia OpenAI-compatible, chat, riwayat, memori, alat tingkat 0-1. Kontrak sudah ada di `docs/design.md` bagian 12.4 (`AiTool`, `AiProvider`) dan skill `ai-tool-tiers`. Yang sudah dapat dipakai ulang: `core` (tingkat risiko, penyaring rahasia), `db` (skema `ai_providers`, `conversations`, `messages`, `tool_calls`, `memories`, isolasi tenant, audit berantai-hash), `net` (klien HTTP/TLS, 0016), `collectors`/`parity`/`inventory` (sumber data alat baca).

M2 tidak boleh mengubah server apa pun. Alat tingkat 2 ke atas tidak ada di M2; yang boleh hanya tingkat 0 (baca) dan tingkat 1 (menyusun Usulan sebagai data, tanpa pelaksana).

## Keputusan (usulan)
Bangun M2 dalam lima irisan kecil, masing-masing dapat diuji dan di-commit sendiri. Urutan mengikuti ketergantungan, bukan nilai demo.

| Irisan | Isi | Crate | Selesai bila |
|---|---|---|---|
| M2-a | Kontrak `AiTool` + registri alat + validasi skema argumen + lapisan eksekusi yang menolak tingkat > 1 dan argumen ngawur. Redaksi hasil alat lewat `core::redact`. Pembungkus "data tak tepercaya" untuk keluaran server. | baru: `agent` | uji kontrak bersama lulus; uji model-salah-panggil ditolak oleh kode, bukan prompt |
| M2-b | Alat tingkat 0 di atas kode yang ada: `list_services`, `read_inventory`, `diff_inventories`, `list_findings`, `explain_finding`. Hanya membaca snapshot/inventaris di DB milik organisasi yang sama. | `agent` | tiap alat punya uji isolasi tenant dan uji redaksi |
| M2-c | `AiProvider` OpenAI-compatible: `chat` dengan function calling, streaming SSE, batas waktu, batas ukuran respons, kunci terenkripsi, base URL dibatasi (anti-SSRF, sama seperti `net`). Uji kontrak dengan server tiruan lokal. | baru: `ai` | tanpa jaringan di CI; kunci tidak pernah muncul di log atau error |
| M2-d | Putaran agen: riwayat percakapan, batas langkah per giliran, pencatatan `tool_calls` + `audit_log`, anggaran token. Alat tingkat 1 `propose_fix` hanya menulis baris `proposals` berstatus `pending` (tanpa pelaksana, tanpa tombol setuju). | `agent` | giliran yang berputar tanpa akhir terhenti; semua panggilan tercatat |
| M2-e | Memori organisasi dan tugas (lihat, ubah, hapus oleh pengguna), pencarian teks FTS5 atas inventaris dan temuan, perintah CLI `infra ask` sebagai antarmuka pertama. UI/Telegram menyusul. | `agent`, `cli` | pengguna bisa menghapus memori; tidak ada rahasia tersimpan |

## Alasan
- **Lapisan eksekusi lebih dulu (M2-a), model belakangan (M2-c).** Aturan 1 dan 3 proyek: keamanan ada di kode, bukan di prompt. Dengan urutan ini, setiap alat sudah terbukti aman sebelum ada model yang bisa memanggilnya.
- **CLI dulu, bukan UI.** Kerangka UI dan perilaku jendela belum final; `infra ask` membuktikan nilai tanpa menambah anggaran biner UI.
- **Pencarian teks (FTS5) dulu, embedding ditunda.** Pertanyaan terbuka 3 di `docs/design.md` bagian 11. Embedding menambah ketergantungan dan biaya; FTS5 sudah ada di SQLite. Putuskan ulang bila uji nyata menunjukkan jawaban meleset.

## Keputusan pemilik (2026-11-19)
1. **Klien HTTP penyedia AI:** pakai ulang crate `net` (hyper + tokio-rustls), diperluas dengan POST dan SSE. Satu jalur TLS dan satu aturan anti-SSRF. Kriteria pemilik: kinerja dan kecepatan maksimal.
2. **Enkripsi kunci penyedia:** pustaka yang paling optimal, tanpa solusi sementara. Calon utama `ring` (AES-256-GCM), karena sudah ada di pohon dependensi lewat rustls sehingga tidak menambah ukuran biner. Dibuktikan dengan Context7 dan pengukuran di keputusan 0021 sebelum M2-c; kunci dari lingkungan bukan jalan pintas.
3. **Cakupan:** semua irisan M2-a sampai M2-e dikerjakan berurutan, tanpa menunggu demo.
4. **Model uji:** Muse (Meta) dengan kredit gratis pemilik. Endpoint kompatibel-OpenAI (Chat Completions, tool-calling, streaming; model `muse-ember-1.0`), seperti yang sudah dipakai proyek lain pemilik. Kunci hanya lewat variabel lingkungan saat dijalankan; tidak pernah di repo, log, atau prompt. Uji otomatis di CI tetap memakai server tiruan.

## Alternatif yang dipertimbangkan
- **Mulai dari UI chat:** nilai demo lebih cepat terlihat, tapi memaksa keputusan kerangka UI dan lapisan keamanan belum teruji.
- **Satu irisan besar:** lebih sedikit commit, tetapi review keamanan jadi sulit dan CI merah sulit dilokalisasi.
- **Pustaka agen pihak ketiga:** menambah dependensi besar dan perilaku yang belum diaudit; bertentangan dengan 0005.

## Dampak
- Dua crate baru (`agent`, `ai`); anggaran biner dihitung ulang setelah M2-c (bukan sebelumnya).
- Setelah disetujui: `docs/NEXT-STEPS.md` dan bagian Struktur/Status di `CLAUDE.md` diperbarui per irisan; `CHANGELOG.md` dan kedua README hanya menyebut M2 sebagai "direncanakan" sampai ada kode yang berjalan.
- Tidak ada perubahan perilaku produk saat ini.
