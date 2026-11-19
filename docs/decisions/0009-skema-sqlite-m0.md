# 0009. Skema SQLite M0 dan lapisan kueri `db` (usulan)

- Tanggal: 2026-11-19
- Status: diterima oleh pemilik ("setuju", 2026-11-19)
- Pengambil keputusan: pemilik proyek

## Konteks
`docs/design.md` bagian 3 menulis skema untuk PostgreSQL. 0005 dan 0007 menetapkan SQLite tertanam sebagai basis data bawaan (`rusqlite` + `rusqlite_migration`, tanpa ORM). M0 membutuhkan fondasi identitas, node, dan audit. Crate `crates/db` mengimplementasikan hanya: bagian 3.1 (organizations, users, memberships, api_tokens), 3.2 (nodes, agents, node_secrets), dan `audit_log` dari 3.6. Tabel lain (inventaris, migrasi, AI, usulan, lisensi) tidak dibuat sampai tahapnya tiba.

## Keputusan

### Penyesuaian Postgres ke SQLite
| Postgres (design.md) | SQLite (M0) | Catatan |
|---|---|---|
| `uuid` | `TEXT`, UUID v4 huruf kecil, `CHECK(length(id)=36)` | Dibangkitkan lewat `randomblob` SQLite (CSPRNG) agar tidak menambah dependensi. v7 (terurut waktu) ditunda; urutan memakai `created_at`/`seq`. |
| `timestamptz` | `TEXT` ISO-8601 UTC (`2026-11-19T10:00:00.000Z`) | Urut leksikografis sama dengan urut waktu. `expires_at` dicek dengan `datetime()`. |
| `jsonb` | `TEXT` + `CHECK(json_valid(..))` | Tanpa indeks GIN; belum dibutuhkan oleh tabel M0. |
| RLS | tidak ada | SQLite tidak punya RLS. Isolasi dijaga di tiga lapis: setiap fungsi data menerima `OrgId` dan memfilter `org_id` di SQL; kunci asing komposit `(org_id, node_id) -> nodes(org_id, id)` membuat baris anak tidak bisa menunjuk node org lain; uji isolasi memanggil setiap fungsi publik dengan id milik org lain. |
| enum | `CHECK(col IN (...))` | role, kind, access, status, actor_kind, result. Daftar yang sama ada di enum Rust. |
| tabel `STRICT` | dipakai | Tipe kolom ditegakkan (butuh SQLite 3.37+, tertanam via `bundled`). |

Tambahan di luar design.md: kolom `audit_log.seq` (urutan per org, unik) dan `UNIQUE(org_id, prev_hash)` agar rantai tidak bisa bercabang; `UNIQUE(org_id, name)` pada nodes.

Koneksi: `PRAGMA foreign_keys=ON` (per koneksi, wajib), `journal_mode=WAL` untuk berkas (gagal bila tidak terterapkan), `busy_timeout=5000 ms`. Semua koneksi harus lewat `db::open`/`db::open_in_memory`. Migrasi: berkas SQL berurutan di `crates/db/migrations/`, ditanam dengan `include_str!`, dijalankan `rusqlite_migration` (versi di `PRAGMA user_version`). Berkas yang sudah dirilis tidak boleh diedit; tambah berkas baru.

### Lapisan kueri
Fungsi biasa di atas `rusqlite::Connection`, tanpa ORM. Tabel milik org hanya terjangkau lewat fungsi yang menerima `OrgId`. Baris org lain dan baris yang tidak ada sama-sama menghasilkan `Error::NotFound` (tidak membocorkan keberadaan). Satu-satunya pencarian lintas org adalah `get_user_by_email` untuk login (users tabel global). Id memakai tipe berbeda (`OrgId`, `NodeId`, ...) dengan `parse` yang memvalidasi bentuk UUID. `OrgId` didefinisikan di crate `db`, bukan di `core-domain`, supaya `core-domain` tidak berubah; bila crate lain butuh, pindahkan.

### Rahasia
`node_secrets` hanya menyimpan `ciphertext` (BLOB) dan `key_id`. **Crate ini tidak mengenkripsi.** `SealedSecret` dan `SecretText` mencetak `[REDACTED]` di `Debug`. Hash token dan password tidak pernah dibaca kembali oleh kueri daftar. Subjek audit diredaksi lewat `core_domain::redact` sebelum disimpan.

### Audit berantai-hash
Per org: `hash = SHA-256(tag "infra-copilot/audit/v1" || bidang berawalan panjang: org, seq, actor_kind, actor_id, action, node_id, subject, result, created_at, prev_hash)`; entri pertama memakai `prev_hash` nol. Trigger menolak `UPDATE` dan `DELETE`. `verify_audit_chain` menghitung ulang dan melaporkan entri pertama yang rusak (`SequenceGap`, `PrevHashMismatch`, `HashMismatch`). Penambahan berjalan di transaksi `IMMEDIATE`. `audit_log.node_id` sengaja tanpa kunci asing agar riwayat bertahan setelah node dihapus.

**SHA-256 ditulis sendiri** (sekitar 60 baris, `crates/db/src/sha256.rs`), diuji dengan vektor NIST FIPS 180-4 (kosong, "abc", 448 bit, 896 bit, sejuta "a") dan batas padding. Alasan: instruksi tugas melarang dependensi kripto tanpa persetujuan. Ini fungsi hash saja, bukan kripto yang menjaga rahasia. Usulan: ganti dengan `sha2` (RustCrypto) saat pustaka kripto dipilih, bersama AES-256-GCM; hash yang tersimpan tetap kompatibel karena SHA-256 baku.

## Risiko dan batas
- **Hash tanpa kunci**: siapa pun yang bisa menulis ulang seluruh berkas dapat menghitung ulang rantai. Pemotongan entri terbaru tidak terdeteksi tanpa jangkar luar (mis. kepala rantai dikirim berkala ke tempat lain, atau HMAC dengan kunci di luar DB). Ditunda.
- **Trigger bisa di-drop** oleh pemegang berkas; rantai yang menangkapnya (diuji), tetapi bukan pencegahan.
- **Tanpa RLS**: satu fungsi yang lupa memfilter `org_id` adalah celah. Mitigasi: uji isolasi per fungsi publik; fungsi baru wajib ditambahkan ke uji itu. Pencarian lintas org tetap harus ditinjau (`security-review`).
- `append_audit` tidak memeriksa `node_id` milik org yang sama (tanpa FK); hanya string. Perlu diputuskan apakah dicek saat penambahan.
- `create_api_token` mensyaratkan pengguna adalah anggota org; tabel `api_tokens` sendiri tidak punya kunci asing komposit ke `memberships`.
- Akses SQLite sinkron; model async (`spawn_blocking` vs `tokio-rusqlite`) tetap terbuka di 0007.
- **Ketidakcocokan versi pada 0007**: tabel 0007 menulis `rusqlite` 0.43 dengan `rusqlite_migration` 2.7, tetapi `rusqlite_migration` 2.7.0 bergantung pada `rusqlite` 0.41 (`libsqlite3-sys` hanya boleh satu). Dipakai `rusqlite` 0.41 (resolver Cargo menolak 0.43). 0007 perlu dikoreksi, atau tunggu rilis `rusqlite_migration` yang mendukung 0.43.

## Ditunda atau terbuka
- **Enkripsi rahasia**: pustaka AES-256-GCM belum dipilih; kunci utama di luar DB, rotasi `key_id`, siapa yang memanggil `get_node_secret`. Butuh keputusan pemilik.
- Pustaka hash (lihat di atas) dan jangkar rantai audit.
- Tabel lain: inventories, comparisons, findings, migrasi, AI, usulan/persetujuan/eksekusi, lisensi, usage_counters.
- Nilai enum `actor_kind` (`user|agent|api_token|system`) dan `result` (`success|failure|denied`) adalah usulan, design.md belum menetapkannya. `plan` organisasi belum dibatasi enum.
- Retensi `audit_log` (design.md 3.8) dan UUID v7.
- Kompatibilitas Postgres untuk cloud (M6): migrasi SQL ini khusus SQLite; antarmuka fungsi dijaga sempit agar bisa diganti.

## Alternatif
- `sqlx`: async tetapi lebih berat (0007).
- Menambah `sha2` sekarang: lebih aman secara rekayasa, tetapi menambah dependensi tanpa persetujuan.
- `org_id` di fungsi lewat objek "sesi tenant" alih-alih argumen: lebih ringkas, namun menyembunyikan filter; argumen eksplisit dipilih agar mudah diaudit.

## Dampak
Crate `db` bergantung pada `core-domain`, `rusqlite` (hanya fitur `bundled`), dan `rusqlite_migration` (tanpa fitur bawaan). Ukuran biner dan waktu mulai belum diukur (0005, 0010).

## Perbaikan setelah tinjauan (2026-11-19)
Migrasi 0001 dan 0003 diubah langsung (belum ada rilis atau DB nyata); setelah rilis, perubahan skema harus lewat migrasi baru.
- `api_tokens` kini punya FK komposit `(org_id, user_id)` ke `memberships` dengan `ON DELETE CASCADE`: token anggota yang dikeluarkan ikut terhapus.
- `audit_log`: trigger `BEFORE INSERT` memaksa `seq` berurutan dan `prev_hash` sama dengan hash entri sebelumnya, sehingga `INSERT OR REPLACE` tidak bisa menimpa entri diam-diam.
- Redaksi subjek audit tidak sadar-JSON dan bisa merusak JSON (mis. kunci `token_count` tanpa tanda kutip). Bila masukan valid tetapi hasil redaksi tidak, teks hasil disimpan sebagai string di `{"redacted_subject": ...}`. Redaktor di `core-domain` sendiri belum diperbaiki; usulan: buat sadar-JSON saat dibutuhkan.
- Belum diperbaiki (catatan tinjauan): `append_audit` membuka transaksinya sendiri, sehingga belum bisa dipakai di dalam transaksi pemanggil; `node_id` audit tidak diperiksa milik org yang sama.

## Pembaruan (2026-11-19, 0011)
`rusqlite_migration` dihapus dan `rusqlite` dinaikkan ke 0.43; migrasi dijalankan oleh `crates/db/src/migrate.rs`. Catatan ketidakcocokan versi di atas sudah tidak berlaku.

## Pembaruan (2026-11-19, 0012)
SHA-256 tulisan sendiri diganti crate `sha2`; hash tersimpan tetap kompatibel.
