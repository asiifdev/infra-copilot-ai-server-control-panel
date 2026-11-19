# 0015. Item `env_ref` dan versi layanan (Modul C)

- Tanggal: 2026-11-19
- Status: diterima (rencana Modul C disetujui pemilik, 2026-11-19)
- Pengambil keputusan: pemilik proyek

## Konteks
Aturan 2 dan 7 di `docs/plan.md` bagian 6 (rujukan ke `localhost` dari container; rujukan ke host/port yang tidak ada) dan aturan 9 (versi layanan berbeda) memerlukan data yang belum dikumpulkan. `docs/design.md` 4.1 mendefinisikan `env_ref`.

## Keputusan
1. `Kind::EnvRef` ditambahkan (nama kabel `env_ref`), bentuk data seperti dokumentasi `Kind::EnvRef` di `inventory`.
2. **Nilai variabel tidak pernah disimpan.** Sumber: `env` di `dump.pm2` dan `Config.Env` dari `docker inspect`. Kolektor membaca nilai hanya sebagai string sementara di memori, mengurai host dan port (URL `scheme://[userinfo@]host[:port]/...`, `host:port`, atau pasangan `<PREFIX>_HOST` + `<PREFIX>_PORT`), lalu membuangnya. Yang tersimpan hanya `target_host` dan `target_port`. Userinfo, path, query, dan nilai yang tidak terurai tidak pernah masuk item, peringatan, atau log. `/proc/<pid>/environ` dan berkas `.env` tetap tidak dibaca.
3. `resolves_to` dihitung setelah semua kolektor selesai, hanya terhadap `listener` pada inventaris yang sama, dan hanya untuk host loopback atau `0.0.0.0`. Host lain: `null` (tidak diklaim).
4. Versi layanan (aturan 9) pada tahap ini hanya dari tag image Docker: item proses `docker:<nama>` mendapat `version` (tag, atau null untuk `latest`/tanpa tag/digest). Jenis item `service` ditunda sampai ada sumber versi yang lain (paket, database).
5. Aturan: PAR-007 (rujukan loopback ke port tanpa listener di mesin yang sama), PAR-002 (rujukan loopback dari dalam container ke port yang hanya ada sebagai listener host), PAR-009 (versi mayor berbeda pada item dengan kunci sama). Semuanya hanya menyatakan yang terlihat; inventaris `partial` menurunkan aturan yang mengklaim ketiadaan.

## Alasan
Memenuhi aturan 2, 7, 9 tanpa menyimpan rahasia, dan tanpa jenis item baru yang belum punya sumber data.

## Batas
Nilai tidak diuji terhadap server nyata. Pasangan `_HOST`/`_PORT` ditebak dari nama. Rujukan melalui nama host (bukan loopback) belum di-resolve.
