# 0041. Kompatibilitas Windows (`x86_64-pc-windows-msvc`) dan alur rilis GitHub (Linux, macOS, Windows)

- Tanggal: 2026-11-27
- Status: diterima (persiapan distribusi biner lintas platform)
- Pengambil keputusan: pemilik proyek

## Konteks
Biner `infra` (yang menanamkan UI web `web/dist` di `crates/server`) selama ini hanya dibangun dan diuji di Linux (`ubuntu-latest`) dan macOS (`macos-latest`). Ketergantungan langsung `crates/collectors` pada `rustix` dan modul `std::os::unix` di `rootfs.rs` (serta beberapa berkas uji) membuat kompilasi pada target Windows (`x86_64-pc-windows-msvc`) gagal, padahal biner CLI dan UI web lokal `infra serve` dibutuhkan oleh operator di Windows juga.

Selain itu, repositori belum memiliki workflow rilis otomatis di GitHub Actions untuk menghasilkan artefak biner resmi lintas platform (Linux, macOS, Windows) beserta daftar checksum `SHA256SUMS.txt`.

## Keputusan
1. **Kompatibilitas kompilasi Windows (`x86_64-pc-windows-msvc`)**:
   - **`crates/collectors`**: `rustix` dipindahkan ke `[target.'cfg(unix)'.dependencies]` pada `Cargo.toml`. Implementasi traversal deskriptor aman (`openat`/`openat2`, `O_NOFOLLOW`) di `crates/collectors/src/rootfs.rs` dipertahankan di bawah `#[cfg(unix)]` (0017), sedangkan pada `#[cfg(not(unix))]` disediakan jalur non-Unix yang aman dan eksplisit (kolektor Linux tetap ditujukan untuk host Linux, sehingga jalur non-Unix menolak atau mengembalikan ketidaksediaan tanpa menyentuh `std::os::unix`).
   - **Pengujian lintas platform**: seluruh impor `std::os::unix` pada berkas uji (`crates/collectors/tests/rootfs.rs`, `collect.rs`, dan sejenisnya) dibatasi dengan `#[cfg(unix)]` atau `#[cfg(target_os = "linux")]` seperti pola yang sudah dipakai di `crates/bootstrap` dan `crates/server`.
   - **CI (`ci.yml`)**: matrix job `rust` dapat mencakup `windows-latest` untuk `cargo clippy` dan `cargo test --workspace --locked`, sementara job `perf-budget` (`scripts/measure.sh` yang memakai `bash` dan `ps`) tetap berjalan di `ubuntu-latest` dan `macos-latest`.

2. **Alur rilis GitHub (`.github/workflows/release.yml`)**:
   - **Pemicu**: tag versi `v*` (serta opsional `workflow_dispatch` untuk uji coba).
   - **Prinsip hak akses minimal & keamanan suplai**:
     - `permissions: contents: read` ditetapkan di tingkat workflow; **hanya job `publish`** yang memiliki `permissions: contents: write` untuk membuat GitHub Release dan mengunggah aset.
     - Seluruh action eksternal (`actions/checkout`, `actions/setup-node`, `actions/upload-artifact`, `actions/download-artifact`, dan action publikasi rilis) **wajib di-pin ke SHA commit penuh**, konsisten dengan `.github/workflows/ci.yml`.
   - **Matrix build (Linux, macOS, Windows)**:
     - Target awal: `ubuntu-latest` (`x86_64-unknown-linux-gnu`), `macos-latest` (`aarch64-apple-darwin`), dan `windows-latest` (`x86_64-pc-windows-msvc`).
     - Urutan build tiap target: membangun aset UI (`cd web && npm ci --no-audit --no-fund && npm run build`) terlebih dahulu agar `web/dist` tertanam ke dalam biner, lalu `cargo build --release --locked -p cli`.
     - Format arsip: `.tar.gz` untuk Linux dan macOS (`infra`), serta `.zip` untuk Windows (`infra.exe`).
   - **Job `publish`**: mengunduh seluruh arsip hasil matrix build, menghasilkan berkas `SHA256SUMS.txt` untuk verifikasi integritas, dan menerbitkan GitHub Release beserta seluruh arsip dan `SHA256SUMS.txt`.

## Belum
- Penandatanganan kode (*code signing*) biner macOS (Apple Developer ID / notarization) dan Windows (Authenticode), serta paket installer (`.deb`, `.rpm`, `.msi`, Homebrew tap) belum disertakan pada tahap ini.
- Target Linux ARM64 (`aarch64-unknown-linux-gnu`) dan macOS Intel (`x86_64-apple-darwin`) dapat ditambahkan ke matrix bila runner atau *cross-compilation* disiapkan.

## Alternatif
- **Menjaga kompilasi hanya untuk Unix (Linux/macOS)**: ditolak; menghalangi pengguna Windows menjalankan `infra serve` dan CLI lokal secara native.
- **Memberi `permissions: contents: write` di tingkat atas workflow rilis**: ditolak; melanggar prinsip hak akses minimal (*least privilege*) bila job build mengunduh dependensi pihak ketiga.
- **Menggunakan tag versi mengambang (`@v4`) alih-alih pinned SHA di `release.yml`**: ditolak; tidak selaras dengan kebijakan keamanan `ci.yml`.
