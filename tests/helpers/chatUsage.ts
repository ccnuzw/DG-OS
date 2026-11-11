import type { ChatUsageSummary } from '../../src/stores/chatStore';

/** Synthetic durable-session projection, independent of loaded message pages. */
export const chatUsage = (overrides: Partial<ChatUsageSummary> = {}): ChatUsageSummary => ({
  total_tokens: 1600, input_tokens: 1200, output_tokens: 400,
  cache_read_tokens: 600, cache_write_tokens: null, reasoning_tokens: null,
  duration_ms: 2500, cache_hit_rate: 0.5,
  call_count: 2, reported_count: 2, pending_count: 0, partial: false, legacy: false,
  coverage: { input: 2, output: 2, cache_read: 2, cache_write: 0, reasoning: 0, duration: 2, total: 2 },
  latest: { model_id: 'chat1', total_tokens: 800, input_tokens: 600, output_tokens: 200, cache_read_tokens: 300, duration_ms: 1250, created_at: 2000, partial: false },
  ...overrides,
});
