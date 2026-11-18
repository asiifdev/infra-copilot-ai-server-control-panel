# Desain teknis (netral terhadap stack)

Status: draf 1. Melengkapi `docs/plan.md`. Belum final, dan mengikat hanya setelah dicatat di `docs/decisions/`.
Ditulis tanpa memilih bahasa/framework. Skema memakai PostgreSQL sebagai acuan (plan bagian 8), tetapi konsepnya tidak bergantung pada ORM.

Semua contoh memakai data fiktif (`example.test`, `203.0.113.x`).

## 1. Istilah

| Istilah | Arti |
|---|---|
| Organisasi (org) | batas kepemilikan dan isolasi data |
| Node | satu mesin (VPS atau lokal) yang dikelola |
| Agen | proses ringan di Node, membuka koneksi **keluar** ke control plane |
| Inventaris | potret read-only keadaan satu Node pada satu waktu |
| Temuan | selisih atau masalah hasil membandingkan dua inventaris |
| Usulan | aksi yang diajukan (oleh AI atau manusia), menunggu persetujuan |
| Aksi | operasi dari daftar yang diizinkan, dijalankan agen/pelaksana |
| Rencana migrasi | dokumen terstruktur sumber → target dengan langkah berurutan |

## 2. Prinsip desain

1. **Satu sumber kebenaran: database.** Konfigurasi pengguna di database. Di lingkungan hanya: URL DB, kunci enkripsi utama, kunci tanda tangan lisensi.
2. **Tak bisa diubah (append-only)** untuk audit, inventaris (snapshot), dan hasil aksi.
3. **Setiap baris milik satu org** (`org_id` di semua tabel kecuali tabel global), dan setiap query memfilternya.
4. **Rahasia tidak disimpan sebagai teks biasa dan tidak pernah dikembalikan oleh API.** API hanya menyatakan "tersimpan" atau "belum".
5. **Aksi = data.** Aksi didefinisikan oleh daftar tertutup dengan skema input, bukan perintah bebas.
6. **Idempoten dan dapat diulang.** Setiap langkah punya kunci idempotensi dan status yang jelas.

## 3. Skema database (draf)

Tipe: `id` = UUID (v7 bila tersedia), waktu = `timestamptz`, `jsonb` untuk data semi-terstruktur. Semua tabel punya `created_at`. Tabel yang bisa diubah punya `updated_at`.

### 3.1 Identitas dan tenant

```
organizations(id, name, slug unique, plan, created_at)
users(id, email unique, password_hash null, name, created_at)         -- password_hash null bila SSO
memberships(id, org_id, user_id, role, created_at)                    -- role: owner|admin|operator|viewer
                                                                      -- unique(org_id, user_id)
api_tokens(id, org_id, user_id, name, token_hash, scopes jsonb, expires_at, revoked_at)
```

### 3.2 Node dan agen

```
nodes(id, org_id, name, kind, access, status, labels jsonb, created_at)
  kind:   vps | local
  access: agent | ssh | local
  status: pending | online | offline | revoked
agents(id, org_id, node_id, token_hash, version, os, arch, last_seen_at, revoked_at)
node_secrets(id, org_id, node_id, purpose, ciphertext, key_id, created_at)
  purpose: ssh_key | ssh_password | sudo_password ...
  -- ciphertext AES-256-GCM, key_id merujuk versi kunci utama (rotasi)
```

### 3.3 Inventaris dan temuan

```
inventories(id, org_id, node_id, taken_at, collector_version, status, summary jsonb)
  status: running | complete | partial | failed
inventory_items(id, org_id, inventory_id, kind, key, data jsonb)
  -- kind: service | listener | process | container | cron | database | env_ref
  --       vhost | dns | cert | package | runtime | queue_link
  -- key: pengenal stabil per kind (lihat bagian 4), unique(inventory_id, kind, key)
comparisons(id, org_id, source_inventory_id, target_inventory_id, plan_id null, created_at)
findings(id, org_id, comparison_id, rule_id, severity, subject_kind, subject_key,
         title, explanation, evidence jsonb, status, resolved_by, resolved_at)
  severity: critical | important | info
  status:   open | accepted | fixed | wontfix
  -- accepted/wontfix wajib punya catatan alasan (di tabel finding_notes)
finding_notes(id, org_id, finding_id, user_id, body, created_at)
```

### 3.4 Migrasi

```
migration_plans(id, org_id, name, source_node_id, target_node_id, status, spec jsonb,
                created_by, created_at)
  status: draft | reviewed | approved | running | paused | done | aborted
  spec: model migrasi (bagian 6)
migration_steps(id, org_id, plan_id, seq, kind, spec jsonb, status, depends_on jsonb,
                started_at, finished_at, result jsonb)
  status: pending | proposed | approved | running | succeeded | failed | skipped | rolled_back
transports(id, org_id, name, kind, config jsonb, secret_id null, verified_at, last_check jsonb)
  kind: rclone | rsync | scp | ssh_pipe | managed
```

### 3.5 AI

```
ai_providers(id, org_id, user_id null, name, base_url, default_model, secret_id,
             params jsonb, is_default, created_at)
  -- user_id null = milik org; terisi = milik pribadi
  -- kunci API di tabel secrets, tidak di sini
ai_provider_models(id, org_id, provider_id, model, role, params jsonb)
  -- role: chat | planner | summarizer | embedding
conversations(id, org_id, user_id, title, scope jsonb, provider_id, created_at, archived_at)
  -- scope: node/plan/finding yang menjadi fokus percakapan
messages(id, org_id, conversation_id, seq, role, content, tool_call_id null, token_usage jsonb,
         redactions int, created_at)
  -- role: system | user | assistant | tool
  -- content sudah diredaksi (bagian 7)
tool_calls(id, org_id, conversation_id, message_id, tool, tier, args jsonb, status, result jsonb,
           proposal_id null, created_at)
memories(id, org_id, kind, scope jsonb, content, embedding vector null, source jsonb,
         created_by, created_at, deleted_at)
  kind: org_fact | task | inventory_summary | preference
```

### 3.6 Usulan, persetujuan, audit

```
proposals(id, org_id, conversation_id null, plan_step_id null, tool, tier, target_node_id,
          args jsonb, reason, impact, dry_run jsonb, status, expires_at, created_by, created_at)
  status: pending | approved | rejected | expired | executed | failed | cancelled
approvals(id, org_id, proposal_id, user_id, decision, comment, typed_confirmation bool,
          created_at)
  -- decision: approve | reject; tingkat 4 di Cloud memerlukan dua approval dari user berbeda
executions(id, org_id, proposal_id, node_id, started_at, finished_at, status, output_ref, verification jsonb)
audit_log(id, org_id, actor_kind, actor_id, action, node_id null, subject jsonb, result, created_at, prev_hash, hash)
  -- append-only; hash berantai untuk mendeteksi perubahan
```

### 3.7 Lisensi dan pemakaian

```
licenses(id, org_id, key_id, tier, limits jsonb, valid_from, valid_to, instance_id)
usage_counters(id, org_id, metric, period, value)    -- nodes, ai_messages, migrations
```

### 3.8 Indeks dan aturan

- Indeks `(org_id, ...)` di semua tabel; kunci unik mengikutkan `org_id` bila relevan.
- `inventory_items`: `(inventory_id, kind)` dan GIN pada `data` untuk pencarian.
- Row-level security (RLS) di Postgres sebagai lapisan kedua isolasi tenant, tidak menggantikan filter di aplikasi.
- Retensi: `messages`, `tool_calls`, dan `inventories` punya kebijakan hapus per paket. `audit_log` tidak dihapus oleh retensi biasa.

## 4. Format inventaris

Inventaris adalah kumpulan `inventory_items`. Setiap item: `kind`, `key` (stabil, dapat dibandingkan antar Node), dan `data`.

Aturan umum:
- **Read-only.** Kolektor tidak mengubah apa pun.
- **Tanpa nilai rahasia.** Untuk konfigurasi simpan nama variabel dan host/port yang dirujuk, bukan nilainya. Kata sandi dalam URL dihapus sebelum disimpan.
- `key` dinormalisasi agar dua mesin bisa dibandingkan (misalnya port, bukan PID).
- Setiap item membawa `seen_via` (cara ditemukan: `proc`, `systemd`, `docker`, `config`) agar temuan bisa dijelaskan.

### 4.1 Jenis item

| kind | key | data (inti) |
|---|---|---|
| `listener` | `tcp:5432` | `{proto, port, bind, process, pid_hint, exposed: public\|loopback\|lan}` |
| `service` | `postgresql` | `{manager: systemd\|launchd\|pm2\|docker, state, version, unit}` |
| `process` | `pm2:web-app` | `{cmd, cwd, mode: dev\|prod\|unknown, supervised: bool, env_refs: [..]}` |
| `container` | `redis` | `{image, ports, networks, env_refs, mounts, restart_policy}` |
| `cron` | `user:root:0 3 * * *:backup` | `{schedule, command_hint, owner, source: crontab\|timer\|panel}` |
| `database` | `pg:5432:appdb` | `{engine, version, size_bytes, tables, rows_total, top_tables:[{name,rows}]}` |
| `env_ref` | `web-app:DATABASE_URL` | `{app, name, target_host, target_port, target_db, resolves_to: key\|null}` |
| `vhost` | `app.example.test` | `{server_names, upstream\|root, tls: bool}` |
| `dns` | `app.example.test` | `{type, value, proxied, provider, account_scope: own\|external}` |
| `cert` | `app.example.test` | `{issuer, not_after, managed_by}` |
| `queue_link` | `redis:6380` | `{producers:[app], consumers:[app], backend_key}` |
| `runtime` / `package` | `php` | `{version, source}` |

`env_ref.resolves_to` memetakan rujukan konfigurasi ke item nyata (misalnya ke `listener` atau `database`), atau `null` bila tidak ditemukan. Itu dasar aturan "rujukan ke host/port yang tidak ada".

### 4.2 Contoh item (fiktif)

```json
{"kind":"env_ref","key":"worker:REDIS_URL",
 "data":{"app":"worker","name":"REDIS_URL","target_host":"127.0.0.1","target_port":6379,
         "resolves_to":"listener:tcp:6379","seen_via":"config"}}
{"kind":"env_ref","key":"api:REDIS_URL",
 "data":{"app":"api","name":"REDIS_URL","target_host":"127.0.0.1","target_port":6380,
         "resolves_to":"listener:tcp:6380","seen_via":"config"}}
```
Dua item ini berbeda backend → aturan antrean memicu temuan `critical` (producer dan consumer tidak satu Redis).

## 5. Perbandingan dan aturan

Pembanding menerima dua inventaris (sumber dan target), menjalankan setiap aturan, dan menghasilkan `findings`.
- Aturan adalah fungsi murni: `(sourceInventory, targetInventory, context) → Finding[]`.
- Setiap temuan membawa `evidence` (item yang menjadi dasar) agar AI dan manusia dapat menelusuri.
- Aturan terdaftar dengan `id` stabil (`PAR-001`...), bobot, dan versi. Detail penulisan: skill `parity-rules`.
- **Uji fungsional** (login, koneksi dari dalam proses) berbeda dari kolektor read-only: ia membuat koneksi nyata, jadi digolongkan **aksi tingkat 1** dan memakai kredensial uji yang disediakan pengguna, bukan kredensial produksi.

## 6. Spesifikasi rencana migrasi (`migration_plans.spec`)

```json
{
  "source": {"node_id": "..."},
  "target": {"node_id": "..."},
  "backup": {"location": "source|local|intermediary", "strategy": "snapshot+delta",
             "exclude": ["logs", "node_modules", "cache"]},
  "transport": {"transport_id": "...", "encryption": "client_side|none"},
  "data": {"databases": [{"key":"pg:5432:appdb","mode":"restore|skip|copy_as_suffix"}],
           "conflict_policy": "never_overwrite"},
  "cutover": {"method": "tunnel|dns", "dns_snapshot": true, "rollback": true},
  "gates": {"no_open_critical_findings": true, "keep_source_until_verified": true}
}
```
Langkah (`migration_steps.kind`): `inventory`, `backup`, `transfer`, `restore`, `verify`, `delta`, `cutover`, `post_verify`, `decommission_source`.
`decommission_source` selalu tingkat 4 dan tidak bisa dijalankan selama `gates` tidak terpenuhi.
`conflict_policy: never_overwrite`: database prod yang sudah berisi tidak ditimpa; data lain disimpan sebagai salinan bersufiks.

## 7. Lapisan AI

### 7.1 Alur satu giliran
1. Pengguna mengirim pesan → disimpan setelah redaksi.
2. Server menyusun konteks: instruksi sistem, memori relevan (org, tugas), ringkasan inventaris/temuan relevan (RAG, bukan seluruh data), riwayat percakapan terpangkas.
3. Panggil penyedia OpenAI-compatible dengan daftar alat (`tools`).
4. Bila model meminta alat: validasi nama dan argumen terhadap skema → tentukan tingkat dari **definisi alat** → tingkat 0–1 dijalankan; tingkat 2+ membuat `proposal`, **tidak dijalankan**.
5. Hasil alat diredaksi, disimpan di `tool_calls`, dikirim kembali ke model.
6. Ulangi sampai model menjawab. Batasi jumlah putaran dan token per giliran.

### 7.2 Redaksi
Sebelum apa pun disimpan atau dikirim ke model: hapus pola rahasia (kata sandi dalam URL, token, kunci pribadi, nilai variabel yang namanya mengandung `SECRET|TOKEN|KEY|PASSWORD`), dan hitung ke `messages.redactions`. Redaksi berlaku juga pada hasil alat dan keluaran Node.

### 7.3 Injeksi prompt
Keluaran dari Node masuk ke model dalam blok bertanda "data tak tepercaya". Penegakan tidak bergantung pada instruksi ke model: alat tingkat 2+ selalu butuh persetujuan di lapisan eksekusi.

### 7.4 Memori
- Ditulis atas permintaan pengguna atau saran AI yang disetujui pengguna. Dapat dilihat, diedit, dihapus.
- Pencarian memakai embedding (bila penyedia mendukung) atau pencarian teks sebagai cadangan.
- Tidak menyimpan rahasia. Terikat org.

## 8. Protokol agen

- **Pendaftaran:** pengguna membuat Node → mendapat token pendaftaran sekali pakai (berumur pendek) → agen menukarnya dengan kredensial agen. Kredensial dapat dicabut.
- **Koneksi:** agen membuka koneksi **keluar** berkelanjutan (WebSocket atau long-poll, usulan) ke control plane lewat TLS. Tidak ada port masuk.
- **Pesan dari server ke agen:** `run_action {action, args, idempotency_key, deadline}`, `collect_inventory {collectors}`, `cancel {id}`.
- **Pesan dari agen ke server:** `hello {version, os, arch}`, `heartbeat`, `result {id, status, output, duration}`, `progress {id, ...}`.
- **Daftar aksi:** tertutup, ada di sisi agen, dengan skema argumen. Contoh awal: `collect_inventory`, `read_log {path_allowlist}`, `service_status`, `restart_service {name}`. Aksi baru ditambahkan lewat rilis agen, bukan oleh server.
- **Keluaran** dibatasi ukuran dan diredaksi di agen sebelum dikirim.
- **Mode SSH (self-host):** pelaksana menjalankan aksi yang sama lewat SSH, dengan daftar dan validasi yang sama. Kunci disimpan terenkripsi.

## 9. Alur Usulan dan persetujuan

```
AI/manusia → proposal(pending, dry_run) → [peninjau menyetujui / menolak]
  → execution(running) → verification → proposal(executed | failed) → audit
```
- `expires_at` membatasi masa berlaku persetujuan.
- Persetujuan mengikat pada **hash isi Usulan** (aksi + argumen + target). Bila isi berubah, persetujuan batal.
- Tingkat 3: dry-run dan cadangan otomatis wajib. Tingkat 4: ketik nama target, dan di Cloud butuh dua persetujuan dari dua pengguna berbeda.
- Pemohon (termasuk AI) tidak boleh menjadi penyetuju Usulan miliknya sendiri.
- Kegagalan aksi tingkat 3–4 tidak diulang otomatis.

## 10. API (garis besar)

Kelompok sumber daya, semua dibatasi org: `orgs`, `nodes`, `agents`, `inventories`, `comparisons`, `findings`, `plans`, `transports`, `conversations`, `proposals`, `approvals`, `audit`, `licenses`.
- Autentikasi: sesi (web) dan token API dengan cakupan.
- Aksi yang mengubah: POST dengan token CSRF/Origin check.
- Tidak ada endpoint yang mengembalikan nilai rahasia.

## 11. Pertanyaan terbuka

1. Satu paket protokol agen untuk SSH dan agen, atau dua pelaksana terpisah?
2. WebSocket atau long-poll sebagai transport agen (kompatibilitas di belakang NAT/proxy)?
3. Apakah embedding wajib untuk MVP, atau pencarian teks cukup?
4. Skema `inventory_items.data` per `kind` diberi versi dan divalidasi dengan JSON Schema?
5. Penyimpanan output besar aksi (log) di database atau objek terpisah (`output_ref`)?
6. Kebijakan retensi bawaan per paket (pesan, inventaris).

## 12. Tambahan: monitor, notifikasi, DNS, adapter (2026-11-19)

### 12.1 Skema monitor dan insiden

```
monitors(id, org_id, node_id null, name, kind, target, interval_sec, timeout_sec, expect jsonb,
         probe_from, confirm_after int, is_active, current_status, last_checked_at, created_at)
  kind: http | tcp | ping | dns | tls | heartbeat
  expect: {status_codes, keyword, max_ms, days_before_expiry}   -- sesuai kind
  probe_from: cloud | agent | both
monitor_checks(id, org_id, monitor_id, probe, status, response_ms, error, checked_at)
  -- index (monitor_id, checked_at); data lama diringkas per jam/hari
incidents(id, org_id, monitor_id, started_at, resolved_at null, cause, acknowledged_by null, severity)
maintenance_windows(id, org_id, monitor_ids jsonb, starts_at, ends_at, reason)
heartbeats(id, org_id, monitor_id, token_hash, last_ping_at, grace_sec)
```
Status: `up | down | degraded | unknown`. Insiden dibuka setelah `confirm_after` kegagalan beruntun dan ditutup setelah satu pengecekan sukses. Bila `probe_from=both`, selisih (lokal sehat, publik gagal) menghasilkan temuan tersendiri.

### 12.2 Notifikasi

```
notification_channels(id, org_id, kind, name, config jsonb, secret_id null, verified_at, is_active)
  kind: telegram  (awal; email|whatsapp|slack|discord|webhook menyusul lewat adapter)
alert_rules(id, org_id, name, match jsonb, channel_ids jsonb, min_severity, quiet_hours jsonb,
            repeat_every_sec null, escalate_to jsonb null, is_active)
  match: {source: monitor|finding|execution|proposal|backup, monitor_ids?, tags?}
alert_events(id, org_id, source, severity, title, body, subject jsonb, dedupe_key, created_at)
alert_deliveries(id, org_id, event_id, channel_id, status, attempts, last_error, sent_at)
  status: queued | sent | failed | suppressed
```
Alur: modul mana pun memanggil satu fungsi `emitAlert(event)`. Mesin alert menulis `alert_events`, mencocokkan `alert_rules`, menerapkan jam tenang, pengelompokan (`dedupe_key`), dan batas laju, lalu mengantre pengiriman per channel lewat adapter. Kegagalan satu channel tidak memblokir channel lain atau pencatatan.

### 12.3 DNS dan tunnel

```
dns_providers(id, org_id, kind, name, secret_id, scope jsonb, verified_at)
  kind: cloudflare | (route53 ...)    -- secret berisi token izin minimum
dns_zones(id, org_id, provider_id, zone_ref, name, account_scope, synced_at)
dns_snapshots(id, org_id, provider_id, zone_id, taken_at, records jsonb, reason)   -- append-only
tunnels(id, org_id, provider_id, node_id, tunnel_ref, name, status, ingress jsonb, connector jsonb)
```
Semua perubahan DNS/tunnel = Usulan tingkat 3: snapshot otomatis sebelum ubah, `dry_run` berisi diff rekaman, rollback dari snapshot. Hostname di luar akun terhubung ditandai `account_scope=external` dan tidak bisa diubah, hanya dibuatkan petunjuk untuk pemilik.

### 12.4 Antarmuka adapter (kontrak, netral terhadap bahasa)

```
NotificationChannel { kind; validateConfig(config); send(message, config, secret) -> {ok, ref, error?}; test() }
DnsProvider         { kind; listZones(); listRecords(zone); plan(changes) -> diff; apply(plan); snapshot(zone); restore(snapshot) }
TunnelProvider      { create(); setIngress(); status(); connectorCommand() }            // bisa bagian DnsProvider Cloudflare
Transport           { kind; configure(); verify(); estimate(size); push(src, dst); pull(src, dst); progress() }
Collector           { kind; run(node) -> InventoryItem[] }                             // read-only
ParityRule          { id; severity; evaluate(source, target, ctx) -> Finding[] }
AiTool              { name; tier; schema; run(args, ctx) -> result; verify(); undo()? }
ServiceManager      { kind; list(); status(name); restart(name) }
AiProvider          { chat(messages, tools, params) -> stream; embed(text)? }
```
Setiap adapter diuji dengan **uji kontrak bersama** (satu rangkaian uji dijalankan pada semua implementasi). Adapter baru tidak boleh mengubah inti.

### 12.5 Komponen UI bersama

Satu paket UI dan token desain. Komponen minimum: `DataTable`, `StatusBadge` (up/down/degraded, tingkat risiko), `Card`, `ConfirmTyped` (ketik nama target), `ProposalCard` (aksi, alasan, dampak, dry-run, setujui/tolak), `DiffView`, `LogViewer`, `EmptyState`, `Toast`, `Filters`, `Sparkline` (riwayat respons), `Timeline` (insiden). Setiap fitur baru memakainya; komponen baru ditambahkan ke paket, bukan ke halaman.

## 13. Tambahan: kejadian error, analisis AI, bot Telegram (2026-11-19)

### 13.1 Skema error dan analisis

```
error_sources(id, org_id, node_id, kind, ref, filter jsonb, is_active)
  kind: log_file | pm2 | systemd | container | cron | http_probe | sdk | webhook
  ref:  pengenal sumber; filter: pola yang disertakan/dikecualikan; hanya jalur dalam allowlist
error_groups(id, org_id, node_id, app_ref, fingerprint, title, level, first_seen, last_seen,
             count, status, sample jsonb)
  status: new | ongoing | resolved | ignored    -- unique(org_id, node_id, fingerprint)
error_events(id, org_id, group_id, occurred_at, message_redacted, stack_redacted, context jsonb)
  -- context tanpa rahasia; retensi dan sampling per paket
analyses(id, org_id, subject_kind, subject_id, status, summary, confidence, created_at, model, token_usage jsonb)
  subject_kind: error_group | incident | finding
analysis_causes(id, org_id, analysis_id, rank, cause, confidence, evidence jsonb)
  -- evidence: rujukan yang bisa dibuka (potongan log, item inventaris, perubahan konfigurasi, probe)
analysis_recommendations(id, org_id, analysis_id, seq, text, kind, proposal_id null)
  kind: info | manual_step | proposal      -- proposal = menaut ke Usulan (bagian 9)
```
Aturan: analisis wajib menyertakan `evidence` per penyebab. Penyebab tanpa bukti ditandai `confidence` rendah dan label dugaan. Rekomendasi aksi tidak dieksekusi, hanya menjadi `proposal`.

Alur penangkapan: agen mengekor sumber terdaftar (read-only, hanya jalur allowlist) → redaksi di agen → kirim `error_event` → pengelompokan berdasarkan sidik jari → bila ambang terpenuhi (kelompok baru, lonjakan, kritis) memanggil `emitAlert` dan, bila diaktifkan, menjadwalkan analisis AI.

### 13.2 Skema bot Telegram

```
telegram_bots(id, org_id, name, secret_id, mode, default_language, is_active)    -- mode: webhook | poll
telegram_links(id, org_id, bot_id, user_id, chat_id, language, status, verified_at)
  status: pending | active | revoked
telegram_link_codes(id, org_id, user_id, code_hash, expires_at, used_at)           -- kode sekali pakai dari panel
telegram_callbacks(id, org_id, link_id, token_hash, action, subject jsonb, content_hash, expires_at, used_at)
  -- token tombol: sekali pakai, berumur pendek, terikat hash isi Usulan dan pengguna
```
Aturan eksekusi dari bot:
- Perintah dan tombol dipetakan ke **alat yang sama** dengan asisten panel, melalui lapisan eksekusi yang sama. Tidak ada jalur khusus bot.
- Tingkat 0–1 langsung; tingkat 2 satu tombol dari chat pribadi terhubung; tingkat 3–4 bot hanya mengirim tautan ke Usulan di panel.
- Callback diverifikasi: pengguna terhubung dan aktif, token belum dipakai dan belum kedaluwarsa, hash isi Usulan masih sama. Semua dicatat di `audit_log`.
- Pesan keluar diredaksi. Bahasa dipilih per `telegram_links.language`; teks pesan berasal dari katalog terjemahan (komponen reusable `i18n`).

### 13.3 Adapter tambahan

```
LogSource      { kind; tail(spec) -> stream<line>; validate(spec) }          // agen: hanya jalur allowlist
ErrorGrouper   { fingerprint(event) -> string }                               // dapat diganti per bahasa/runtime
Analyzer       { analyze(subject, evidence) -> Analysis }                     // memakai AiProvider + alat baca
ChatBotChannel { kind; handleUpdate(update); send(message, buttons) }         // Telegram; Slack/Discord kemudian
```
`NotificationChannel` (12.4) dan `ChatBotChannel` dipisah: yang pertama satu arah, yang kedua dua arah (perintah dan tombol). Telegram mengimplementasikan keduanya.

## 14. Tambahan: i18n, node macOS, selisih OS/arsitektur (2026-11-19)

### 14.1 i18n (ID/EN minimal)
- `users.locale` dan `organizations.default_locale` (kode BCP 47, awal `en`, `id`). Pengguna memilih bahasa; bot Telegram memakai `telegram_links.language`.
- Semua teks yang tampil ke pengguna lewat katalog terjemahan (kunci stabil, bukan teks tetap): antarmuka, pesan galat, notifikasi, pesan bot, templat laporan temuan. Kunci yang belum diterjemahkan jatuh ke bahasa Inggris dan dicatat.
- **AI menjawab dalam bahasa pengguna.** Instruksi sistem memuat locale, tetapi kunci dan nama alat tetap bahasa Inggris. Redaksi dan aturan keamanan tidak bergantung bahasa.
- Penjelasan aturan kesetaraan (`ParityRule`) punya teks per locale di katalog, bukan di kode aturan.
- Tanggal, angka, dan zona waktu mengikuti locale dan zona waktu pengguna; data internal selalu UTC.

### 14.2 Mesin lokal (macOS) dan selisih OS/arsitektur
Target utama tetap Linux; macOS adalah dukungan tambahan. Agen yang sama berjalan di semua OS, hanya kolektor dan `ServiceManager`-nya yang berbeda.
- `nodes.os` dan `nodes.arch` (`darwin|linux`, `arm64|x86_64`) dicatat dari agen. `ServiceManager` punya implementasi `launchd`, `brew services`, `pm2`, `docker`.
- Kolektor tambahan: `brew` (formula dan layanan), `launchd` (job dan jadwal), Docker Desktop (container dan arsitektur image).
- Item inventaris tambahan: `platform` (`{os, arch, version}`), dan pada `container` field `image_arch` serta `emulated: bool`.
- Aturan kesetaraan baru (nomor lanjutan `PAR-0xx`): (a) selisih OS/arsitektur antara sumber dan target, (b) biner atau image tanpa padanan arsitektur target, (c) jalur atau berkas yang peka huruf besar/kecil, (d) rujukan ke alamat atau port dev lokal (`localhost`, port dev) yang ikut ke target, (e) layanan yang dikelola launchd tanpa padanan systemd.
- Provisioning target kosong: rencana menghasilkan langkah (`install_package`, `install_runtime`, `create_service`) yang masing-masing menjadi Usulan; tidak ada langkah yang berjalan tanpa persetujuan.

## 15. Tambahan: deployment (2026-11-19)

### 15.1 Skema
```
git_connections(id, org_id, node_id null, kind, auth, secret_id null, status, created_at)
  kind: github | gitlab | generic
  auth: server_session | oauth | deploy_key     -- server_session: memakai kredensial git milik server, tidak disimpan oleh produk
projects(id, org_id, node_id, name, repo_url, git_connection_id, branch, stack, status, created_at)
build_profiles(id, org_id, project_id, detected jsonb, overrides jsonb, source, updated_at)
  -- detected: {stack, runtime, version, install, build, start, output_dir, port, health_path, evidence:[berkas penanda]}
  -- overrides: nilai yang diubah pengguna; source: ui | repo_file (panel.yml)
processes(id, org_id, project_id, kind, command, instances, resources jsonb)    -- kind: web | worker | scheduler
env_vars(id, org_id, project_id, environment, name, value_secret_id, is_secret)  -- nilai terenkripsi, tidak dikembalikan API
deployments(id, org_id, project_id, environment, ref, commit_sha, trigger, status, started_at, finished_at, release_id null)
  trigger: manual | webhook | ai_proposal | rollback      status: queued|building|deploying|live|failed|rolled_back
deployment_steps(id, org_id, deployment_id, seq, name, status, log_ref, started_at, finished_at)
releases(id, org_id, project_id, deployment_id, path, created_at, is_current)    -- rilis lama disimpan untuk rollback
domains_bindings(id, org_id, project_id, hostname, dns_record_ref null, tls_status)
```

### 15.2 Adapter tambahan
```
GitProvider    { kind; listRepos(); clone(repo, auth); fetch(); webhookSetup() }       // auth server_session: memakai git/SSH milik server
StackDetector  { id; detect(repoTree) -> {stack, confidence, evidence} | null }        // satu per stack, diurut berdasarkan keyakinan
Builder        { stack; plan(profile) -> steps; run(plan) -> release }                  // Node, PHP, Python, Go, .NET
Runtime        { kind: systemd|pm2|docker|php-fpm; start/stop/reload/health }
ReleaseSwitch  { prepare(); switch(); rollback() }                                       // nginx upstream/symlink tanpa henti
```
Detektor memberi `evidence` (berkas yang membuat kesimpulan), sehingga tampilan UI dan AI dapat menjelaskan "kenapa dianggap Laravel". Bila ada beberapa kandidat, pengguna memilih; detektor tidak menebak diam-diam.

### 15.3 Aturan eksekusi
- Build berjalan sebagai pengguna sistem terpisah tanpa akses ke kunci git atau rahasia produksi di luar yang dibutuhkan.
- Kredensial `server_session` tidak pernah dibaca atau ditampilkan; hanya dipakai oleh langkah `clone`/`fetch` yang dijalankan agen untuk repo yang dipilih.
- Pengalihan lalu lintas hanya setelah pemeriksaan kesehatan lulus. Gagal → rilis lama tetap aktif, deploy ditandai gagal, analisis AI dapat dijadwalkan.
- `panel.yml` di repo menimpa `build_profiles.overrides`; perbedaannya ditampilkan di UI.
- Deploy dari UI oleh manusia berjalan langsung; deploy yang diusulkan AI menjadi `proposal` (tingkat 3 untuk produksi, tingkat 2 untuk pratinjau).
