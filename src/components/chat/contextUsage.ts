import { formatNumber } from '../../i18n';
import type { ChatContextBudget, ChatUsageSummary } from '../../stores/chatStore';

/** Chat inspection already includes the image reserve in estimated_input_tokens. */
export function contextUsage(budget: ChatContextBudget) {
  const requested = budget.estimated_input_tokens + budget.reserved_output_tokens;
  const window = budget.model_window_tokens !== null && budget.model_window_tokens > 0 ? budget.model_window_tokens : null;
  const percent = window === null ? null : requested / window * 100;
  const hostNearLimit = budget.host_text_limit_bytes > 0 && budget.text_bytes / budget.host_text_limit_bytes >= 0.8;
  return { requested, window, percent, nearLimit: hostNearLimit || (percent !== null && percent >= 80) };
}

export function compactTokens(value: number): string {
  if (value >= 1_000_000) return `${formatNumber(Math.round(value / 100_000) / 10)}M`;
  if (value >= 1_000) return `${formatNumber(Math.round(value / 100) / 10)}k`;
  return formatNumber(value);
}

export function usageFieldPartial(usage: ChatUsageSummary, field: keyof ChatUsageSummary['coverage']): boolean {
  return usage.coverage[field] > 0 && (usage.legacy || usage.coverage[field] < usage.call_count);
}

/** History may change during compression; every other budget input must stay identical. */
export function compareContextBudgets(before: ChatContextBudget, after: ChatContextBudget, sameInputs: boolean) {
  if (!sameInputs || before.model_id !== after.model_id) return null;
  const previous = contextUsage(before), current = contextUsage(after);
  const percentages = previous.percent !== null && current.percent !== null && previous.window === current.window;
  return {
    before: percentages ? previous.percent! : previous.requested,
    after: percentages ? current.percent! : current.requested,
    unit: percentages ? 'percent' as const : 'tokens' as const,
    lowered: current.requested < previous.requested,
  };
}
