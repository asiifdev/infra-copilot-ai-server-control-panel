# 0022. Klien AI OpenAI-compatible yang ditulis sendiri di atas `net`

- Tanggal: 2026-11-19
- Status: diterima (pemilik, 2026-11-19: kinerja dan kecepatan maksimal; Muse sebagai model uji)
- Pengambil keputusan: pemilik proyek

## Konteks
M2-c membutuhkan klien Chat Completions dengan function calling dan streaming SSE untuk Muse (kredit gratis pemilik), OpenAI, dan model lokal (Ollama). Aturan 0016: satu jalur TLS dan satu aturan anti-SSRF lewat crate `net`. Aturan 0005: dependensi harus membenarkan ukuran dan waktu kompilasinya.

## Keputusan
Crate `ai` menulis sendiri format kawat OpenAI-compatible (sekitar 1.000 baris termasuk uji) di atas `net`: pembuat permintaan, pengurai jawaban, dekoder SSE, perakit tool call yang terpecah, penyaring blok penalaran `<think>`, percobaan ulang dengan backoff, dan pemutus sirkuit. Jaringan di balik trait `Transport` (uji memakai transport tiruan dan server soket lokal).

## Alasan
- **Satu jalur jaringan.** `async-openai` 0.43 (diperiksa 2026-11-19) menarik `reqwest` untuk TLS, yaitu klien HTTP kedua dengan pohon dependensi sendiri, aturan SSRF sendiri, dan tanpa batas ukuran/waktu yang kita tetapkan di `net`. 0016 mengukur `reqwest` 1,9-3,3 MB lebih besar daripada `hyper` + `tokio-rustls` yang sudah dipakai.
- **Kompatibilitas nyata, bukan nominal.** Server "kompatibel" berbeda dalam hal yang tidak ditangani pustaka berskema OpenAI ketat: id tool call yang hilang, argumen berupa objek, `content` berupa daftar bagian, `[DONE]` yang tidak ada, JSON biasa walau `stream: true`, `stream_options` yang ditolak. Semuanya ditangani dan diuji.
- **Keamanan di tempat yang kita kontrol.** Kunci hanya di header `Authorization` (ditandai sensitif), galat bertulisan tetap, setiap jawaban dibatasi (byte, jumlah tool call, ukuran argumen), `null` tidak pernah dikirim, dan hanya kolom yang dikonfigurasi yang dikirim.
- **Percobaan ulang yang aman.** Hanya untuk kegagalan sementara dan hanya sebelum teks sampai ke pemanggil (agar teks tidak terulang); `Retry-After` dihormati; kegagalan kredit (`insufficient_quota`) tidak diulang; penolakan permintaan tidak menghitung terhadap pemutus sirkuit.

## Alternatif yang dipertimbangkan
- `async-openai`: ditolak (di atas). Skema berjenis kuat juga menolak jawaban dari penyedia yang sedikit menyimpang.
- `rig`, `genai`, `llm`: abstraksi multi-vendor lebih luas dari yang dibutuhkan, tanpa jaminan jalur jaringan kita.
- `openai-api-rs` dan sejenisnya: sama-sama membawa klien HTTP sendiri.

## Yang belum diuji
- Uji hidup terhadap Muse dan Ollama belum dijalankan oleh saya: kunci tidak saya baca. Uji tersedia (`live_smoke_test`, `--ignored`, kunci dari lingkungan) dan perlu dijalankan pemilik sekali sebelum M2-c dianggap terbukti pada penyedia nyata.
- Dukungan `stream_options.include_usage`, `parallel_tool_calls` dan `tool_choice` Muse belum diketahui; semuanya mati bawaan dan dapat dinyalakan lewat `params`.
- Ukuran biner klien ini belum diukur; diperiksa job `perf-budget` saat `ai` tertanam di server.

## Dampak
- `AiProvider` (trait) dipakai putaran agen M2-d lewat `ScriptedProvider` pada uji.
- Pemuatan penyedia dari database (baris `ai_providers` + kunci dari `vault` + `ProviderConfig::from_params`) dikerjakan di M2-d bersama putaran agen. Selesai di M2-d (`DbProviderLoader`, 0023).
