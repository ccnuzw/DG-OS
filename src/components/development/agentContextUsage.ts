import type { AgentSession, Json } from '../../stores/agentRunnerStore';

export interface AgentContextUsage { used: number | null; limit: number | null; source: string; kind: 'estimated' | 'unavailable'; seq: number | null }
const object = (value: Json | undefined): Record<string, Json> => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
const count = (value: Json | undefined): number | null => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;

// Last native turn input/output is a context estimate, never a lifetime usage total.
// No model-name lookup or guessed window; without native evidence the UI shows unavailable.
export function agentContextUsage(session?: AgentSession): AgentContextUsage {
  if (session?.engine === 'codex') {
    for (let i = session.events.length - 1; i >= 0; --i) {
      const event = session.events[i];
      if (event.kind !== 'thread/tokenUsage/updated') continue;
      const usage = object(object(event.data).tokenUsage);
      const last = object(usage.last);
      const input = count(last.inputTokens), output = count(last.outputTokens);
      const limit = count(usage.modelContextWindow);
      return { used: input === null || output === null ? null : input + output, limit: limit && limit > 0 ? limit : null, source: 'thread/tokenUsage/updated · tokenUsage.last', kind: input === null || output === null ? 'unavailable' : 'estimated', seq: event.seq };
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
      const used = input === null || output === null || cached === null || created === null ? null : input + output + cached + created;
      let limit: number | null = null;
      if (typeof data.model === 'string') {
        for (let j = session.events.length - 1; j > i; --j) {
          const result = session.events[j];
          if (result.kind !== 'result' || result.run !== event.run) continue;
          limit = count(object(object(object(result.data).model_limits)[data.model]).contextWindow);
          break;
        }
      }
      return { used: used !== null && Number.isSafeInteger(used) ? used : null, limit: limit && limit > 0 ? limit : null, source: 'assistant.message.usage · native_response; result.modelUsage.contextWindow', kind: used === null ? 'unavailable' : 'estimated', seq: event.seq };
    }
  }
  return { used: null, limit: null, source: '', kind: 'unavailable', seq: null };
}
