import type { ChatContextBudget, ChatMessage } from '../../stores/chatStore';

/** Chat inspection already includes the image reserve in estimated_input_tokens. */
export function contextUsage(budget: ChatContextBudget) {
  const requested = budget.estimated_input_tokens + budget.reserved_output_tokens;
  const window = budget.model_window_tokens !== null && budget.model_window_tokens > 0 ? budget.model_window_tokens : null;
  const percent = window === null ? null : requested / window * 100;
  const hostNearLimit = budget.host_text_limit_bytes > 0 && budget.text_bytes / budget.host_text_limit_bytes >= 0.8;
  return { requested, window, percent, nearLimit: hostNearLimit || (percent !== null && percent >= 80) };
}

/** Only provider-reported totals on ended assistant messages in the loaded window. */
export function reportedUsage(messages: readonly ChatMessage[] | undefined) {
  let count = 0;
  let reported = 0;
  let sum = 0;
  let latest: ChatMessage | undefined;
  for (const message of messages ?? []) {
    if (message.role !== 'assistant' || message.status === 'generating') continue;
    count++;
    if (!latest || message.seq > latest.seq) latest = message;
    if (message.total_tokens !== null && Number.isFinite(message.total_tokens) && message.total_tokens >= 0) {
      reported++;
      sum += message.total_tokens;
    }
  }
  const lastTotal = latest?.total_tokens;
  return {
    count,
    reported,
    missing: count - reported,
    total: reported > 0 ? sum : null,
    latest: lastTotal !== undefined && lastTotal !== null && Number.isFinite(lastTotal) && lastTotal >= 0 ? lastTotal : null,
  };
}
