# 0021. Enkripsi rahasia: AES-256-GCM lewat ring

- Tanggal: 2026-11-19
- Status: diterima (pemilik, 2026-11-19: pustaka paling optimal, tanpa solusi sementara)
- Pengambil keputusan: pemilik proyek

## Konteks
Rahasia (kunci API penyedia AI, kelak token bot dan kredensial transport) harus terenkripsi saat disimpan, dengan kunci utama di luar database (CLAUDE.md aturan 10, `docs/design.md` 3.1). 0009 menunda pilihan pustaka. M2-c membutuhkannya untuk kunci penyedia AI. Sumber rahasia sementara dari lingkungan (0018) hanya untuk pengamat mandiri; tempat rahasia produk adalah database terenkripsi.

## Keputusan
Crate `vault` memakai **`ring` 0.17 (`aead::LessSafeKey`, AES-256-GCM)**, nonce acak 96-bit dari `SystemRandom` per penyegelan, dan `zeroize` untuk menghapus bahan kunci dari memori.

- Bentuk tersimpan: `versi (1 byte) || nonce (12) || ciphertext || tag (16)`; `key_id` di kolom terpisah.
- **Data tambahan terautentikasi (AAD)** = `nodivara-vault/v1` + org + tujuan + `key_id`, masing-masing diawali panjangnya. Nilai tersegel yang disalin ke baris organisasi lain, dipakai untuk tujuan lain, atau diberi label `key_id` lain gagal dibuka (diuji, termasuk salinan tingkat database).
- Kunci utama dari lingkungan: `NODIVARA_MASTER_KEY=<id>:<64 hex>`; kunci lama di `NODIVARA_MASTER_KEYS_RETIRED` (dipisah koma) tetap bisa membuka, dan `rotate_org` menyegel ulang ke kunci terbaru. Spesifikasi yang salah ditolak keras.
- Tabel `secrets` (org-level) dan `ai_providers` (migrasi 0005). Kunci penyedia dirujuk lewat `secret_id`; komposit `(org_id, secret_id)` mencegah penyedia satu organisasi menunjuk rahasia organisasi lain.

## Alasan
- **Kecepatan dan ukuran:** `ring` sudah ada di pohon dependensi lewat `rustls` (0016), dengan implementasi AES-GCM beraksen perakitan. Tidak ada kode kripto baru yang ikut terpasang di biner; satu-satunya tambahan crate baru adalah `zeroize` (tanpa dependensi bila fitur bawaan dimatikan). Ukuran biner tidak diukur terpisah; anggaran diperiksa job `perf-budget` saat `vault` tertanam di server.
- **Kebenaran:** uji memakai vektor uji 16 dari McGrew dan Viega (AES-256, IV 96-bit, dengan AAD) dan cocok byte demi byte; plus uji ubah-satu-bit di setiap posisi, potong di setiap batas, dan keunikan nonce (2000 penyegelan).
- **Nonce acak aman untuk pemakaian kita:** peluang tabrakan nonce 96-bit baru berarti pada ~2^32 pesan per kunci; volume rahasia jauh di bawah itu. Rotasi kunci menjaga batas itu.

## Alternatif yang dipertimbangkan
- `aes-gcm` (RustCrypto): murni Rust dan seragam dengan `sha2`, tetapi menambah `aes`, `ghash`, `ctr`, dan lainnya ke biner yang sudah memuat kode AES-GCM dari `ring`. Ditolak untuk ukuran.
- `aws-lc-rs`: lebih cepat di beberapa CPU, tetapi 0016 sengaja memilih `ring` tanpa `aws-lc-rs` (ukuran, waktu kompilasi).
- XChaCha20-Poly1305 (nonce 192-bit): bermanfaat bila volume sangat besar; tidak diperlukan, dan AES-256-GCM diminta oleh desain (`docs/design.md`).
- Kunci dari berkas atau Keychain/KMS: bisa menjadi sumber kunci utama kelak lewat antarmuka yang sama; lingkungan adalah pengecualian yang sudah ditetapkan desain.

## Dampak
- 0009 butir "pustaka AES-256-GCM belum dipilih" selesai. `notifier::Secret` dipindah ke `core_domain::secret::Secret` (notifier tetap mengekspor ulang) agar crate lain memakai satu tipe.
- Belum dikerjakan: sumber kunci utama selain lingkungan, enkripsi `node_secrets` lewat `vault` (tabel dan API ada, pemanggil belum), dan penyedia milik pribadi (`user_id`) di `ai_providers`.
