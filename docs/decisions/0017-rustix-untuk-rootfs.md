# 0017. rustix untuk menutup TOCTOU direktori induk di RootFs

- Tanggal: 2026-11-19
- Status: diusulkan
- Pengambil keputusan: pemilik proyek (belum diputuskan; dokumen ini hanya evaluasi)

## Konteks
`crates/collectors/src/rootfs.rs` meresolusi jalur di dalam root secara gaya chroot (komponen demi komponen, `symlink_metadata` + `read_link`), lalu membuka hasilnya dengan `fs::File`. Batas yang diterima (keputusan 0013): proses lain yang bisa **menulis di dalam root saat dibaca** dapat menukar sebuah komponen induk menjadi symlink di antara resolusi dan pembukaan (TOCTOU) sehingga pembukaan keluar dari root. Komponen terakhir sudah ditutup lewat `read_to_string_nolink` (cek setelah buka, dev+inode). Menutup celah induk membutuhkan `openat2(RESOLVE_IN_ROOT)` atau penelusuran `openat` per komponen dari fd direktori, dan keduanya butuh syscall mentah. Aturan ruang kerja `unsafe_code = "forbid"` melarang kita memanggilnya sendiri, dan `std` tidak menyediakan `openat`. Pertanyaan: apakah `rustix` layak sebagai satu-satunya dependensi baru untuk itu (prinsip 0005: teringan dan tercepat, dependensi dibenarkan oleh biayanya).

Semua angka di bawah diukur 2026-11-19 di salinan sementara repo (di luar workspace asli), toolchain `rustc 1.100.0`, profil release proyek (opt-level 3, thin LTO, strip, panic abort), host macOS arm64.

## Keputusan
(Usulan) Tambahkan `rustix` ke crate `collectors` saja, dengan fitur minimum, dan tulis ulang internal `RootFs` di atas fd direktori. API publik tidak berubah.

## Fakta hasil evaluasi

**Versi.** `cargo info rustix` -> **1.1.6** (stable terbaru; MSRV 1.65, jauh di bawah toolchain kita). Dokumentasi diperiksa lewat Context7 (`/websites/rs_rustix`, ditambah `/bytecodealliance/rustix` yang indeksnya masih v1.1.5): `fs::openat2(dirfd, path, OFlags, Mode, ResolveFlags)` dengan `ResolveFlags::IN_ROOT`, `NO_MAGICLINKS`, `BENEATH`, `NO_SYMLINKS` hanya Linux dan butuh fitur `fs`; `openat`, `OFlags::{NOFOLLOW, DIRECTORY, NONBLOCK, CLOEXEC, PATH}` tersedia, `PATH` hanya Linux.

**Fitur minimum.** `default-features = false, features = ["std", "fs"]`.
- `fs` memberi `openat`, `openat2`, `statat`, `readlinkat`, `fstat`, `Dir`.
- `std` perlu agar `rustix::io::Errno` dapat diubah menjadi `std::io::Error` dan `OwnedFd` dapat menjadi `File`; `std` menarik `alloc` (dibutuhkan `readlinkat` yang mengembalikan `CString` dan `Dir`). Tanpa `std`/`alloc` tidak bisa dipakai untuk kebutuhan ini.
- Tidak perlu `use-libc`, `linux_*`, `all-apis`, `net`, `process`, dll. Fitur `linux_5_11`/`linux_latest` hanya menaikkan kepastian ketersediaan syscall pada waktu kompilasi; tidak dipakai agar `openat2` tetap dideteksi saat jalan.

**Crate transitif yang masuk ke `Cargo.lock`:** hanya **2** crate baru: `rustix 1.1.6` dan `linux-raw-sys 0.12.1`. `bitflags 2.13.2`, `errno 0.3.15` dan `libc 0.2.191` sudah ada di kunci karena `proto-server` (tokio). Untuk pohon dependensi `cli`:

| Target | Crate baru di pohon `cli` |
|---|---|
| macOS (backend libc) | rustix, bitflags, errno, libc |
| Linux x86_64/aarch64/arm/x86/riscv64 (backend `linux_raw`, tanpa libc) | rustix, bitflags, linux-raw-sys |

Di Linux backend `linux_raw` memanggil syscall lewat assembly inline, sehingga binari tidak butuh `libc` untuk bagian ini. Pada arsitektur selain daftar di atas (mis. mips, ppc64 tanpa flag eksperimental) rustix otomatis memakai backend libc.

**Lisensi vs `deny.toml`.** rustix dan linux-raw-sys: `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`; bitflags, errno, libc: `MIT OR Apache-2.0`. Semuanya sudah ada di `allow`. Tidak ada perubahan `deny.toml`.

**cargo deny / audit.** Dijalankan di salinan sementara dengan rustix terpasang: `cargo deny check` -> `advisories ok, bans ok, licenses ok, sources ok`; `cargo audit` memindai 91 crate tanpa temuan. `[bans] multiple-versions = "warn"` tidak dipicu oleh dependensi baru (libc 0.2.178 vs 0.2.191 yang berganda sudah ada sebelumnya dan hanya peringatan). `wildcards = "deny"` terpenuhi dengan versi eksplisit `"1.1.6"`. **Tidak ada yang memblokir di sisi deny/audit.**

**Kode C / build script.** Tidak ada kode C dan tidak ada kompilasi `cc`. rustix punya `build.rs` (281 baris, murni Rust) yang hanya mendeteksi fitur kompiler dan memilih backend; ia menjalankan `rustc` sebagai proses untuk memeriksa dukungan fitur (kecil, tapi tetap eksekusi build script, sama jenisnya dengan `serde`/`libc` yang sudah ada). `linux-raw-sys` berisi `build = false` (hanya konstanta dan tipe). Waktu bangun release `cli` dari bersih bertambah sekitar 7 detik di mesin ini (rustix + 3 crate kecil).

**Ukuran biner terukur (crate `cli`, biner `infra`, release, macOS arm64).**

| | Byte |
|---|---|
| Sebelum | 817.792 |
| Sesudah (prototipe: penelusuran fd per komponen dengan `statat`/`readlinkat`/`openat`, `Dir`, `openat2` di balik `cfg(linux)`) | 835.056 |
| Selisih | **+17.264 byte (+16,9 KiB, +2,1%)** |

Catatan: sisi `cfg(target_os = "linux")` (jalur `openat2`) tidak ikut terkompilasi di Mac, dan tidak ada target lintas Linux terpasang (tidak ada `rustup`), jadi **ukuran di Linux belum terukur**. Perkiraan: backend `linux_raw` memanggil syscall langsung dan biasanya lebih kecil dari backend libc, jadi selisih tidak diharapkan lebih besar; harus diukur di CI Linux sebelum menetapkan anggaran. Anggaran biner di `docs/perf-budget.json` (1.572.864 byte) berlaku untuk `proto-server`, bukan `cli`; `scripts/measure.sh` hanya membangun `proto-server`, jadi tidak ada gerbang CI yang terpengaruh. Memori diam tidak berubah berarti (tanpa alokasi global atau thread baru). Prototipe tersebut tidak dimasukkan ke repo; hanya angka ukurannya yang dipakai.

## Perilaku Linux dan macOS

| | Linux | macOS / lainnya |
|---|---|---|
| Jalur cepat | `openat2(root_fd, rel, RDONLY\|NONBLOCK\|CLOEXEC, RESOLVE_IN_ROOT\|NO_MAGICLINKS)`: kernel meresolusi atomik, `/` dan `..` tidak pernah keluar dari root, symlink absolut dibaca relatif terhadap root (persis semantik chroot yang sudah kita jelaskan) | tidak ada (`openat2` hanya Linux) |
| Jalur cadangan | penelusuran `openat` per komponen; dipakai bila `openat2` gagal dengan `ENOSYS` (kernel < 5.6), atau `EPERM` (profil seccomp kontainer lama memblokirnya) | satu-satunya jalur: penelusuran `openat` per komponen |
| Direktori | `O_DIRECTORY\|O_NOFOLLOW\|O_CLOEXEC` (`O_PATH` boleh dipakai untuk komponen perantara) | `O_DIRECTORY\|O_NOFOLLOW\|O_CLOEXEC` (tidak ada `O_PATH`) |
| Symlink | `readlinkat`, diperluas di tumpukan langkah seperti sekarang | sama |

Jalur cadangan lebih lemah dari `openat2` dalam satu hal: `..` dalam target symlink dikerjakan di sisi pengguna lewat tumpukan fd (tidak pernah memanggil `openat(.., "..")`, jadi tidak bisa keluar root), tetapi setiap komponen dibuka dengan `O_NOFOLLOW` sehingga penukaran menjadi symlink saat berjalan hanya menghasilkan `ELOOP` (gagal tertutup, dilaporkan `FsError::SymlinkLoop`/`WrongType`), tidak pernah mengikuti keluar. Dengan fd direktori yang dipegang, penukaran **direktori induk** oleh pihak lain tidak lagi memengaruhi: fd menunjuk inode yang sudah dibuka. Sisa risiko di kedua jalur: komponen yang diganti dengan direktori lain yang sah di dalam root (itu tetap di dalam root, bukan pelarian). `openat2` dapat mengembalikan `EAGAIN` saat balapan rename/mount: ulangi beberapa kali lalu gagal. Hanya jalur macOS/cadangan yang dapat diuji lokal; jalur `openat2` diuji di CI Linux (uji perlu dibuat untuk kedua jalur dengan pemilih paksa).

## Pengaruh pada `read_to_string_nolink` (cek-setelah-buka)
- Komponen terakhir dibuka dengan `O_NOFOLLOW`: symlink menghasilkan `ELOOP` secara atomik di kernel. `lstat` sebelum + `lstat` sesudah + pembandingan dev/inode (`same_file`) dan fungsi `open_nonblocking` dengan konstanta `O_NONBLOCK` hasil tabel per OS **dapat dihapus**: `OFlags::NONBLOCK` dari rustix benar di semua platform, sehingga FIFO terplanting tidak memblokir pembukaan di platform mana pun (hari ini di platform di luar tabel itu hanya pengecekan metadata yang melindungi).
- Yang tetap: `fstat` pada fd terbuka untuk memastikan berkas biasa (`is_file`), batas byte, dan pemetaan galat. `ELOOP` pada komponen terakhir dipetakan ke `FsError::WrongType` untuk `nolink` (perilaku saat ini) dan ke `SymlinkLoop` hanya bila dari rantai symlink yang kita ikuti sendiri.
- `read_to_string` (mengikuti symlink) memakai penelusuran/`openat2` yang sama dengan komponen terakhir ikut diperluas.

## Sketsa desain RootFs baru
API publik tetap: `read_to_string`, `read_to_string_nolink`, `read_dir`, `read_link`, `validate`, `new`, `host`; `FsError`, `FileText`, `DirNames` tidak berubah.

1. `RootFs` menyimpan `base: PathBuf` (tetap, untuk `PartialEq`/`Debug`). Fd akar dibuka **per panggilan** (`openat(CWD, base, RDONLY|DIRECTORY|CLOEXEC)`), bukan disimpan, supaya `RootFs` tetap `Clone + Eq` dan tanpa status; biayanya satu syscall. (Alternatif: `Arc<OwnedFd>`; ditunda sampai ada ukuran yang membenarkan.)
2. Validasi masukan sama seperti sekarang (`/` di awal, tanpa NUL, tanpa `..`).
3. `open_in_root(root, rel, mode)`: mode = `Final::Follow | Final::NoFollow`, hasil `OwnedFd`.
   - Linux: coba `openat2(... IN_ROOT | NO_MAGICLINKS)` (untuk `NoFollow` cukup menambah `O_NOFOLLOW` pada flag buka, yang hanya menolak symlink pada komponen terakhir; `NO_SYMLINKS` tidak dipakai karena menolak symlink di seluruh jalur). Pada `ENOSYS`/`EPERM` simpan hasil di `static AtomicU8` (belum diketahui / ada / tidak ada) dan pakai penelusuran.
   - Penelusuran: tumpukan `Vec<OwnedFd>` (fd akar paling bawah) + tumpukan nama tertunda seperti `Step` sekarang; per komponen `statat(AT_SYMLINK_NOFOLLOW)`; bila symlink dan boleh diikuti: `readlinkat`, hitung batas `MAX_SYMLINKS = 40`, symlink absolut memotong tumpukan ke akar, `..` mem-pop tumpukan (tidak pernah di bawah akar); selain itu `openat` dengan `DIRECTORY|NOFOLLOW` (komponen tengah) atau `NOFOLLOW|NONBLOCK` (terakhir). Komponen terakhir `read_link` memakai `readlinkat` pada fd induk dan tidak pernah membukanya.
4. `read_file(fd)`: `fstat` -> harus berkas biasa; `File::from(fd).take(max+1).read_to_end` seperti sekarang.
5. `read_dir`: buka direktori lewat `open_in_root`, `Dir::read_from(fd)`, lewati `.`/`..`, urut dan batasi seperti sekarang (hitungan `non_utf8` tetap).
6. `validate`: `fstat` pada fd akar; tanpa perubahan semantik.
7. Galat: `Errno` -> `io::Error` -> `FsError` lewat `From<&io::Error>` yang ada; tambah pemetaan `ELOOP` seperti di atas.
8. Uji: semua uji fixture yang ada tetap; tambah uji penukaran (thread yang terus mengganti direktori induk dengan symlink keluar-root sementara pembaca berjalan) yang harus gagal tertutup, dijalankan pada kedua jalur; uji kontrak bersama untuk "jalur cadangan == `openat2`" pada pohon fixture yang sama (di CI Linux).
9. Dokumen modul: ganti paragraf "Limit (accepted)" dengan penjelasan jaminan baru dan satu sisa batas (root itu sendiri dan `/proc` yang dikelola kernel).

## Alasan
- Satu-satunya cara menutup celah induk tanpa `unsafe` di kode kita; `unsafe` ada di dalam rustix (assembly syscall) yang dipakai luas (setara dengan mempercayai `std`), kita tetap `forbid(unsafe_code)`.
- Biaya kecil dan terukur: +16,9 KiB (+2,1%) di Mac, 2 crate baru di kunci, tanpa C, lisensi semua sudah diizinkan, deny/audit bersih.
- Menyederhanakan kode: menghapus tabel `O_NONBLOCK` per OS dan cek-setelah-buka yang rumit; fd tetap menjaga identitas.

## Alternatif yang dipertimbangkan
- **Tetap seperti sekarang (batas diterima).** Nol biaya; risiko tersisa hanya bila root dapat ditulis penyerang saat pengumpulan. Valid sebagai opsi bila MVP hanya mengumpulkan dari `/` hidup, disk lama, atau snapshot.
- **`libc` langsung.** Butuh `unsafe` di kode kita: dilarang aturan ruang kerja.
- **`nix`.** API lebih besar, bergantung `libc` dan `cfg-if`, tidak punya `openat2`; lebih berat dan kurang lengkap.
- **`cap-std` / `cap-primitives`.** Sandboxing sesuai tetapi menarik banyak crate (`io-lifetimes`, `ambient-authority`, dll.) dan dibangun di atas rustix juga; terlalu berat untuk kebutuhan ini (0005).
- **`chroot`/`unshare` proses pembantu.** Butuh hak istimewa dan proses tambahan; bertentangan dengan prinsip read-only tanpa hak khusus.

## Dampak
- `crates/collectors/Cargo.toml` (+1 baris), `Cargo.lock` (+`rustix`, +`linux-raw-sys`), `rootfs.rs` ditulis ulang di dalam (API sama; kira-kira satu berkas, tidak ada perubahan di kolektor lain).
- Dokumen yang harus ikut: komentar modul `rootfs.rs`, keputusan 0013 (catat bahwa batas TOCTOU induk ditutup oleh 0017), `CHANGELOG.md` (`[Unreleased]`), README (struktur/status, dua bahasa), `docs/NEXT-STEPS.md`; jalankan `python3 scripts/check-docs.py`.
- Perlu diverifikasi sebelum berstatus "diterima": ukuran biner di Linux (CI), perilaku `openat2` di kontainer pengguna (seccomp), dan uji penukaran di kedua jalur.
- Menjadi lebih sulit: dua jalur kode di Linux (cepat dan cadangan) yang harus diuji bersama; satu dependensi yang perlu dinaikkan versinya mengikuti aturan "versi stable terbaru".
- Keputusan lisensi/anggaran biner `cli` belum ada; bila diterima, tambahkan anggaran `cli` ke `docs/perf-budget.json` melalui keputusan tertulis (0005, 0010).
