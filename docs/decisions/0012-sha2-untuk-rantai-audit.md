# 0012. `sha2` untuk rantai hash audit

- Tanggal: 2026-11-19
- Status: diterima oleh pemilik ("setuju", 2026-11-19)
- Pengambil keputusan: pemilik proyek

## Konteks
0009 mencatat SHA-256 tulisan sendiri (±60 baris) untuk rantai hash `audit_log`, diuji dengan vektor NIST, dengan usulan menggantinya bila pustaka kripto dipilih. Tinjauan menyatakan implementasinya benar, tetapi kripto buatan sendiri tetap risiko kebijakan.

## Keputusan
Pakai crate `sha2` 0.11 (RustCrypto, MIT/Apache-2.0, Rust murni), `default-features = false`. Pembungkus `sha256_hex` di `crates/db/src/sha256.rs` dipertahankan beserta vektor NIST sebagai penjaga bahwa keluarannya tetap heks huruf kecil baku.

## Alasan
- Kode kripto diaudit dan dirawat orang lain; kita tidak memegang implementasi sendiri.
- SHA-256 baku, jadi hash audit yang sudah tersimpan tetap valid (tes rantai lolos tanpa perubahan).
- Dampak kecil: satu crate (plus `digest` dan sejenisnya) tanpa fitur bawaan.

## Belum diputuskan
- Pustaka AES-256-GCM untuk `node_secrets` (belum dievaluasi; dilakukan saat enkripsi rahasia dikerjakan).
- Hash audit masih tanpa kunci (batas yang sudah tercatat di 0009).
