// A loadable value: loading, ready or error, with abort of the previous request.

import { ApiError } from './api';

export type ResourceStatus = 'idle' | 'loading' | 'ready' | 'error';

/**
 * A resource that loads when the component starts and again whenever a reactive value read
 * synchronously by `fetcher` changes; the request in flight is aborted when the component goes.
 * Call during component initialisation.
 */
export function useResource<T>(fetcher: (signal: AbortSignal) => Promise<T>): Resource<T> {
  const resource = new Resource<T>();
  $effect(() => {
    resource.load(fetcher);
    return () => resource.abort();
  });
  return resource;
}

export class Resource<T> {
  status = $state<ResourceStatus>('idle');
  data = $state.raw<T | null>(null);
  error = $state.raw<ApiError | null>(null);
  #controller: AbortController | null = null;
  #fetcher: ((signal: AbortSignal) => Promise<T>) | null = null;

  /** Starts (or restarts) loading; an older request in flight is aborted and ignored. */
  load(fetcher: (signal: AbortSignal) => Promise<T>): void {
    this.#fetcher = fetcher;
    this.#controller?.abort();
    const controller = new AbortController();
    this.#controller = controller;
    this.status = 'loading';
    this.error = null;
    fetcher(controller.signal).then(
      (value) => {
        if (controller.signal.aborted) return;
        this.data = value;
        this.status = 'ready';
      },
      (cause: unknown) => {
        if (controller.signal.aborted) return;
        this.error = cause instanceof ApiError ? cause : new ApiError(0, 'internal', '');
        this.status = 'error';
      },
    );
  }

  reload(): void {
    if (this.#fetcher) this.load(this.#fetcher);
  }

  abort(): void {
    this.#controller?.abort();
  }
}
