[English](README.md) | Bahasa Indonesia

<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="brand/nodivara-lockup-dark.png">
  <img src="brand/nodivara-lockup-light.png" alt="Logo Nodivara: pita hijau lime dan perak yang membentuk huruf N, di samping tulisan Nodivara" width="620">
</picture>

### Kendalikan infrastruktur Anda. Percayai setiap operasi.

**AI mengusulkan. Anda menyetujui.** Bandingkan dua server sebelum migrasi: sudah berjalan hari ini, hanya-baca.

[![CI](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/actions/workflows/ci.yml/badge.svg)](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/actions/workflows/ci.yml)
[![Rilis](https://img.shields.io/github/v/release/asiifdev/infra-copilot-ai-server-control-panel?include_prereleases&style=flat-square&labelColor=0d1020&color=b6f205)](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/releases)
![Status: pra-alfa](https://img.shields.io/badge/status-pra--alfa-b6f205?style=flat-square&labelColor=0d1020)
![Lisensi: AGPL-3.0-only, sementara](https://img.shields.io/badge/lisensi-AGPL--3.0--only%20%28sementara%29-b6f205?style=flat-square&labelColor=0d1020)
![Dibangun dengan Rust dan Svelte 5](https://img.shields.io/badge/dibangun%20dengan-Rust%20%2B%20Svelte%205-b6f205?style=flat-square&labelColor=0d1020)
![Bahasa: Inggris dan Indonesia](https://img.shields.io/badge/UI-EN%20%7C%20ID-b6f205?style=flat-square&labelColor=0d1020)

[Unduh](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/releases) · [Yang sudah berjalan](#yang-sudah-berjalan-hari-ini) · [Untuk siapa](#nodivara-untuk-siapa) · [Cara kerjanya](#cara-kerjanya) · [Dibangun untuk dipercaya](#dibangun-untuk-dipercaya) · [Peta jalan](#peta-jalan) · [FAQ](docs/faq.id.md) · [Perbandingan](docs/comparison.id.md)

</div>

---

**Nodivara adalah panel pengelolaan server open source, untuk pekerjaan yang sama dengan aaPanel dan cPanel, dengan agen AI bawaan yang mengusulkan perubahan dan menunggu persetujuan manusia.** Ditulis dengan Rust dan antarmuka Svelte, menyasar server Linux dan aplikasi desktop, dan masih pra-alfa: installer desktop pratinjau tanpa tanda tangan kode dan biner CLI tersedia di [GitHub Releases](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/releases).

Terakhir diperbarui: 2026-11-28.

## Yang sudah berjalan hari ini

> **Status apa adanya.** Installer desktop pra-alfa tanpa tanda tangan kode (`.exe`/`.msi`, `.dmg`, `.AppImage`/`.deb`) dan biner CLI sudah tersedia di [GitHub Releases](https://github.com/asiifdev/infra-copilot-ai-server-control-panel/releases), namun belum ada panel pengelola server produksi penuh (belum ada SSL ACME, multi-versi PHP, atau administrasi DB penuh). Yang ada adalah inventaris hanya-baca (termasuk `vhost`, `cert`, `db`, dan `launchd`) dan pemeriksaan kesetaraan di **11 aturan** (`PAR-001`..`PAR-011`) yang bisa dijalankan dari kode sumber, baris perintah lokal (`infra`) yang menyimpan inventaris, mengelola rencana migrasi (`draft`, `approve`, `run`) serta pengaturan transport (dengan pembaruan kredensial *write-only*), dan berbicara dengan penyedia AI yang Anda atur, UI web lokal (`infra serve`) di atas data yang sama (hanya loopback, satu pengguna, tanpa TLS, tanpa login sungguhan), serta cangkang desktop Tauri v2 tipis (`src-tauri/`, `nodivara-desktop`) yang menanamkan server loopback tersebut pada `127.0.0.1:0`. Usulan yang disetujui (tingkat 2–4, dengan *dry-run*, cadangan `sha256`, konfirmasi ketik untuk tingkat 4, dan *rollback* satu kali) serta rencana migrasi dapat berjalan secara lokal saat `infra serve --enable-execution` diberikan (eksekusi **mati** secara bawaan). Eksekusi jarak jauh (`SshExecutor` berbasis argv `ssh` saja dan API agen *pull-only* `/api/agents/poll` + `/api/agents/result`), penampil log layanan hanya-baca (`GET /api/nodes/{id}/logs`), serta manajemen situs lokal `.test` (`GET/POST /api/sites`) sudah terimplementasi dan teruji; pemantauan uptime, alerting dengan jam tenang, dan bot Telegram interaktif (khusus tingkat 0–2) berjalan di `infra serve`. Biner agen mandiri untuk dipasang di server jarak jauh dan uji hidup ke penyedia AI nyata belum dibangun atau dijalankan.

| Berjalan sekarang (dari kode sumber) | Sudah berupa pustaka, belum terhubung penuh atau masih terbatas | Direncanakan |
|---|---|---|
| Mengumpulkan inventaris Linux/macOS secara hanya-baca: port, proses, cron, PM2, systemd, Docker, `vhost`, `cert`, `db`, `launchd` | Penegakan DNS + `is_internal_ip` untuk infrastruktur bersama sudah ada di `transport::ProcessRunner`, menunggu pelaksana Cloud/jarak jauh | Akses jarak jauh dengan TLS, login sungguhan multi-pengguna, dan aturan dua penyetuju di Cloud |
| Membandingkan dua inventaris dengan **11 aturan kesetaraan** (`PAR-001`..`PAR-011`), laporan dalam bahasa Indonesia atau Inggris | Klien AI OpenAI-compatible (belum pernah dijalankan ke penyedia nyata; diuji dengan tiruan lokal) | Pemasangan biner agen mandiri di server jarak jauh |
| Kode keluar 1 bila ada celah kritis, jadi cocok untuk daftar periksa migrasi | Skema database (versi 14) dengan isolasi tenant, antrean agen *pull-only* (`agent_tokens`/`agent_jobs`), dan log audit berantai hash | Deploy dari Git, DNS, dan Cloudflare Tunnel |
| Menyimpan inventaris dan hasil perbandingan per node di database SQLite lokal (`--save`), dan mencarinya | Cangkang desktop Tauri v2 tipis (`src-tauri/`, server `127.0.0.1:0` tertanam; installer pra-alfa tanpa penandatanganan kode) | Installer desktop bertanda tangan (`.dmg`, `.msi`, `.deb`) |
| Menyusun draf, menyetujui, dan menjalankan rencana migrasi dengan pemeriksaan gerbang (`infra plan`) serta mengelola transport dengan pembaruan kredensial *write-only* (`infra transport`) | | Model AI diuji ke penyedia nyata (Muse, OpenAI, Ollama) |
| Menyetujui Usulan, menjalankan aksi lokal/SSH/antrean agen (tingkat 2–4 dengan `--enable-execution`, *dry-run*, cadangan `sha256`, konfirmasi ketik, *rollback*), melihat log layanan, mengelola situs lokal `.test`, serta menjalankan monitor uptime + alert jam tenang + bot Telegram (tingkat 0–2) | | Fitur panel server produksi M5 penuh: SSL ACME, multi-versi PHP, manajemen database penuh |

### Coba sendiri

```sh
git clone https://github.com/asiifdev/infra-copilot-ai-server-control-panel.git
cd infra-copilot-ai-server-control-panel

cargo run -q -p cli -- collect > source.json        # di server lama (hanya-baca)
cargo run -q -p cli -- collect > target.json        # di server baru
cargo run -q -p cli -- compare source.json target.json --lang id
```

Pakai sebagai daftar periksa migrasi server: jalankan `collect` di mesin lama dan baru, lalu `compare`.

### UI web lokal

`infra serve` menjalankan server web lokal dengan UI yang dibangun dari [`web/`](web) (Svelte 5, Vite, TypeScript, tanpa SvelteKit, tanpa dependensi runtime). Ia butuh database dari `infra init`. Bangun UI lebih dulu agar biner dapat menanamkannya; tanpa itu server menampilkan halaman pengganti.

```sh
./scripts/build-web.sh               # npm ci, pemeriksaan tipe, tes, build produksi ke web/dist
cargo build -p cli                   # menanam web/dist ke biner infra
target/debug/infra serve             # opsional: --port N, --db PATH, --org SLUG (port bawaan 7878, 0 memilih port bebas)
# mencetak:  http://127.0.0.1:7878/#t=<token sekali pakai>   buka di peramban

./scripts/demo-serve.sh --fake-ai    # data demo sementara dan penyedia AI palsu di loopback, data Anda tidak tersentuh
./scripts/smoke-serve.sh             # uji asap biner nyata (yang dijalankan CI)
```

Alamat yang dicetak memasukkan Anda satu kali: token berlaku 10 menit, hanya bisa dipakai sekali, dan berada di fragmen URL sehingga tidak pernah terkirim ke server atau situs lain. Server hanya mengikat `127.0.0.1` dan menolak alamat lain. Belum ada TLS dan kata sandi: sesi berakhir setelah 8 jam tanpa aktivitas, saat keluar, atau saat server dimulai ulang, lalu Anda menjalankan `infra serve` lagi. Halamannya: Ringkasan, Node (dengan inventaris, log layanan, dan rotasi token agen *pull-only*), Temuan, Monitor, Usulan, Asisten, Rencana (teks draf, langkah, gerbang, setujui/jalankan), Transport, Memori, dan Pengaturan (formulir penyedia dengan tombol "Uji koneksi", situs lokal `.test`, saluran notifikasi, jam tenang, bahasa, tema). Kunci penyedia hanya diterima dan disegel di vault; tidak ada respons yang memuat rahasia. Sudah diperiksa di Chromium (terang dan gelap, lebar 375 px, keyboard); **belum** diperiksa di mode kontras tinggi Windows yang asli atau dengan pembaca layar sungguhan. CI memeriksa biner `infra` nyata dengan UI tertanam terhadap `docs/perf-budget.json` (`ui_js_raw_bytes` ~179.794 / 180.000 B, `ui_gzip_bytes` ~61.547 / 64.000 B).

Kode keluar `infra`: 0 berhasil, 1 ada temuan kritis atau cutover terblokir, 2 galat penggunaan, konfigurasi, atau I/O, 3 asisten atau penyedia gagal.

### Baris perintah lokal

Selain perintah berbasis berkas di atas, `infra` menyimpan keadaannya di satu database SQLite lokal (`--db PATH`, lalu `NODIVARA_DB`, lalu direktori data pengguna). Ia bertindak sebagai pemilik satu organisasi lokal: belum ada login. Jalankan perintah tanpa argumen untuk melihat teks penggunaan yang tepat. Contohnya menulis `infra` untuk biner hasil build `crates/cli` (atau `cargo run -q -p cli --`).

```sh
infra init                                                  # membuat organisasi lokal dan pemiliknya
infra node add --name old-vps --kind vps                    # juga: --name new-vps
infra collect --save --node old-vps                         # menyimpan inventaris untuk sebuah node
infra compare --save --source-node old-vps --target-node new-vps
infra plan draft --source old-vps --target new-vps          # hanya draf; juga: plan list | show NAMA | gates NAMA
infra transport add --name backup --kind rclone ... --credentials-stdin
infra transport check backup                                # uji coba: mencetak perintah, rahasia ditutup
infra provider add --name muse --base-url URL --model MODEL --key-stdin
infra ask "port apa saja yang terbuka di server baru?"
infra serve                                                 # UI web lokal di 127.0.0.1, lihat di atas
infra memory pending                                        # catatan usulan asisten; juga: approve ID | delete ID
```

Penyedia pertama yang Anda tambahkan otomatis menjadi bawaan organisasi. Variabel lingkungan: `NODIVARA_DB` (jalur database), `NODIVARA_MASTER_KEY` (kunci yang menyegel rahasia tersimpan, format `<id>:<64 hex>`, misalnya dari `openssl rand -hex 32`), `NODIVARA_PROVIDER_KEY` (kunci API penyedia AI, alternatif `--key-stdin`). Kunci dan kredensial tidak pernah diterima sebagai argumen baris perintah. `infra transport check` tidak menjalankan apa pun kecuali Anda menambah `--run`, dan produk tidak pernah memasang rclone sendiri. Tidak ada `plan approve` atau `plan run`: persetujuan dan eksekusi belum ada.

Keluaran asli, dari data uji anonim di repositori ini (dipersingkat):

```text
6 temuan dari 7 aturan:

[Kritis] PAR-001: Port 7700 tidak punya padanan di target
  Di sumber, searchd mendengarkan di port 7700 (alamat bind 127.0.0.1). Di target tidak ada yang
  mendengarkan di port itu. Jika searchd tidak ikut pindah, apa pun yang bergantung padanya akan gagal ...
  Saran: Periksa apakah searchd memang harus berjalan di target. Jika ya, pasang dan jalankan di sana ...
  (pendengar tcp:7700)

[Penting] PAR-001: Port 6379 tidak punya padanan di target
  ...

Ini perbandingan hanya-baca dari dua inventaris. Ini tidak membuktikan kedua server setara: hanya
aturan berikut yang diperiksa: PAR-001, PAR-002, PAR-006, PAR-007, PAR-008, PAR-009, PAR-010.
```

Laporannya menyebut apa yang **tidak** diperiksa. Itu disengaja: alat kesetaraan yang mengklaim lebih dari yang ia buktikan lebih buruk daripada tidak ada.

## Nodivara untuk siapa?

| Jika Anda | Masalahnya | Yang Anda dapat |
|---|---|---|
| **Developer atau sysadmin** | Satu perintah salah di server produksi lebih mahal daripada waktu yang dihemat asisten. | **Hari ini:** inventaris hanya-baca dan pemeriksaan kesetaraan dari command line, kode sumber terbuka untuk dibaca. **Direncanakan:** agen yang hanya bisa memilih dari daftar aksi tertutup dan butuh persetujuan Anda. |
| **Arsitek IT atau kepala infrastruktur** | Anda butuh bukti sebelum peralihan dan catatan sesudahnya. | **Hari ini:** laporan yang bisa diulang dan menyebut apa yang tidak diperiksa. **Direncanakan:** gerbang persetujuan menurut tingkat risiko dan log audit (skema database untuk keduanya sudah ada). |
| **Agensi, software house, atau pemilik bisnis** | Situs klien rusak setelah dipindah karena satu job atau layanan terlupa. | **Hari ini:** daftar periksa migrasi server yang bisa dijalankan: menemukan yang tertinggal saat migrasi VPS sebelum server lama dimatikan. **Direncanakan:** satu panel untuk banyak server, dengan alert Telegram. |

## Cara kerjanya

<p align="center">
  <img src="docs/assets/operating-loop.svg" alt="Alur kerja Nodivara dalam tujuh langkah: hubungkan, pahami, diagnosis, usulkan, setujui, jalankan, verifikasi. Langkah memahami sudah tersedia; menghubungkan dan diagnosis sedang dibangun; sisanya direncanakan. Menyetujui adalah langkah manusia." width="960">
</p>

*Teks pada diagram berbahasa Inggris. Urutannya: Connect (hubungkan), Understand (pahami), Diagnose (diagnosis), Propose (usulkan), Approve (setujui), Execute (jalankan), Verify (verifikasi). NOW berarti sudah berjalan, BUILDING sedang dibangun, PLANNED direncanakan.*

Agen tidak akan diberi shell. Ia memilih dari **daftar aksi tertutup**, diurutkan dari hanya-baca sampai merusak. Semua yang berada di tingkat 2 ke atas menjadi **Usulan** berisi alasan, target, risiko, dan dampak, lalu menunggu Anda.

| Tingkat | Contoh | Cara berjalan |
|---|---|---|
| 0 Baca | daftar layanan, baca log, kumpulkan inventaris | otomatis; tanpa rahasia |
| 1 Rencana | menyusun rencana migrasi | otomatis; hanya membuat Usulan, tidak mengubah apa pun |
| 2 Ubah ringan | mulai ulang aplikasi non-kritis | Usulan, persetujuan satu klik |
| 3 Ubah berisiko | ganti `.env`, ubah DNS, restore database | Usulan, dry-run, ringkasan dampak, cadangan otomatis; disetujui **hanya di panel** |
| 4 Merusak | hapus data, `DROP`, hapus backup | Usulan, ketik nama target, orang kedua di edisi cloud; **hanya di panel**, tidak pernah otomatis |

*Ini adalah desainnya (lihat [docs/plan.md](docs/plan.md)). Di kode, registri menolak alat di atas tingkat 1 dan alat asisten hanya tingkat 0 dan 1; tingkat 2-4 dan alur persetujuan belum dibangun (M4).*

## Mengapa Nodivara

| | Artinya | Status |
|---|---|---|
| **AI dengan persetujuan** | Agen menunjukkan alasan, target, risiko, dan dampak. Tidak ada yang berjalan sebelum Anda setuju. | direncanakan |
| **Kesetaraan migrasi** | Bandingkan server lama dan baru lalu temukan yang tertinggal: layanan yang tidak tercatat, `localhost` yang menunjuk mesin yang salah, rekaman DNS yang tak pernah pindah. | **berjalan hari ini** (7 aturan, hanya-baca) |
| **Diagnosis berbukti** | AI harus menyebut log, inventaris, dan proses yang mendasari diagnosisnya, dan mengaku bila bukti kurang. | direncanakan |

Nodivara bukan lomba jumlah fitur. Perbedaannya dengan panel lain, dan apa yang **tidak** kami klaim, ada di [perbandingan](docs/comparison.id.md).

## Dibangun untuk dipercaya

Alat yang bisa mengubah server harus mendapat kepercayaan lewat aturan yang bisa Anda periksa. Ini yang sudah ditegakkan di repositori ini, dengan tautan ke kode atau keputusannya:

| Aturan | Di mana berlaku | Keadaan |
|---|---|---|
| Tanpa shell bebas: hanya daftar perintah hanya-baca yang tetap, tanpa shell, ada batas waktu dan ukuran | [`collectors/exec.rs`](crates/collectors/src/exec.rs), [keputusan 0014](docs/decisions/0014-pelaksana-perintah-terbatas.md) | terimplementasi, teruji |
| Rahasia tidak pernah masuk log, laporan, atau masukan AI | [`core/redact.rs`](crates/core/src/redact.rs); nilai variabel lingkungan hanya diurai di memori lalu dibuang ([0015](docs/decisions/0015-env-ref-dan-versi-layanan.md)) | terimplementasi, teruji |
| Teks dari server adalah data, bukan perintah | alert hanya membawa enum tertutup, tidak pernah teks server ([`monitor`](crates/monitor), [`notifier`](crates/notifier)) | terimplementasi, teruji |
| Monitor tidak bisa dijadikan alat SSRF | pemeriksaan alamat tepat di titik connect, batas redirect dan ukuran ([`net`](crates/net), [0016](docs/decisions/0016-klien-http-tls-monitor.md)) | terimplementasi, teruji |
| Symlink yang ditanam tidak bisa membelokkan pembacaan | penelusuran jalur lewat descriptor direktori, perangkat tidak pernah dibuka ([`rootfs.rs`](crates/collectors/src/rootfs.rs), [0017](docs/decisions/0017-rustix-untuk-rootfs.md)) | terimplementasi, teruji |
| Tenant tidak bisa melihat data satu sama lain; pengubahan data terlihat | kueri berbatas tenant dan log audit berantai hash ([`db`](crates/db)) | terimplementasi, teruji |
| Rantai pasok dipantau di setiap push | `cargo deny`, `cargo audit`, pemindaian rahasia seluruh riwayat, tes Linux dan macOS | di [CI](.github/workflows/ci.yml) |
| Batas asisten ada di kode, bukan di prompt | putaran alat, panggilan per putaran, riwayat, dan anggaran token ditegakkan per giliran; satu giliran berjalan per percakapan; teks pengguna diredaksi sebelum disimpan atau dikirim; setiap panggilan alat diaudit; giliran butuh keanggotaan yang masih berlaku ([`agent`](crates/agent), [0023](docs/decisions/0023-putaran-asisten-ai-m2-d.md)) | terimplementasi, teruji |
| AI tidak bisa melewati tingkat 1 | registri menolak alat di atas tingkat 1; `propose_fix`, `remember`, dan `draft_migration_plan` hanya bisa menulis Usulan `pending`, memori `pending`, atau rencana `draft`; tidak ada alat yang bisa menyetujui, menjadwalkan, atau menjalankan apa pun | terimplementasi, teruji |
| Perintah transport tidak membawa rahasia di argumen | rahasia hanya lewat lingkungan atau stdin, daftar opsi rclone yang diizinkan per backend dan memisahkan opsi kredensial dari spesifikasi yang tersimpan, tanpa shell ([`transport`](crates/transport), [0025](docs/decisions/0025-transport-dan-konfigurasi-rclone.md)); pelaksana tidak pernah diarahkan ke host nyata dalam uji | terimplementasi, teruji; belum ada yang dieksekusi |
| Server lama tidak bisa dimatikan terlalu dini | urutan dan gerbang rencana di kode, ditambah pemicu database pada `decommission_source` ([`plan`](crates/plan), [0024](docs/decisions/0024-model-rencana-migrasi-crate-plan.md)) | terimplementasi, teruji; belum ada pelaksana |
| Catatan tulisan asisten tidak bisa mengarahkan giliran berikutnya sendirian | memori yang ditulis asisten mulai `pending` (dijamin pemicu database), tidak pernah ditampilkan ke model atau diindeks sampai orang menyetujuinya dengan `infra memory approve`; kueri pencarian dari model dikutip agar tidak bisa memakai sintaks FTS5 ([0026](docs/decisions/0026-memori-pencarian-fts5-dan-cli.md)) | terimplementasi, teruji |
| Kredensial tidak pernah lewat argumen perintah | CLI membaca kredensial transport dan kunci penyedia hanya dari stdin atau lingkungan, menyegelnya dengan vault, dan `transport check` adalah uji coba kecuali `--run` ([0028](docs/decisions/0028-perintah-plan-dan-transport-di-cli.md)) | terimplementasi, teruji; belum pernah dijalankan ke host nyata |
| Server web lokal tidak bisa dijangkau dari jaringan atau dari situs lain | hanya mengikat loopback (alamat lain ditolak); setiap permintaan harus membawa `Host` yang tepat; perubahan butuh `Origin` yang sama dan header `X-Nodivara`; token masuk sekali pakai di fragmen URL, cookie sesi `HttpOnly` `SameSite=Strict`; header keamanan ketat termasuk Content-Security-Policy; tidak ada endpoint yang mengembalikan rahasia, dan kunci penyedia hanya diterima lalu disegel ([`server`](crates/server), [0029](docs/decisions/0029-server-lokal-ui-web-dan-api.md), [0030](docs/decisions/0030-server-lokal-crate-server-dan-bootstrap.md)) | terimplementasi, teruji lewat soket nyata; belum ada TLS dan login sungguhan, jadi hanya loopback |
| Teks asisten yang mengalir diredaksi seperti teks yang tersimpan | teks baru dilepas di titik potong yang hasil redaksinya sama dengan untuk jawaban utuh, dengan batas tahan 64 KiB ([`stream.rs`](crates/server/src/stream.rs)); penyaring inti juga menutup `password = x` dengan spasi dan token `Bearer` setelah spasi berlebih atau tab | terimplementasi, teruji |
| UI web tidak bisa dijadikan penyuntik skrip | tanpa `{@html}`, `innerHTML`, `eval`, atau skrip dan gaya inline; teks dari server, model, dan alat lewat satu komponen khusus teks ([0031](docs/decisions/0031-ui-web-svelte-tanpa-kerangka-tambahan.md)) | terimplementasi, teruji (296 tes, halaman axe-core dalam dua bahasa dan dua tema) |
| Persetujuan manusia untuk apa pun yang mengubah server | alur Usulan (eksekusi lokal, SSH, dan agen *pull-only* tingkat 2–4 di balik `--enable-execution`, *dry-run*, cadangan `sha256`, konfirmasi ketik tingkat 4, *rollback* satu kali; [0034](docs/decisions/0034-keputusan-usulan-oleh-manusia.md), [0037](docs/decisions/0037-eksekusi-aksi-yang-disetujui.md), [0038](docs/decisions/0038-eksekusi-tingkat-3-4-dan-rollback.md), [0043](docs/decisions/0043-eksekutor-jarak-jauh-dan-panel-dasar-m5.md)) | terimplementasi dan teruji untuk lokal, SSH, dan antrean agen *pull-only*; biner agen mandiri dan mode dua penyetuju di Cloud direncanakan |
| Cangkang desktop tidak bisa membuka IPC luas ke webview | cangkang Tauri v2 tipis (`src-tauri/`) menanamkan `crates/server` pada `127.0.0.1:0` dengan `#t=<token>`, membatasi command lewat `AppManifest::commands`, menyetel `withGlobalTauri: false` & `freezePrototype: true`, dan mengikat `capabilities/main.json` ketat ke `main` ([0044](docs/decisions/0044-cangkang-desktop-tauri-v2-tipis.md)) | terimplementasi, teruji |

Menemukan celah keamanan? Laporkan secara privat: lihat [SECURITY.id.md](SECURITY.id.md).

## Peta jalan

| Tahap | Isi | Status |
|---|---|---|
| M0 Fondasi | tipe domain, i18n, skema database, anggaran ukuran dan kecepatan | selesai |
| M1 Inventaris dan kesetaraan, hanya-baca | kolektor, aturan kesetaraan, CLI, monitor uptime dan alert Telegram | selesai: seluruh 11 aturan kesetaraan (`PAR-001`..`PAR-011`), kolektor `vhost`/`cert`/`db`/`launchd`, CLI, monitor uptime + alert jam tenang (`watch_loop`), dan bot Telegram interaktif (tingkat 0–2, ADR 0040, 0042) |
| M2 Asisten AI hanya-baca | penyedia OpenAI-compatible, chat, memori, alat tingkat 0-1 | sebagian besar selesai sebagai pustaka, baris perintah, dan UI web lokal (`infra serve`, ADR 0029-0031): lapisan alat, vault, penyedia, putaran giliran, memori, pencarian FTS5, `infra ask`, obrolan dengan jawaban mengalir, antarmuka bot Telegram; tersisa: uji ke penyedia nyata, akses jarak jauh dengan TLS, login multi-pengguna |
| M3 Perencana migrasi | wizard, backup dan transport, rencana tertulis | selesai untuk cakupan lokal/pustaka: model rencana (M3-a), spesifikasi transport dan wizard (M3-b), alat perencanaan AI (M3-c), `infra plan` dan `infra transport` (M3-d), pembaruan kredensial *write-only* dan pemeriksaan DNS + `is_internal_ip` di runner (ADR 0039); tersisa: eksekusi ke host jarak jauh nyata |
| M4 Eksekusi dengan persetujuan | tingkat 2-4, dry-run, rollback, verifikasi, audit | berjalan: eksekusi lokal tingkat 2–4 (`--enable-execution`), *dry-run*, cadangan `sha256`, konfirmasi ketik, *rollback* satu kali, persetujuan/eksekusi rencana (ADR 0037–0039), ditambah `SshExecutor` dan API agen *pull-only* (`/api/agents/poll`, `/api/agents/result`, ADR 0043); tersisa: biner agen jarak jauh mandiri dan mode dua penyetuju di Cloud |
| M5 Panel server & cangkang desktop | log layanan, situs lokal `.test`, desktop Tauri v2, panel penuh | fondasi selesai: penampil log layanan hanya-baca, kontrak & UI situs lokal `.test` (ADR 0043), serta cangkang desktop Tauri v2 tipis (`src-tauri/`, ADR 0044); tersisa: panel produksi penuh (SSL ACME, multi-PHP, manajemen DB penuh) dan installer desktop bertanda tangan |
| M6 Komersial | lisensi, cloud terkelola, penagihan | direncanakan |
| M7 Rilis terbuka | pembersihan, dokumentasi, pemeriksaan lisensi dan nama final | direncanakan |

## Prinsip

- **Teringan dan tercepat.** Satu biner Rust, UI Svelte 5 yang ditanam di dalamnya, SQLite secara bawaan. Edisi desktop dengan Tauri. Tanpa runtime tambahan di server hanya untuk menjalankan panel.
- **Hak akses minimal.** Daftar aksi tertutup, diurutkan menurut risiko, dengan persetujuan untuk apa pun yang mengubah sesuatu.
- **Rahasia tetap rahasia.** Tidak pernah dicatat di log, ditampilkan, atau dikirim ke model AI.
- **Open core yang jujur.** Intinya open source. Edisi berbayar ditawarkan terpisah; tidak ada bagian inti yang sengaja dilumpuhkan.
- **Inggris dan Indonesia sejak awal.** Semua teks yang tampil ke pengguna lewat katalog terjemahan.

## Jawaban singkat

**Apakah Nodivara siap dipakai?** Belum. Masih pra-alfa. Inventaris hanya-baca, pemeriksaan kesetaraan, baris perintah lokal, dan UI web lokal (hanya di mesin ini, satu pengguna) bisa dijalankan dari kode sumber; belum ada pemasang, akses jarak jauh, atau panel pengelola server.

**Apakah AI akan mengubah server saya sendiri?** Menurut desain, tidak: ia mengusulkan, Anda menyetujui, dan tingkat paling berisiko hanya bisa disetujui di panel. Asistennya baru berupa pustaka, baris perintah lokal, dan UI web lokal, dan hanya bisa membaca atau menulis draf, jadi saat ini tidak ada yang bisa mengubah server.

**Ditulis dengan apa?** Rust untuk inti dan agen, Svelte 5 untuk antarmuka, SQLite secara bawaan.

Selengkapnya di [FAQ](docs/faq.id.md).

<details>
<summary><b>Struktur repositori</b></summary>

```
crates/core         tipe domain bersama: tingkat risiko, penyaring rahasia (tanpa dependensi luar)
crates/i18n         katalog pesan en/id dengan kunci stabil dan fallback
crates/inventory    model inventaris dan kontrak Collector / ParityRule
crates/collectors   kolektor Linux/macOS hanya-baca (port mendengar, proses, cron, PM2, systemd, Docker, vhost, cert, db, launchd)
crates/parity       seluruh 11 aturan kesetaraan PAR-001..PAR-011, pembanding, dan laporan terlokalisasi
crates/net          klien HTTP/TLS async aman SSRF (pemeriksaan sertifikat) untuk monitor
crates/monitor      mesin insiden uptime (ambang, pemeliharaan, persen uptime), logika murni
crates/notifier     kontrak NotificationChannel, channel kirim Telegram, alert terlokalisasi
crates/watch        perekat: probe lewat net, insiden lewat monitor, alert Telegram lewat notifier (pustaka)
crates/actions      daftar tertutup aksi, tingkat tetap, pelaksana lokal & `SshExecutor` argv-saja di belakang `transport::Runner`, kontrak log layanan & situs `.test` (ADR 0037, 0043)
crates/agent        lapisan alat AI dan putaran giliran asisten: kontrak alat, skema argumen ketat, registri berbatas tingkat, percakapan, registri 15 alat, alat tingkat 1 hanya menulis Usulan pending, memori pending, dan draf rencana (pustaka; belum ada persetujuan atau eksekusi)
crates/vault        penyegelan rahasia AES-256-GCM, rotasi kunci (pustaka)
crates/ai           klien chat OpenAI-compatible: tool call, streaming SSE, retry, pemutus sirkuit (pustaka, belum dijalankan ke penyedia nyata)
crates/plan         model rencana migrasi: spesifikasi ketat, urutan langkah dan gerbang, saran kapasitas, rencana tertulis (pustaka murni)
crates/transport    spesifikasi transport, rencana perintah rclone/rsync/scp/ssh_pipe, pelaksana, wizard (pustaka; tidak ada yang dieksekusi ke host nyata)
crates/cli          baris perintah `infra`: collect, compare, init, node, plan, transport, provider, ask, memory, serve (mode lokal satu pengguna)
crates/bootstrap    pembantu penyiapan yang dipakai CLI dan server: lokasi database, satu organisasi lokal, vault dari lingkungan, penyedia dengan kunci tersegel
crates/server       server web lokal (axum, tokio, rust-embed): API JSON, web/dist tertanam, endpoint agen pull-only, log layanan, situs `.test`, hanya loopback, token masuk sekali pakai, header ketat; dijalankan oleh `infra serve`
crates/db           skema SQLite (versi 14, migrasi 0001-0014), kueri berbatas tenant, pencarian FTS5, antrean agen pull-only, log audit berantai hash
src-tauri/          cangkang desktop Tauri v2 tipis (`nodivara-desktop`): menanamkan `crates/server` pada `127.0.0.1:0`, `AppManifest::commands`, `capabilities/main.json` terikat ke `main` (ADR 0044)
web/                UI web: Svelte 5 + Vite + TypeScript, tanpa SvelteKit; dibangun ke web/dist dan ditanam oleh crates/server
brand/              berkas logo Nodivara (sudah dioptimalkan); warna dan pemakaian di docs/brand.md
docs/design.md      desain teknis: skema, format inventaris, protokol agen, alur persetujuan
docs/decisions/     catatan keputusan arsitektur
docs/market-research.md   riset pasar dengan fakta dipisah dari inferensi
scripts/            skrip bantu: build-web.sh, demo-serve.sh, smoke-serve.sh, fake-openai.py, check-secrets.sh, measure.sh (anggaran kinerja), check-docs.py
.claude/ .agents/   konfigurasi dan skill untuk pengembangan dengan bantuan AI
```

</details>

## Membangun dan menguji

Butuh toolchain Rust stable terbaru.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
./scripts/build-web.sh      # UI web: npm ci, pemeriksaan tipe, tes, build produksi (butuh Node 24 atau lebih baru)
```

Terakhir dijalankan: 914 tes Rust lulus dan 1 diabaikan (uji penyedia hidup) pada `cargo test --workspace`, ditambah 296 tes di `web/`. Bangun `web/` sebelum `cargo build -p cli` (atau `nodivara-desktop`) bila ingin UI ada di dalam biner.

## Dokumentasi

- [FAQ](docs/faq.id.md) | [Perbandingan dengan panel lain](docs/comparison.id.md) | [Merek dan warna](docs/brand.md)
- [Desain](docs/design.md) dan [catatan keputusan](docs/decisions/)
- [Berkontribusi](CONTRIBUTING.id.md) | [Kode Etik](CODE_OF_CONDUCT.id.md) | [Kebijakan keamanan](SECURITY.id.md)
- [Lisensi](LICENSING.id.md) | [Pendanaan dan dukungan](FUNDING.id.md) | [Changelog](CHANGELOG.md)

Setiap dokumen punya versi Inggris di sampingnya, kecuali teks lisensi dan changelog (lihat [LICENSING.id.md](LICENSING.id.md)).

## Lisensi

Sementara [AGPL-3.0-only](LICENSE), menunggu keputusan akhir sebelum rilis publik pertama. Lihat [LICENSING.id.md](LICENSING.id.md) untuk artinya dan yang masih terbuka. Nama "Nodivara" ditetapkan pemilik; pemilik melaporkan nama ini masih tersedia dan unik, dan pencarian merek dagang formal belum dilakukan.
