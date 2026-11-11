import type { AgentEvent, Json } from '../../stores/agentRunnerStore';

const object = (value: Json | undefined): Record<string, Json> => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
const text = (value: Json | undefined) => typeof value === 'string' ? value : '';
const summaryDelta = 'item/reasoning/summaryTextDelta';
const summaryPart = 'item/reasoning/summaryPartAdded';
export type CodexTranscriptEntry =
  | { kind: 'summary'; parts: { index: number; text: string }[]; complete: boolean; seq: number }
  | { kind: 'revision'; revisions: AgentEvent[] }
  | { kind: 'malformed' };

const scope = (event: AgentEvent) => {
  const data = object(event.data);
  if (event.run) return ['run', event.run];
  if (text(data.turnId)) return ['turn', text(data.threadId), text(data.turnId)];
  if (text(data.session_id)) return ['session', text(data.session_id)];
  return ['event', event.seq];
};
const itemKey = (event: AgentEvent, id: string) => JSON.stringify([scope(event), text(object(event.data).parent_tool_use_id), id]);
const privateTypes = ['reasoning', 'thinking', 'thinking_delta', 'signature', 'signature_delta', 'redacted_thinking'];
const privateFields = ['thinking', 'thinking_delta', 'signature', 'signature_delta', 'encrypted_content', 'encryptedContent', 'redacted_thinking'];
export function isReasoningEvent(event: AgentEvent) {
  return privateTypes.includes(event.kind) || event.kind.startsWith('item/reasoning/') || privateTypes.includes(text(object(event.data).type)) || privateTypes.includes(text(object(object(event.data).item).type));
}

// Inspector projection only: native evidence stays untouched, but raw reasoning is never
// exposed as prose or JSON. Summary notifications are the only text-bearing allowlist.
export function inspectableAgentEvent(event: AgentEvent): AgentEvent {
  let hasPrivateContent = isReasoningEvent(event);
  const redact = (value: Json): Json => {
    if (Array.isArray(value)) return value.map(redact);
    if (!value || typeof value !== 'object') return value;
    const privateBlock = privateTypes.includes(text(value.type));
    if (privateBlock || Object.keys(value).some(key => privateFields.includes(key))) hasPrivateContent = true;
    return Object.fromEntries(Object.entries(value).filter(([key]) => !privateFields.includes(key) && !(privateBlock && ['content', 'text', 'data', 'delta'].includes(key))).map(([key, entry]) => [key, redact(entry)]));
  };
  if (!isReasoningEvent(event)) {
    const data = redact(event.data);
    return { ...event, text: hasPrivateContent ? null : event.text, data };
  }
  const data = object(event.data);
  const summary = event.kind === summaryDelta || event.kind === summaryPart;
  const safe: Record<string, Json> = {};
  for (const key of ['type', 'id', 'itemId', 'threadId', 'turnId', 'summaryIndex', 'status', 'summary', 'native_message_id', 'block_index', 'parent_tool_use_id']) if (data[key] !== undefined) safe[key] = redact(data[key]);
  if (summary && data.delta !== undefined) safe.delta = data.delta;
  if (data.item !== undefined) safe.item = redact(data.item);
  return { ...event, text: summary ? event.text : null, data: safe };
}

// A map entry with null hides only a superseded projection. All native events remain
// inspectable. Keys include run/item/index; interleaved turns never share a stream.
export function projectCodexTranscript(events: readonly AgentEvent[]): Map<AgentEvent, CodexTranscriptEntry | null> {
  const result = new Map<AgentEvent, CodexTranscriptEntry | null>();
  const streams = new Map<string, { item: string; event: AgentEvent; entry: Extract<CodexTranscriptEntry, { kind: 'summary' }> }>();
  const snapshots = new Map<string, AgentEvent>();
  const revisions = new Map<string, { event: AgentEvent; entries: AgentEvent[] }>();
  for (const event of events) {
    const data = object(event.data);
    if (event.kind === 'turn/plan/updated' || event.kind === 'turn/diff/updated') {
      const key = JSON.stringify([scope(event), event.kind]);
      const previous = revisions.get(key);
      const entries = [...(previous?.entries ?? []), event];
      if (previous) result.set(previous.event, null);
      revisions.set(key, { event, entries });
      result.set(event, { kind: 'revision', revisions: entries });
      continue;
    }
    if (!isReasoningEvent(event)) continue;
    if (event.kind === summaryDelta || event.kind === summaryPart) {
      const id = text(data.itemId), index = data.summaryIndex;
      const delta = typeof data.delta === 'string' ? data.delta : event.text;
      if (!id || typeof index !== 'number' || !Number.isSafeInteger(index) || index < 0 || event.kind === summaryDelta && typeof delta !== 'string') {
        result.set(event, { kind: 'malformed' }); continue;
      }
      const item = itemKey(event, id), key = JSON.stringify([item, index]);
      if (snapshots.has(item)) { result.set(event, null); continue; }
      let stream = streams.get(key);
      if (!stream) {
        const entry: Extract<CodexTranscriptEntry, { kind: 'summary' }> = { kind: 'summary', parts: [{ index, text: '' }], complete: false, seq: event.seq };
        stream = { item, event, entry }; streams.set(key, stream); result.set(event, entry);
      } else result.set(event, null);
      // PartAdded establishes a boundary, not replacement text or a reset.
      if (event.kind === summaryDelta) stream.entry.parts[0].text += delta;
      stream.entry.seq = event.seq;
      continue;
    }
    if (event.kind === 'reasoning' || event.kind === 'item/completed' && object(data.item ?? data).type === 'reasoning') {
      const value = object(data.item ?? data);
      const id = text(value.id);
      if (!id || !Array.isArray(value.summary) || !value.summary.every(part => typeof part === 'string')) {
        result.set(event, { kind: 'malformed' }); continue;
      }
      const item = itemKey(event, id);
      for (const [key, stream] of streams) if (stream.item === item) { result.set(stream.event, null); streams.delete(key); }
      const previous = snapshots.get(item);
      if (previous) result.set(previous, null);
      snapshots.set(item, event);
      result.set(event, { kind: 'summary', parts: value.summary.map((part, index) => ({ index, text: part as string })), complete: true, seq: event.seq });
      continue;
    }
    // Starts and raw reasoning deltas have no user-facing text.
    result.set(event, null);
  }
  return result;
}
