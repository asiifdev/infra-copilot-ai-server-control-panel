# 0016. Klien HTTP/TLS untuk monitor (M1-b)

- Tanggal: 2026-11-19
- Status: diterima (pemilik memilih hyper + tokio-rustls, 2026-11-19). Syarat di bagian "Verifikasi yang masih harus dilakukan" dikerjakan oleh crate `net` dan diperiksa di CI Linux sebelum monitor dipakai; anggaran biner dinaikkan lewat keputusan baru saat klien ini ikut tertanam di server, bukan di crate `net` sendiri.
- Pengambil keputusan: pemilik proyek

## Konteks
Monitor uptime (plan 12.2, 13.4; design 12.1) butuh: pengecekan HTTP(S) GET/HEAD dengan batas waktu dan batas redirect, pengecekan TCP connect, pembacaan masa berlaku dan penerbit sertifikat TLS, dan kelak klien Bot API Telegram (HTTPS JSON, long-poll). Monitor mengambil URL yang diberikan pengguna, jadi ia adalah klien yang bisa disalahgunakan (SSRF, respons raksasa, redirect tanpa akhir). 0007 mencatat `ureq` dulu, `reqwest` bila perlu, tanpa ukuran terukur; 0005 mewajibkan setiap dependensi membenarkan ukuran dan waktu kompilasinya; 0010 mengatakan anggaran dihitung ulang saat klien HTTP masuk.

## Cara mengukur (dan batasannya)
Proyek kecil di direktori scratch (di luar repo, tidak menyentuh workspace), profil rilis sama dengan workspace (`opt-level=3`, LTO thin, `strip`, `panic=abort`, `codegen-units=1`), Rust 1.100.0, macOS arm64 (Apple M1 Pro), 2026-11-19. Setiap biner melakukan satu GET HTTPS ke `example.com` **dan** membaca `notAfter` + penerbit sertifikat daun; hasil dicetak dan diperiksa (status 200, tanggal dan penerbit benar). Biner dasar `println!` = 339.600 B.
- "Delta" = ukuran biner dikurangi biner dasar. "Delta di atas axum" = dibandingkan biner `axum`+`tokio` (fitur seperti 0010, 738.128 B) dengan kode klien yang sama ditambahkan, karena produk sudah memuat `axum` (0007), jadi inilah biaya sebenarnya.
- Jumlah crate = baris unik `cargo tree -e normal` untuk target host (tanpa crate sendiri), termasuk proc-macro.
- Waktu kompilasi = build rilis bersih (`cargo clean`, `--offline`), satu kali, satu mesin; derau beberapa detik.
- RSS = puncak (`/usr/bin/time -l`) proses sekali jalan, **bukan memori diam**. Memori diam monitor yang berjalan terus **tidak diukur**.
- **Tidak diukur:** Linux (CI yang harus melakukannya, acuan utama menurut 0010), waktu per pengecekan, kinerja di bawah beban, biaya `x509-parser` sendirian (ikut di semua varian), `ureq` di bawah `axum` selain angka di bawah.

## Hasil terukur

| Varian | Versi | Biner (B) | Delta (B) | Delta di atas axum (B) | Crate | Build bersih | Puncak RSS | Runtime async |
|---|---|---|---|---|---|---|---|---|
| Dasar | - | 339.600 | 0 | - | 0 | 0 s | 1,8 MB | - |
| **S** `rustls` + `httparse` (HTTP/1.1 tulis sendiri, sinkron) + `x509-parser`, ring | rustls 0.23.46, httparse 1.10.1, x509-parser 0.18.1 | 1.505.216 | +1.165.616 | +1.148.192 | 43 | 14 s | 11,3 MB | tidak |
| **U** `ureq` (fitur `rustls`, ring) + `rustls`/`x509-parser` untuk sertifikat | ureq 3.5.2 | 2.036.864 | +1.697.264 | +1.642.992 | 52 | 14 s | 13,5 MB | tidak |
| **H** `hyper` http1 (`client::conn`) + `tokio-rustls` + `x509-parser`, ring | hyper 1.12.0, hyper-util 0.1.21, tokio-rustls 0.26.6 | 1.800.848 | +1.461.248 | **+1.233.488** | 61 | 14 s | 11,6 MB | ya (tokio, sudah dipakai) |
| R1 `reqwest` async, bawaan (aws-lc-rs + verifier platform) | reqwest 0.13.5 | 3.632.096 | +3.292.496 | - | 83 | 26 s | 10,6 MB | ya |
| R2 `reqwest` blocking, bawaan | 0.13.5 | 3.699.920 | +3.360.320 | - | 85 | 26 s | 11,6 MB | thread + tokio internal |
| R3 `reqwest` async, ring (`rustls-no-provider`) | 0.13.5 | 2.206.000 | +1.866.400 | - | 84 | 16 s | belum dicatat | ya |
| S2 seperti S, akar kepercayaan = `rustls-native-certs` 0.8.5 (bukan `webpki-roots`) | - | 1.508.976 | +1.169.376 | - | 48 | 16 s | belum dicatat | tidak |

Catatan hasil:
- `reqwest` dapat menyerahkan sertifikat peer lewat `tls_info(true)` (terverifikasi: 1002 B DER pada uji). `ureq` **tidak** dibuktikan punya API sertifikat peer (tidak ditemukan di dokumentasi Context7; belum dibuktikan tidak ada), jadi varian U membuka koneksi kedua untuk membaca sertifikat. Varian S dan H membaca sertifikat dari koneksi yang sama dengan permintaan HTTP.
- Semua varian rustls memakai crate `rustls` stabil 0.23.46. `cargo search` menampilkan 0.24.0-dev.3 sebagai terbaru; itu pra-rilis dan tidak dipakai.
- `reqwest` 0.13 menarik `rustls-platform-verifier` dan memakai `aws-lc-rs` secara bawaan; ini yang membuat biner dua kali lebih besar daripada varian ring.

## Lisensi, keamanan rantai pasok
- Semua crate varian S/U/H/R dari `cargo deny check licenses` (dengan `deny.toml` repo): **lolos kecuali satu**, `webpki-roots` 1.0.12 (dan `webpki-root-certs`, ditarik `reqwest`), berlisensi **CDLA-Permissive-2.0**, yang belum ada di daftar `allow`. Pemakaian `rustls-native-certs` (Apache-2.0 OR ISC OR MIT) menghilangkan masalah ini (diuji pada S2: tidak ada lisensi ditolak selain crate scratch yang tanpa lisensi). Alternatifnya, tambahkan CDLA-Permissive-2.0 ke `deny.toml` (butuh persetujuan pemilik dan penelaahan lisensi, `deny.toml` berkomentar bahwa daftar ini harus ditinjau pengacara sebelum rilis).
- `cargo audit` pada varian U dan R1: tidak ada kerentanan terlapor pada basis data 1550 advisori (2026-11-19). Varian lain tidak diaudit terpisah (rustls/ring/x509-parser sama).
- Peringatan `cargo deny`: duplikat `syn` (2 versi) dan `windows-sys` (varian H); level `warn`, tidak menggagalkan.

## Penyedia kripto (ring vs aws-lc-rs)
- `ring` 0.17.14: kode C dan asm dikompilasi lewat crate `cc`; butuh kompiler C di CI (ada di runner Ubuntu dan macOS). Build bersih di macOS lolos tanpa alat tambahan.
- `aws-lc-rs` 1.19.0 / `aws-lc-sys` 0.47.0 (bawaan `reqwest` dan `rustls`): C yang jauh lebih besar; waktu kompilasi 26 s vs 14-16 s dan biner +1,4 MB. Kebutuhan alat tambahan di Linux (mis. cmake atau pustaka lain) **tidak diverifikasi** di sini; wajib diuji di CI Linux sebelum dipilih. Tidak direkomendasikan karena ukuran.
- Keduanya tidak melanggar `unsafe_code = "forbid"` (kode unsafe ada di dependensi, bukan kode kita), tetapi keduanya memperbesar permukaan C yang diaudit pihak ketiga.
- Pilih ring secara eksplisit (`default-features = false`, fitur `ring`), karena rustls 0.23 membawa `aws-lc-rs` bila fitur bawaan tidak dimatikan.

## Pertimbangan SSRF dan DoS (monitor mengambil URL pengguna)
Aturan ini berlaku apa pun klien yang dipilih; yang berbeda adalah seberapa mudah ditegakkan:
1. **Hanya skema `http`/`https`**; tolak yang lain, juga pada setiap lompatan redirect.
2. **Redirect:** batas kecil (usulan 5), diikuti manual, tiap lompatan divalidasi ulang (skema, host, IP). Jangan ikuti downgrade `https` ke `http` kecuali dikonfigurasi. `ureq` punya `max_redirects` dan `https_only`; `reqwest` punya `redirect::Policy`; di S/H kita yang menulis putarannya (±30 baris, mudah diuji).
3. **Blokir rentang privat saat dikonfigurasi** (loopback, link-local termasuk 169.254.169.254, RFC1918, CGNAT, ULA IPv6, multicast). Pemeriksaan harus pada **alamat yang benar-benar dihubungi**, bukan pada nama host, agar tahan DNS rebinding: resolusi sendiri, periksa, lalu `connect` ke `SocketAddr` itu. Di S/H ini langsung (kita memegang `TcpStream::connect`). `ureq` 3 punya trait `Resolver` tetapi berada di modul `unversioned` (API tidak dijamin stabil); `reqwest` punya antarmuka resolver DNS sendiri (tidak diuji di sini). Catatan: monitor produk ini sah memantau layanan internal (host sendiri), jadi blokir adalah **opsi per organisasi**, bawaan aktif di mode cloud multi-tenant (rancangan, belum ada).
4. **Batas ukuran respons:** baca paling banyak N byte (usulan 64 KiB untuk kata kunci; 1 MiB untuk Telegram), lalu berhenti dan jangan terus mengunduh. `ureq` punya `.limit()` pada pembacaan badan (terverifikasi); `http_body_util::Limited` untuk hyper (diuji); `max_response_header_size` bawaan `ureq` 64 KiB. Untuk HEAD tidak ada badan.
5. **Batas waktu menyeluruh:** satu tenggat untuk DNS + connect + TLS + header + badan, bukan hanya per-baca (slowloris). `ureq`: `timeout_global`. Di H: `tokio::time::timeout` membungkus seluruh pengecekan. TCP check: `connect_timeout` saja, tanpa membaca.
6. **Tanpa dekompresi otomatis** (bom gzip) dan tanpa `Authorization`/cookie yang diteruskan lintas host. Kredensial pengguna dirujuk lewat nama (aturan proyek 2).
7. **Konkurensi dibatasi** (semaphore) agar monitor tidak jadi alat pemindai port atau pengirim lalu lintas.
8. Verifikasi sertifikat tidak pernah dimatikan secara diam-diam. Untuk "baca masa berlaku sertifikat yang sudah kedaluwarsa atau tidak tepercaya" (kebutuhan nyata monitor), pakai verifier kustom yang **mencatat hasil verifikasi** sambil tetap menyerahkan sertifikat; jangan memakai `dangerous` tanpa mencatat status galat. Ini **tidak dicoba** dalam pengukuran.
9. Teks dari server adalah data (aturan proyek 4): header dan badan respons tidak pernah dijadikan perintah atau masuk prompt AI tanpa redaksi.

## Keputusan (usulan)
**Varian H:** `hyper` 1.x (hanya `client` + `http1`), `hyper-util` (hanya `tokio`, untuk `TokioIo`), `http-body-util` (`Limited`), `tokio-rustls` 0.26 + `rustls` 0.23 **dengan `ring`, tanpa `aws-lc-rs`**, dan `x509-parser` 0.18 tanpa fitur bawaan. Akar kepercayaan: **`rustls-native-certs`** (dimuat sekali, di-cache), bukan `webpki-roots`. Satu modul "konektor" milik kita menangani resolusi, validasi alamat, `connect` dengan tenggat, jabat tangan TLS, pembacaan sertifikat peer, dan putaran redirect; ia dibungkus antarmuka `Transport`/`Collector` yang sama untuk HTTP check, TLS-expiry check, dan klien Telegram (POST JSON dengan long-poll, tenggat lebih panjang pada `getUpdates`). TCP check memakai `tokio::net::TcpStream` + `time::timeout`, tanpa dependensi baru.

## Alasan
- Produk sudah memuat `tokio` dan `hyper` lewat `axum` (0007). Di atas `axum`, varian H menambah **+1.233.488 B (~1,18 MiB)** dan sekitar +7 s kompilasi bersih (11 s ke 18 s), lebih kecil daripada `ureq` (+1.642.992 B, +8 s) dan jauh lebih kecil daripada `reqwest` bawaan (+3,29 MB di luar `axum`). `reqwest` ring (R3) +1,87 MB di luar `axum`, dengan 84 crate.
- Satu koneksi menghasilkan sertifikat peer dan respons HTTP: pengecekan HTTP dan TLS-expiry bisa berbagi satu jabat tangan, dan pemeriksaan alamat (SSRF) ada tepat di titik `connect`.
- Kode HTTP/1.1 (parsing, chunked, header) tetap milik `hyper` yang diaudit luas. Varian S lebih kecil hanya sekitar 85 KB di atas `axum` tetapi menulis sendiri parser HTTP, dekoder chunked, dan penanganan header; risiko keamanan dan pemeliharaan itu tidak sebanding dengan 85 KB.
- Tanpa `reqwest`, tidak ada `rustls-platform-verifier`, `aws-lc-sys`, atau `security-framework` yang menaikkan ukuran dan area C.
- `rustls-native-certs` memakai penyimpanan sertifikat sistem: apa yang dipercaya host sama dengan yang dipercaya `curl` di host itu (CA internal perusahaan), dan lolos `cargo deny` tanpa mengubah `deny.toml`.

## Dampak terhadap anggaran (`docs/perf-budget.json`)
- Anggaran binary 1.572.864 B (1,5 MiB) dibuat dari prototipe 876-895 KB yang belum punya klien HTTP (0010 menyatakan dihitung ulang saat klien HTTP masuk). Menambah +1.233.488 B pada prototipe 895.024 B memberi sekitar 2.128.512 B (~2,03 MiB) **di atas anggaran**, sebelum SQLite, SSH, dan kode produk lain ikut masuk. Anggaran biner harus dinaikkan lewat keputusan baru (bukan penyuntingan diam-diam, 0005). Angka perlu dihitung ulang dari biner nyata di CI Linux; di sini hanya estimasi penjumlahan, bukan pengukuran biner produk.
- RSS diam (anggaran 8 MiB) dan waktu mulai (100 ms): **belum diukur** untuk monitor yang berjalan. Puncak RSS proses sekali jalan 11,6 MB sudah memuat penyimpanan akar `webpki-roots` yang disalin dan buffer TLS; jangan dibaca sebagai memori diam. Pengukuran diam wajib dilakukan saat modul monitor ada.
- Bundel UI tidak terpengaruh.

## Yang dikorbankan
- Kita menulis dan memelihara sendiri ±150-250 baris: konektor, putaran redirect, penegakan batas respons. Tidak ada pooling koneksi, HTTP/2, proxy dari lingkungan, cookie, atau dekompresi gratis (pengecekan uptime memang sebaiknya membuka koneksi baru; Telegram long-poll dapat memakai koneksi per permintaan).
- Ekosistem `reqwest` yang nyaman (middleware, `.json()`) hilang; `serde_json` dipakai langsung.
- Ukuran: tetap +1,18 MiB di atas `axum`, dan menaikkan anggaran biner.
- Bergantung pada pengelola `hyper-util` untuk `TokioIo` (stabil 0.1.x).
- `rustls-native-certs` bergantung pada keberadaan penyimpanan CA sistem (kontainer minimal tanpa `ca-certificates` gagal-tertutup: perlu galat jelas dan opsi bundel CA sendiri); `webpki-roots` tidak memiliki masalah itu tetapi butuh perubahan `deny.toml`.

## Alternatif yang dipertimbangkan
- **`ureq` 3 (sinkron, di thread):** API lengkap (redirect, tenggat, `.limit()`, `https_only`) dan paling cepat dipakai. Tidak dipilih: +409 KB lebih besar dari H di atas `axum`, tidak menyediakan sertifikat peer yang terbukti (koneksi kedua), hook resolver berada di API `unversioned`, dan menambah model thread paralel di samping `tokio`. Layak sebagai cadangan bila pemilik memprioritaskan kecepatan implementasi.
- **`reqwest` (async/blocking):** terbaik dari sisi kenyamanan dan `tls_info`, tetapi bawaannya menarik `aws-lc-rs` dan `rustls-platform-verifier` (+3,3 MB, 26 s kompilasi, 83-85 crate); varian ring masih +1,87 MB dan 84 crate. Ditolak oleh 0005.
- **Varian S (`rustls` + `httparse`, HTTP tulis sendiri):** terkecil (+1.148.192 B di atas `axum`), 43 crate, tetapi parser HTTP sendiri untuk server tak tepercaya; selisih ukuran terhadap H hanya ~85 KB. Ditolak karena risiko.
- **Pendekatan "native" untuk TLS-expiry:** `native-tls`/`security-framework`/OpenSSL (bukan Rust murni, berbeda per OS, API sertifikat peer terbatas atau memerlukan `openssl` sistem) dan memanggil `openssl s_client` (proses luar, bukan daftar aksi tertutup) ditolak; tidak diukur karena tidak lolos prinsip biner statis (0005). `x509-cert` 0.3.0 (RustCrypto) belum dievaluasi sebagai pengganti `x509-parser` (pertimbangkan bila ukurannya lebih kecil; tidak diukur).
- **Penyedia `aws-lc-rs`:** ditolak karena ukuran dan kebutuhan C yang belum terverifikasi di CI Linux.

## Verifikasi yang masih harus dilakukan sebelum status "diterima"
1. Bangun varian H di CI Linux (ubuntu-latest) dan catat ukuran, waktu, serta alat C yang dibutuhkan `ring`.
2. Ukur memori diam monitor yang berjalan dan waktu mulai, lalu hitung ulang `perf-budget.json`.
3. Uji fungsional konektor: redirect lintas host, loop redirect, respons lebih dari batas, tujuan 127.0.0.1/169.254.169.254 terblokir, DNS rebinding (nama yang beralih IP), sertifikat kedaluwarsa/self-signed tetap terbaca masa berlakunya dengan status verifikasi tercatat.
4. Putuskan sumber akar kepercayaan (`rustls-native-certs` usulan, atau `webpki-roots` + CDLA di `deny.toml`) dan penanganan penyimpanan CA kosong.
5. Verifikasi `cargo deny` dan `cargo audit` pada pohon dependensi nyata di workspace.
6. Perbarui 0007 (baris klien HTTP) bila diterima; dokumen lain mengikuti aturan "dokumen ikut diperbarui".

Data mentah: proyek scratch dan skrip pengukuran ada di direktori scratchpad sesi ini (`.../scratchpad/m1b/`), tidak ikut repo.
