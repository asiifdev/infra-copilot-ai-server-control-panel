// Server-Sent Events decoder over bytes (a port of crates/ai/src/sse.rs). Bytes in, events out,
// whatever the chunk boundaries are. WHATWG rules: \n, \r\n and \r end a line; a blank line ends an
// event; data lines are joined with \n; ":" lines are comments; one leading space after the colon
// is dropped; a leading byte order mark is ignored. Lines are cut only at newline bytes, which
// never occur inside a multi-byte UTF-8 character, so a character split across chunks decodes.

export interface SseEvent {
  event: string | null;
  data: string;
}

export class SseLimitError extends Error {
  constructor() {
    super('event stream exceeds its size limit');
    this.name = 'SseLimitError';
  }
}

const BOM = [0xef, 0xbb, 0xbf];
const LF = 0x0a;
const CR = 0x0d;
const EVENT_NAME_MAX = 100;

export class SseDecoder {
  #b: number[] = [];
  #d = '';
  #h = false;
  #e: string | null = null;
  #s = true;
  readonly #m: number;
  readonly #t = new TextDecoder('utf-8', { fatal: false });

  constructor(maxBytes: number) {
    this.#m = maxBytes;
  }

  /** Feeds bytes and returns the events they complete. Throws SseLimitError past the limit. */
  push(bytes: Uint8Array): SseEvent[] {
    for (const b of bytes) this.#b.push(b);
    if (this.#s) {
      if (this.#b.length < 3 && this.#b.every((b, i) => b === BOM[i])) return [];
      if (this.#b[0] === BOM[0] && this.#b[1] === BOM[1] && this.#b[2] === BOM[2]) this.#b.splice(0, 3);
      this.#s = false;
    }
    const events: SseEvent[] = [];
    let start = 0;
    let i = 0;
    while (i < this.#b.length) {
      const b = this.#b[i];
      if (b !== LF && b !== CR) {
        i += 1;
        continue;
      }
      // A trailing \r may be the first half of \r\n: wait for the next byte.
      if (b === CR && i + 1 === this.#b.length) break;
      const line = this.#str(this.#b.slice(start, i));
      i += b === CR && this.#b[i + 1] === LF ? 2 : 1;
      start = i;
      const event = this.#ln(line);
      if (event) events.push(event);
    }
    this.#b.splice(0, start);
    if (this.#b.length > this.#m) throw new SseLimitError();
    return events;
  }

  /** Ends the stream; returns an event that was started but not closed by a blank line. */
  finish(): SseEvent | null {
    // A lone \r at the very end ended the last line (it was held back waiting for a \n).
    if (this.#b[this.#b.length - 1] === CR) this.#b.pop();
    if (this.#b.length > 0) {
      const line = this.#str(this.#b);
      this.#b = [];
      this.#ln(line);
    }
    return this.#out();
  }

  #str(bytes: number[]): string {
    return this.#t.decode(new Uint8Array(bytes));
  }

  #ln(line: string): SseEvent | null {
    if (line === '') return this.#out();
    if (line.startsWith(':')) return null;
    const colon = line.indexOf(':');
    const field = colon < 0 ? line : line.slice(0, colon);
    let value = colon < 0 ? '' : line.slice(colon + 1);
    if (value.startsWith(' ')) value = value.slice(1);
    if (field === 'data') {
      if (this.#d.length + value.length + 1 > this.#m) throw new SseLimitError();
      if (this.#h) this.#d += '\n';
      this.#d += value;
      this.#h = true;
    } else if (field === 'event') {
      this.#e = [...value].slice(0, EVENT_NAME_MAX).join('');
    }
    return null;
  }

  #out(): SseEvent | null {
    const event = this.#e;
    this.#e = null;
    if (!this.#h) return null;
    this.#h = false;
    const data = this.#d;
    this.#d = '';
    return { event, data };
  }
}

/**
 * Reads a fetch body to the end and calls onEvent for each complete event. Resolves when the
 * stream ends; rejects on a read error (including abort) or a size limit.
 */
export async function readSse(
  body: ReadableStream<Uint8Array>,
  onEvent: (event: SseEvent) => void,
  maxBytes = 1 << 20,
): Promise<void> {
  const decoder = new SseDecoder(maxBytes);
  const reader = body.getReader();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      for (const event of decoder.push(value)) onEvent(event);
    }
    const last = decoder.finish();
    if (last) onEvent(last);
  } finally {
    reader.releaseLock();
  }
}
