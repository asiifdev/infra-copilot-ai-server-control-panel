// Relative time ("in 2 days", "3 hours ago") in the language of the page, from an ISO timestamp.

const STEPS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['day', 86_400_000],
  ['hour', 3_600_000],
  ['minute', 60_000],
];

export interface Relative {
  /** True when the moment already passed. */
  past: boolean;
  text: string;
}

export function relativeTime(value: string | null | undefined, locale: string, now: number = Date.now()): Relative | null {
  if (!value) return null;
  const at = new Date(value).getTime();
  if (Number.isNaN(at)) return null;
  const diff = at - now;
  const abs = Math.abs(diff);
  let unit: Intl.RelativeTimeFormatUnit = 'minute';
  let size = 60_000;
  for (const [name, ms] of STEPS) {
    if (abs >= ms) {
      unit = name;
      size = ms;
      break;
    }
  }
  const amount = Math.round(diff / size);
  try {
    return { past: diff < 0, text: new Intl.RelativeTimeFormat(locale, { numeric: 'auto' }).format(amount, unit) };
  } catch {
    return null;
  }
}
