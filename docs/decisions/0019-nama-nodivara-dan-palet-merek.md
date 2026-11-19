# 0019. Nama produk Nodivara dan palet merek

- Tanggal: 2026-11-19
- Status: diterima (pemilik menetapkan nama dan menyediakan logo, 2026-11-19)
- Pengambil keputusan: pemilik proyek

## Konteks
Nama sementara "Infra Copilot" (keputusan 0001 dan 0006) diganti. Pemilik memilih **Nodivara** dan menyediakan tiga logo (hijau lime neon dan putih keperakan di atas latar hampir hitam). UI harus senada dan kontras dengan logo, dalam versi gelap dan terang.

## Keputusan
1. **Nodivara adalah nama produk dan platform.** Nama repositori (`infra-copilot-ai-server-control-panel`), nama crate, nama biner (`infra`), dan konstanta protokol (domain hash audit `infra-copilot/audit/v1`, yang jika diubah memutus rantai hash yang ada) **tidak diganti**; pemilik menyatakan penggantian nama repo tidak diperlukan. Dokumen publik, `llms.txt`, dan judul UI memakai nama baru. Keputusan 0001 dan 0006 tetap sebagai catatan sejarah.
2. **Palet diambil dari piksel logo** (latar `#070812`, lime merek `#b6f205`, lime terang `#e9fc61`, hijau `#429004`, perak `#d9deea`) dan ditetapkan sebagai token di `ui/src/tokens.css`. Tema gelap mengikuti logo; tema terang memakai hijau tua `#2c7a0b` untuk teks, tautan, dan tombol karena lime di atas putih hanya 1,3:1.
3. **Kontras diperiksa otomatis**: `scripts/check-palette.py` memeriksa 24 pasangan di kedua tema (WCAG 2.2 AA: 4,5:1 teks, 3:1 elemen UI) dan bahwa dua blok gelap sama. Pemeriksa diuji dengan nilai buruk dan dengan blok yang berbeda; keduanya membuatnya gagal. Dijalankan di CI.
4. **Aset logo** yang dioptimalkan ada di `brand/` (tanda 512 dan 128 px, lockup gelap 1000 px, palet terkuantisasi). Berkas induk asli tetap milik pemilik di luar repo.

## Belum diselesaikan (jangan diklaim bebas)
- **Pemeriksaan merek dan domain belum dilakukan oleh siapa pun.** Daftar kerja: WIPO Global Brand Database, USPTO, daftar nasional yang relevan, variasi ejaan dan kemiripan bunyi; domain (`nodivara.com`, `.io`, `.dev`, `.cloud`); akun GitHub, npm, Docker Hub, media sosial. Tanpa hasil, statusnya **belum diketahui**, bukan tersedia. Saran ahli hukum merek sebelum rilis publik.
- Wordmark untuk latar terang belum ada (logo yang ada putih di atas gelap). UI memakai teks "Nodivara" yang mengikuti tema, atau tanda N di dalam ubin gelap, sampai ada versi terang dari pemilik.
- Hak cipta logo (dibuat dengan alat generatif): pemilik perlu memastikan hak pakai dan kepemilikannya sebelum rilis publik.

## Alasan
Palet yang diukur dari aset nyata dan dijaga oleh pemeriksa otomatis mencegah kontras yang rusak diam-diam saat UI bertambah.

## Alternatif
Palet buatan sendiri tanpa logo sebagai acuan (ditolak: tidak senada); lime sebagai warna tautan di tema terang (ditolak: 1,3:1).
