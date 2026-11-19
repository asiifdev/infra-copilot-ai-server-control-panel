# 0008. Katalog i18n sendiri (tanpa Fluent)

- Tanggal: 2026-11-19
- Status: diterima oleh pemilik ("setuju", 2026-11-19)
- Pengambil keputusan: pemilik proyek (menunggu persetujuan)

## Konteks
`docs/design.md` bagian 14.1 mewajibkan semua teks tampilan lewat katalog dengan kunci stabil (ID/EN, fallback ke Inggris yang dicatat). Prinsip `0005` menuntut dependensi minimal; `0007` menetapkan crate `i18n` hanya bergantung pada `core-domain`. Kebutuhan awal kecil: ±puluhan kunci, substitusi placeholder bernama, dua bahasa.

## Keputusan
Crate `i18n` memakai format teks sendiri (`kunci = teks {placeholder}`, komentar `#`) di `crates/i18n/locales/{en,id}.txt`, ditanam dengan `include_str!`, dengan parser kecil dan tipe galat manual (berisi nomor baris). `Catalog::get` mengembalikan `Translated { text, fell_back, missing, unresolved }`. Kunci berformat `area.subjek.aksi` (tiga segmen huruf kecil). Tanpa dependensi luar.

## Alasan
- Nol dependensi: tidak menambah ukuran biner, waktu kompilasi, atau permukaan rantai pasok.
- Fallback dan penanda `missing`/`fell_back` sesuai desain 14.1 dan mudah diuji (kesamaan kunci dan placeholder en/id, penolak teks tetap).
- Kompromi yang diterima: belum ada bentuk jamak, gender, atau pemformatan angka/tanggal per locale; semua itu harus ditambahkan sendiri atau lewat pustaka bila dibutuhkan.

## Alternatif yang dipertimbangkan
- **Fluent (`fluent-bundle`)**: mendukung jamak dan pilihan kompleks, tetapi menambah beberapa crate dan kompleksitas; kebutuhan sekarang belum membenarkannya.
- **`rust-i18n` / `gettext`**: makro/berkas tambahan dan dependensi; format PO kurang cocok untuk kunci stabil gaya `area.subjek.aksi`.
- **JSON + serde**: menambah `serde`/`serde_json` hanya untuk peta datar; kurang nyaman untuk komentar.

Pintu keluar: karena akses hanya lewat `Catalog::get` dan `MessageKey`, mengganti mesin ke Fluent nanti tidak mengubah pemanggil. Tinjau ulang bila perlu jamak (mis. "{count} node") atau format tanggal/angka per locale.

## Dampak
- Kode: `crates/i18n` (parser, katalog, `Locale`), uji kontrak katalog, dan uji penolak teks tetap yang memindai `ui/src/**/*.svelte` bila ada (berbasis pola, bukan bukti lengkap; batasannya ada di doc-comment uji).
- Dokumen: bagian "Kunci i18n" di skill `rust-conventions`.
- Lebih sulit: terjemahan dengan jamak/gender memerlukan perluasan format atau penggantian mesin; UI (Svelte) perlu pembungkus kecil `t(key, args)` yang memakai katalog ini (belum dibuat).
