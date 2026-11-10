import type { AgentSession, Json } from '../../stores/agentRunnerStore';

export interface AgentTokenCounts { input: number | null; output: number | null; cached: number | null; reasoning: number | null; total: number | null }
export interface AgentContextUsage {
  used: number | null; limit: number | null; source: string; kind: 'estimated' | 'unavailable'; seq: number | null;
  latest: AgentTokenCounts | null; cumulative: AgentTokenCounts | null; previous: boolean;
}
const object = (value: Json | undefined): Record<string, Json> => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
const count = (value: Json | undefined): number | null => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
function codexCounts(value: Json | undefined): AgentTokenCounts {
  const data = object(value);
  return { input: count(data.inputTokens), output: count(data.outputTokens), cached: count(data.cachedInputTokens), reasoning: count(data.reasoningOutputTokens), total: count(data.totalTokens) };
}

// Last native turn input/output is a context estimate, never a lifetime usage total.
// Cached/reasoning counts are subsets of input/output, not extra context consumption.
// No model-name lookup or guessed window; without native evidence the UI shows unavailable.
export function agentContextUsage(session?: AgentSession): AgentContextUsage {
  const previous = Boolean(session && (session.active_run || ['running', 'waiting'].includes(session.status)));
  if (session?.engine === 'codex') {
    for (let i = session.events.length - 1; i >= 0; --i) {
      const event = session.events[i];
      if (event.kind !== 'thread/tokenUsage/updated') continue;
      const usage = object(object(event.data).tokenUsage);
      const latest = codexCounts(usage.last), cumulative = codexCounts(usage.total);
      const total = latest.input === null || latest.output === null ? null : latest.input + latest.output;
      const used = total !== null && Number.isSafeInteger(total) ? total : null;
      const limit = count(usage.modelContextWindow);
      return { used, limit: limit && limit > 0 ? limit : null, latest, cumulative, previous, source: 'thread/tokenUsage/updated · tokenUsage.last / tokenUsage.total / tokenUsage.modelContextWindow', kind: used === null ? 'unavailable' : 'estimated', seq: count(event.seq) };
    }
  }
  if (session?.engine === 'claude') {
    for (let i = session.events.length - 1; i >= 0; --i) {
      const event = session.events[i], data = object(event.data);
      if (event.kind !== 'message' || data.type !== 'assistant' || data.history === true || data.parent_tool_use_id || data.usage_source !== 'native_response') continue;
      const usage = object(data.usage);
      const input = count(usage.input_tokens), output = count(usage.output_tokens);
      const cached = usage.cache_read_input_tokens === undefined ? 0 : count(usage.cache_read_input_tokens);
      const created = usage.cache_creation_input_tokens === undefined ? 0 : count(usage.cache_creation_input_tokens);
      const total = input === null || output === null || cached === null || created === null ? null : input + output + cached + created;
      const used = total !== null && Number.isSafeInteger(total) ? total : null;
      let limit: number | null = null;
      if (typeof data.model === 'string' && event.run) {
        for (let j = session.events.length - 1; j > i; --j) {
          const result = session.events[j];
          if (result.kind !== 'result' || result.run !== event.run) continue;
          limit = count(object(object(object(result.data).model_limits)[data.model]).contextWindow);
          break;
        }
      }
      return { used, limit: limit && limit > 0 ? limit : null, latest: null, cumulative: null, previous, source: 'assistant.message.usage · native_response; result.modelUsage.contextWindow', kind: used === null ? 'unavailable' : 'estimated', seq: count(event.seq) };
    }
  }
  return { used: null, limit: null, source: '', kind: 'unavailable', seq: null, latest: null, cumulative: null, previous: false };
}
