[English](SECURITY.md) | Bahasa Indonesia

# Kebijakan keamanan

Infra Copilot dirancang untuk memegang akses istimewa ke server, jadi celah di sini bisa serius.
Kami menanggapi laporan dengan sungguh-sungguh dan meminta Anda melaporkannya secara privat.

## Status

Proyek ini **pra-alfa** dan belum punya versi rilis, jadi belum ada versi yang didukung. Setelah ada
rilis, bagian ini akan mendaftar versi mana yang mendapat perbaikan keamanan.

## Cara melapor

**Jangan membuka issue, pull request, atau diskusi publik untuk masalah keamanan.**

Laporkan secara privat lewat salah satu cara ini:

1. **Pelaporan kerentanan privat GitHub** di repositori ini (tombol "Report a vulnerability" di tab
   Security), setelah diaktifkan.
2. **Surel:** **[kontak keamanan: akan diisi pengelola sebelum repositori dibuka ke publik]**

Sertakan yang Anda temukan, di mana, cara mereproduksinya, dan dampak menurut Anda. Jangan
menyertakan rahasia asli, data pelanggan asli, atau akses ke sistem yang bukan milik Anda.

## Yang paling ingin kami dengar

- Apa pun yang membuat sebuah aksi berjalan **tanpa persetujuan yang disyaratkan**, atau
  menurunkan tingkat risiko sebuah aksi.
- Cara apa pun untuk membaca, mencatat, atau membocorkan **rahasia**, atau mengirimnya ke model AI.
- **Prompt injection** yang membuat agen bertindak atas teks yang dibaca dari server, log, atau
  berkas.
- Jebolnya **isolasi antar organisasi** (isolasi tenant).
- Injeksi perintah atau argumen lewat agen, sidecar, atau cangkang desktop.
- Titik lemah pada autentikasi, sesi, penautan akun dan callback bot Telegram, atau penandatanganan
  pembaruan.
- Izin yang terlalu luas pada capabilities cangkang desktop.

## Yang bisa Anda harapkan

Kami akan mengakui laporan Anda, menyelidiki, mengabari perkembangannya, dan mencantumkan nama Anda
pada perbaikan bila Anda mau. Target waktu tanggapan akan diumumkan pada rilis pertama; sampai
saat itu kami tidak menjanjikan waktu selain upaya dengan itikad baik.

## Pengungkapan yang bertanggung jawab

Mohon beri kami kesempatan yang wajar untuk memperbaiki masalahnya sebelum Anda mengungkapkannya
ke publik, dan hindari mengakses data yang bukan milik Anda, mengganggu layanan, atau menguji
sistem yang bukan milik Anda. Kebijakan ini bukan nasihat hukum dan tidak memberi izin menguji
pihak ketiga.
