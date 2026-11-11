import { renderToString } from 'katex';
import { mathInputFailure, MAX_RICH_OUTPUT_CHARS } from './richMarkdownSafety';

// Dedicated worker: no document, host IPC, network fetches or persistent macro dictionary.
self.onmessage = (event: MessageEvent<unknown>) => {
  const input = event.data as { source?: unknown; display?: unknown; token?: unknown } | null;
  if (!input || typeof input.source !== 'string' || typeof input.display !== 'boolean' || typeof input.token !== 'string') return;
  const failure = mathInputFailure(input.source);
  if (failure) { self.postMessage({ token: input.token, failure }); return; }
  try {
    const markup = renderToString(input.source, {
      displayMode: input.display, output: 'mathml', throwOnError: true, trust: false,
      strict: 'error', maxExpand: 200, maxSize: 10, globalGroup: false, macros: {},
    });
    self.postMessage(markup.length > MAX_RICH_OUTPUT_CHARS ? { token: input.token, failure: 'limit' } : { token: input.token, markup });
  } catch { self.postMessage({ token: input.token, failure: 'failed' }); }
};
