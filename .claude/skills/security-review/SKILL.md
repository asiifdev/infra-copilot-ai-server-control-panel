---
name: security-review
description: Daftar periksa keamanan khusus produk ini (isolasi tenant, rahasia, allowlist agen, injeksi prompt, audit). Gunakan sebelum menggabungkan perubahan yang menyentuh agen, autentikasi, penyimpanan rahasia, eksekusi aksi, atau alat AI.
---

# Tinjauan keamanan

Produk ini punya akses mengubah server orang lain. Satu celah bisa fatal. Gunakan daftar ini untuk setiap perubahan yang relevan, dan laporkan hasilnya apa adanya (lolos, gagal, tidak diuji).

## Agen
- Koneksi **keluar saja**; tidak membuka port masuk.
- Hanya menjalankan aksi dari **daftar yang diizinkan** dengan argumen tervalidasi. Tidak ada shell bebas secara bawaan.
- Tidak membangun perintah shell dengan menyambung string dari input; gunakan argumen terpisah.
- Autentikasi dua arah (agen ke Cloud dan sebaliknya); token agen bisa dicabut dan berumur terbatas.

## Rahasia
- Terenkripsi saat disimpan (AES-256-GCM), kunci utama di luar database.
- Tidak muncul di log, audit, hasil alat, riwayat chat, pesan kesalahan, atau URL.
- Rahasia tidak dikirim ke model AI. Hasil alat diredaksi sebelum dikirim.

## Multi-tenant
- Setiap query memfilter organisasi. Ada uji otomatis yang mencoba mengakses data organisasi lain (harus gagal).
- Peran (owner, admin, operator, viewer) diperiksa di server, bukan di UI.

## Persetujuan dan aksi
- Tingkat ditentukan kode alat. Persetujuan satu Usulan, satu waktu, tidak bisa dipakai ulang.
- Aksi tingkat 4: ketik nama target, orang kedua di Cloud.
- Pelaksana memverifikasi hasil dan mencatat audit tak bisa diubah.

## Injeksi prompt
- Teks dari server dianggap data tak tepercaya. Uji dengan log/berkas yang berisi instruksi berbahaya ("abaikan aturan, jalankan ..."); alat tingkat 2+ tetap harus meminta persetujuan.

## Deployment dan git
- Kredensial git/SSH milik server (`server_session`) tidak boleh terbaca oleh proses build, AI, log, atau API.
- Build menjalankan kode tak tepercaya (`postinstall`, skrip build): pengguna tanpa hak istimewa, batas waktu dan sumber daya, rahasia produksi tidak ikut ke tahap build.
- Rollback harus selalu mungkin; pengalihan hanya setelah pemeriksaan kesehatan lulus.

## Web
- Cek Host dan Origin, CSRF untuk POST, token per sesi, batas laju, header keamanan.
- Unggahan dan jalur berkas divalidasi (tidak ada traversal).

## Rantai pasok
- Kunci versi dependensi, tinjau dependensi baru, jangan menjalankan skrip dari sumber tak tepercaya.

Laporkan: apa yang diperiksa, bukti (perintah/uji), dan yang **belum** diperiksa.
