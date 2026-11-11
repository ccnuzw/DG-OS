import type { ChatContextBudget, ChatSummary, ChatUsageSummary } from '../../src/stores/chatStore';

/** Isolated production-UI scenarios only: these values never enter the desktop host. */
export function contextFixture(kind: string | null, modelId: string, summary: ChatSummary | null): ChatContextBudget {
  const blocked = kind === 'blocked';
  return {
    model_id: modelId, model_window_tokens: kind === 'unknown' ? null : blocked ? 200_000 : 32_768,
    host_text_limit_bytes: 24_576, reserved_output_tokens: 1024,
    estimated_input_tokens: summary && kind !== 'summary-no-reduction' ? 3072 : kind === 'near' ? 26_000 : 16_384,
    text_bytes: blocked ? 26_000 : 8192, image_reserve_tokens: 0,
    skill_bytes: 128, tool_schema_bytes: 256, history_bytes: summary ? 1024 : 6144,
    dropped_turns: kind === 'trimmed' ? 3 : 0, summary,
    can_send: !blocked, rejection: blocked ? 'fixed_context_exceeds_limit' : null,
    can_summarize: !summary && kind !== 'empty', summary_unavailable_reason: summary || kind === 'empty' ? 'no_history' : null,
  };
}

export function usageFixture(kind: string | null, summary: ChatSummary | null): ChatUsageSummary {
  const usage: ChatUsageSummary = {
    total_tokens: summary ? 148_960 : 144_960, input_tokens: summary ? 123_400 : 120_000, output_tokens: summary ? 25_560 : 24_960,
    cache_read_tokens: 72_000, cache_write_tokens: 2048, reasoning_tokens: 4096,
    duration_ms: summary ? 43_200 : 40_700, cache_hit_rate: summary ? 72_000 / 123_400 : 0.6,
    call_count: summary ? 13 : 12, reported_count: summary ? 13 : 12, pending_count: 0,
    partial: false, legacy: false,
    coverage: { input: summary ? 13 : 12, output: summary ? 13 : 12, cache_read: summary ? 13 : 12, cache_write: 6, reasoning: 8, duration: summary ? 13 : 12, total: summary ? 13 : 12 },
    latest: { model_id: 'preview-model', total_tokens: summary ? 4000 : 12_080, input_tokens: summary ? 3400 : 10_000, output_tokens: summary ? 600 : 2080, cache_read_tokens: summary ? 0 : 6000, duration_ms: summary ? 2500 : 3200, created_at: 1793966400000, partial: kind === 'partial' },
  };
  if (kind === 'partial') {
    usage.partial = true; usage.legacy = true; usage.reported_count = 8; usage.pending_count = 1;
    usage.cache_hit_rate = null; usage.coverage = { input: 8, output: 7, cache_read: 6, cache_write: 4, reasoning: 4, duration: 9, total: 8 };
  }
  if (['zero', 'unreported', 'empty'].includes(kind ?? '')) {
    const explicitZero = kind === 'zero';
    usage.total_tokens = explicitZero || kind === 'empty' ? 0 : null;
    usage.input_tokens = usage.output_tokens = usage.cache_read_tokens = usage.cache_write_tokens = usage.reasoning_tokens = explicitZero ? 0 : null;
    usage.duration_ms = explicitZero ? 0 : null; usage.cache_hit_rate = null;
    usage.call_count = kind === 'empty' ? 0 : 1; usage.reported_count = explicitZero ? 1 : 0;
    usage.partial = kind === 'unreported'; usage.pending_count = 0;
    usage.coverage = { input: +explicitZero, output: +explicitZero, cache_read: +explicitZero, cache_write: +explicitZero, reasoning: +explicitZero, duration: +explicitZero, total: +explicitZero };
    usage.latest = kind === 'empty' ? null : { model_id: 'preview-model', total_tokens: usage.total_tokens, input_tokens: usage.input_tokens, output_tokens: usage.output_tokens, cache_read_tokens: usage.cache_read_tokens, duration_ms: usage.duration_ms, created_at: 1793966400000, partial: !explicitZero };
  }
  return usage;
}
