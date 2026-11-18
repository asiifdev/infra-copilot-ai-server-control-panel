# Daftar periksa sebelum repositori dibuka ke publik

Repositori ini privat sampai semua butir di bawah selesai. Tandai `[x]` bila sudah.

## Hukum dan kepemilikan
- [ ] Nasihat hukum atas lisensi (`AGPL-3.0-only` atau alternatif) dan model open core
- [ ] Putuskan DCO atau CLA/penyerahan hak cipta (lihat `docs/decisions/0006`)
- [ ] Tetapkan pemegang hak cipta hukum; ganti "Infra Copilot contributors" bila perlu
- [ ] Pemeriksaan merek dagang untuk nama (kata "Copilot"); tetapkan nama final
- [ ] Tinjau lisensi semua dependensi (`cargo deny`) dan bahan pihak ketiga

## Penanda yang harus diisi
- [ ] Kontak pelaporan di `CODE_OF_CONDUCT.md` dan `CODE_OF_CONDUCT.id.md`
- [ ] Kontak keamanan di `SECURITY.md` dan `SECURITY.id.md`
- [ ] Aktifkan pelaporan kerentanan privat GitHub
- [ ] Setel `contact_links` pada templat issue bila perlu

## Kebersihan data
- [ ] Jalankan `scripts/check-personal-data.sh` hingga bersih
- [ ] Tulis ulang `docs/plan.md` secara generik (kini memuat nama dan domain pribadi), atau keluarkan dari repo publik
- [ ] Periksa `docs/market-research.md` dan `docs/decisions/` dari data pribadi
- [ ] Putuskan nasib `.agents/`, `.claude/`, `reports/`, `research_notes/` (tidak ikut publik bila tidak perlu)
- [ ] Pindai riwayat git untuk rahasia (mis. gitleaks); bila ada, bersihkan riwayat atau mulai repositori baru

## Kualitas
- [ ] CI: `cargo fmt`, `clippy -D warnings`, `cargo test`, `cargo audit`, `cargo deny`
- [ ] Anggaran kinerja (ukuran biner, memori diam, waktu mulai) terukur dan dijaga di CI
- [ ] Perlindungan cabang `main`, tinjauan wajib untuk area sensitif
- [ ] README dan dokumen tidak menjanjikan fitur yang belum ada

## Kemudahan ditemukan (SEO, AEO, GEO)
- [ ] Nama repositori final dan semua URL di `llms.txt` diperbarui bila nama berubah
- [ ] Deskripsi, topik, dan situs web repositori terisi; gambar pratinjau sosial dibuat
- [ ] Kalimat definisi di README, FAQ, dan perbandingan diperiksa ulang: tidak ada klaim fitur yang belum ada
- [ ] Fakta tentang produk lain diverifikasi ulang dan diberi tanggal terbaru (harga berubah)
- [ ] Setelah ada situs web: `robots.txt` yang membedakan crawler pencarian dan pelatihan, JSON-LD (`SoftwareApplication`, `FAQPage`), `llms.txt`
- [ ] Setelah publik: uji sebulan sekali 10-20 pertanyaan di ChatGPT, Perplexity, Google, Claude (3-5 kali per pertanyaan) dan catat apakah proyek dikutip dan apakah isinya benar
- [ ] Hanya kehadiran yang asli: tidak ada ulasan, bintang, atau sebutan palsu; umumkan dengan jujur sebagai pra-alfa
- [ ] Bila ada kanal sponsor: isi `FUNDING.md`/`FUNDING.id.md` dan buat `.github/FUNDING.yml` (butuh akun milik pemilik; jangan memakai tautan yang belum ada)
