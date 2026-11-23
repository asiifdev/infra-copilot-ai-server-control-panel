# 0029. Server lokal, UI web dan kontrak API (irisan UI pertama)

- Tanggal: 2026-11-23
- Status: diterima (pemilik, 2026-11-23: "lanjut ke UI"; uji hidup Muse ditunda sampai ada UI yang bisa dijalankan, jadi UI pertama harus memuat pengaturan penyedia AI dan percakapan asisten)
- Pengambil keputusan: pemilik proyek

## Konteks
Semua kemampuan M0 sampai M3 ada sebagai pustaka dan CLI, tetapi belum ada yang bisa dilihat dan dipakai orang non-teknis, dan jalur AI belum pernah diuji ke penyedia nyata. Keputusan lama yang berlaku: Svelte 5 + Vite + TypeScript tanpa SvelteKit (CLAUDE.md), `axum` + `tokio` + `rust-embed` (0007), anggaran kinerja (0005, 0010), palet dan token (0019, `docs/brand.md`), komponen UI bersama (`docs/design.md` 12.5), API dibatasi organisasi dan tanpa endpoint yang mengembalikan rahasia (`docs/design.md` 10). Prototipe `ui/` dan `crates/proto-server` adalah probe anggaran yang diukur CI; keduanya tidak diubah.

## Keputusan
1. **Crate baru `server`** (axum, tokio, rust-embed) menyajikan API JSON dan UI. Dijalankan lewat **`infra serve [--port N] [--db PATH] [--org SLUG]`** (satu biner; `cli` bergantung pada `server`). Mode **lokal satu pengguna**: bertindak sebagai pemilik organisasi lokal, sama seperti CLI. **Hanya mengikat loopback** (`127.0.0.1`/`::1`); alamat lain ditolak pada versi ini (akses jarak jauh butuh TLS dan login sungguhan, belum dikerjakan).
2. **UI baru di `web/`** (Svelte 5 + Vite + TypeScript, tanpa SvelteKit), dibuild ke `web/dist` dan ditanam ke biner. `ui/` dan `proto-server` tetap sebagai probe anggaran sampai anggaran dihitung ulang dari server sungguhan (keputusan terpisah). Token warna dipakai ulang dari `ui/src/tokens.css` lewat alias Vite, tidak disalin.
3. **Satu sumber teks:** katalog i18n Rust (`crates/i18n/locales/{en,id}.txt`) tetap satu-satunya sumber. Server menyajikan kunci berawalan `ui.` lewat `GET /api/i18n/{locale}`; UI tidak punya katalog sendiri. Teks temuan (`parity.*`) dirender di server dalam locale permintaan.

## Autentikasi dan keamanan (wajib, diuji)
- Saat mulai, server mencetak `http://127.0.0.1:PORT/#t=<token>`. Token acak 256-bit, **sekali pakai, berlaku 10 menit**, hanya disimpan sebagai hash. Berada di *fragment* agar tidak terkirim ke server atau masuk header Referer. UI menukarnya lewat `POST /api/auth/exchange {token}` dan server mengatur cookie sesi `nv_session` (`HttpOnly`, `SameSite=Strict`, `Path=/`, tanpa `Secure` hanya karena loopback HTTP). Sesi: acak 256-bit, disimpan hash di memori (restart server menghapus semua sesi), kedaluwarsa 8 jam tanpa aktivitas, dibatasi 5 sesi. Percobaan tukar token dibatasi lajunya (gagal berulang menghapus token).
- Setiap permintaan: header `Host` harus persis alamat ikat (menolak DNS rebinding); permintaan yang mengubah data (non-GET) wajib `Origin` sama dengan `Host` **dan** header `X-Nodivara: 1`; tanpa itu 403. Badan JSON maksimum 64 KiB, `Content-Type: application/json` wajib, waktu habis per permintaan, satu giliran asisten berjalan per percakapan (409 bila sibuk).
- Header respons: `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`, `Cache-Control: no-store` untuk `/api/*`. UI tidak boleh memakai `{@html}`, gaya atau skrip inline, atau `eval`.
- Tidak ada endpoint yang mengembalikan rahasia: kunci API penyedia hanya diterima (`POST`), disegel dengan `vault`, dan tidak pernah dikembalikan. Galat berupa kode tetap dan pesan terlokalisasi; tidak pernah teks server, URL, atau kunci. Semua kueri dibatasi organisasi lewat crate yang ada.
- Kunci utama (`NODIVARA_MASTER_KEY`) tetap hanya dari lingkungan. Tanpa itu UI menampilkan saran yang jelas dan menolak menyimpan kunci penyedia (`409 master_key_missing`), tetapi tetap berfungsi untuk penyedia tanpa kunci.

## Kontrak API (semua di `/api`, JSON UTF-8, kecuali dicatat; locale dari `?lang=` lalu `Accept-Language`, bawaan `en`)
Galat: status HTTP yang sesuai dan `{"error":{"code":"<kode_tetap>","message":"<terlokalisasi>"}}`.

| Metode dan jalur | Isi |
|---|---|
| `GET /healthz` (tanpa auth, di luar `/api`) | `{"status":"ok"}` |
| `POST /api/auth/exchange` `{token}` | menukar token dengan cookie sesi; 204; 401 `invalid_token` |
| `POST /api/auth/logout` | menghapus sesi; 204 |
| `GET /api/session` | `{user:{email,name}, org:{name,slug}, locale, master_key_configured, version}` |
| `GET /api/i18n/{locale}` | peta datar `{ "ui.x.y": "teks" }` untuk kunci `ui.*` |
| `GET /api/nodes` | `[{name, kind, status, latest_inventory:{taken_at,status,counts:{listener,process,cron,env_ref}}\|null}]` |
| `GET /api/nodes/{name}/inventory?kind=&limit=` | `{taken_at,status,warnings:[...],items:[{kind,key,data}]}`; 404 `not_found` |
| `GET /api/findings?min_severity=&limit=` | `{comparison:{source,target,created_at}\|null, findings:[{rule_id,severity,subject:{kind,key},title,explanation,advice,note,evidence:[{kind,key}]}]}` |
| `GET /api/plans`, `GET /api/plans/{name}` | daftar ringkas; rincian `{name,status,spec,steps,rendered,gates:{...}}` (teks rencana dari `plan::render_plan`) |
| `GET /api/transports` | `[{name,kind,verified_at,last_check}]` (tanpa konfigurasi dan tanpa rujukan rahasia) |
| `GET /api/providers`, `POST /api/providers` `{name,base_url,model,params?,key?,is_default?}`, `DELETE /api/providers/{name}` | daftar `{name,base_url,model,params,has_key,is_default}`; kunci hanya masuk |
| `POST /api/providers/{name}/test` | satu giliran kecil ke penyedia (tanpa alat, jawaban dibatasi) -> `{ok:true,latency_ms,model}` atau `{ok:false,code,message}` dengan saran lokal (tombol "uji koneksi" untuk uji hidup Muse) |
| `GET /api/memories?status=pending\|active`, `POST /api/memories/{id}/approve`, `DELETE /api/memories/{id}` | memori; persetujuan hanya lewat sini (dan CLI) |
| `GET /api/proposals?status=` | daftar Usulan, **hanya baca** (tidak ada setujui atau jalankan; itu M4) |
| `GET /api/conversations`, `POST /api/conversations` `{title?}`, `GET /api/conversations/{id}/messages` | percakapan milik pengguna |
| `POST /api/conversations/{id}/turn` `{message}` -> `text/event-stream` | peristiwa `delta {text}`, `tool {name,tier,status,duration_ms}`, `done {rounds,tool_calls,usage?}`, `error {code,message}`; 409 `busy` |

## Alasan
- **Token sekali pakai di fragment + cookie `HttpOnly` `SameSite=Strict` + cek Host/Origin + header khusus** adalah rangkaian terkecil yang menahan CSRF, DNS rebinding dan pencurian token lewat Referer untuk aplikasi loopback tanpa akun. Login sungguhan (kata sandi, multi-pengguna) menyusul bersama akses jarak jauh.
- **Katalog tunggal** mencegah dua versi teks yang melenceng dan memenuhi aturan i18n proyek.
- **Tombol uji koneksi penyedia** memberi jalan uji hidup Muse yang aman: kunci dimasukkan lewat UI, disegel, tidak pernah ditampilkan lagi.
- **`web/` terpisah dari `ui/`** menjaga job `perf-budget` tetap valid sampai ada keputusan anggaran baru.

## Alternatif yang dipertimbangkan
- Login kata sandi sekarang: lebih banyak permukaan serangan (penyimpanan hash, brute force) untuk manfaat nol pada mode satu pengguna di loopback.
- Mengganti isi `ui/`: merusak probe anggaran 0010.
- Katalog UI terpisah dengan sinkronisasi: sudah terbukti melenceng; ditolak.
- SvelteKit: bertentangan dengan keputusan final.

## Dampak
- Dokumen README, CLAUDE.md, NEXT-STEPS, design diperbarui saat tiap bagian selesai; `check-docs.py` mewajibkan crate `server` tercantum.
- Belum dikerjakan: akses jarak jauh dan TLS, login multi-pengguna, persetujuan dan eksekusi Usulan (M4), anggaran kinerja UI sungguhan (diukur setelah UI ada), pensiun `ui/` dan `proto-server`.
