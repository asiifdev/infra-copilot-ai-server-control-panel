# 0033. Kebijakan endpoint transport pada infrastruktur bersama

- Tanggal: 2026-11-24
- Status: diterima (pemilik: "gas kerjakan aja"; menutup butir yang dicatat di 0028 dan NEXT-STEPS)
- Pengambil keputusan: pemilik proyek

## Konteks
Di mesin pemilik, transport boleh menunjuk ke mana saja (jaringan itu miliknya). Pada edisi Cloud, perintah transport berjalan di infrastruktur yang dipakai bersama: tanpa pembatasan, satu pelanggan bisa membuat infrastruktur itu terhubung ke loopback, jaringan internal, atau alamat metadata cloud (SSRF).

## Keputusan
1. Modul `transport::policy`: `EndpointPolicy::{OwnNetwork, SharedInfrastructure}` dan `TransportSpec::check_endpoints(policy)`. `OwnNetwork` tidak membatasi apa pun.
2. Di `SharedInfrastructure` ditolak: loopback, privat (RFC 1918), link-local (termasuk 169.254.169.254), ruang alamat bersama (RFC 6598), tak terspesifikasi, siaran, multicast, rentang dokumentasi/benchmark, IPv6 unique-local/link-local/site-local, 6to4, dan IPv4 yang dibungkus IPv6 (mapped, compatible, NAT64); nama `localhost`, akhiran `.local .internal .lan .home .corp .intranet .home.arpa .localdomain`; (`Internal`). Ditolak sebagai `Ambiguous`: bentuk angka tersamar (desimal tunggal, heksa, oktal, `127.1`) dan nama satu label (`metadata`, `db`). URL (`webdav.url`, `endpoint` S3/B2) harus `https://` tanpa user-info (`BadUrl`).
3. Endpoint yang diperiksa: host SSH (rsync, scp, ssh pipe), opsi `host` (sftp), `url` (webdav), `endpoint` (s3, b2).
4. Fungsi `is_internal_ip` diekspor. **Belum tercakup:** nama DNS publik yang mengarah ke alamat privat. Pelaksana yang berjalan di infrastruktur bersama wajib me-resolve sendiri, memeriksa setiap jawaban dengan `is_internal_ip`, lalu terhubung ke alamat itu (bukan ke nama lagi, agar tahan DNS rebinding). Pelaksana itu belum ada (M4 dan seterusnya); sampai ada, edisi Cloud tidak boleh menjalankan transport.
5. Tidak ada perubahan perilaku CLI/server lokal: keduanya memakai `OwnNetwork`.

## Alternatif
Daftar putih host: ditolak, terlalu membatasi pengguna sah. Menolak semua IP literal: ditolak, banyak penyimpanan sah memakainya.
