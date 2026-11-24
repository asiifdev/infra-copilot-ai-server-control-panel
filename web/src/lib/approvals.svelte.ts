// How many proposals wait for a decision, for the badge in the navigation. Loaded when the shell
// starts and again after every decision. A failure keeps the last known number (the Approvals
// page shows the real error).

import { api } from './api';

class Approvals {
  pending = $state(0);

  async refresh(signal?: AbortSignal): Promise<void> {
    try {
      const list = await api.proposals('pending', signal);
      if (!signal?.aborted) this.pending = list.length;
    } catch {
      // keep the previous count
    }
  }
}

export const approvals = new Approvals();
