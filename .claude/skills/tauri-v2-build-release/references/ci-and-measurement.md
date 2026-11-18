# Referensi: workflow CI dan penempatan rahasia

Sumber: v2.tauri.app/distribute/pipelines/github/ dan /distribute/sign/macos/ (dibaca 2026-11-19). Versi action mengikuti halaman itu; periksa ulang sebelum dipakai (`checkout@v7`, `setup-node@v6`, `tauri-action@v1`).

## Workflow dasar (dari dokumen resmi, diringkas)

```yaml
name: publish
on:
  workflow_dispatch:
  push:
    branches: [release]
jobs:
  publish-tauri:
    permissions:
      contents: write
    strategy:
      fail-fast: false
      matrix:
        include:
          - { platform: macos-latest, args: '--target aarch64-apple-darwin' }
          - { platform: macos-latest, args: '--target x86_64-apple-darwin' }
          - { platform: ubuntu-22.04, args: '' }
          - { platform: windows-latest, args: '' }   # nanti, sesuai 0003
    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v7
      - name: deps (ubuntu)
        if: matrix.platform == 'ubuntu-22.04'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf xdg-utils
      - uses: actions/setup-node@v6
        with: { node-version: lts/*, cache: npm }
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.platform == 'macos-latest' && 'aarch64-apple-darwin,x86_64-apple-darwin' || '' }}
      - uses: swatinem/rust-cache@v2
        with: { workspaces: './src-tauri -> target' }
      - run: npm ci
      - uses: tauri-apps/tauri-action@v1
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          APPLE_API_ISSUER: ${{ secrets.APPLE_API_ISSUER }}
          APPLE_API_KEY: ${{ secrets.APPLE_API_KEY }}
          # APPLE_API_KEY_PATH: butuh berkas .p8 di runner; cara menulisnya dari rahasia
          # TIDAK ada di dokumen yang dibaca (tidak terverifikasi), tulis ke berkas sementara.
        with:
          tagName: app-v__VERSION__
          releaseName: 'App v__VERSION__'
          releaseDraft: true
          prerelease: false
          args: ${{ matrix.args }}
```

Catatan:
- Bila build macOS tanpa sertifikat Apple, dokumen menyarankan identitas ad-hoc agar build Apple Silicon dari GitHub Releases tidak dianggap "rusak".
- Ubuntu ARM (`ubuntu-22.04-arm`) hanya di repo publik; QEMU (`arm-runner-action`) sangat lambat (uncached ~1 jam untuk proyek baru).
- Token `GITHUB_TOKEN` default hanya baca; atur `permissions: contents: write` atau izin tulis di pengaturan repo (galat "Resource not accessible by integration").
- Nilai rahasia tidak boleh dicetak; jangan `echo` ke log (aturan 2 CLAUDE.md).

## Pengukuran per OS (praktik umum, TIDAK dari dokumen Tauri, tidak terverifikasi)

| Metrik | macOS | Linux | Windows |
|---|---|---|---|
| Ukuran biner/bundel | `stat -f%z`, `du -sh App.app` | `stat -c%s`, `ls -l *.deb *.AppImage` | `(Get-Item x).Length` |
| Memori diam | `ps -o rss= -p <pid>` + proses WebKit turunan | `ps -o rss=` + proses WebKitGTK turunan | `Get-Process` + proses `msedgewebview2` |
| Waktu mulai | cap waktu proses -> event "window ready" di log | sama | sama |

Mengukur hanya proses utama menyesatkan karena webview berjalan di proses terpisah; jumlahkan keluarga proses. Tetapkan anggaran dari median beberapa kali jalan pada mesin CI tetap, bukan angka tebakan (0005).
