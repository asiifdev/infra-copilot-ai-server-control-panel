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
  #buf: number[] = [];
  #data = '';
  #hasData = false;
  #event: string | null = null;
  #atStart = true;
  readonly #max: number;
  readonly #text = new TextDecoder('utf-8', { fatal: false });

  constructor(maxBytes: number) {
    this.#max = maxBytes;
  }

  /** Feeds bytes and returns the events they complete. Throws SseLimitError past the limit. */
  push(bytes: Uint8Array): SseEvent[] {
    for (const b of bytes) this.#buf.push(b);
    if (this.#atStart) {
      if (this.#buf.length < 3 && this.#buf.every((b, i) => b === BOM[i])) return [];
      if (this.#buf[0] === BOM[0] && this.#buf[1] === BOM[1] && this.#buf[2] === BOM[2]) {
        this.#buf.splice(0, 3);
      }
      this.#atStart = false;
    }
    const events: SseEvent[] = [];
    let start = 0;
    let i = 0;
    while (i < this.#buf.length) {
      const b = this.#buf[i];
      if (b !== LF && b !== CR) {
        i += 1;
        continue;
      }
      // A trailing \r may be the first half of \r\n: wait for the next byte.
      if (b === CR && i + 1 === this.#buf.length) break;
      const line = this.#decode(this.#buf.slice(start, i));
      i += b === CR && this.#buf[i + 1] === LF ? 2 : 1;
      start = i;
      const event = this.#line(line);
      if (event) events.push(event);
    }
    this.#buf.splice(0, start);
    if (this.#buf.length > this.#max) throw new SseLimitError();
    return events;
  }

  /** Ends the stream; returns an event that was started but not closed by a blank line. */
  finish(): SseEvent | null {
    if (this.#buf.length > 0) {
      const line = this.#decode(this.#buf);
      this.#buf = [];
      this.#line(line);
    }
    return this.#dispatch();
  }

  #decode(bytes: number[]): string {
    return this.#text.decode(new Uint8Array(bytes));
  }

  #line(line: string): SseEvent | null {
    if (line === '') return this.#dispatch();
    if (line.startsWith(':')) return null;
    const colon = line.indexOf(':');
    const field = colon < 0 ? line : line.slice(0, colon);
    let value = colon < 0 ? '' : line.slice(colon + 1);
    if (value.startsWith(' ')) value = value.slice(1);
    if (field === 'data') {
      if (this.#data.length + value.length + 1 > this.#max) throw new SseLimitError();
      if (this.#hasData) this.#data += '\n';
      this.#data += value;
      this.#hasData = true;
    } else if (field === 'event') {
      this.#event = [...value].slice(0, EVENT_NAME_MAX).join('');
    }
    return null;
  }

  #dispatch(): SseEvent | null {
    const event = this.#event;
    this.#event = null;
    if (!this.#hasData) return null;
    this.#hasData = false;
    const data = this.#data;
    this.#data = '';
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
