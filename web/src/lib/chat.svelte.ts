// State of one assistant conversation on screen. The answer arrives through server-sent events;
// everything in it (model text, tool names, error text) is untrusted and only ever rendered as
// text. Stopping aborts the fetch, which ends the stream; the server then closes the turn.

import { ApiError, streamTurn } from './api';
import type { ChatMessage, TurnEvent } from './types';

export interface ToolChip {
  name: string;
  tier: number | string;
  status: string;
  durationMs?: number;
}

export interface DisplayMessage {
  role: 'user' | 'assistant';
  text: string;
  tools: ToolChip[];
  /** A fixed error code and the untrusted detail text, when the turn ended in an error. */
  error: { code: string; detail: string } | null;
  stopped: boolean;
}

export function fromHistory(history: ChatMessage[]): DisplayMessage[] {
  const out: DisplayMessage[] = [];
  for (const message of history) {
    if (message.role !== 'user' && message.role !== 'assistant') continue; // tool/system rows are not shown
    out.push({ role: message.role, text: String(message.content ?? ''), tools: [], error: null, stopped: false });
  }
  return out;
}

export class ChatSession {
  messages = $state<DisplayMessage[]>([]);
  streaming = $state(false);
  #controller: AbortController | null = null;

  reset(history: ChatMessage[] = []): void {
    this.stop();
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
    this.messages.push({ role: 'user', text, tools: [], error: null, stopped: false });
    this.messages.push({ role: 'assistant', text: '', tools: [], error: null, stopped: false });
    const reply = this.messages[this.messages.length - 1]!;
    const onEvent = (event: TurnEvent) => {
      switch (event.type) {
        case 'delta':
          reply.text += event.text;
          break;
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
      if (!finished && !reply.error) reply.error = { code: 'network', detail: '' };
    } catch (cause) {
      if (controller.signal.aborted) {
        reply.stopped = true;
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
