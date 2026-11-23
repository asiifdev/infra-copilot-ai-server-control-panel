import { describe, expect, it } from 'vitest';
import { readSse, SseDecoder, SseLimitError, type SseEvent } from '../src/lib/sse';

const enc = new TextEncoder();

function decodeAll(chunks: Uint8Array[]): SseEvent[] {
  const decoder = new SseDecoder(1 << 20);
  const out: SseEvent[] = [];
  for (const chunk of chunks) out.push(...decoder.push(chunk));
  const last = decoder.finish();
  if (last) out.push(last);
  return out;
}

const data = (events: SseEvent[]) => events.map((e) => e.data);

describe('SseDecoder', () => {
  it('decodes simple events with every line ending', () => {
    for (const sep of ['\n', '\r\n', '\r']) {
      const text = `data: one${sep}${sep}data: two${sep}${sep}`;
      expect(data(decodeAll([enc.encode(text)]))).toEqual(['one', 'two']);
    }
  });

  it('gives the same result for every way of cutting the bytes (two pieces and single bytes)', () => {
    const text = 'data: {"a":"é😀"}\r\n\r\n: comment\r\nevent: x\r\ndata: l1\r\ndata: l2\r\n\r\ndata: [DONE]\n\n';
    const bytes = enc.encode(text);
    const whole = decodeAll([bytes]);
    expect(data(whole)).toEqual(['{"a":"é😀"}', 'l1\nl2', '[DONE]']);
    expect(whole[1]?.event).toBe('x');
    for (let cut = 1; cut < bytes.length; cut += 1) {
      expect(decodeAll([bytes.slice(0, cut), bytes.slice(cut)]), `cut at ${cut}`).toEqual(whole);
    }
    const singles = Array.from(bytes, (b) => Uint8Array.of(b));
    expect(decodeAll(singles)).toEqual(whole);
  });

  it('does not count a CRLF split across two chunks as two line ends', () => {
    expect(data(decodeAll([enc.encode('data: a\r'), enc.encode('\ndata: b\r\n\r\n')]))).toEqual(['a\nb']);
  });

  it('ignores comments and unknown fields, and drops one space after the colon only', () => {
    const events = decodeAll([enc.encode(': hi\nid: 7\nretry: 5\ndata:  two spaces\n\nevent: only-event\n\n')]);
    expect(data(events)).toEqual([' two spaces']);
  });

  it('ignores a byte order mark even when it is split', () => {
    const bytes = Uint8Array.from([0xef, 0xbb, 0xbf, ...enc.encode('data: x\n\n')]);
    expect(data(decodeAll([bytes]))).toEqual(['x']);
    expect(data(decodeAll([bytes.slice(0, 1), bytes.slice(1, 2), bytes.slice(2)]))).toEqual(['x']);
  });

  it('keeps an unterminated last event but not a lone comment', () => {
    expect(data(decodeAll([enc.encode('data: tail')]))).toEqual(['tail']);
    expect(decodeAll([enc.encode(': just a comment')])).toEqual([]);
  });

  it('replaces invalid UTF-8 instead of failing', () => {
    const bytes = Uint8Array.from([...enc.encode('data: a'), 0xff, ...enc.encode('b\n\n')]);
    expect(data(decodeAll([bytes]))[0]).toContain('�');
  });

  it('truncates a very long event name', () => {
    const events = decodeAll([enc.encode(`event: ${'n'.repeat(500)}\ndata: x\n\n`)]);
    expect(events[0]?.event?.length).toBe(100);
  });

  it('refuses oversized lines and events', () => {
    const small = new SseDecoder(16);
    expect(() => small.push(enc.encode('data: ' + 'x'.repeat(100))).length).toThrow(SseLimitError);
    const events = new SseDecoder(16);
    expect(() => {
      for (let i = 0; i < 10; i += 1) events.push(enc.encode('data: xxxx\n'));
    }).toThrow(SseLimitError);
  });
});

describe('readSse', () => {
  it('reads a stream to the end and reports each event', async () => {
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(enc.encode('event: delta\ndata: {"text":"a"}\n'));
        controller.enqueue(enc.encode('\nevent: done\ndata: {}\n\n'));
        controller.close();
      },
    });
    const seen: string[] = [];
    await readSse(stream, (e) => seen.push(`${e.event}:${e.data}`));
    expect(seen).toEqual(['delta:{"text":"a"}', 'done:{}']);
  });

  it('rejects when the stream errors (an abort)', async () => {
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.error(new DOMException('aborted', 'AbortError'));
      },
    });
    await expect(readSse(stream, () => {})).rejects.toThrow('aborted');
  });
});
