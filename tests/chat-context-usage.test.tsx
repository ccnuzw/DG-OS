import { describe, expect, it } from 'vitest';
import { contextUsage, reportedUsage } from '../src/components/chat/contextUsage';
import type { ChatMessage } from '../src/stores/chatStore';
import { chatBudget } from './helpers/chatBudget';

const message = (seq: number, overrides: Partial<ChatMessage> = {}): ChatMessage => ({
  id: `reply-${seq}`, session_id: 'current-session', seq, role: 'assistant', content: 'reply',
  status: 'completed', task_id: null, finish_reason: 'stop', error_code: null,
  total_tokens: null, result_unknown: false, created_at: 1, updated_at: 1, ...overrides,
});

describe('chat context and loaded usage contracts', () => {
  it('counts the image reserve only once and keeps the host byte cap separate', () => {
    expect(contextUsage(chatBudget({ estimated_input_tokens: 4096, reserved_output_tokens: 1024, image_reserve_tokens: 2048, model_window_tokens: 10240 }))).toEqual({
      requested: 5120, window: 10240, percent: 50, nearLimit: false,
    });
    expect(contextUsage(chatBudget({ model_window_tokens: null, text_bytes: 20000 }))).toMatchObject({ window: null, percent: null, nearLimit: true });
  });

  it('reports only ended assistant usage in the loaded window without treating unknown as zero', () => {
    expect(reportedUsage([
      message(1, { role: 'user', total_tokens: 999 }),
      message(2, { total_tokens: 12 }),
      message(3, { total_tokens: 0 }),
      message(4, { status: 'generating', total_tokens: 999 }),
      message(5, { status: 'interrupted', total_tokens: null, result_unknown: true }),
    ])).toEqual({ count: 3, reported: 2, missing: 1, total: 12, latest: null });
    expect(reportedUsage([message(1)])).toEqual({ count: 1, reported: 0, missing: 1, total: null, latest: null });
    expect(reportedUsage(undefined)).toEqual({ count: 0, reported: 0, missing: 0, total: null, latest: null });
  });
});
