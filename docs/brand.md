# Nodivara: panduan merek untuk UI

Terakhir diperbarui: 2026-11-19. Keputusan: `docs/decisions/0019`. Dokumen kerja internal (Indonesia).

## Warna (sumber: `web/src/tokens.css`)

Jangan menulis kode warna langsung di komponen; pakai variabel `--nv-*`. Tema terang bawaan, gelap mengikuti sistem atau `data-theme="light|dark"` pada `<html>`.

| Token | Terang | Gelap | Dipakai untuk |
|---|---|---|---|
| `--nv-bg` | `#f6f8f2` | `#070812` | latar halaman |
| `--nv-surface` / `-2` | `#ffffff` / `#eef2e6` | `#0d1020` / `#151a2c` | kartu, tabel, panel |
| `--nv-text` | `#0b1020` | `#eef1f8` | teks utama |
| `--nv-text-muted` | `#434c61` | `#a9b3c7` | teks sekunder |
| `--nv-primary` / `-hover` | `#1f5a07` / `#174804` | `#b6f205` / `#e9fc61` | tombol utama (teks `--nv-on-primary`) |
| `--nv-link`, `--nv-focus` | `#1f5a07`, `#1f5a07` | `#b6f205`, `#e9fc61` | tautan, cincin fokus |
| `--nv-accent` | `#b6f205` | `#b6f205` | sorotan dan pilihan teks (teks `--nv-on-accent`), bukan teks di latar terang |
| `--nv-success/warning/danger/info` | `#1f5a07 #6a4300 #951c12 #1740b3` | `#5fd01a #f5b83d #ff8a8a #7aa7ff` | status |
| `--nv-border`, `--nv-border-strong` | `#d5dccb`, `#6f7c62` | `#262d44`, `#6c789a` | garis dekoratif, batas kontrol input |

## Kontras terukur (rasio WCAG)

| Pasangan (nilai terendah dari tiga permukaan) | Terang | Gelap | Syarat |
|---|---|---|---|
| teks utama | 16,7 | 15,3 | 7 (AAA) |
| teks sekunder | 7,6 | 8,2 | 7 (AAA) |
| tautan | 7,3 | 12,9 | 7 (AAA) |
| status (success, warning, danger, info) | 7,3 | 7,2 | 7 (AAA) |
| teks tombol di tombol utama (normal dan hover) | 8,3 | 14,9 | 7 (AAA) |
| teks di atas sorotan (`accent`) | 14,9 | 14,9 | 7 (AAA) |
| isi tombol utama dan cincin fokus terhadap permukaan | 7,3 | 12,9 | 3 |
| batas kontrol input | 3,9 | 3,9 | 3 |

Teks: AAA (7:1) di **semua tiga permukaan** (`bg`, `surface`, `surface-2`), bukan hanya di latar halaman. Elemen UI: 3:1 (WCAG 2.2, 1.4.11). Diperiksa `python3 scripts/check-palette.py` (36 pasangan per tema; rumusnya diuji pada nilai WCAG yang diketahui, dan pemeriksa gagal bila nilai diturunkan) dan di CI. Mengubah token tanpa lulus pemeriksa tidak boleh digabung. Pemeriksa hanya menjangkau token warna: gambar, logo, dan warna yang ditulis langsung di komponen tidak tercakup (pemindaian terakhir: tidak ada warna tertulis langsung di `ui/src`).

## Diukur pada halaman yang dirender (2026-11-19)

Selain menghitung token, halaman prototipe dibuka di browser dan setiap teks diukur dari warna terhitung dan latar efektifnya:

| | Terang | Gelap |
|---|---|---|
| teks terendah di halaman | 7,78 | 8,79 |
| teks isi kotak pilihan | 18,9 | di atas 7 (token yang sama) |
| batas kotak pilihan terhadap halaman | 4,1 | 3,9 (token) |
| override manual `data-theme` | bekerja | bekerja |

Temuan dari pengukuran ini: kotak pilihan bahasa semula memakai warna bawaan browser (lolos kontras tetapi tidak senada); kini memakai token. Yang **belum** tercakup: keadaan yang belum ada di prototipe (nonaktif, hover tombol utama, tautan terkunjungi, grafik), dan layar nyata di Windows dengan mode kontras tinggi (`forced-colors`). Tambahkan ke pemeriksaan setiap komponen baru yang memakai warna.

## Aturan pakai

- Lime (`#b6f205`) hanya sebagai teks atau tombol di atas latar gelap; di latar terang ia hanya sorotan dengan teks gelap di atasnya.
- Warna tidak boleh menjadi satu-satunya penanda status: sertakan teks atau ikon (status di tabel sudah berteks).
- Warna status hijau (`success`) dan merek hijau sengaja berdekatan; bedakan lewat teks dan ikon, bukan warna saja.

## Aset logo (`brand/`)

| Berkas | Isi | Dipakai untuk |
|---|---|---|
| `nodivara-lockup-dark.png` | tanda + tulisan, putih di atas gelap, sudut membulat | latar gelap (README tema gelap) |
| `nodivara-lockup-light.png` | tanda + tulisan, teks gelap di atas putih | latar terang (README tema terang) |
| `nodivara-mark-512.png`, `-128.png` | tanda N di ubin gelap | ikon aplikasi, favicon di latar gelap |
| `nodivara-mark-light-512.png`, `-128.png` | tanda N di atas putih | ikon di latar terang |
| `nodivara-square-light-640.png` | tanda + tulisan + gelombang hijau, persegi | kartu, avatar |
| `nodivara-social-1280x640.png` | logo, slogan, dan "AI proposes. You approve." di atas gelap | pratinjau sosial GitHub (diunggah manual di Settings, tidak lewat API) |

Latar semua PNG opak (putih atau gelap), bukan transparan. Belum ada: SVG vektor dan versi transparan. Minta ke pemilik; jangan menyunting PNG yang ada untuk membuatnya.

Catatan 2026-11-24: UI web memakai `web/public/favicon.svg`, tanda N vektor kecil dengan warna merek. Itu **gambar ulang sementara** yang menyerupai tanda, bukan SVG resmi dan bukan turunan berkas asli; pernyataan di atas tetap benar: SVG resmi belum ada. Token warna UI web dipakai ulang dari `ui/src/tokens.css` lewat alias Vite (tidak disalin).
