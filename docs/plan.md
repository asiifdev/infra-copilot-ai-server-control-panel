# Rencana produk: platform migrasi + kelola infrastruktur + asisten AI

Status: rencana awal (2026-11-19); dokumen ini tidak ditulis ulang, status pembangunan ada di `docs/NEXT-STEPS.md` (per 2026-11-20 kode sudah ada: M0, M1-a, M2-a sampai M2-e, M3-a sampai M3-d; belum ada eksekusi atau persetujuan; per 2026-11-24 juga server lokal `infra serve` dan UI web di `web/`, keputusan 0029-0031). Nama produk: **Nodivara** (keputusan 0019); tulisan ini memakai "Produk" sebagai sebutan umum.
Asal ide: sebuah migrasi VPS nyata ke laptop, dan prototipe panel internal yang dibangun saat itu.

## 1. Masalah dan klaim utama

Pindah server itu mudah dimulai dan sulit diselesaikan. Backup dan restore biasanya berhasil. Yang gagal adalah hal yang tidak ada di daftar siapa pun, dan baru ketahuan setelah server lama mati.

Produk ini menjawab satu pertanyaan: **"Apakah server baru benar-benar setara dengan server lama, dan apa yang terlewat?"** Migrasi, backup, dan panel kelola adalah sarananya. Asisten AI membantu menjelaskan selisih dan mengambil keputusan, dengan persetujuan manusia.

Bukti kebutuhan dari migrasi nyata (semuanya lolos dari tes "HTTP 200"):

| Kegagalan nyata | Pelajaran untuk produk |
|---|---|
| Meilisearch tidak ikut pindah (tidak ada di inventaris) | Inventaris harus dari **apa yang berjalan** (proses, port, unit systemd, container), bukan dari daftar yang ditulis tangan |
| `localhost:3307` di dalam container Docker menunjuk ke container, bukan host | Uji koneksi harus dari **sudut pandang proses itu sendiri** |
| Worker membaca Redis :6379, API menulis ke :6380 | Periksa konsistensi antar proses yang berbagi antrean |
| DNS satu domain tidak pernah dipindah, tes lokal tetap "sehat" | Bandingkan **jalur publik**, bukan hanya jalur lokal |
| Database sama nama, isi beda (prod vs dev), data asli ada di salah satunya | Bandingkan **jumlah baris** dan tandai konflik, jangan menimpa |
| Postgres lama ternyata di versi lain, aplikasi menunjuk salinan data uji | Cocokkan versi dan sumber data nyata |
| Aplikasi jalan di mode dev dari terminal terbuka | Deteksi proses yang tidak dikelola supervisor |
| Skema URL rusak karena karakter `@` di password | Validator konfigurasi |
| Domain di akun Cloudflare lain, tidak terjangkau | Pemeriksaan kepemilikan DNS sebelum cutover |
| Cron dan backup lama (aaPanel) tidak punya padanan | Inventaris penjadwal, termasuk cron panel |

Daftar ini jadi bahan **aturan pemeriksaan awal** (bagian 6).

## 2. Model bisnis (open-core, mengikuti pola n8n/FlyEnv)

Tiga cara pakai, satu basis kode:

1. **Self-host gratis** (open source): fitur dasar dan terbatas (jumlah server, jumlah migrasi, 1 organisasi, AI dengan kunci sendiri).
2. **Self-host berlisensi**: buka batas lewat kunci lisensi (tanda tangan Ed25519, terikat instans, boleh offline beberapa hari).
3. **Cloud terkelola (SaaS)**: langsung pakai versi premium, bayar bulanan/tahunan.

Pola lisensi open-core yang sudah dipakai pemilik pada produk lain berlaku di sini juga: repo publik hanya berisi antarmuka `EntitlementProvider`/`LimitsService` dan klien lisensi tipis. Harga, langganan, pembayaran, dan server lisensi di **repo privat**. Tidak memakai pola `ee/` yang membuka kode premium ke publik.

Pilihan lisensi kode publik perlu diputuskan (AGPL, atau lain). Ini keputusan hukum dan bisnis, bukan teknis.

Fitur yang membedakan paket (usulan awal, belum final):

| Kemampuan | Gratis | Berlisensi / Cloud |
|---|---|---|
| Inventaris dan laporan kesetaraan | ya | ya |
| Jumlah server terhubung | 2 | lebih banyak |
| Organisasi dan anggota | 1 org, 1 user | banyak, peran |
| Asisten AI | kunci sendiri, batas pesan | tanpa batas, memori panjang |
| Eksekusi migrasi dengan persetujuan | dasar | alur persetujuan multi-orang, audit lanjutan |
| Penyimpanan perantara terkelola | tidak | ya (Cloud) |
| SSO, retensi audit | tidak | ya |

## 3. Cakupan fungsi

### 3.1 Model migrasi (dipilih pengguna lewat wizard)

- **Sumber dan target:** VPS ke VPS, lokal ke VPS, VPS ke lokal, lokal ke lokal. Setiap titik adalah satu "Node".
- **Cara masuk ke Node:** agen ringan (disarankan), SSH, atau lokal.
- **Lokasi backup:** di server sumber, di mesin lokal pengguna, atau di perantara.
- **Transport:** `rclone` (S3, Google Drive, B2, dll), `rsync`/`scp` langsung, pipa SSH, atau penyimpanan terkelola produk (Cloud).
- **Penanganan konfigurasi rclone otomatis:** wizard membuat remote, menjalankan otorisasi OAuth, menguji koneksi (`rclone about`/`lsd`), mengecek kuota, lalu mengukur kecepatan. Tidak ada kredensial yang dicetak.
- **Strategi data:** snapshot penuh, lalu delta susulan sebelum cutover (seperti `backup delta` semalam).
- **Cutover:** DNS (Cloudflare Tunnel atau ganti A record), dengan snapshot DNS dan rollback.

### 3.2 Inventaris dan kesetaraan (inti pembeda)

Kolektor read-only di Node sumber dan target: layanan, port, proses dan direktori kerjanya, container, cron/timer, database (versi, ukuran, jumlah baris), variabel konfigurasi yang merujuk host/port, vhost, sertifikat, DNS, paket dan versi runtime.
Mesin pembanding menghasilkan temuan berbobot (kritis, penting, info) dengan penjelasan dan saran perbaikan.

### 3.3 Panel kelola

Lanjutan `panel_v2`: layanan, aplikasi, situs (termasuk `.test` + HTTPS lokal), versi runtime, database, backup, log, tunnel. Aksi yang mengubah selalu lewat konfirmasi dan log audit.

### 3.4 Asisten AI (fitur utama)

Bukan chatbot. Ia bertugas **membantu mengambil keputusan** dengan alat nyata dan persetujuan.

- **Penyedia:** OpenAI-compatible (base URL, kunci, model), diatur per pengguna/organisasi. Mendukung model lokal (Ollama dll) dan pengaturan model berbeda per tugas.
- **Function calling:** alat berkelompok menurut tingkat risiko (bagian 5).
- **Memori:** (a) riwayat chat per percakapan, (b) memori organisasi (fakta tetap: "Mac ini server produksi", "domain X milik klien"), (c) memori tugas (rencana migrasi yang sedang berjalan), (d) hasil inventaris sebagai konteks yang dicari (RAG), bukan dimasukkan semua ke prompt.
- **Alur persetujuan:** AI membuat **Usulan** (aksi, alasan, dampak, hasil dry-run). Manusia menyetujui, menolak, atau mengubah. Hanya setelah itu pelaksana menjalankannya, lalu hasilnya diverifikasi dan dicatat.
- **Pembatasan:** AI tidak pernah menerima rahasia mentah (password, token, nilai `.env`). Rahasia dirujuk lewat nama, dan pelaksana menyuntikkannya di sisi server.

## 4. Konfigurasi di database

Semua konfigurasi pengguna ada di database, bukan file. Pengecualian yang harus tetap di lingkungan: URL database, kunci enkripsi utama, dan kunci tanda tangan lisensi.

Entitas utama:

- `organizations`, `users`, `memberships` (peran: owner, admin, operator, viewer)
- `nodes` (jenis, cara masuk, status agen), `node_credentials` (terenkripsi, lihat bagian 7)
- `inventories` dan `inventory_items` (snapshot bertanggal, bisa dibandingkan)
- `findings` (temuan kesetaraan, status: baru, diterima, diperbaiki)
- `migration_plans`, `migration_steps`, `transports` (konfigurasi rclone/scp/rsync per rencana)
- `ai_providers` (per org/user, kunci terenkripsi), `ai_models`
- `conversations`, `messages`, `tool_calls`, `memories` (dengan jenis dan cakupan)
- `proposals`, `approvals` (siapa menyetujui, kapan, dengan apa)
- `audit_log` (tidak bisa diubah, mencatat siapa, apa, di node mana)
- `licenses`, `entitlements`, `usage_counters`

## 5. Tingkat risiko alat AI dan persetujuan

| Tingkat | Contoh | Aturan |
|---|---|---|
| 0 Baca | `list_services`, `read_log`, `collect_inventory`, `diff_inventories` | otomatis, tanpa persetujuan, tanpa rahasia |
| 1 Rencana | `draft_migration_plan`, `propose_fix` | otomatis, hanya membuat Usulan |
| 2 Ubah ringan | mulai ulang aplikasi non-kritis, tambah situs | persetujuan satu klik |
| 3 Ubah berisiko | ganti `.env`, ubah DNS, restore database | persetujuan + dry-run + ringkasan dampak + cadangan otomatis |
| 4 Merusak | hapus data, `DROP`, matikan server, hapus backup | persetujuan + ketik nama target + (Cloud) persetujuan orang kedua. **Tidak pernah otomatis, juga bukan dalam mode "yakin"** |

Aturan lintas tingkat: setiap aksi punya langkah verifikasi sesudahnya, aksi massal dipecah dan tidak boleh satu persetujuan untuk banyak target berbeda, persetujuan hanya berlaku untuk satu Usulan dan satu waktu, dan teks yang dibaca dari server (log, file, balasan) diperlakukan sebagai data, bukan perintah (perlindungan injeksi prompt).

## 6. Aturan pemeriksaan awal (dari migrasi nyata)

1. Proses mendengarkan di sumber tanpa padanan di target (kasus Meilisearch).
2. Konfigurasi di dalam container yang menunjuk `localhost`/`127.0.0.1` ke layanan di host.
3. Producer dan consumer antrean memakai Redis/DB yang berbeda.
4. Hostname di DNS yang tidak mengarah ke target, atau berada di akun DNS lain.
5. Database bernama sama dengan isi berbeda (jumlah baris), dan database yang kosong di sumber tapi terisi di tempat lain.
6. Aplikasi berjalan di luar supervisor atau dengan mode dev di produksi.
7. Variabel lingkungan yang merujuk port/host yang sudah tidak ada.
8. Cron, timer, dan backup terjadwal tanpa padanan, termasuk milik panel hosting.
9. Versi layanan berbeda (database, runtime) yang mengubah format data.
10. Port yang terbuka ke internet di sumber dan di target (keamanan).
11. Uji fungsional: login ke aplikasi dengan kredensial uji, koneksi database dari dalam proses, bukan hanya status HTTP.

## 7. Keamanan (risiko terbesar produk ini)

Produk punya akses mengubah server orang lain. Satu kebocoran bisa fatal, jadi desain mulai dari sini:

- **Agen keluar-saja (pull)**: agen di server pengguna membuka koneksi keluar ke Cloud. Tidak ada port masuk, dan Cloud tidak menyimpan SSH root pengguna. SSH langsung hanya untuk self-host.
- Agen hanya menjalankan **daftar aksi yang diizinkan** (bukan shell bebas), berlaku juga untuk self-host. Mode "shell bebas" ada tapi mati secara bawaan dan selalu lewat persetujuan.
- Kunci dan token terenkripsi (AES-256-GCM), dengan kunci utama di luar database. Tidak ada rahasia dalam log, audit, atau prompt AI.
- Pemisahan tenant ketat di semua query, uji otomatis untuk kebocoran antar organisasi.
- Log audit tidak bisa diubah, dan dapat diekspor.
- Pengelolaan data pelanggan di perantara: enkripsi di sisi klien sebelum unggah (kunci milik pengguna) agar kita tidak membaca isinya.
- Tinjauan pihak ketiga sebelum Cloud dibuka untuk umum.

## 8. Arsitektur dan teknologi (usulan, perlu keputusan Anda)

- **Control plane dan agen (diputuskan, lihat `docs/decisions/0004` dan `0005`):** satu inti Rust ringan dengan UI Svelte 5 yang ditanam ke biner, SQLite tertanam sebagai bawaan dan PostgreSQL untuk cloud. Draf awal di bagian ini yang menyebut NestJS, Next.js, dan Go sudah digantikan keputusan tersebut.
- **Agen:** satu binary kecil (Go atau Rust, usulan Go) untuk macOS, Linux x86/ARM. Tugasnya: kumpulkan inventaris, jalankan aksi dari daftar yang diizinkan, kirim hasil.
- **Lapisan AI:** klien OpenAI-compatible (chat completions + tool calls), abstraksi penyedia, penyimpanan percakapan, pengambilan memori (pgvector).
- **Panel kelola lokal** sekarang (`panel_v2`) menjadi dasar UI/aksi agen, tapi ditulis ulang bersih. Kode migrasi semalam tidak dipublikasikan apa adanya (berisi data pribadi).

## 9. Tahapan

| Tahap | Isi | Nilai | Risiko |
|---|---|---|---|
| M0 Fondasi | repo, multi-tenant, auth, DB, enkripsi rahasia, entitlement kosong | dasar | rendah |
| **M1 Inventaris + kesetaraan (read-only)** | agen, kolektor, pembanding, 11 aturan di bagian 6, laporan | **sudah berguna sendiri, tanpa risiko mengubah server** | rendah |
| **M2 Asisten AI read-only** | penyedia OpenAI-compatible, chat, riwayat, memori, alat tingkat 0–1 | pembeda utama | sedang |
| M3 Perencana migrasi | wizard sumber/target/backup/transport, konfigurasi rclone otomatis, rencana tertulis | inti produk | sedang |
| M4 Eksekusi + persetujuan | alat tingkat 2–4, dry-run, rollback, verifikasi, audit | berisiko tinggi, nilai tinggi | **tinggi** |
| M5 Panel kelola | situs `.test`, versi PHP, layanan dev, DB browser | pengganti FlyEnv | sedang |
| M6 Komersial | lisensi, cloud, tagihan, perantara terkelola | pendapatan | sedang |
| M7 Rilis open source | bersihkan data pribadi, dokumentasi, lisensi, contoh | adopsi | sedang |

**MVP yang disarankan: M0 + M1 + M2.** Ini paling cepat membuktikan nilai (laporan "apa yang terlewat" + asisten yang menjelaskannya), dan tidak ada aksi yang mengubah server, jadi risikonya kecil. Eksekusi (M4) baru dibangun setelah M1–M2 terbukti berguna.

Panel untuk Anda sendiri (M5) bisa jalan paralel dengan rilis awal, karena kebutuhan Anda mencopot FlyEnv tidak boleh menunggu produknya.

## 10. Risiko dan hal yang belum diketahui

- **Pasar:** Coolify, Dokploy, CapRover, Portainer, Cloudron, dan alat migrasi cloud sudah ada. Saya belum meriset apakah ada yang melakukan pemeriksaan kesetaraan pra-cutover. Perlu riset sebelum menetapkan klaim pemasaran.
- **Tanggung jawab hukum:** alat yang bisa menghapus atau mengubah server pelanggan memerlukan syarat layanan, batas tanggung jawab, dan kemungkinan asuransi. Perlu nasihat hukum sebelum Cloud dibuka.
- **Data pelanggan di perantara:** menyimpan backup orang lain menambah kewajiban privasi (UU PDP, GDPR bila pasar global). Pertimbangkan tidak menyimpan sama sekali pada rilis awal.
- **Biaya AI:** siapa yang menanggung token di Cloud (kunci sendiri vs paket).
- **Keandalan AI:** alat tingkat 3–4 tidak boleh bergantung pada penilaian AI saja. Itu sebabnya persetujuan dan dry-run wajib.
- **Beban pengembangan:** ini produk besar. Fokus sempit di awal (M1–M2) adalah mitigasi utamanya.
- **Data pribadi di repo prototipe internal:** prototipe yang menjadi asal ide ini memuat domain, ID akun dan tunnel, jalur kunci server, dan nama klien. Tidak boleh dipublikasikan apa adanya; kode ditulis ulang secara generik.

## 11. Keputusan yang saya butuhkan dari Anda

1. Nama produk dan lokasi repo (repo baru publik + repo privat untuk bagian komersial). Nama: Nodivara (ditetapkan pemilik 2026-11-19, lihat keputusan 0019; pemeriksaan merek belum).
2. Lisensi kode publik (AGPL, atau lain).
3. Stack: sudah diputuskan, Rust + Tauri + Svelte (0004, 0005).
4. MVP: setuju M0 + M1 + M2 dulu, M4 (eksekusi) belakangan?
5. Pasar awal: Indonesia dulu atau langsung global (memengaruhi bahasa, harga, pembayaran)?
6. Untuk migrasi dan panel Anda sendiri: boleh saya terus melanjutkan M5 (situs `.test`, versi PHP) secara paralel?

## 12. Tambahan (2026-11-19): komponen reusable, monitor uptime, notifikasi, DNS dan tunnel

### 12.1 Komponen reusable (wajib)

Aturan, berlaku untuk UI dan backend:
- **Cari dulu, baru buat.** Sebelum menulis komponen/modul baru, periksa yang sudah ada. Tidak ada salin-tempel antar fitur.
- **UI:** satu paket komponen bersama (tombol, tabel data, kartu status, dialog konfirmasi ketik-nama, kartu Usulan, kolom filter, tampilan diff, empty state, toast, penampil log, badge tingkat risiko). Semua halaman memakainya. Token desain (warna, jarak, tipografi, tema terang/gelap) satu tempat.
- **Backend: antarmuka adapter**, bukan kode khusus per layanan. Satu kontrak, banyak implementasi:
  - `NotificationChannel` (Telegram, email, lalu WhatsApp, Slack, Discord, webhook, dst.)
  - `DnsProvider` (Cloudflare dulu, lalu Route53, DigitalOcean, dst.)
  - `Transport` (rclone, rsync, scp, ssh_pipe, managed)
  - `Collector` (inventaris), `ParityRule`, `AiTool`, `ServiceManager` (systemd, launchd, pm2, docker)
  - `AiProvider` (OpenAI-compatible)
- Setiap komponen punya kontrak tertulis (input, output, error), uji kontrak yang sama dijalankan ke semua implementasi, dan contoh pemakaian.
- Menambah channel/provider baru = menulis satu adapter + uji kontrak, tanpa menyentuh inti.
- Komponen UI dan adapter yang dipakai di dua tempat atau lebih tidak boleh diduplikasi; jadi satu sumber.

### 12.2 Monitor uptime

Monitor mencakup **uptime** dan **kejadian error** (lihat 13.4). Dasar awal: pemantauan status aplikasi dan host di Stack Panel yang dipakai sekarang (status online, CPU, memori, uptime, jumlah restart, cek host). Ditambah:
- Jenis pengecekan: HTTP(S) (kode, kata kunci, batas waktu respons), TCP port, ping, DNS, kedaluwarsa sertifikat TLS, dan **heartbeat** (cron/worker melapor, alarm bila berhenti).
- Pemeriksaan dari agen di Node (jalur lokal) dan dari Cloud (jalur publik), sehingga selisih keduanya terlihat (kasus DNS yang belum pindah).
- **Insiden**: pelacakan mulai, selesai, dan lama gangguan; konfirmasi gagal beruntun (N kali) sebelum alarm untuk menghindari alarm palsu; pemberitahuan pulih.
- Jendela pemeliharaan (alarm dibisukan), halaman status publik (opsional, paket berbayar), laporan uptime persen per periode.
- Terhubung ke inventaris: host yang ada di vhost/DNS bisa diusulkan sebagai monitor otomatis.
- Terhubung ke migrasi: selama cutover, monitor membandingkan sumber dan target.

### 12.3 Notifikasi

- Channel awal: **hanya Telegram** (bot interaktif, lihat 13.5). Channel lain (email, WhatsApp, Slack, dst.) menyusul lewat adapter (12.1).
- Pengaturan per organisasi (dan per monitor/aturan): channel mana, severity minimum, jam tenang, dan grup penerima.
- Aturan eskalasi (misalnya ulangi tiap N menit, lalu ke channel kedua), pengelompokan agar tidak banjir, dan batas laju.
- Tombol uji kirim, status pengiriman (terkirim/gagal) dengan riwayat, dan percobaan ulang.
- Kredensial channel (token bot, SMTP) terenkripsi, tidak pernah dikembalikan oleh API.
- Semua modul (monitor, temuan kesetaraan, hasil aksi, Usulan menunggu persetujuan, backup gagal) mengirim lewat **satu mesin alert**, seperti yang sudah ada di monitor sekarang, bukan lewat jalur sendiri-sendiri.
- Notifikasi persetujuan: aturan persetujuan lewat Telegram ada di 13.5 (menggantikan aturan lama "selalu di aplikasi").

### 12.4 Manajemen DNS dan tunnel Cloudflare

- Antarmuka `DnsProvider`; **Cloudflare** pertama: daftar zona dan rekaman, buat/ubah/hapus rekaman, snapshot sebelum perubahan, rollback.
- **Cloudflare Tunnel:** buat tunnel, atur ingress (hostname → layanan lokal), jalankan/pantau konektor, buat rekaman CNAME ke tunnel, uji rekaman sementara lalu hapus.
- Pemeriksaan kepemilikan: hostname di akun lain atau di luar akun terhubung ditandai, disertai petunjuk untuk pemilik domain (rekaman yang harus ditambah).
- Dipakai oleh migrasi (cutover per host yang sehat) dan oleh pengguna biasa (kelola DNS).
- Semua perubahan DNS adalah **aksi tingkat 3** (Usulan + persetujuan + snapshot + rollback). Token API memakai izin minimum, disimpan terenkripsi, dan tidak pernah sampai ke AI.
- Batasan yang harus ditampilkan ke pengguna: streaming video besar lewat proxy Cloudflare dilarang di paket gratis; tunnel ikut mati bila mesin sumber mati.

### 12.5 Katalog fitur (ringkas)

**A. Fondasi:** akun dan organisasi, peran, token API, Node dan agen, enkripsi rahasia, audit log, lisensi dan batas paket.
**B. Inventaris dan kesetaraan:** kolektor, pembanding, 11 aturan awal, temuan dengan status, laporan, uji fungsional.
**C. Migrasi:** wizard sumber/target/backup/transport, konfigurasi rclone otomatis, snapshot dan delta, rencana tertulis, gerbang sebelum mematikan server lama, cutover dan rollback.
**D. Monitor uptime dan notifikasi:** bagian 12.2 dan 12.3.
**E. DNS dan tunnel:** bagian 12.4.
**F. Asisten AI:** penyedia OpenAI-compatible per org/user, chat, riwayat, memori, alat bertingkat, Usulan dan persetujuan, ringkasan temuan dan saran.
**G. Panel kelola (M5):** layanan, aplikasi, situs (termasuk `.test` + HTTPS), versi runtime, database, backup terjadwal, log, konfigurasi.
**H. Backup:** jadwal, tujuan, uji pulih berkala, peringatan bila backup basi.
**I. Komersial:** paket, lisensi, tagihan, cloud terkelola.
**J. Platform:** paket komponen UI bersama, antarmuka adapter, tema terang/gelap, i18n (Indonesia dan Inggris), dokumentasi, rilis open source.

### 12.6 Dampak ke tahapan

| Tahap | Perubahan |
|---|---|
| M0 | tambah paket komponen UI bersama dan kerangka adapter + uji kontrak |
| M1 | tambah monitor uptime dasar (HTTP, TCP, TLS), insiden, penangkapan error, mesin alert + channel Telegram |
| M2 | AI dapat membaca insiden dan temuan, menjelaskan, dan mengusulkan |
| M3 | `DnsProvider` Cloudflare baca-saja (daftar zona/rekaman), pemeriksaan kepemilikan |
| M4 | DNS dan tunnel bisa diubah (tingkat 3), cutover terpandu |
| M5 | panel kelola, channel notifikasi tambahan |

Monitor + notifikasi masuk MVP karena memberi nilai sendiri tanpa risiko mengubah server. Perlu ditinjau ulang bersama cakupan baru di bagian 13.

## 13. Penempatan ulang produk (2026-11-19): panel infrastruktur dengan agen AI bawaan

### 13.1 Arah baru

Produk bukan hanya alat migrasi. Ia adalah **panel pengelolaan server open source**, setara aaPanel dan cPanel versi berbayar, dengan **agen AI yang tertanam mendalam**. Migrasi dan pemeriksaan kesetaraan menjadi salah satu fitur unggulan, bukan satu-satunya klaim.

Rumusan yang disepakati dari percakapan:
- Kombinasi **aaPanel premium** (kuat untuk pengguna semi-lanjut) dan **cPanel premium** (lengkap, nyaman untuk orang awam yang tinggal klik), keduanya digabung dan ditambah AI.
- Open source dengan model open-core (bagian 2): versi gratis tidak dibatasi pada hal yang membuat panel tidak berguna; fitur berat dan pengelolaan banyak server masuk berlisensi atau cloud.
- Pembeda utama: **agen AI bawaan** yang memahami keadaan server (inventaris, log, error, konfigurasi) dan dapat bertindak dengan persetujuan. Pembandingnya adalah memasang agen AI sendiri (misalnya OpenClaw atau Hermes) di server; di sini agen itu terintegrasi dengan sistem, izin, audit, dan antarmuka panel, bukan sekadar skrip terpisah.
- Harga pembanding yang Anda sebut (aaPanel Pro sekitar 200 USD per tahun atau 700 USD seumur hidup; cPanel lebih mahal) **belum saya verifikasi**. Cek halaman harga resmi sebelum dipakai di materi pemasaran.

### 13.2 Dua lapis pengguna

| Lapis | Pengguna | Antarmuka |
|---|---|---|
| Awam | pemilik situs, agensi kecil | alur klik-klik terpandu, bahasa sehari-hari, AI menjelaskan dan mengusulkan |
| Semi-lanjut | pengembang, admin | akses penuh ke layanan, konfigurasi, log, dan CLI/API |

Tugas desain: satu produk, dua kedalaman. Tampilan awam tidak boleh menyembunyikan yang dibutuhkan lapis lanjut, dan sebaliknya.

### 13.3 Cakupan fitur panel (calon, bukan komitmen)

Berdasarkan apa yang dikenal dari aaPanel dan cPanel, **perlu diverifikasi terhadap produk aslinya** sebelum menjadi komitmen:
- **Situs dan aplikasi:** situs PHP/Node/Python/statis, reverse proxy, SSL otomatis (Let's Encrypt), versi runtime per situs, instalasi aplikasi satu klik, deploy dari Git.
- **Database:** MySQL/MariaDB, PostgreSQL, Redis; buat, impor, ekspor, pengguna, browser data.
- **File:** pengelola file, izin, arsip, editor.
- **Layanan sistem:** start/stop/restart, konfigurasi, pemantauan sumber daya.
- **Keamanan:** firewall, fail2ban, pemindai malware, kunci SSH, 2FA, log akses.
- **Backup:** terjadwal ke berbagai tujuan, uji pulih, peringatan basi.
- **Cron dan antrean:** penjadwal dengan log dan peringatan gagal.
- **DNS dan domain:** 12.4.
- **Email hosting** (dalam cPanel): ditunda, kompleks (deliverability, reputasi IP). Keputusan terpisah.
- **Multi-pengguna dan reseller** (dalam cPanel): ditunda ke tahap komersial.
- **Docker dan container.**
- **Migrasi dan kesetaraan:** bagian 3.1 dan 3.2.

Risiko cakupan: ini sangat luas. Tanpa pemotongan tegas, produk tidak akan selesai. Lihat 13.6.

### 13.4 Monitor: uptime dan kejadian error, dengan analisis AI

Monitor tidak hanya "naik atau turun". Ia juga menangkap dan menganalisis **error**:
- **Sumber error:** log aplikasi dan sistem (agen mengekor log yang diizinkan), keluaran proses (PM2/systemd/container), kegagalan cron dan worker, error HTTP 5xx dari pemeriksaan, restart berulang, kehabisan memori/disk. SDK atau webhook opsional untuk aplikasi yang ingin melapor sendiri.
- **Pengelompokan:** error serupa digabung (sidik jari dari pesan yang dinormalkan dan tumpukan panggilan), dengan hitungan, waktu pertama dan terakhir, dan tren.
- **Analisis AI:** untuk kelompok error atau insiden, AI mengumpulkan bukti (log sekitar kejadian, perubahan konfigurasi terbaru, status layanan terkait, hasil inventaris), lalu menyusun: ringkasan, **kemungkinan penyebab** dengan tingkat keyakinan dan bukti yang dikutip, serta **rekomendasi penanganan** bertahap. Rekomendasi yang berupa aksi dibuat sebagai **Usulan** (bagian 5), tidak dijalankan sendiri.
- **Kejujuran analisis:** AI harus menyatakan bila bukti kurang, membedakan fakta dari dugaan, dan tidak mengarang penyebab. Setiap penyebab menyertakan bukti yang bisa dibuka.
- **Redaksi:** log disaring dari rahasia sebelum disimpan dan sebelum dikirim ke model.

### 13.5 Bot Telegram interaktif (channel awal, satu-satunya)

Telegram dipilih karena mendukung perintah, pesan Markdown berformat, tombol aksi (inline keyboard), dan percakapan interaktif.
- **Perintah:** status, daftar masalah, `/restart`, `/log`, `/analisis`, `/bahasa` (atur bahasa bot), `/bisukan`, `/bantuan`, dan sebagainya. Bahasa bot dapat diatur per pengguna.
- **Pesan notifikasi:** ringkas, berformat, dengan tombol aksi langsung (misalnya "Analisis", "Mulai ulang", "Bisukan 1 jam", "Buka di panel").
- **Tanya AI lewat bot:** pengguna bisa bertanya dan memerintah dalam bahasa biasa; jawaban memakai alat yang sama dengan asisten di panel, dengan batas izin yang sama.
- **Keamanan (penting):**
  - Chat dihubungkan ke pengguna lewat kode sekali pakai dari panel; chat yang belum terhubung diabaikan.
  - Tombol memuat token bertanda tangan, sekali pakai, berumur pendek, terikat pada hash isi Usulan dan pengguna yang terhubung.
  - **Aturan persetujuan lewat Telegram (usulan, perlu keputusan Anda):** aksi tingkat 0 dan 1 boleh sepenuhnya lewat bot. Tingkat 2 (ubah ringan, misalnya mulai ulang aplikasi non-kritis) boleh disetujui dengan satu tombol dari chat yang terhubung. **Tingkat 3 dan 4 hanya bisa disetujui di panel** (bot hanya mengirim tautan ke Usulan), karena akun Telegram yang diambil alih tidak boleh sampai bisa mengubah DNS, menghapus data, atau memulihkan database.
  - Grup: perintah yang mengubah hanya dari chat pribadi yang terhubung, bukan dari grup.
  - Token bot terenkripsi, bot hanya mendengarkan lewat webhook atau long-poll milik instans.

### 13.6 Dampak ke prioritas

Cakupan baru jauh lebih besar dari rencana awal. Usulan pemotongan agar tetap terkirim:
1. **Pembeda tetap yang pertama dikirim:** agen AI + monitor error/uptime + bot Telegram + kesetaraan migrasi (read-only). Itu yang tidak dimiliki aaPanel/cPanel.
2. **Fitur panel dasar secukupnya** (situs, SSL, database, layanan, backup, cron, file) menyusul bertahap, dengan prioritas dari apa yang sudah ada di Stack Panel.
3. **Tunda:** email hosting, reseller, multi-pengguna ala cPanel, instalasi aplikasi satu klik, pemindai malware.
4. Tinjau ulang tabel tahapan bagian 9 dan 12.6 setelah keputusan Anda atas bagian 13.6 dan 11.

### 13.7 Pertanyaan baru

1. Urutan prioritas: pembeda AI lebih dulu atau fitur panel dasar lebih dulu?
2. Setuju batas persetujuan Telegram (tingkat 0 sampai 2 lewat bot, tingkat 3 dan 4 hanya di panel)?
3. Apakah email hosting dan fitur reseller ala cPanel benar-benar masuk cakupan, atau ditunda?
4. Target pengguna awal: agensi/pengembang (lapis semi-lanjut) atau orang awam?
5. Panel berjalan di mesin yang dikelola (satu mesin) atau terpusat mengelola banyak server sejak awal?
6. Server MCP (13.8) dibuka kapan: setelah M2 (usulan) atau lebih lambat?

### 13.8 Pembanding: cPanel AI (dibaca dari halaman resmi pada 2026-11-19)

Sumber: halaman produk cPanel AI di situs resmi cPanel. Ini halaman pemasaran, bukan dokumentasi teknis, dan harga fitur AI tidak tertera di halaman itu.

Yang dinyatakan halaman itu:
- **Meridian**: antarmuka pengguna akhir baru (dikirim bersama tema Jupiter mulai cPanel & WHM 138) dengan onboarding terpandu, dasbor "yang perlu perhatian", dan pengelompokan menurut tindakan (situs, email, file, database, keamanan, performa). FAQ menyatakan Meridian termasuk lisensi cPanel standar tanpa biaya tambahan. FAQ **tidak** menjawab apakah semua fitur AI-nya ikut termasuk.
- **AI Assistant** di dalam panel: menjawab pertanyaan rutin (SSL, email, disk, database, setup) dan dapat bertindak atas nama pelanggan, **selalu dengan konfirmasi pelanggan lebih dulu**.
- **AI Builder** (Nova, Nova untuk WordPress, Sitejet AI): membuat situs/aplikasi dari deskripsi.
- **AI App Hosting**: menerbitkan aplikasi buatan alat AI (ChatGPT, Claude, Lovable, Bolt, v0, Replit).
- **MCP**: menghubungkan alat AI pilihan pengguna (misalnya Claude Desktop atau Cursor) ke server hosting dengan izin terbatas (scoped) dan audit trail penuh; dapat menjalankan tugas pengelolaan, memeriksa log, mengelola lingkungan.
- Sasaran pemasaran: penyedia hosting dan pelanggannya (retensi, biaya dukungan, pendapatan baru).

Pelajaran untuk produk ini:
1. **Pola konfirmasi sebelum bertindak dan MCP dengan izin terbatas + audit** sama dengan arah desain kita (bagian 5 dan 7). Itu memvalidasi pendekatannya, dan menunjukkan pasar mengharapkannya.
2. **Tambahkan server MCP sebagai integrasi**: alat AI kita (tingkat 0–2, dan Usulan untuk tingkat 3–4) dapat diekspos lewat MCP dengan token ber-scope. Aksi berisiko tetap lewat Usulan dan persetujuan. Cukup satu adapter di atas registri `AiTool` yang sama (komponen reusable). Perlu keputusan kapan (usulan: setelah M2).
3. **Sudut berbeda:** yang dinyatakan halaman itu berfokus pada pembangunan situs dan dukungan pelanggan di lingkungan penyedia hosting (lisensi cPanel). Halaman itu tidak menyebut analisis penyebab error dari log, pemeriksaan kesetaraan migrasi, atau bot chat interaktif. Itu bisa berarti belum ada, atau hanya tidak dipasarkan di halaman itu. Saya belum memeriksa dokumentasinya.
4. **Open source dan self-host di server mana pun** (bukan terikat lisensi WHM per server) adalah pembeda yang jelas.
5. **UX awam:** pola dasbor "yang perlu perhatian", onboarding terpandu, dan navigasi berdasarkan tindakan layak ditiru secara konsep, tanpa menyalin desain atau teksnya.

Yang masih perlu dicek sebelum memakai perbandingan ini di pemasaran: harga dan paket fitur AI cPanel, dokumentasi MCP-nya, dan apakah ada fitur analisis error atau migrasi yang tidak tampil di halaman pemasaran.

## 14. Keputusan 2026-11-19 dan hasil riset pasar

Lihat `docs/decisions/0002` dan `docs/market-research.md`.

### 14.1 Keputusan
- **Tetap dibangun, pengguna pertama adalah pemilik.** Server baru langsung memakai produk ini; migrasi dari MacBook ke server baru memakai produk ini.
- **Pasar internasional.** Harga USD, pembayaran global (kelayakan penyedia seperti Paddle/Lemon Squeezy/Stripe untuk badan usaha Indonesia **belum diverifikasi**), kepatuhan GDPR di samping UU PDP Indonesia.
- **Bahasa minimal ID/EN** di antarmuka, bot Telegram, notifikasi, dan jawaban AI. Bahasa bawaan Inggris. Tidak ada teks tetap di kode; semua lewat katalog terjemahan (komponen reusable `i18n`).

### 14.2 Skenario uji penerimaan MVP (dogfooding)
Produk ditujukan untuk **server Linux** (VPS/dedicated) sebagai target utama. Mesin lokal (laptop) adalah jenis Node tambahan. Skenario uji pemilik: memigrasikan stack dari MacBook (macOS, Homebrew, launchd, PM2, Docker Desktop) ke server Linux baru, lalu membuktikan kesetaraannya. Ini menambah kebutuhan yang belum ada:
- **Node mesin lokal (macOS)**: dukungan sebagai sumber/target tambahan, bukan fokus produk. Cara aksesnya belum diputuskan: agen tetap (launchd), kolektor sekali jalan (laporan read-only tanpa proses tetap), atau SSH. Untuk migrasi satu kali, kolektor sekali jalan mungkin cukup di MVP. Kolektor untuk Homebrew, launchd, PM2, Docker Desktop.
- **Selisih OS dan arsitektur** sebagai aturan kesetaraan: macOS ke Linux, arm64 ke x86_64 atau sebaliknya (biner, image Docker, emulasi), perbedaan jalur dan sistem berkas (huruf besar/kecil), perbedaan versi runtime.
- **Rujukan ke `localhost`/alamat Mac** dan port dev yang ikut terbawa ke produksi.
- **Server target kosong:** provisioning dasar (paket, runtime, layanan) dari hasil inventaris sumber, dengan Usulan per langkah.
Kriteria sukses: migrasi nyata pemilik selesai tanpa ada yang terlewat yang baru ketahuan setelah cutover.

### 14.3 Posisi berdasarkan riset
- Kategori panel/PaaS self-hosted terbukti besar (Coolify ±66 ribu, Dokploy ±38,6 ribu, 1Panel ±37,5 ribu bintang GitHub per 2026-11-19). Permintaan khusus untuk agen AI pengelola server baru inferensi.
- Petahana sudah menambal sebagian AI/MCP (cPanel v138, Plesk, 1Panel, Dokploy). Belum ada panel umum yang terverifikasi memiliki loop lengkap diagnosis, usulan, persetujuan, eksekusi dengan rollback dan audit.
- **Posisi yang dipegang: agen server yang dapat diaudit**, bukan "panel dengan chatbot". Pembeda yang konkret: pemeriksaan kesetaraan migrasi, aksi dari daftar tertutup dengan persetujuan, dan analisis penyebab error dengan bukti.
- Pembanding terdekat yang harus dipantau: 1Panel (GPL-3.0, Pro $80 per tahun atau $200 seumur hidup), cPanel AI, Dokploy.
- Keamanan adalah risiko dan peluang: panel ber-root sering diretas (cPanel, Webmin, Coolify, 1Panel, HestiaCP). Desain produk tetap: tanpa shell bebas, izin terbatas, audit.

### 14.4 Validasi paralel (tidak menunda pembangunan)
Wawancara 15–20 developer atau agensi yang pernah migrasi server, dan uji waitlist dengan tiga pesan (panel + AI, pemeriksaan kesetaraan migrasi, agen yang aman). Detail ada di bagian penilaian `docs/market-research.md`.

### 14.5 Dampak tahapan
| Tahap | Perubahan |
|---|---|
| M0 | katalog i18n (ID/EN) dan konvensi kunci terjemahan sejak awal |
| M1 | kolektor Linux (utama); dukungan mesin lokal macOS (kolektor sekali jalan atau agen, belum diputuskan); aturan selisih OS/arsitektur |
| M3 | perencana migrasi Mac ke server Linux, provisioning server kosong |
| M7 | dokumentasi publik berbahasa Inggris, nama dan lisensi final |

## 15. Modul deployment ala Vercel (permintaan 2026-11-19)

### 15.1 Tujuan
Selain kelola infrastruktur dan migrasi, panel memudahkan **deployment**: hubungkan repo git, produk mendeteksi stack, lalu satu klik deploy dengan konfigurasi minimal. Mode lanjutan memungkinkan penyesuaian penuh. Karena panel berjalan di dalam server, ia bisa menjalankan perintah build dan layanan langsung di server.

### 15.2 Stack yang didukung (target)
Node (npm/pnpm/yarn: Next.js, Nuxt, Vite, Express, NestJS), PHP (Laravel, Symfony, WordPress, PHP polos), Python (Django, Flask, FastAPI), Go, dan .NET. Daftar bertambah lewat adapter (bagian 12.1), bukan kode khusus per bahasa.
Catatan: pemahaman saya, Vercel mendukung selain Node (misalnya Python dan Go untuk fungsi) tetapi tidak PHP atau .NET secara native. Itu belum saya verifikasi hari ini; jangan dipakai di pemasaran sebelum dicek.

### 15.3 Alur satu klik
1. **Hubungkan git:** GitHub, GitLab, atau URL git biasa. Opsi bawaan: **memakai sesi git yang sudah ada di server** (kredensial git/SSH milik pengguna server), tanpa memasukkan token baru. Opsi lain: OAuth/deploy key per repo.
2. **Deteksi stack** dari berkas penanda: `package.json` (+ lockfile, dependensi framework), `composer.json`, `requirements.txt`/`pyproject.toml`, `go.mod`, `*.csproj`/`*.sln`, `Dockerfile` bila ada. Hasil deteksi berupa **profil build** yang bisa dilihat dan diubah: runtime dan versinya, perintah install, build, start, direktori keluaran, port, jalur kesehatan.
3. **Pratinjau rencana** (apa yang akan dipasang, perintah, port, domain) lalu **Deploy**.
4. Build di server, buat rilis baru, jalankan pemeriksaan kesehatan, **alihkan lalu lintas tanpa henti**, simpan rilis lama untuk rollback.
5. Domain dan SSL otomatis (memakai modul DNS bagian 12.4), variabel lingkungan sebagai rahasia terenkripsi.

### 15.4 Mode lanjutan
Perintah dan versi runtime sendiri, `Dockerfile` atau tanpa container, beberapa proses per proyek (web, worker, scheduler), hook sebelum/sesudah deploy (migrasi, cache), direktori bersama (`storage`, unggahan), pengaturan proxy dan header, batas sumber daya, dan konfigurasi sebagai berkas di repo (`panel.yml`) yang menimpa pengaturan UI.

### 15.5 Fitur turunan
Deploy otomatis dari webhook push, deploy per cabang dan pratinjau, rollback satu klik, log build dan log aplikasi, riwayat deploy, hook pemberitahuan (Telegram), pemantauan kesehatan setelah deploy (terhubung ke monitor), dan **analisis AI atas deploy yang gagal**: AI membaca log build, menjelaskan penyebab, dan mengusulkan perbaikan (Usulan).

### 15.6 Keamanan khusus deployment
- **Kredensial git milik server adalah kredensial ambien.** Mengizinkan panel memakainya berarti panel (dan setiap kode build) bisa memakai akses itu. Aturannya: dipakai hanya untuk clone/fetch repo yang dipilih pengguna; tidak pernah dibaca, ditampilkan, atau dikirim ke AI; build berjalan sebagai pengguna terpisah tanpa akses ke kunci git; rekomendasi utama tetap deploy key per repo, hanya-baca.
- Build menjalankan kode tak tepercaya dari repo (skrip install, `postinstall`, dsb). Jalankan sebagai pengguna tanpa hak istimewa, dengan batas waktu dan sumber daya, dan tanpa rahasia produksi di tahap build kecuali yang diperlukan.
- Deploy yang dipicu **pengguna di UI** berjalan langsung (dengan konfirmasi untuk aksi merusak). Deploy atau perubahan konfigurasi yang diusulkan **AI** selalu lewat Usulan (bagian 5).
- Pemisahan lingkungan (preview, staging, produksi) dan rahasia per lingkungan, supaya satu kesalahan tidak menyentuh produksi.

### 15.7 Dampak ke cakupan dan tahapan
Modul ini besar. Untuk dogfooding, ini kebutuhan nyata begitu server baru berjalan: aplikasi Anda (Laravel, Next.js, Node, Go) perlu dideploy ulang di sana.
| Tahap | Isi |
|---|---|
| M5 (panel kelola) | deteksi stack dan profil build, satu proyek per stack (Node, PHP dulu), git lewat sesi server dan deploy key, rilis dan rollback |
| M5b | Python, Go, .NET, mode lanjutan (`panel.yml`), deploy per cabang, webhook |
| M6 | pratinjau, lingkungan terpisah, analisis AI atas deploy gagal |

Pertanyaan terbuka: build native (systemd/PHP-FPM/PM2) atau container sebagai bawaan? (Anda sebelumnya menolak Docker di mesin lokal, jadi produk perlu mendukung keduanya, bawaan belum diputuskan.) Evaluasi juga memakai ulang penyusun yang sudah ada (misalnya Cloud Native Buildpacks, Nixpacks) dibanding menulis detektor sendiri; belum saya teliti.

## 16. Dua edisi: server dan aplikasi desktop (diusulkan, lihat `docs/decisions/0003`)

- **Edisi server:** dipasang di VPS/dedicated Linux; UI lewat browser; agen/inti ringan.
- **Edisi desktop:** aplikasi untuk macOS, Windows, dan Linux seperti FlyEnv; mengelola mesin lokal dan, bila perlu, terhubung ke server. Aplikasi desktop adalah agen mesin lokal itu sendiri.
- **Satu inti, satu UI:** logika dan komponen reusable ditulis sekali. Detail dan risiko di `0003`.
- **Pengiriman bertahap:** Linux server → macOS → Windows. Arsitektur lintas OS sejak M0 (abstraksi `ServiceManager`, jalur, paket, izin).
- Menjawab sebagian pertanyaan 14.2: pada edisi desktop tidak perlu agen macOS terpisah.
- **Cangkang desktop: Tauri; inti, agen, dan cangkang ditulis dengan Rust** (0003, 0004). Kerangka UI: Svelte 5 + Vite + TypeScript tanpa SvelteKit (diputuskan final; dibangun di `web/`, keputusan 0029-0031).
- Pertanyaan terbuka: seberapa jauh mode desktop menyaingi FlyEnv (mengelola biner runtime lokal) pada MVP; saya sarankan di luar MVP.
