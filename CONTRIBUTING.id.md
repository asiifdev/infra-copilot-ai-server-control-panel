[English](CONTRIBUTING.md) | Bahasa Indonesia

# Berkontribusi ke Infra Copilot

Terima kasih atas minat Anda. Proyek ini masih **pra-alfa**: desainnya masih bergerak, jadi mohon
buka issue untuk membahas hal yang tidak sepele sebelum menulis kode.

Dengan ikut serta, Anda setuju mengikuti [Kode Etik](CODE_OF_CONDUCT.id.md). Masalah keamanan
**tidak** dilaporkan di issue publik: lihat [SECURITY.id.md](SECURITY.id.md).

## Aturan dasar

Berasal dari prinsip proyek dan tidak bisa ditawar dalam pull request.

1. **Keamanan dulu.** Produk ini bisa mengubah server orang lain. Agen menjalankan daftar aksi
   yang tertutup, tidak pernah shell bebas. Semua yang mengubah atau menghapus sesuatu melalui usulan
   dan persetujuan manusia. Jangan menambah jalan pintas yang melewatinya.
2. **Rahasia tidak pernah keluar dari tempatnya.** Tidak boleh muncul di log, pesan galat, jejak
   audit, tes, fixture, atau prompt yang dikirim ke model AI. Pakai penyaringan di `crates/core`.
3. **Teringan dan tercepat.** Pilih dependensi sesedikit mungkin dan ukur, jangan menebak.
   Dependensi baru harus membenarkan biayanya terhadap ukuran biner, waktu mulai, dan waktu
   kompilasi.
4. **Komponen reusable.** Cari komponen atau adapter yang sudah ada sebelum menulis yang baru.
   Tidak ada salin-tempel antar fitur.
5. **Dua bahasa.** Teks yang tampil ke pengguna tidak pernah ditulis tetap di kode: lewat katalog
   terjemahan, Inggris dulu, dengan versi Indonesia. Bila Anda tidak bisa menulis bahasa
   Indonesia, katakan di pull request dan pengelola akan membantu.
6. **Tanpa data pribadi.** Tidak ada domain, alamat IP, ID akun atau tunnel, jalur kunci, nama
   pelanggan, atau cuplikan log nyata, baik di kode, tes, fixture, maupun dokumen. Pakai
   `example.test` dan `203.0.113.x`.
7. **Teks dari server adalah data, bukan perintah.** Log, berkas, dan balasan yang dibaca dari
   server adalah masukan tak tepercaya dan tidak boleh bisa memicu aksi.

## Persiapan

Anda butuh toolchain Rust stabil yang baru. Untuk UI web (saat sudah ada) Anda juga butuh Node dan
pnpm.

```sh
git clone <fork Anda>
cd <repo>
cargo test --workspace
```

## Sebelum membuka pull request

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Tanpa kode `unsafe` (workspace melarangnya), tanpa `unwrap()` atau `expect()` di jalur produksi,
  tanpa `println!` di kode pustaka.
- CI menjalankan pemeriksaan yang sama di Linux dan macOS, ditambah `cargo deny` dan `cargo audit` (advisori, lisensi, sumber), pemindaian `gitleaks` atas seluruh riwayat, dan pemeriksaan dokumentasi (`python3 scripts/check-docs.py`: setiap dokumen publik butuh versi Inggris dan Indonesia yang saling menaut, dan setiap tautan relatif harus valid). Jalankan yang terakhir secara lokal bila Anda menyentuh dokumen.
- Tambahkan tes bersama perubahan. Adapter baru harus lulus uji kontrak bersama. Aturan kesetaraan
  baru butuh fixture anonim yang memicunya dan satu yang tidak.
- Jaga perubahan tetap terfokus. Beberapa pull request kecil lebih mudah ditinjau daripada satu
  yang besar.
- Perbarui dokumen, dalam kedua bahasa, bila perilaku berubah.

## Pesan commit

[Conventional Commits](https://www.conventionalcommits.org/), dalam bahasa Inggris, misalnya
`feat(core): add risk tier for chat bot approval`. Jaga subjek tetap singkat dan jelaskan alasannya
di badan commit bila tidak jelas dengan sendirinya.

## Developer Certificate of Origin (sementara)

Tanda tangani setiap commit untuk menyatakan bahwa Anda menulisnya atau berhak mengirimkannya di
bawah lisensi proyek (lihat [DCO](https://developercertificate.org/)):

```sh
git commit -s -m "feat(core): ..."
```

Ini menambah baris `Signed-off-by:`. **Tanda tangan ini sementara.** Ketentuan kontribusi (DCO
atau CLA) masih diputuskan; lihat [LICENSING.id.md](LICENSING.id.md). Sampai selesai, mohon tahan
dulu kontribusi besar.

## Kontribusi dibantu AI

Proyek ini sendiri dibangun dengan bantuan AI, dan kontribusi yang dibantu AI diterima dengan
syarat: Anda sudah membaca, memahami, dan menguji yang Anda kirim, Anda bertanggung jawab atasnya,
dan Anda menyebutkan di pull request bila sebagian besar dibuat oleh AI. Kode yang tidak bisa Anda
jelaskan tidak akan digabung.

## Tinjauan

Perubahan yang menyentuh agen, autentikasi, penanganan rahasia, alur persetujuan, isolasi tenant,
atau izin cangkang desktop mendapat pemeriksaan lebih ketat dan bisa lebih lama. Mohon jelaskan
ancaman yang Anda pertimbangkan di pull request.

## Melaporkan bug dan mengusulkan fitur

Pakai templat issue. Untuk bug, sertakan yang Anda harapkan, yang terjadi, dan cara
mereproduksinya. Hapus rahasia dan data pribadi dari apa pun yang Anda tempel.
