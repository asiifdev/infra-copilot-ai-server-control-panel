# Prosedur audit capability Tauri v2

Dasar: halaman resmi Capabilities, Permissions, Command Scopes, Runtime Authority, dan referensi konfigurasi (dibaca 2026-11-19). Langkah praktis di bawah adalah metode kerja, bukan teks dokumentasi.

## Langkah

1. **Inventaris.** `ls src-tauri/capabilities src-tauri/permissions`; baca `app.security.capabilities` di `tauri.conf.json`. Aturan resmi: semua file di `capabilities/` aktif otomatis, kecuali `capabilities` diisi eksplisit (maka hanya itu yang aktif).
2. **Target.** Untuk tiap capability catat `windows`, `webviews`, `platforms`, `local`, `remote`. Tolak `"*"` tanpa alasan. Label window, bukan judul, yang menjadi batas.
3. **Izin.** Untuk tiap entri: string (izin penuh tanpa scope tambahan) atau objek dengan `allow`/`deny`. Tandai: `core:default`, `<plugin>:default`, izin `execute|spawn|write|remove|install`, dan izin tanpa scope.
4. **Scope.** Deny mengalahkan allow. Periksa glob terlalu luas (`$HOME/**`, `**`), folder data webview, jalur kredensial. Periksa apakah command pelaksana benar-benar menegakkan scope (audit kode Rust; Tauri tidak melakukannya untuk command buatan sendiri).
5. **Union.** Hitung izin efektif tiap window: gabungan semua capability yang mencakupnya.
6. **Command.** Bandingkan `invoke_handler`, `AppManifest::commands`, dan izin. Command terdaftar tapi tidak dibatasi manifest bisa dipanggil semua window (perilaku bawaan resmi).
7. **Remote.** Setiap `remote.urls` = temuan tingkat tinggi; di Linux dan Android iframe tidak dibedakan dari window.
8. **Diff.** Bandingkan dengan versi sebelumnya; tiap izin baru butuh alasan tertulis.
9. **Uji.** Dari DevTools window tanpa capability, `invoke` harus ditolak; window dengan capability hanya bisa memanggil izin yang tercantum. Catat yang belum diuji.

## Templat laporan

| Capability | Window | Izin baru/berubah | Scope | Risiko | Keputusan |
|---|---|---|---|---|---|

Bagian: diperiksa; bukti (perintah/uji); belum diperiksa.

## Tanda bahaya cepat

- `shell:allow-execute` / `spawn` dengan `cmd` bebas, `sh`, `bash`, `cmd`, atau validator `\S+`.
- `fs:*` dengan `$HOME`, `/`, atau `**` tanpa deny.
- `http:default` dengan URL `*`.
- `remote.urls` apa pun; `frontendDist` berupa URL; `csp: null`.
- `dangerousDisableAssetCspModification`, `dangerousInsecureTransportProtocol` bernilai true.
- `windows: ["*"]` dengan izin plugin berat.
