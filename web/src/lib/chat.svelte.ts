// State of one assistant conversation on screen. The answer arrives through server-sent events;
// everything in it (model text, tool names, error text) is untrusted and only ever rendered as
// text. Stopping aborts the fetch, which ends the stream; the server then closes the turn.

import { ApiError, streamTurn } from './api';
import { TEXT_LIMIT } from './text';
import type { ChatMessage, TurnEvent } from './types';

/** A tool call. A reloaded conversation only knows the name; a live turn also has the rest. */
export interface ToolChip {
  name: string;
  tier?: number | string;
  status?: string;
  durationMs?: number;
}

export interface DisplayMessage {
  role: 'user' | 'assistant';
  text: string;
  tools: ToolChip[];
  /** A fixed error code and the untrusted detail text, when the turn ended in an error. */
  error: { code: string; detail: string } | null;
  stopped: boolean;
  /** The reply reached the size limit; the rest was dropped and the turn was ended. */
  truncated: boolean;
}

/**
 * Rebuilds the screen from stored rows. The server stores the tool calls of a turn on an
 * assistant row of its own with empty text; those names are shown as chips on the answer that
 * follows (or on their own when the turn never produced one).
 */
export function fromHistory(history: ChatMessage[]): DisplayMessage[] {
  const out: DisplayMessage[] = [];
  let carried: ToolChip[] = [];
  const flush = () => {
    if (carried.length === 0) return;
    out.push({ role: 'assistant', text: '', tools: carried, error: null, stopped: false, truncated: false });
    carried = [];
  };
  for (const message of history) {
    if (message.role !== 'user' && message.role !== 'assistant') continue; // tool/system rows are not shown
    const tools = (Array.isArray(message.tools) ? message.tools : []).map((name) => ({ name: String(name) }));
    const text = String(message.content ?? '');
    if (message.role === 'assistant' && text === '') {
      carried.push(...tools);
      continue;
    }
    if (message.role === 'user') flush();
    out.push({ role: message.role, text, tools: [...carried, ...tools], error: null, stopped: false, truncated: false });
    carried = [];
  }
  flush();
  return out;
}

export class ChatSession {
  messages = $state<DisplayMessage[]>([]);
  streaming = $state(false);
  /** True once this view sent a turn: only then is the end of a reply announced. */
  live = $state(false);
  #controller: AbortController | null = null;

  reset(history: ChatMessage[] = []): void {
    this.stop();
    this.live = false;
    this.messages = fromHistory(history);
  }

  /** Aborts the turn in flight, if any. */
  stop(): void {
    this.#controller?.abort();
  }

  async send(conversationId: string | number, text: string): Promise<void> {
    if (this.streaming) return;
    const controller = new AbortController();
    this.#controller = controller;
    this.streaming = true;
    this.live = true;
    this.messages.push({ role: 'user', text, tools: [], error: null, stopped: false, truncated: false });
    this.messages.push({ role: 'assistant', text: '', tools: [], error: null, stopped: false, truncated: false });
    const reply = this.messages[this.messages.length - 1]!;
    const onEvent = (event: TurnEvent) => {
      switch (event.type) {
        case 'delta': {
          // The text of one reply is capped (a runaway or hostile stream must not fill memory):
          // keep what fits, say so, and end the turn instead of reading on.
          const room = TEXT_LIMIT - reply.text.length;
          if (event.text.length > room) {
            reply.text += event.text.slice(0, Math.max(0, room));
            reply.truncated = true;
            controller.abort();
          } else {
            reply.text += event.text;
          }
          break;
        }
        case 'tool':
          reply.tools.push({ name: event.name, tier: event.tier, status: event.status, durationMs: event.duration_ms });
          break;
        case 'error':
          reply.error = { code: event.code, detail: event.message };
          break;
        case 'done':
          finished = true;
          break;
      }
    };
    let finished = false;
    try {
      await streamTurn(conversationId, text, onEvent, controller.signal);
      // A stream that ends without "done" or "error" was cut off.
      if (!finished && !reply.error && !reply.truncated) reply.error = { code: 'network', detail: '' };
    } catch (cause) {
      if (controller.signal.aborted) {
        if (!reply.truncated) reply.stopped = true;
      } else if (cause instanceof ApiError) {
        reply.error = { code: cause.code, detail: cause.message };
      } else {
        reply.error = { code: 'network', detail: '' };
      }
    } finally {
      this.streaming = false;
      if (this.#controller === controller) this.#controller = null;
    }
  }
}
