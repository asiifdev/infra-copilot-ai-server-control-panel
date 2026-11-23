# 0031. UI web di `web/`: Svelte 5 tanpa pustaka tambahan, tiruan API, dan anggaran ukuran

- Tanggal: 2026-11-23
- Status: diterima (pemilik, 2026-11-23: "lanjut ke UI"; melaksanakan keputusan 0029)
- Pengambil keputusan: pemilik proyek

## Konteks
Keputusan 0029 menetapkan server lokal, kontrak API, dan UI baru di `web/` (Svelte 5 + Vite + TypeScript, tanpa SvelteKit). Server dibangun di jalur lain, jadi UI dibangun terhadap **tiruan yang setia** pada kontrak itu. Prinsip kinerja 0005 berlaku: teknologi teringan yang memenuhi kebutuhan. Aturan proyek: komponen bersama (`docs/design.md` 12.5), tanpa `{@html}`/gaya inline/`eval` (CSP 0029), semua teks lewat katalog i18n (0008), kontras token (0019).

## Keputusan
1. **`web/`** berisi Svelte 5 (mode runes) + Vite + TypeScript ketat, tanpa SvelteKit, tanpa kerangka UI, tanpa pustaka router, tanpa pustaka markdown, tanpa kerangka CSS. Skrip: `npm run dev` (terhadap tiruan), `build` (ke `web/dist`), `check` (`svelte-check`, peringatan dianggap gagal), `test` (`vitest`).
2. **Token** warna dipakai ulang dari `ui/src/tokens.css` lewat alias Vite `$tokens` (tidak disalin). Skala (jarak, ukuran, radius) ada di `web/src/app.css` sebagai variabel `--nv-*`; komponen tidak memuat kode warna atau angka ajaib (diuji).
3. **Satu paket komponen** (`web/src/components/`): `DataTable`, `StatusBadge`, `Card`, `EmptyState`, `Toast`, `ProposalCard` (hanya baca, tanpa tombol setujui/jalankan; catatan bahwa itu datang dengan M4), `Button`, `TextField`, `Select`, `Checkbox`, `ConfirmDialog`, `Spinner`, ditambah `TextBlock` (satu-satunya jalan teks dari server/model/alat ke layar: teks, jeda baris dipertahankan), `ErrorNotice`, `AsyncView`, `PageHeader`, `ChatMessage`, `ProviderForm`. Halaman hanya menyusun komponen ini.
4. **Fondasi** (`web/src/lib/`): klien API bertipe (`api.ts`), pembaca SSE (`sse.ts`, port dari `crates/ai/src/sse.rs`), toko i18n, router hash, tema, toast, `Resource` (pemuatan dengan pembatalan), sesi (alur tukar token), `chat` (satu giliran asisten).
5. **Tiruan** di `web/mock/` mengimplementasikan setiap titik akhir 0029 (termasuk SSE dengan jeda, 401 sebelum tukar token, 409 busy, `master_key_missing`, uji penyedia berhasil dan gagal, aturan CSRF `X-Nodivara`, token sekali pakai, cookie sesi). Ia menyajikan **katalog Rust yang asli** (`crates/i18n/locales`) sehingga UI dan katalog tidak bisa melenceng. Mode `hostile` mengisi setiap teks bebas dengan string serangan. Dipakai oleh `npm run dev` (middleware Vite, hanya `serve`, tidak ikut build; memeriksa Host dan Origin seperti server) dan oleh uji (`createMock().fetch`).
6. **Keamanan UI (diuji, `web/tests/static.test.ts` dan lainnya):** tanpa `{@html}`, `innerHTML`, `eval`, `new Function`, `<script>` atau atribut `style="..."` inline; jaringan hanya di `lib/api.ts` dan hanya ke `/api/*` satu asal; penyimpanan browser hanya `nv.locale` dan `nv.theme`; token di fragmen dihapus dari bilah alamat sebelum hal lain berjalan dan tidak pernah masuk state, penyimpanan, atau URL; kunci API penyedia ada di satu variabel sampai permintaan terkirim, dikosongkan setelah setiap pengiriman (berhasil atau gagal), tidak pernah dirender kembali. Teks yang tampil dari server/model dibersihkan dari kendali arah (bidi) dan karakter lebar nol serta dipotong pada 200 000 karakter.
7. **i18n:** 248 kunci `ui.*` ditambahkan di akhir `en.txt` dan `id.txt`. Uji `static.test.ts` memindai semua literal `'ui.x.y'` di `web/src` dan gagal bila ada kunci yang hilang di salah satu berkas, ada kunci `ui.*` yang tidak dipakai, kunci/penampung tidak identik, atau panggilan `t()` memberi penampung yang salah; ia juga gagal bila ada teks tampilan yang tertulis di markup.

## Dependensi (semua versi stabil terbaru saat ini, dipin persis; `npm view` 2026-11-23)
| Paket | Versi | Fungsi dan alasan |
|---|---|---|
| `svelte` | 5.59.1 | kerangka UI yang diputuskan (CLAUDE.md); kompilator tanpa runtime besar |
| `vite` | 8.3.8 | pembangun yang diputuskan; sama dengan `ui/` |
| `@sveltejs/vite-plugin-svelte` | 7.3.1 | integrasi Svelte-Vite, sama dengan `ui/` |
| `vitest` | 5.1.1 | pelari uji yang direkomendasikan Svelte untuk proyek Vite; berbagi konfigurasi dengan Vite |
| `jsdom` | 31.0.1 | lingkungan DOM untuk uji komponen. Dipilih dibanding `happy-dom` karena lebih lengkap untuk fokus, `details`, dan aturan aksesibilitas yang dijalankan `axe-core` |
| `axe-core` | 4.14.1 | uji aksesibilitas otomatis atas semua halaman, dua bahasa, dua tema; tidak ikut bundel |
| `svelte-check` | 4.7.8 | pemeriksa tipe dan a11y Svelte (`npm run check`) |
| `typescript` | **6.0.3** | TypeScript 7.0.2 sudah rilis tetapi `svelte-check` 4.7.8 hanya menerima `^5 \|\| ^6` (peer dependency), jadi 6.0.3 (stabil 6 terbaru) dipakai. Naikkan ke 7 begitu `svelte-check` mendukungnya |
| `@types/node` | 26.6.6 | tipe untuk tiruan dan uji (Node 24) |

Tidak ada dependensi runtime: bundel produksi hanya berisi Svelte (kompilasi) dan kode sendiri.

## Ukuran bundel produksi (`npm run build`, 2026-11-23)
| Berkas | Mentah | gzip -9 |
|---|---|---|
| `assets/index-*.js` | 120 135 B (117,3 KiB) | 38 275 B (37,4 KiB) |
| `assets/style-*.css` | 15 232 B (14,9 KiB) | 3 159 B (3,1 KiB) |
| `index.html` | 441 B | |
| **Total aset** | 135 367 B | 41 434 B |

**Usulan anggaran** (bagian dari keputusan anggaran baru 0005/0010, dihitung ulang dari server sungguhan; diusulkan, belum diperiksa CI): JS maksimum 160 KiB mentah dan 50 KiB gzip; CSS maksimum 24 KiB mentah dan 5 KiB gzip; tidak ada font atau gambar eksternal; tidak ada berkas `.map` di biner. Selisih sekitar 30 persen dari ukuran sekarang memberi ruang untuk M4 (persetujuan Usulan).

## Hasil uji
`npm test`: 158 uji lulus di 8 berkas (klien API, dekoder SSE untuk setiap pemotongan byte dan tiga akhir baris, i18n, router, tema, toast, `Resource`, sesi dan alur fragmen, pemindaian sumber, komponen, halaman Asisten dan Pengaturan, 44 pemeriksaan axe-core: 11 rute x 2 bahasa x 2 tema, dan 11 halaman dengan data bermusuhan). `npm run check`: 0 galat, 0 peringatan. `cargo test -p i18n` lulus dengan kunci baru. Setiap berkas Svelte dan `.svelte.ts` diperiksa `svelte-autofixer` sampai tanpa isu; sisa saran adalah heuristik "memanggil fungsi di dalam `$effect`" pada efek pemuatan data yang memang bertugas demikian (`useResource`, pemuatan percakapan), sudah ditinjau dan sengaja dipertahankan.

## Pembacaan kontrak yang ambigu (dicatat sebagai penyimpangan, perlu disamakan dengan jalur server)
1. **`GET /api/i18n/{locale}` tanpa autentikasi.** Kontrak 0029 hanya mengecualikan `/healthz`, tetapi halaman masuk perlu teks sebelum token ditukar. UI mencoba memuat katalog lebih dulu dan memuat ulang setelah sesi ada; bila server tetap meminta autentikasi, halaman masuk menampilkan nama kunci. Usulan: server menyajikan katalog UI (hanya teks, tanpa rahasia) tanpa sesi. Tiruan melakukannya.
2. **Bentuk yang tidak ditulis kontrak:** memori `{id, content, status, created_at?}`; Usulan `{id, action, reason, impact, status, tier?}`; percakapan `{id, title, created_at?}`; pesan `{role, content}` (hanya `user` dan `assistant` ditampilkan); `tier` pada peristiwa `tool` diterima sebagai angka 0-4 atau nama; keparahan temuan `info | important | critical` (dari `crates/inventory`); `gates` ditampilkan sebagai daftar kunci-nilai (kunci dikenal diterjemahkan, sisanya apa adanya). Server sebaiknya mengikuti bentuk ini atau UI disesuaikan.
3. **Kode galat** yang dikenali UI: `invalid_token`, `unauthorized`, `forbidden`, `not_found`, `busy`, `master_key_missing`, `bad_request`, `payload_too_large`, `rate_limited`, `timeout`, `internal`, serta untuk peristiwa `error` dan uji koneksi `no_provider`, `provider_unreachable`, `provider_auth`, `provider_error`. Kode lain tampil sebagai pesan umum; teks dari server hanya muncul sebagai baris rincian bertanda teks biasa.
4. **CSP** tidak dipasang oleh tiruan pada halaman (Vite dev menyuntikkan skrip inline untuk HMR); build produksi tidak memakai skrip atau gaya inline sehingga CSP penuh dari 0029 berlaku di server sungguhan. Tiruan hanya memasang `nosniff`, `no-referrer`, dan `no-store` pada API.
5. **Keterbatasan uji aksesibilitas:** jsdom tidak punya tata letak, jadi kontras warna dan ukuran target tidak diukur otomatis di sini (kontras token diperiksa `scripts/check-palette.py`; ukuran target dipaksa lewat token `--nv-target` pada setiap kontrol). Pemeriksaan di browser nyata (zoom, mode kontras tinggi, pembaca layar) belum dilakukan.

## Alasan
- Tanpa pustaka tambahan, bundel kecil dan permukaan serangan rantai pasok hanya Svelte dan Vite yang sudah dipakai.
- Tiruan yang memakai katalog dan aturan CSRF asli membuktikan klien mengirim `X-Nodivara` dan tidak melenceng dari teks.
- Satu jalur teks tidak tepercaya (`TextBlock`/`inlineText`) membuat aturan "data, bukan perintah" dapat diuji di satu tempat.

## Alternatif yang dipertimbangkan
- Pustaka router atau markdown: ditolak; router hash sederhana cukup, dan teks model tidak boleh menjadi HTML.
- `happy-dom` atau pengujian di browser nyata saja: `jsdom` lebih mendekati aturan axe; browser nyata ditambahkan nanti bila perlu.
- Menyalin katalog ke UI: ditolak oleh 0029.
- TypeScript 7: ditunda karena `svelte-check` belum mendukungnya.

## Dampak
- Perlu dilakukan orkestrator: README/CLAUDE.md/NEXT-STEPS/CHANGELOG (struktur `web/`, perintah, hasil), `scripts/check-docs.py` harus melewati `web/node_modules/` dan `web/dist/`, CI perlu langkah `npm ci && npm run check && npm test && npm run build` di `web/` dan (setelah anggaran diputuskan) pemeriksaan ukuran, dan server harus menyajikan `web/dist` (rust-embed) serta menyamakan butir 1-3 di atas.
- Belum dikerjakan: persetujuan dan eksekusi Usulan (M4), pemeriksaan di browser nyata, anggaran CI untuk `web/`.
