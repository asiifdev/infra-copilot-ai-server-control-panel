# 0002. Tetap dibangun: dogfood dulu, pasar internasional, bahasa ID/EN

- Tanggal: 2026-11-19
- Status: diterima
- Pengambil keputusan: pemilik proyek

## Konteks
Riset pasar (`docs/market-research.md`) menunjukkan kategori panel self-hosted terbukti besar, tetapi permintaan khusus untuk agen AI pengelola server baru berupa inferensi, dan petahana (cPanel, Plesk, 1Panel, Dokploy) sudah menambah sebagian AI/MCP. Pemilik tetap ingin membangunnya.

## Keputusan
1. **Tetap dibangun.** Alasan utama: pemilik adalah pengguna pertama. Server baru akan langsung memakai produk ini, dan migrasi dari MacBook lokal ke server baru dilakukan dengan produk ini.
2. **Pasar internasional**, bukan hanya Indonesia.
3. **Bahasa minimal Indonesia dan Inggris (ID/EN)** sejak awal, di antarmuka, bot Telegram, notifikasi, dan jawaban AI.

## Alasan
- Dogfooding menurunkan risiko: kegagalan terburuk adalah waktu terbuang, sementara kebutuhan nyata pemilik (migrasi Mac ke server, kelola server) terpenuhi.
- Migrasi Mac ke server adalah uji nyata untuk pembeda produk (pemeriksaan kesetaraan).
- Pasar internasional memperluas jangkauan, tetapi menambah beban: harga dalam USD, pembayaran global, GDPR selain UU PDP, dan dokumentasi berbahasa Inggris.

## Alternatif yang dipertimbangkan
- Memvalidasi pasar dulu (wawancara, waitlist) sebelum membangun: tetap disarankan **berjalan paralel**, tetapi tidak menunda pembangunan.
- Hanya pasar Indonesia: ditolak, karena riset menunjukkan data lokal tipis dan pemilik menargetkan global.

## Dampak
- Skenario uji penerimaan utama MVP: memigrasikan stack MacBook ke server Linux baru dan memverifikasi kesetaraannya.
- i18n adalah komponen reusable wajib (katalog terjemahan, tidak ada teks tetap di kode).
- Dokumen publik berbahasa Inggris menjelang rilis (M7). Dokumen internal boleh tetap Indonesia.
- Riset tambahan yang masih perlu: DNS dan Cloudflare Tunnel, penyedia pembayaran global, regulasi di luar Indonesia.
