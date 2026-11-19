# Nodivara: panduan merek untuk UI

Terakhir diperbarui: 2026-11-19. Keputusan: `docs/decisions/0019`. Dokumen kerja internal (Indonesia).

## Warna (sumber: `ui/src/tokens.css`)

Jangan menulis kode warna langsung di komponen; pakai variabel `--nv-*`. Tema terang bawaan, gelap mengikuti sistem atau `data-theme="light|dark"` pada `<html>`.

| Token | Terang | Gelap | Dipakai untuk |
|---|---|---|---|
| `--nv-bg` | `#f6f8f2` | `#070812` | latar halaman |
| `--nv-surface` / `-2` | `#ffffff` / `#eef2e6` | `#0d1020` / `#151a2c` | kartu, tabel, panel |
| `--nv-text` | `#0b1020` | `#eef1f8` | teks utama |
| `--nv-text-muted` | `#4b556b` | `#a9b3c7` | teks sekunder |
| `--nv-primary` | `#2c7a0b` | `#b6f205` | tombol utama (teks `--nv-on-primary`) |
| `--nv-link`, `--nv-focus` | `#2c7a0b` | `#b6f205`, `#e9fc61` | tautan, cincin fokus |
| `--nv-accent` | `#b6f205` | `#b6f205` | sorotan dan pilihan teks (teks `--nv-on-accent`), bukan teks di latar terang |
| `--nv-success/warning/danger/info` | `#2c7a0b #8a5a00 #b42318 #1d4ed8` | `#5fd01a #f5b83d #ff7b7b #7aa7ff` | status |
| `--nv-border`, `--nv-border-strong` | `#d5dccb`, `#7d8a6f` | `#262d44`, `#5a6685` | garis dekoratif, batas kontrol input |

## Kontras terukur (rasio WCAG)

| Pasangan | Terang | Gelap |
|---|---|---|
| teks / latar | 17,7 | 17,6 |
| teks muted / latar | 7,0 | 9,5 |
| tautan / latar | 5,0 | 14,9 |
| teks tombol / tombol utama | 5,4 | 14,9 |
| fokus / latar | 5,0 | 17,6 |
| status terendah (success, warning, danger, info) | 5,0 | 8,0 |
| batas kontrol / latar | 3,4 | 3,5 |

Batas: 4,5:1 untuk teks, 3:1 untuk elemen UI. Diperiksa `python3 scripts/check-palette.py` (dan di CI). Mengubah token tanpa lulus pemeriksa tidak boleh digabung.

## Aturan pakai

- Lime (`#b6f205`) hanya sebagai teks atau tombol di atas latar gelap; di latar terang ia hanya sorotan dengan teks gelap di atasnya.
- Warna tidak boleh menjadi satu-satunya penanda status: sertakan teks atau ikon (status di tabel sudah berteks).
- Warna status hijau (`success`) dan merek hijau sengaja berdekatan; bedakan lewat teks dan ikon, bukan warna saja.

## Aset logo (`brand/`)

| Berkas | Isi | Catatan |
|---|---|---|
| `nodivara-mark-512.png`, `-128.png` | tanda N di ubin gelap | ikon aplikasi, favicon; cocok di tema apa pun sebagai ubin |
| `nodivara-lockup-dark.png` | tanda + tulisan, putih di atas gelap | hanya untuk latar gelap |

Belum ada: wordmark untuk latar terang, SVG vektor. Minta ke pemilik; jangan menyunting PNG yang ada untuk membuatnya.
