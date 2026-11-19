[English](faq.md) | Bahasa Indonesia

# Pertanyaan yang sering diajukan

Terakhir diperbarui: 2026-11-19. Nodivara masih **pra-alfa**: fitur yang di sini disebut
direncanakan belum tersedia.

## Apa itu Nodivara?

Nodivara adalah panel pengelolaan server open source, untuk pekerjaan yang sama dengan aaPanel
dan cPanel, dengan agen AI bawaan yang mengusulkan perubahan dan menunggu persetujuan manusia.
Ditulis dengan Rust dan antarmuka Svelte, menyasar server Linux dan aplikasi desktop, dan masih
pra-alfa: belum ada yang bisa dipasang.

## Bisakah saya memasangnya sekarang?

Belum. Belum ada rilis. Repositori ini saat ini berisi desain, keputusan arsitektur, dan crate inti
pertama (tingkat risiko dan penyaring rahasia). Semua hal lain di README masih direncanakan.

## Apa itu agen AI pengelola server dalam proyek ini?

Sebuah model AI, memakai penyedia kompatibel OpenAI yang Anda atur, yang bisa membaca keadaan
server lewat daftar alat yang tertutup dan mengusulkan aksi. Ia tidak menjalankan shell bebas.
Setiap aksi yang mengubah atau menghapus sesuatu menjadi usulan yang harus disetujui orang lebih
dulu.

## Bisakah AI menjalankan perintah di server saya sendiri?

Tidak, untuk apa pun yang mengubah server. Aksi diberi lima tingkat, dari hanya-baca sampai
merusak. Hanya tingkat 0 dan 1 (membaca dan merencanakan) yang berjalan otomatis. Mulai tingkat 2,
usulan tersimpan butuh persetujuan manusia, dan aksi merusak juga butuh nama targetnya diketik.

## Apakah rahasia saya dikirim ke model AI?

Menurut desain, tidak. Rahasia seperti kata sandi di URL, nilai `.env`, token, dan kunci privat
disamarkan sebelum apa pun dicatat, disimpan, ditampilkan, atau dikirim ke model. Kode
penyamarannya sudah ada di repositori ini dan punya tes. Bagian alur lainnya belum dibangun.

## Apa bedanya dengan aaPanel, cPanel, 1Panel, Coolify, atau Dokploy?

Produk-produk itu sudah ada dan berjalan; Nodivara belum. Perbedaan yang dituju: alur
persetujuan yang teraudit untuk aksi AI, pemeriksaan kesetaraan sebelum cutover pada migrasi
server, dan analisis error yang menyebutkan buktinya. Lihat [perbandingan](comparison.id.md) yang
bertanggal dan bersumber.

## Apa itu pemeriksaan kesetaraan migrasi?

Membandingkan apa yang berjalan di server lama dengan yang berjalan di server baru, lalu melaporkan
yang tertinggal sebelum server lama dimatikan: layanan yang tidak tercatat, `localhost` yang
menunjuk mesin yang salah, rekaman DNS yang tidak pernah dipindah. Direncanakan, belum dibangun.

## Sistem operasi apa yang didukung?

Targetnya server Linux lebih dulu. Edisi desktop untuk macOS, Linux, dan Windows direncanakan
dengan Tauri. Linux lebih dulu, lalu macOS, lalu Windows. Belum ada yang dirilis.

## Bahasa pemrograman dan kerangka kerja apa yang bisa dideploy?

Direncanakan: Node, PHP, Python, Go, dan .NET, dengan deteksi stack dari repositori dan deploy satu
klik. Belum dibangun.

## Apakah mendukung bahasa Indonesia dan Inggris?

Ya menurut desain: semua teks yang tampil ke pengguna lewat katalog terjemahan Inggris dan
Indonesia, dan setiap dokumen publik punya kedua versi.

## Lisensinya apa?

Sementara AGPL-3.0-only, menunggu tinjauan penasihat hukum sebelum rilis publik pertama. Lihat
[LICENSING.id.md](../LICENSING.id.md) untuk ringkasan bahasa sehari-hari dan hal yang masih
terbuka.

## Apakah akan ada edisi berbayar?

Direncanakan sebagai model open core: inti open source gratis, ditambah edisi berbayar yang
ditawarkan terpisah (batas tambahan, lisensi, cloud terkelola). Tidak ada fitur di inti open source
yang sengaja dimatikan. Harga belum ditetapkan.

## Bagaimana cara berkontribusi atau melaporkan masalah keamanan?

Baca [CONTRIBUTING.id.md](../CONTRIBUTING.id.md) untuk kontribusi. Laporkan masalah keamanan secara
privat, jangan di issue publik; lihat [SECURITY.id.md](../SECURITY.id.md).
