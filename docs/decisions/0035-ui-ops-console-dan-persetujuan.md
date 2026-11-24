# 0035. UI "ops console": cangkang baru, palet perintah, halaman Persetujuan

- Tanggal: 2026-11-24
- Status: diterima (pemilik: UI/UX harus maksimal, modern, rapi, responsif; library boleh "jika diperlukan")
- Pengambil keputusan: pemilik proyek

## Keputusan
1. **Tanpa library baru.** `web/package.json` tidak berubah: ikon (`Icon.svelte`, 31 ikon, sekitar 0,9 KB gzip) dan palet perintah ditulis sendiri karena lebih kecil daripada `@lucide/svelte` dan `bits-ui` (dievaluasi oleh ukuran). 0031 ("tanpa dependensi runtime") tetap berlaku. Library hanya dipertimbangkan lagi bila ada kebutuhan yang tidak masuk akal ditulis sendiri (misalnya grafik), dengan syarat 0005 dan pemeriksaan rantai pasok.
2. **Cangkang:** bilah samping lengket berkelompok (Monitor, Work, Workspace) dengan ikon, tombol "Jump to" (Ctrl/⌘+K), nama organisasi; di ponsel bilah atas dengan menu `aria-expanded` yang menutup saat navigasi dan Escape.
3. **Palet perintah** (`CommandPalette.svelte`): dialog combobox/listbox, panah/Enter/Escape, jebakan fokus dan pengembalian fokus, 9 halaman + 3 aksi (percakapan baru, ganti tema, ganti bahasa), pencocokan fuzzy.
4. **Ringkasan** menjadi dasbor: empat ubin (Persetujuan, Node, Temuan, Asisten AI), daftar "perlu perhatian", aksi cepat.
5. **Halaman Persetujuan** (`#/proposals`, lencana jumlah di navigasi): filter Menunggu/Disetujui/Ditolak/Kedaluwarsa/Semua; setujui tingkat 1-2 satu klik, tingkat 3-4 meminta konfirmasi tambahan, tolak selalu meminta konfirmasi; menunggu jawaban server (tanpa optimisme), 409 memuat ulang. Teks menyatakan bahwa **menyetujui hanya mencatat** (0034).
6. **Token:** skala tipografi, radius, bayangan, tint, dan `--nv-scrim` di `web/src/app.css`, semuanya turunan `color-mix` dari token yang ada; `tokens.css` tidak berubah dan `scripts/check-palette.py` lulus. Pasangan tint baru dihitung minimal 7,1:1 di kedua tema.
7. **Pemuatan:** kerangka (skeleton) menggantikan spinner; data lama tetap tampil saat muat ulang.
8. Kalimat berhitung dibuat tanpa bentuk jamak ("Providers set up: 1") karena katalog belum mendukung jamak.

## Ukuran (2026-11-24)
JS 156 150 B mentah (49 067 B gzip), CSS 39 790 B (6 970 B gzip); total aset sekitar 55,9 KB gzip, **hanya sekitar 100 B di bawah anggaran 56 000** (0032). **Anggaran dinaikkan** di keputusan ini untuk blok `infra`: UI gzip 56 000 → 64 000 B dan JS mentah 165 000 → 180 000 B (margin sekitar 14%, agar selisih implementasi gzip antar-OS tidak merusak CI). Alasan: UI baru adalah perubahan yang disengaja dan diminta pemilik, tanpa library. Biner, RSS, dan waktu mulai tidak berubah. Kenaikan berikutnya butuh keputusan baru.

## Belum diverifikasi
Pembaca layar nyata, mode kontras tinggi Windows, halaman Settings/Memory/Findings/Nodes/Transports dan dialog konfirmasi di layar (hanya axe dan tes unit), dan tampilan gelap desktop. Diperiksa di server sungguhan (`infra serve` dengan penyedia palsu): Ringkasan terang (desktop dan 375 px), tanpa galat konsol dan tanpa pelanggaran CSP di Ringkasan.
