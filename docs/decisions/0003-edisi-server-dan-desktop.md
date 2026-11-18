# 0003. Dua edisi dari satu inti: server dan aplikasi desktop

- Tanggal: 2026-11-19
- Status: sebagian diterima. **Cangkang desktop: Tauri (dipilih pemilik 2026-11-19).** Bahasa inti: **Rust** (0004). Kerangka UI masih menunggu keputusan.
- Pengambil keputusan: pemilik proyek

## Konteks
Pemilik ingin produk dapat (a) dipasang di server, dan (b) berjalan di desktop (macOS, Windows, Linux) sebagai aplikasi seperti FlyEnv. Pemilik juga ingin yang ringan dijalankan di server, dan mempertanyakan Next.js.

## Keputusan yang diusulkan
1. **Satu inti, dua cangkang.**
   - **Inti (engine)**: satu program tanpa ketergantungan runtime berat, berisi API, penjadwal, kolektor inventaris, pelaksana aksi, dan agen. Dikompilasi untuk Linux, macOS, dan Windows (arm64 dan x86_64).
   - **UI**: satu aplikasi web (SPA) yang dilayani inti. Di server dibuka lewat browser. Di desktop dibungkus cangkang aplikasi (webview) yang menyalakan inti secara lokal.
   - Hasilnya, UI, logika, dan komponen reusable ditulis **sekali**.
2. **Mode desktop = inti lokal + UI.** Aplikasi desktop adalah agen mesin lokal itu sendiri, jadi tidak perlu agen terpisah untuk Mac. Ia bisa mengelola mesin lokal (seperti FlyEnv) dan, bila perlu, terhubung ke server lewat SSH atau agen server.
3. **Cross-platform dirancang sejak awal, dikirim bertahap**: Linux (server) dulu, macOS kedua (dibutuhkan skenario uji pemilik), Windows terakhir.
4. **Abstraksi OS sejak M0**: `ServiceManager`, penyedia jalur/berkas, penyedia paket, dan izin berjalan di balik antarmuka per OS, bukan percabangan kondisi di seluruh kode.
5. **Basis data per edisi lewat satu lapisan akses:** SQLite tertanam untuk desktop dan self-host kecil; PostgreSQL untuk cloud multi-tenant dan instalasi besar. Skema yang sama, satu kumpulan uji.

## Keputusan pemilik: Tauri
Cangkang aplikasi desktop memakai **Tauri**: webview bawaan OS membungkus UI SPA yang sama dengan edisi server. Konsekuensinya:
- Cangkang ditulis dengan Rust (bagian backend Tauri). Inti bisa Rust (satu bahasa untuk inti, agen, dan cangkang) atau Go/lainnya sebagai proses pendamping (sidecar) yang dijalankan Tauri.
- Tampilan webview dapat berbeda antar OS (WebKit di macOS dan Linux, WebView2 di Windows), jadi UI harus diuji di ketiganya dan hindari fitur CSS/JS yang belum didukung WebKit lama.
- Tahap pertama desktop tetap boleh "inti lokal + buka di browser"; cangkang Tauri ditambahkan sesudahnya tanpa mengubah inti dan UI.
- Pembaruan otomatis, penandatanganan, dan notarisasi perlu disiapkan (biaya akun belum diverifikasi).

## Alasan
- Memenuhi keinginan "ringan di server": satu biner dengan UI tertanam, tanpa Node/PHP di server hanya untuk menjalankan panel.
- Mengurangi duplikasi: dua edisi tanpa menulis dua produk.
- Pilihan SPA + inti ringan juga menjawab pertanyaan "selain Next.js": Next.js tidak wajib untuk produk ini karena tidak butuh render sisi server dan SEO untuk panel.

## Risiko dan biaya
- **Matriks uji membesar** (3 OS x 2 arsitektur), terutama Windows (layanan Windows, jalur, izin, tanpa nginx/PHP-FPM bawaan).
- **Mode desktop ala FlyEnv** berarti mengelola dan mendistribusikan biner runtime (PHP, MySQL, nginx, dll) per OS dan versi. Itu pekerjaan besar dan berbeda dari panel server. Perlu dipotong ke ruang lingkup kecil di awal.
- **Penandatanganan dan notarisasi** aplikasi (Apple, Windows) membutuhkan akun dan biaya tahunan; belum saya verifikasi nilainya.
- **Desktop di belakang router tidak bisa dihubungi dari luar**, jadi mengelola server dari desktop memakai koneksi keluar (SSH atau agen), bukan sebaliknya.
- Stack inti belum diputuskan; pilihan bahasa (misalnya Go atau Rust) menentukan seberapa mudah kompilasi lintas OS. Belum saya teliti terhadap kebutuhan ini.

## Alternatif
- Hanya edisi server dulu, desktop belakangan: lebih cepat, tetapi risiko arsitektur kelak sulit mendukung desktop.
- Electron/Next.js sebagai inti: cepat bagi tim web, tetapi berat dan bertentangan dengan keinginan "ringan".
