import type { AgentEvent, AgentSession, Json } from '../../stores/agentRunnerStore';

export interface AgentTokenCounts { input: number | null; output: number | null; cached: number | null; reasoning: number | null; total: number | null }
export interface AgentContextUsage {
  used: number | null; limit: number | null; source: string; kind: 'estimated' | 'unavailable'; seq: number | null;
  latest: AgentTokenCounts | null; cumulative: AgentTokenCounts | null; previous: boolean;
  cacheHit: number | null; costUsd: number | null; costEstimated: boolean; durationMs: number | null;
}
const object = (value: Json | undefined): Record<string, Json> => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
const count = (value: Json | undefined): number | null => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
const amount = (value: Json | undefined): number | null => typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null;
const sum = (...values: (number | null)[]): number | null => values.some(value => value === null) ? null : count(values.reduce<number>((total, value) => total + value!, 0));
function codexCounts(value: Json | undefined): AgentTokenCounts {
  const data = object(value), input = count(data.inputTokens), output = count(data.outputTokens);
  // Cached input and reasoning output are subsets, never extra tokens.
  return { input, output, cached: count(data.cachedInputTokens), reasoning: count(data.reasoningOutputTokens), total: count(data.totalTokens) ?? sum(input, output) };
}
function claudeCounts(value: Json | undefined): AgentTokenCounts {
  const data = object(value);
  // Anthropic's input excludes cache reads and writes. Omitted cache categories
  // contribute no tokens to arithmetic, but remain unknown in their own row.
  const input = sum(count(data.input_tokens), data.cache_read_input_tokens === undefined ? 0 : count(data.cache_read_input_tokens), data.cache_creation_input_tokens === undefined ? 0 : count(data.cache_creation_input_tokens));
  const output = count(data.output_tokens);
  return { input, output, cached: count(data.cache_read_input_tokens), reasoning: null, total: sum(input, output) };
}
function combine(values: AgentTokenCounts[]): AgentTokenCounts | null {
  if (!values.length) return null;
  return Object.fromEntries((['input', 'output', 'cached', 'reasoning', 'total'] as const).map(key => [key, sum(...values.map(value => value[key]))])) as unknown as AgentTokenCounts;
}
const identity = (event: AgentEvent): string | null => event.run || (typeof object(event.data).request_id === 'string' ? object(event.data).request_id as string : null);
function receiptTotals(receipts: Map<string, AgentEvent>, field: 'duration_ms'): number | null {
  if (!receipts.size) return null;
  let total = 0;
  for (const event of receipts.values()) {
    const value = amount(object(event.data)[field]);
    if (value === null) return null;
    total += value;
  }
  return Number.isFinite(total) ? total : null;
}

/** Only native receipts establish usage. Never infer prices or windows from model names. */
export function agentContextUsage(session?: AgentSession): AgentContextUsage {
  const previous = Boolean(session && (session.active_run || ['running', 'waiting'].includes(session.status)));
  const result: AgentContextUsage = { used: null, limit: null, source: '', kind: 'unavailable', seq: null, latest: null, cumulative: null, previous, cacheHit: null, costUsd: null, costEstimated: false, durationMs: null };
  if (!session) return result;
  const receipts = new Map<string, AgentEvent>();
  const receiptOrder: string[] = [];
  // Last receipt wins per run; preserve completion order for latest cumulative snapshots.
  let incomplete = !!session.history_truncated || (session.events[0]?.seq ?? 1) > 1;
  for (const event of session.events) {
    const data = object(event.data);
    if (event.kind === 'history' || data.history === true) incomplete = true;
    if (data.history === true || data.parent_tool_use_id) continue;
    if (event.kind === 'result' || event.kind === 'turn/completed') {
      const id = identity(event);
      if (id && !receipts.has(id)) receiptOrder.push(id);
      if (id) receipts.set(id, event);
    }
  }
  if (session.engine === 'codex') {
    for (let i = session.events.length - 1; i >= 0; --i) {
      const event = session.events[i];
      if (event.kind !== 'thread/tokenUsage/updated') continue;
      const usage = object(object(event.data).tokenUsage);
      result.latest = codexCounts(usage.last);
      result.cumulative = codexCounts(usage.total);
      result.used = sum(result.latest.input, result.latest.output);
      const limit = count(usage.modelContextWindow);
      result.limit = limit && limit > 0 ? limit : null;
      result.seq = count(event.seq);
      result.source = 'thread/tokenUsage/updated';
      break;
    }
  } else {
    // Native modelUsage is cumulative across streaming-input results, includes
    // subagents and compaction, and survives resume. Read the latest snapshot,
    // even after event truncation; never sum it with per-turn message.usage.
    const receipt = receiptOrder.length ? receipts.get(receiptOrder[receiptOrder.length - 1]) : undefined;
    if (receipt) {
      // Result usage is the final main-loop turn receipt; completed assistant
      // blocks carry provisional usage and must never be added to this total.
      result.latest = claudeCounts(object(receipt.data).usage);
      const models = Object.values(object(object(receipt.data).model_usage));
      result.cumulative = combine(models.map(value => {
        const model = object(value);
        const input = sum(count(model.inputTokens), count(model.cacheReadInputTokens), count(model.cacheCreationInputTokens));
        const output = count(model.outputTokens);
        return { input, output, cached: count(model.cacheReadInputTokens), reasoning: null, total: sum(input, output) };
      }));
    }
    for (let i = session.events.length - 1; i >= 0; --i) {
      const event = session.events[i], data = object(event.data);
      if (event.kind !== 'message' || data.type !== 'assistant' || data.history === true || data.parent_tool_use_id || !['native_response', 'native_response_input'].includes(String(data.usage_source))) continue;
      // Only the latest request input estimates occupancy. Block output is
      // provisional, while result/model_usage output is cumulative across calls.
      result.used = claudeCounts(data.usage).input;
      const run = identity(event), receipt = run ? receipts.get(run) : undefined;
      if (typeof data.model === 'string' && receipt) {
        const limit = count(object(object(object(receipt.data).model_limits)[data.model]).contextWindow);
        result.limit = limit && limit > 0 ? limit : null;
      }
      result.seq = count(event.seq);
      result.source = 'assistant.message.input_usage';
      break;
    }
  }
  if (!incomplete) {
    // Duration is per native turn; missing or truncated runs cannot be totalled.
    result.durationMs = receiptTotals(receipts, 'duration_ms');
  }
  const lastReceipt = receiptOrder.length ? receipts.get(receiptOrder[receiptOrder.length - 1]) : undefined;
  if (lastReceipt) {
    const data = object(lastReceipt.data);
    const actual = amount(data.actual_cost_usd);
    const models = Object.values(object(data.model_usage)).map(object);
    if (actual !== null) result.costUsd = actual;
    else if (data.cost_kind === 'actual') result.costUsd = amount(data.total_cost_usd);
    else if (session.engine === 'claude' && models.length && models.every(model => model.costBasis !== 'unknown' && amount(model.costUSD) !== null)) {
      // Native SDK declares absent costBasis equivalent to list pricing. An
      // explicit unknown basis means a guessed default-model rate: hide it.
      result.costUsd = amount(data.total_cost_usd);
      result.costEstimated = result.costUsd !== null;
    }
  }
  const totals = result.cumulative;
  if (totals && totals.input !== null && totals.cached !== null && totals.input > 0 && totals.cached <= totals.input) result.cacheHit = totals.cached / totals.input;
  result.kind = result.used === null ? 'unavailable' : 'estimated';
  return result;
}
