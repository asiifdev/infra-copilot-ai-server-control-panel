[English](README.md) | Bahasa Indonesia

# Infra Copilot

[![CI](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/actions/workflows/ci.yml/badge.svg)](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/actions/workflows/ci.yml)

**Infra Copilot adalah panel pengelolaan server open source, untuk pekerjaan yang sama dengan aaPanel dan cPanel, dengan agen AI bawaan yang mengusulkan perubahan dan menunggu persetujuan manusia.** Ditulis dengan Rust dan antarmuka Svelte, menyasar server Linux dan aplikasi desktop, dan masih pra-alfa: belum ada yang bisa dipasang.

Terakhir diperbarui: 2026-11-19.

> **Status: pra-alfa.** Belum ada rilis yang bisa dipasang. Repositori ini saat ini berisi desain,
> keputusan arsitektur, crate fondasi pertama (tipe domain, katalog pesan, skema database), prototipe
> pengukur ukuran dan kecepatan, dan baris perintah offline yang mengumpulkan inventaris Linux secara hanya-baca
> serta membandingkan dua inventaris dengan tiga aturan kesetaraan. Belum ada agen, panel, atau monitor. "Infra Copilot" adalah nama sementara dan bisa
> berubah.

## Apa ini

Infra Copilot bertujuan menjadi panel ringan untuk menjalankan dan memigrasikan server, sejalan
dengan aaPanel dan cPanel, dengan agen AI yang memahami keadaan mesin dan hanya bertindak setelah
disetujui manusia. Ia berjalan di server Linux (VPS atau dedicated) dan, kelak, sebagai aplikasi
desktop.

Semua yang tertulis di bawah ini **direncanakan**, belum dirilis:

- **Kelola server:** layanan, aplikasi, situs, database, backup, log, cron, berkas.
- **Deploy dari Git, satu klik:** mendeteksi stack (Node, PHP, Python, Go, .NET), build di server,
  mengalihkan lalu lintas tanpa henti, dan rollback. Ada mode lanjutan untuk kendali penuh.
- **Pantau uptime dan error:** pengecekan dari server dan dari luar, kejadian error yang
  dikelompokkan, serta analisis penyebab oleh AI beserta bukti yang dipakai dan langkah yang
  disarankan.
- **Bot Telegram:** notifikasi, perintah, dan tombol aksi, dalam bahasa Anda.
- **Migrasi dengan pemeriksaan kesetaraan:** inventaris server lama dan baru, lalu laporkan apa
  yang tertinggal (layanan yang tidak tercatat, `localhost` yang menunjuk mesin yang salah, rekaman
  DNS yang tidak pernah dipindah) sebelum server lama dimatikan.
- **DNS dan Cloudflare Tunnel:** kelola rekaman dan tunnel, selalu dengan snapshot dan rollback.
- **Agen AI yang bisa diaudit:** penyedia kompatibel OpenAI pilihan Anda, pemanggilan alat,
  memori, dan alur persetujuan. AI mengusulkan, Anda yang memutuskan.

## Prinsip

- **Teringan dan tercepat.** Satu biner Rust, UI Svelte 5 yang dikompilasi ke dalamnya, SQLite
  sebagai bawaan. Edisi desktop dengan Tauri. Tidak ada runtime tambahan di server Anda hanya untuk
  menjalankan panel.
- **Hak akses minimal.** Agen menjalankan daftar aksi yang tertutup, bukan shell bebas. Aksi
  diberi tingkat dari hanya-baca sampai merusak, dan semua yang mengubah atau menghapus sesuatu
  melalui usulan dan persetujuan manusia.
- **Rahasia tetap rahasia.** Tidak pernah dicatat di log, ditampilkan, atau dikirim ke model AI.
- **Open core yang jujur.** Intinya open source. Edisi berbayar (batas tambahan, lisensi, cloud
  terkelola) akan ditawarkan terpisah; tidak ada fitur di inti open source yang sengaja dilumpuhkan.
- **Bahasa Inggris dan Indonesia sejak awal.** Semua teks yang tampil ke pengguna lewat katalog
  terjemahan.

## Edisi (rencana)

| Edisi | Tempat berjalan | Status |
|---|---|---|
| Server | VPS / dedicated Linux, UI lewat browser | direncanakan |
| Desktop | macOS, Linux, Windows (Tauri) | direncanakan |
| Cloud | Dikelola oleh proyek | direncanakan |

## Struktur repositori

```
crates/core         tipe domain bersama: tingkat risiko, penyaring rahasia (tanpa dependensi luar)
crates/i18n         katalog pesan en/id dengan kunci stabil dan fallback
crates/inventory    model inventaris dan kontrak Collector / ParityRule
crates/collectors   kolektor Linux hanya-baca (port mendengar, proses, cron)
crates/parity       aturan kesetaraan PAR-001, PAR-006, PAR-008, PAR-010, pembanding, dan laporan terlokalisasi
crates/cli          `infra collect` dan `infra compare` (offline, hanya-baca)
crates/db           skema SQLite, kueri per organisasi, log audit berantai-hash
crates/proto-server prototipe sementara untuk mengukur ukuran biner, memori, dan waktu mulai
ui/                 halaman prototipe Svelte 5 + Vite (ditanam ke biner prototipe)
docs/design.md      desain teknis: skema, format inventaris, protokol agen, alur persetujuan
docs/decisions/     catatan keputusan arsitektur
docs/market-research.md   riset pasar, fakta dipisahkan dari inferensi
scripts/            skrip bantu (termasuk measure.sh untuk anggaran kinerja)
.claude/ .agents/   konfigurasi dan skill untuk pengembangan dibantu AI
```

## Build dan uji

Membutuhkan toolchain Rust stabil yang baru.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Belum ada yang bisa dijalankan selain pengujian.

## Dokumentasi

- [FAQ](docs/faq.id.md) | [Perbandingan dengan panel lain](docs/comparison.id.md)
- [Desain](docs/design.md) dan [catatan keputusan](docs/decisions/)
- [Kontribusi](CONTRIBUTING.id.md) | [Kode Etik](CODE_OF_CONDUCT.id.md) | [Kebijakan keamanan](SECURITY.id.md)
- [Lisensi](LICENSING.id.md) | [Pendanaan dan dukungan](FUNDING.id.md) | [Changelog](CHANGELOG.md)

Setiap dokumen punya versi Indonesia di sebelahnya (`*.id.md`), kecuali teks lisensi dan changelog
(lihat [LICENSING.id.md](LICENSING.id.md)).

## Lisensi

Sementara [AGPL-3.0-only](LICENSE), menunggu keputusan akhir sebelum rilis publik pertama. Lihat
[LICENSING.id.md](LICENSING.id.md) untuk artinya dan hal yang masih terbuka.
