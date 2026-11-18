// Temporary i18n module for the performance prototype (M0 step 4).
// The real catalogs live in the Rust i18n crate; this one only mirrors the
// "no literal UI text in markup" rule so the measured bundle is representative.

export type Locale = 'en' | 'id';

export const LOCALES: readonly Locale[] = ['en', 'id'];

const catalogs = {
  en: {
    'app.title': 'Infra Copilot',
    'app.subtitle': 'Node inventory (prototype, dummy data)',
    'lang.label': 'Language',
    'lang.en': 'English',
    'lang.id': 'Indonesian',
    'table.caption': 'Nodes',
    'table.col.name': 'Node name',
    'table.col.status': 'Status',
    'table.col.risk': 'Risk tier',
    'table.col.lastSeen': 'Last seen',
    'status.online': 'Online',
    'status.degraded': 'Degraded',
    'status.offline': 'Offline',
    'risk.1': 'Tier 1 (read)',
    'risk.2': 'Tier 2 (low)',
    'risk.3': 'Tier 3 (proposal)',
    'risk.4': 'Tier 4 (destructive)',
    'time.minutes': 'min ago',
    'footer.count': 'rows shown',
  },
  id: {
    'app.title': 'Infra Copilot',
    'app.subtitle': 'Inventaris node (prototipe, data dummy)',
    'lang.label': 'Bahasa',
    'lang.en': 'Inggris',
    'lang.id': 'Indonesia',
    'table.caption': 'Node',
    'table.col.name': 'Nama node',
    'table.col.status': 'Status',
    'table.col.risk': 'Tingkat risiko',
    'table.col.lastSeen': 'Terakhir terlihat',
    'status.online': 'Daring',
    'status.degraded': 'Menurun',
    'status.offline': 'Luring',
    'risk.1': 'Tingkat 1 (baca)',
    'risk.2': 'Tingkat 2 (rendah)',
    'risk.3': 'Tingkat 3 (usulan)',
    'risk.4': 'Tingkat 4 (merusak)',
    'time.minutes': 'mnt lalu',
    'footer.count': 'baris ditampilkan',
  },
} as const;

export type MessageKey = keyof (typeof catalogs)['en'];

/** Look up a message; falls back to English, then to the key itself. */
export function translate(locale: Locale, key: MessageKey): string {
  return catalogs[locale][key] ?? catalogs.en[key] ?? key;
}
