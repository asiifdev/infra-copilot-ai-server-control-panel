# 0034. Manusia menyetujui atau menolak Usulan (hanya mencatat, belum mengeksekusi)

- Tanggal: 2026-11-24
- Status: diterima (pemilik: "Langsung implementasi bertahap"; langkah pertama M4)
- Pengambil keputusan: pemilik proyek

## Konteks
Usulan (`proposals`) hanya bisa dibuat asisten sebagai `pending` dan hanya dibaca. Aturan proyek: aksi yang mengubah selalu lewat Usulan + persetujuan manusia (CLAUDE.md butir 3). Eksekusi (M4 penuh) butuh pelaksana, snapshot, dan rollback yang belum ada.

## Keputusan
1. `db::decide_proposal(org, id, Approve|Reject, user)` memindahkan `pending` menjadi `approved`/`rejected` dengan satu UPDATE bersyarat dalam satu transaksi bersama entri audit berantai-hash (`proposal.approve` / `proposal.reject`, aktor `user`, subjek hanya id, tingkat, dan hash argumen). Dua keputusan tidak bisa sama-sama menang. Yang sudah diputuskan menghasilkan `AlreadyDecided`; yang kedaluwarsa ditandai `expired` dan tidak bisa disetujui.
2. `append_audit` dipecah menjadi pembungkus bertransaksi dan `append_audit_in_tx` agar entri audit ikut commit atau rollback bersama perubahannya.
3. Server: `POST /api/proposals/{id}/decision {"decision":"approve|reject"}` → 200 `{id,status}`, 409 `proposal_decided` atau `proposal_expired`, 404 untuk id asing atau salah bentuk. Hanya pengguna yang masuk (bukan alat AI) yang bisa memanggilnya; tidak ada alat agen yang mencapai persetujuan.
4. **Menyetujui tidak menjalankan apa pun.** UI harus mengatakannya. Eksekusi, snapshot, rollback, konfirmasi ketik, dan dua penyetuju (tingkat 4 di Cloud) tetap tugas M4 berikutnya.
5. Tabel `approvals` dan `executions` (design.md 3.6) belum dibuat: audit berantai-hash cukup untuk satu pengguna lokal. Dibuat bersama eksekusi.
