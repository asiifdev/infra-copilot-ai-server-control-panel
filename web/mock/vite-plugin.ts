// Serves the mock API from the Vite dev server (npm run dev). Development only: it is never part
// of the production build. It applies the Host and Origin rules of ADR 0029 on top of the
// contract rules implemented in server.ts, and prints the one-time sign-in link like the real
// server does.

import type { IncomingMessage } from 'node:http';
import type { Plugin } from 'vite';
import { createMock, MOCK_TOKEN, type MockRequest } from './server.ts';

const BODY_LIMIT = 64 * 1024;

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on('data', (chunk: Buffer) => {
      size += chunk.length;
      if (size <= BODY_LIMIT + 1) chunks.push(chunk);
    });
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

export function mockApiPlugin(): Plugin {
  return {
    name: 'nodivara-mock-api',
    apply: 'serve',
    configureServer(server) {
      const mock = createMock({ turnDelayMs: 40, splitChunks: false });
      server.httpServer?.once('listening', () => {
        const address = server.httpServer?.address();
        if (address && typeof address === 'object') {
          server.config.logger.info(`\n  Sign in (one-time link, mock): http://127.0.0.1:${address.port}/#t=${MOCK_TOKEN}\n`);
        }
      });
      server.middlewares.use(async (req, res, next) => {
        const url = new URL(req.url ?? '/', 'http://placeholder.invalid');
        if (!url.pathname.startsWith('/api/') && url.pathname !== '/healthz') return next();
        const host = req.headers.host ?? '';
        const refuse = (status: number) => {
          res.statusCode = status;
          res.end();
        };
        // Host must be the bind address (DNS rebinding); changing requests must come from it too.
        if (!/^127\.0\.0\.1:\d+$/.test(host)) return refuse(403);
        const method = (req.method ?? 'GET').toUpperCase();
        if (method !== 'GET' && req.headers.origin !== `http://${host}`) return refuse(403);
        if (mock.state.sessions.size === 0) mock.state.token.used = false;

        const headers = new Headers();
        for (const [key, value] of Object.entries(req.headers)) {
          if (typeof value === 'string') headers.set(key, value);
        }
        const controller = new AbortController();
        res.on('close', () => controller.abort());
        const request: MockRequest = {
          method,
          path: url.pathname,
          query: url.searchParams,
          headers,
          body: method === 'GET' ? '' : await readBody(req),
          signal: controller.signal,
        };
        const response = await mock.handle(request);
        res.statusCode = response.status;
        response.headers.forEach((value, key) => res.setHeader(key, value));
        res.setHeader('X-Content-Type-Options', 'nosniff');
        res.setHeader('Referrer-Policy', 'no-referrer');
        if (!response.body) return res.end();
        const reader = response.body.getReader();
        try {
          for (;;) {
            const { done, value } = await reader.read();
            if (done) break;
            res.write(value);
          }
        } catch {
          // The client went away: the stream was aborted.
        }
        res.end();
      });
    },
  };
}
