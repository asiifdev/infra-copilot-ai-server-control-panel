// Forgiving matching for the command palette: a substring beats a word start beats letters that
// appear in order ("nde" finds "Nodes"). Returns -1 for no match, otherwise a score (lower is better).

export function fuzzyScore(query: string, text: string): number {
  const q = query.trim().toLowerCase();
  if (q === '') return 0;
  const target = text.toLowerCase();
  const at = target.indexOf(q);
  if (at === 0) return 0;
  if (at > 0) return target[at - 1] === ' ' ? 1 : 2 + at / 100;
  // Letters in order, gaps allowed: the tighter the span, the better.
  let from = 0;
  let first = -1;
  let last = -1;
  for (const ch of q) {
    if (ch === ' ') continue;
    const found = target.indexOf(ch, from);
    if (found < 0) return -1;
    if (first < 0) first = found;
    last = found;
    from = found + 1;
  }
  return 10 + (last - first);
}
