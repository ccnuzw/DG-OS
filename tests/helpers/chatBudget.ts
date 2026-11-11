import type { ChatContextBudget } from '../../src/stores/chatStore';

/** Synthetic host estimate: no Provider calls are made by frontend regressions. */
export const chatBudget = (overrides: Partial<ChatContextBudget> = {}): ChatContextBudget => ({
  model_id: 'chat1', model_window_tokens: 32768, host_text_limit_bytes: 24576,
  reserved_output_tokens: 1024, estimated_input_tokens: 128, text_bytes: 512,
  image_reserve_tokens: 0, skill_bytes: 0, tool_schema_bytes: 0, history_bytes: 0,
  dropped_turns: 0, summary: null, can_send: true, rejection: null,
  can_summarize: true, summary_unavailable_reason: null, ...overrides,
});
