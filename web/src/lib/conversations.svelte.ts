// Starting a conversation, shared by the Assistant page and the command palette. A revision counter
// lets the conversation list on the Assistant page reload when one is made from elsewhere.

import { api, ApiError } from './api';
import { errorKey } from './errors';
import { t } from './i18n.svelte';
import { router } from './router.svelte';
import { toasts } from './toast.svelte';

class ConversationEvents {
  revision = $state(0);
}

export const conversationEvents = new ConversationEvents();

/** Creates an empty conversation and opens it. Failures are shown as a toast. */
export async function startConversation(): Promise<boolean> {
  try {
    const created = await api.createConversation();
    conversationEvents.revision += 1;
    router.go({ name: 'assistant', conversation: String(created.id) });
    return true;
  } catch (cause) {
    toasts.push('error', t(errorKey(cause instanceof ApiError ? cause.code : 'internal')));
    return false;
  }
}
