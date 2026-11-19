# 0023. Putaran asisten AI (M2-d): batas di kode, hanya merekomendasikan

- Tanggal: 2026-11-20
- Status: diterima
- Pengambil keputusan: pemilik proyek (arah M2 di 0020); rincian oleh pelaksana track A

## Konteks
M2-a/b memberi lapisan alat tanpa model (0020), M2-c klien OpenAI-compatible (0022), dan `vault` rahasia (0021). M2-d menyambungkannya menjadi satu putaran percakapan (docs/design.md 7.1). Model dan server tidak tepercaya: teks dari alat adalah data, bukan perintah (aturan 4), rahasia tidak boleh sampai ke model atau log (aturan 2), dan di M2 tidak ada alat di atas tingkat 1 serta tidak ada jalur persetujuan atau eksekusi (aturan 3).

## Keputusan
Modul `agent::assistant` menjalankan satu giliran (`Assistant::turn`) generik atas `ai::AiProvider`. Semua batas ditegakkan kode, bukan prompt:

1. **Redaksi sebelum apa pun.** Pesan pengguna dilewatkan `core_domain::redact` sebelum disimpan atau dikirim. Balasan model dan argumen tool call juga diredaksi sebelum disimpan.
2. **Satu giliran per percakapan.** Klaim atomik (`UPDATE ... WHERE` pada `turn_token`) di tabel `conversations`; giliran kedua mendapat `Busy`. Klaim kedaluwarsa dengan detak (`touch_turn`) sehingga giliran yang mati tidak mengunci percakapan selamanya; giliran yang klaimnya diambil alih berhenti dengan `ClaimLost`.
3. **Isolasi.** Percakapan milik pasangan (org, pengguna). Org atau pengguna lain mendapat `NotFound`, tidak dapat dibedakan dari percakapan yang tidak ada. Setiap query memakai `org_id`; anak memakai kunci asing komposit (`org_id`, id).
4. **Konteks.** Prompt sistem tetap per locale (id/en), 20 pesan selesai terakhir, pesan baru. Prompt hanyalah panduan; pertahanan sebenarnya ada di kode.
5. **Semua tool call dijawab.** Setiap `tool_call_id` dari satu respons dijalankan lewat `Executor` (plafon tingkat, skema argumen tertutup, redaksi, bungkus "tak tepercaya"). Panggilan di atas `max_calls_per_round` (8) dijawab dengan penolakan `too_many_calls`, bukan dibuang. Id kosong, kembar, atau tidak wajar diganti id sintetis agar permintaan berikutnya tetap valid. Argumen yang bukan JSON dijawab sebagai galat ke model dan disimpan sebagai JSON valid.
6. **Catatan per panggilan.** Satu kelompok (pesan asisten, hasil alat, baris `tool_calls`) disimpan dalam satu transaksi, dan satu entri audit berantai (`assistant.tool_call`, aktor `System`) per panggilan: argumen teredaksi dan kode status saja, tidak pernah keluaran mentah.
7. **Batas putaran.** Paling banyak 6 putaran dengan alat, lalu satu panggilan akhir tanpa alat yang meminta jawaban dari data yang terkumpul. Tool call pada jawaban akhir itu diabaikan, tidak pernah dijalankan.
8. **Anggaran token per giliran** (bawaan 100.000). Dihitung dari `usage` penyedia; bila tidak dilaporkan, diperkirakan byte/4 dan `TurnReport::tokens_estimated` memberi tahu. Melampaui anggaran mengembalikan `BudgetExceeded`.
9. **Kejujuran.** `FinishReason::Length` ditandai `cut_off`. Galat penyedia dikembalikan apa adanya dan TIDAK ada balasan karangan yang disimpan; jawaban kosong menjadi `NoAnswer`. Pesan kosong, bernilai NUL, atau lebih dari 16 KiB ditolak sebelum menyentuh model.
10. **Tulis hanya lewat `ProposalSink`.** `ToolContext` mendapat seam tulis terpisah dari `ReadStore` (yang tetap hanya-baca). Satu-satunya kemampuannya: membuat proposal berstatus `pending`, terikat ke percakapan giliran berjalan (model tidak bisa menyebut percakapan lain), maksimal 20 pending per percakapan. Tidak ada metode setujui, ubah, atau jalankan. Alat baru `propose_fix` (tingkat 1; judul <= 120, alasan <= 1000, tindakan <= 500 karakter) hanya menyisipkan proposal; dampaknya dideklarasikan kode alat. `read_only_registry` diganti nama `assistant_registry`.
11. **Pemuat penyedia.** `DbProviderLoader` mengubah baris `ai_providers` menjadi `OpenAiCompatible<NetTransport>` (`ProviderConfig::from_params`, kunci dibuka `Vault::load` pada saat itu, tidak disimpan di pemuat). Tiap pemuatan membangun `net::Client` sendiri (transport memegang kliennya; `net::Client` tidak diubah di track ini).

Migrasi 0006 menambah `conversations` (dengan kolom klaim), `messages` (PK `org_id, conversation_id, seq`), `tool_calls`, dan `proposals` (status `pending|approved|rejected|executed|failed|expired`; di M2 hanya `pending` yang dibuat). `db::sha256_hex` diekspos untuk `args_hash`.

## Alasan
- Batas di kode tidak bisa dilanggar model yang disusupi prompt; prompt hanya menurunkan kemungkinan salah langkah.
- Memisahkan `ProposalSink` dari `ReadStore` membuat "alat hanya-baca" tetap benar secara tipe, dan membuat satu-satunya efek samping alat dapat diaudit di satu tempat.
- Menjawab setiap panggilan (bahkan yang ditolak) dan menyimpan argumen yang sudah dibersihkan menjaga percakapan tetap valid bagi penyedia pada giliran berikutnya.

## Alternatif yang dipertimbangkan
- **Giliran tanpa klaim, mengandalkan satu pekerja.** Ditolak: dua tab atau dua permintaan dapat menyisipkan pesan bersilangan; klaim di database murah dan tahan restart.
- **Menyimpan keluaran mentah model/alat untuk debugging.** Ditolak: melanggar aturan rahasia; kode status dan argumen teredaksi cukup.
- **Menghentikan giliran di batas putaran dengan galat.** Ditolak: orang mendapat jawaban lebih baik dari satu panggilan akhir dari data yang sudah ada; batasnya tetap keras.
- **`propose_fix` langsung menulis ke tabel lewat `ReadStore`.** Ditolak: merusak jaminan hanya-baca.
- **Membagikan satu `net::Client` ke semua pemuatan** (lewat `Clone` atau `Arc`). Ditunda: butuh perubahan di `net`/`ai` di luar track ini; biayanya satu pembacaan akar sertifikat per giliran.

## Yang belum diuji
- Jalur hidup `DbProviderLoader` terhadap penyedia nyata belum diuji (tanpa kunci); loop diuji dengan `ScriptedProvider`.
- Pemisahan konteks saat riwayat melebihi 20 pesan memotong di batas pesan, belum di batas token.

## Dampak
- Jalur persetujuan dan eksekusi proposal ada di M4; status selain `pending` belum dipakai kode.
- CHANGELOG, README (en/id), NEXT-STEPS, dan bagian Struktur CLAUDE.md perlu mencatat modul `assistant`, migrasi 0006, dan alat `propose_fix` (dikerjakan orkestrator).
