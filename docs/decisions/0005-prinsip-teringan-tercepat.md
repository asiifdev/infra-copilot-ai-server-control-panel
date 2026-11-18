# 0005. Prinsip: teknologi teringan dan tercepat

- Tanggal: 2026-11-19
- Status: diterima
- Pengambil keputusan: pemilik proyek

## Konteks
Produk berjalan di server milik pengguna (sering VPS kecil) dan di laptop sebagai aplikasi desktop. Pemilik menetapkan: "pokoknya teknologi teringan dan tercepat".

## Keputusan
Pada setiap pilihan teknologi, **yang teringan dan tercepat yang memenuhi kebutuhan** menang, kecuali ada alasan tertulis yang kuat. Berlaku untuk bahasa, pustaka, UI, basis data, dan cara pengemasan.

## Penerapan
- **Inti dan agen:** Rust, satu biner statis; tanpa runtime tambahan (Node, PHP, Python, JVM) hanya untuk menjalankan produk.
- **Desktop:** Tauri (webview bawaan OS), bukan Electron.
- **UI:** SPA yang dikompilasi dan ditanam ke biner. Kerangka diutamakan yang menghasilkan bundel kecil dan tanpa beban runtime besar; **Svelte 5** (diputuskan pemilik, lihat catatan analisis di bawah). Tanpa SSR, tanpa Next.js.
- **Basis data:** SQLite tertanam sebagai bawaan (self-host kecil dan desktop); PostgreSQL hanya untuk cloud multi-tenant dan instalasi besar. Hindari ORM berat; pakai lapisan kueri tipis.
- **Web/API:** pustaka HTTP minimal berbasis async Rust; tanpa kerangka besar yang tidak diperlukan.
- **Agen:** target konsumsi minimal (memori diam, ukuran biner, waktu mulai), karena berjalan di setiap server.
- **Dependensi:** sedikit; setiap dependensi baru harus membenarkan biayanya terhadap ukuran biner dan waktu kompilasi.

## Catatan analisis UI: Svelte vs Solid (2026-11-19)
- Svelte 5 (runes `$state`, `$derived`) stabil, dapat dipakai dengan Vite polos tanpa SvelteKit (`mount` dari `svelte`, plugin `@sveltejs/vite-plugin-svelte`). Sumber: dokumentasi Svelte via Context7.
- Solid: dokumentasi menunjukkan **Solid 2.0 masih versi kandidat rilis (`v2.0.0-rc.0`)** dengan perubahan yang memutus kompatibilitas (di antaranya `classList`, `use:`, `on:`, `attr:`, `bool:` dihapus; hanya `onClick` berhuruf kapital yang mengikat event). Memulai produk baru berarti memilih 1.x yang akan jadi lama, atau 2.0 RC yang belum stabil, dan pustaka pendamping (router dan lain-lain) dalam cabang `next`.
- **Keputusan final pemilik (2026-11-19): Svelte 5 + Vite + TypeScript, tanpa SvelteKit.** Alasan pemilik: Svelte terbuka dan sadar terhadap pengembangan dengan AI (menyediakan skill, MCP, subagent, dan plugin resmi), dan hal yang perlu dipelajari di era ini adalah AI-nya. Pengukuran prototipe di M0 tetap dilakukan untuk menetapkan anggaran kinerja, bukan lagi untuk memilih kerangka.

## Cara menegakkan
- **Anggaran kinerja** untuk ukuran biner, memori diam, dan waktu mulai, ditetapkan di M0 dari **pengukuran prototipe** (bukan angka karangan) dan diperiksa di CI. Regresi melewati anggaran harus dijelaskan atau ditolak.
- Setiap pilihan pustaka inti mencatat ukuran dan dampak kinerjanya.

## Kompromi yang diterima
- Rust dan Tauri membuat iterasi awal lebih lambat dan webview berbeda antar OS.
- Ekosistem lebih kecil daripada pilihan yang lebih umum; sebagian fungsi mungkin harus ditulis sendiri.
- "Teringan dan tercepat" tidak boleh mengorbankan keamanan atau ketepatan; bila bertentangan, keamanan menang.
