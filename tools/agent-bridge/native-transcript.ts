import { createHash } from 'node:crypto';
import { z } from 'zod';

export const responseUsage = z.object({ input_tokens: z.number().nonnegative().optional(), output_tokens: z.number().nonnegative().optional(), cache_read_input_tokens: z.number().nonnegative().nullable().optional(), cache_creation_input_tokens: z.number().nonnegative().nullable().optional() });
const object = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const string = (value: unknown): string | undefined => typeof value === 'string' && value.length > 0 ? value : undefined;
const privateBlocks: Record<string, true> = { thinking: true, redacted_thinking: true, signature: true, thinking_delta: true, signature_delta: true };
const nativeType = (value: unknown): string | undefined => typeof value === 'string' && value.length <= 128 && /^[a-zA-Z0-9_/.-]+$/.test(value) ? value : undefined;
export function safeUnknownEvent(value: unknown): Record<string, unknown> | undefined {
  const frame = object(value), type = nativeType(frame.type) ?? 'unknown';
  const subtype = nativeType(frame.subtype);
  if (privateBlocks[type] || /auth|credential|signature|thinking|login|logout/i.test(`${type}/${subtype ?? ''}`)) return undefined;
  return { diagnostic: 'unsupported_native_event', native_type: type, ...(subtype ? { native_subtype: subtype } : {}), payload_omitted: true };
}

/** Bind native tool/task executions once; active send changes are not provenance. */
export class NativeRunBindings {
  private owners = new Map<string, string>();
  clear() { this.owners.clear(); }
  private bind(id: string, request: string) {
    if (!this.owners.has(id) && this.owners.size < 8192) this.owners.set(id, request);
  }
  resolve(value: unknown, fallback?: string): string | undefined {
    const frame = object(value), message = object(frame.message), event = object(frame.event);
    const blocks = Array.isArray(message.content) ? message.content.map(object) : [];
    const references = [frame.parent_tool_use_id, frame.tool_use_id, ...blocks.filter(block => block.type === 'tool_result').map(block => block.tool_use_id)];
    const execution = typeof frame.task_id === 'string' && typeof frame.run_id === 'string' ? JSON.stringify(['task', frame.task_id, frame.run_id]) : undefined;
    if (execution) references.push(execution);
    else references.push(frame.task_id);
    const owners = new Set(references.filter((id): id is string => typeof id === 'string').map(id => this.owners.get(id)).filter((id): id is string => id !== undefined));
    if (owners.size > 1) return undefined;
    const referencedResult = blocks.some(block => block.type === 'tool_result');
    const lateProgress = frame.type === 'tool_progress' || frame.type === 'system' && ['task_progress', 'task_notification'].includes(String(frame.subtype));
    const request = (owners.values().next().value as string | undefined) ?? (referencedResult || lateProgress || typeof frame.parent_tool_use_id === 'string' ? undefined : fallback);
    if (!request) return undefined;
    for (const id of [execution, frame.task_id, frame.tool_use_id, ...blocks.filter(block => block.type === 'tool_use').map(block => block.id), object(event.content_block).type === 'tool_use' ? object(event.content_block).id : undefined]) {
      if (typeof id === 'string' && id.length > 0 && id.length <= 512) this.bind(id, request);
    }
    return request;
  }
}

// Strip private model internals at the bridge boundary, including nested tool
// result content. Error bodies may contain provider credentials or request data.
function safeContent(value: unknown, blockPosition = false, structured = false): unknown {
  if (Array.isArray(value)) return value.map(value => safeContent(value, blockPosition, structured)).filter(value => value !== undefined);
  if (value === null || typeof value !== 'object') return value;
  const block = object(value);
  if (privateBlocks[String(block.type)] === true) return undefined;
  if (blockPosition && !['text', 'tool_use', 'tool_result', 'image', 'document', 'resource', 'resource_link', 'audio', 'input_audio', 'search_result', 'web_search_tool_result', 'web_search_result', 'server_tool_use', 'code_execution_tool_result', 'bash_code_execution_tool_result', 'text_editor_code_execution_tool_result', 'container_upload'].includes(String(block.type))) return { type: nativeType(block.type) ?? 'unknown', diagnostic: 'unsupported_native_event', payload_omitted: true };
  if (block.type === 'tool_result' && block.is_error === true) return { type: 'tool_result', tool_use_id: block.tool_use_id, is_error: true, content: 'engine_failed' };
  return Object.fromEntries(Object.entries(block).filter(([key]) => !['thinking', 'signature', 'redacted_thinking'].includes(key)).map(([key, value]) => [key, safeContent(value, !structured && key === 'content' && Array.isArray(value) && (blockPosition || block.type === undefined), structured || key === 'structuredContent')]));
}
export function safeNativeDetails(value: unknown): Record<string, unknown> {
  const message = object(value), content = object(message.message).content;
  const toolResult = object(message.tool_use_result);
  const failed = message.is_error === true || message.isApiErrorMessage === true || message.error !== undefined || Array.isArray(message.errors) && message.errors.length > 0
    || toolResult.is_error === true || toolResult.isError === true || toolResult.error !== undefined
    || Array.isArray(content) && content.some(value => object(value).type === 'tool_result' && object(value).is_error === true);
  const businessFailure = toolResult.is_error === true || toolResult.isError === true || toolResult.error !== undefined;
  const projectedToolResult = failed ? 'engine_failed' : safeContent(message.tool_use_result, Array.isArray(message.tool_use_result));
  if (!failed && Array.isArray(toolResult.results)) {
    // SDK WebSearchOutput groups contain untyped hit records, not content
    // blocks. Project exactly that native schema instead of inferring a type.
    object(projectedToolResult).results = toolResult.results.map(value => {
      if (typeof value === 'string') return value;
      const group = object(value);
      if (typeof group.tool_use_id !== 'string' || !group.tool_use_id || !Array.isArray(group.content)) return { diagnostic: 'unsupported_native_event', native_type: 'search_result_group', payload_omitted: true };
      return { tool_use_id: group.tool_use_id, content: group.content.map(value => {
        const hit = object(value);
        if (typeof hit.title !== 'string' || typeof hit.url !== 'string') return { diagnostic: 'unsupported_native_event', native_type: 'search_hit', payload_omitted: true };
        return { title: hit.title, url: hit.url, ...(typeof hit.snippet === 'string' ? { snippet: hit.snippet } : {}) };
      }) };
    });
  }
  return {
    ...(message.tool_use_result !== undefined ? { tool_use_result: projectedToolResult } : {}),
    ...(message.error !== undefined || message.isApiErrorMessage === true ? { error: 'engine_failed' } : {}),
    ...(Array.isArray(message.errors) && message.errors.length ? { errors: ['engine_failed'] } : {}),
    ...(businessFailure ? { is_error: true } : {}),
  };
}
type Block = { type: string; text?: string; toolId?: string; stopped: boolean; delivered: boolean };
type Message = { id?: string; model?: string; parent: string | null; blocks: Map<number, Block> };
export type TranscriptEvent = { kind: string; data: Record<string, unknown> };

/** SDK 0.3.317 emits one assistant frame per completed block, not per API message. */
export class NativeTranscriptProjection {
  private lanes = new Map<string, Message>();
  private messages = new Map<string, Message>();
  private delivered = new Set<string>();
  clear() { this.lanes.clear(); this.messages.clear(); this.delivered.clear(); }
  clearMain() {
    for (const [key, state] of this.lanes) if (state.parent === null) this.lanes.delete(key);
    for (const [key, state] of this.messages) if (state.parent === null) this.messages.delete(key);
  }
  private lane(value: Record<string, unknown>): string { return JSON.stringify([value.session_id, value.parent_tool_use_id ?? null]); }

  project(value: unknown, history = false): TranscriptEvent[] {
    const frame = object(value), parent = frame.parent_tool_use_id;
    if (frame.type === 'stream_event') return this.stream(frame);
    if (frame.type === 'tool_progress') return [{ kind: 'tool_progress', data: {
      tool_name: frame.tool_name, tool_use_id: frame.tool_use_id, task_id: frame.task_id,
      parent_tool_use_id: parent, elapsed_time_seconds: frame.elapsed_time_seconds,
    } }];
    if (frame.type === 'system' && ['task_started', 'task_progress', 'task_notification'].includes(String(frame.subtype))) {
      const data: Record<string, unknown> = {};
      for (const key of ['task_id', 'run_id', 'tool_use_id', 'parent_tool_use_id', 'parent_task_id', 'task_type', 'agent_id', 'status', 'description', 'summary']) {
        if (typeof frame[key] === 'string' || key === 'parent_tool_use_id' && frame[key] === null) data[key] = frame[key];
      }
      // Failure summaries can contain the same untrusted error body as a result.
      if (frame.status === 'failed' && data.summary !== undefined) data.summary = 'engine_failed';
      if (Array.isArray(frame.resource_links)) {
        const links = frame.resource_links.slice(0, 50).flatMap(value => {
          const link = object(value);
          if (typeof link.uri !== 'string' || typeof link.name !== 'string') return [];
          const safe: Record<string, unknown> = { uri: link.uri, name: link.name };
          for (const key of ['title', 'description', 'mimeType']) if (typeof link[key] === 'string') safe[key] = link[key];
          if (typeof link.size === 'number' && Number.isSafeInteger(link.size) && link.size >= 0) safe.size = link.size;
          return [safe];
        });
        if (Buffer.byteLength(JSON.stringify(links)) <= 64 * 1024) data.resource_links = links;
      }
      return [{ kind: String(frame.subtype), data }];
    }
    if (frame.type !== 'assistant' && frame.type !== 'user') {
      const summary = safeUnknownEvent(frame);
      return summary ? [{ kind: 'native/unsupported', data: summary }] : [];
    }
    const native = object(frame.message), id = string(native.id);
    const state = !history && id ? this.messages.get(JSON.stringify([this.lane(frame), id])) : undefined;
    const details = safeNativeDetails(frame);
    const failed = frame.is_error === true || 'error' in details || 'errors' in details;
    const original = failed ? 'engine_failed' : native.content;
    const fingerprint = createHash('sha256').update(JSON.stringify(original) ?? 'null').digest('hex');
    const delivery = JSON.stringify([this.lane(frame), frame.type, frame.uuid, fingerprint]);
    if (frame.uuid && this.delivered.has(delivery)) return [];
    if (frame.uuid) this.delivered.add(delivery);
    const content: unknown[] = [], indices: (number | null)[] = [];
    const source = Array.isArray(original) ? original : undefined;
    const uniqueToolResult = source?.filter(value => object(value).type === 'tool_result').length === 1;
    if (source) for (const raw of source) {
      const block = object(raw), projected = safeContent(details.is_error === true && uniqueToolResult && block.type === 'tool_result' ? { ...block, is_error: true } : raw, true);
      const candidates = state ? [...state.blocks.entries()].filter(([, candidate]) => !candidate.delivered && (candidate.stopped || candidate.type === 'text' || !!candidate.toolId) && candidate.type === block.type
        && (candidate.type !== 'text' || candidate.text === block.text)
        && (!candidate.toolId || candidate.toolId === block.id)) : [];
      // Never synthesize an index from content array position or receive order.
      const match = candidates.length === 1 ? candidates[0] : undefined;
      if (match) match[1].delivered = true;
      if (projected !== undefined) { content.push(projected); indices.push(match?.[0] ?? null); }
    }
    const safe = source ? content : safeContent(original);
    const usage = responseUsage.safeParse(native.usage);
    return [{ kind: 'message', data: {
      type: frame.type, content: safe, ...(source ? { block_indices: indices } : {}),
      native_message_id: id, parent_tool_use_id: parent, agent_id: string(frame.agent_id),
      model: string(native.model), aborted: frame.aborted === true ? true : undefined,
      usage: frame.type === 'assistant' && usage.success ? usage.data : undefined,
      usage_source: frame.type === 'assistant' && usage.success ? 'native_response_input' : undefined,
      usage_final: frame.type === 'assistant' && usage.success ? false : undefined,
      ...details,
    } }];
  }

  private stream(frame: Record<string, unknown>): TranscriptEvent[] {
    const event = object(frame.event), lane = this.lane(frame);
    let state = this.lanes.get(lane);
    if (event.type === 'message_start') {
      const native = object(event.message);
      state = { id: string(native.id), model: string(native.model), parent: typeof frame.parent_tool_use_id === 'string' ? frame.parent_tool_use_id : null, blocks: new Map() };
      this.lanes.set(lane, state);
      if (state.id) this.messages.set(JSON.stringify([lane, state.id]), state);
      return [{ kind: 'message_start', data: { native_message_id: state.id, model: state.model, parent_tool_use_id: frame.parent_tool_use_id } }];
    }
    const blockIndex = typeof event.index === 'number' && Number.isSafeInteger(event.index) && event.index >= 0 ? event.index : undefined;
    const block = blockIndex === undefined ? undefined : state?.blocks.get(blockIndex);
    const data = { native_message_id: state?.id, block_index: blockIndex, parent_tool_use_id: frame.parent_tool_use_id };
    if (event.type === 'content_block_start' && blockIndex !== undefined) {
      const native = object(event.content_block), type = string(native.type);
      if (state && type) state.blocks.set(blockIndex, { type, text: type === 'text' && typeof native.text === 'string' ? native.text : undefined, toolId: string(native.id), stopped: false, delivered: false });
      if (!type || privateBlocks[type] === true) return [];
      const content = type === 'text' ? { type, text: typeof native.text === 'string' ? native.text : '' }
        : type === 'tool_use' ? { type, id: string(native.id), name: string(native.name), input: safeContent(native.input) } : safeContent(native, true);
      return [{ kind: 'content_block_start', data: { ...data, content_block: content } }];
    }
    if (event.type === 'content_block_delta') {
      const delta = object(event.delta);
      if (delta.type !== 'text_delta' || typeof delta.text !== 'string' || block && block.type !== 'text') return [];
      if (block) block.text = (block.text ?? '') + delta.text;
      return [{ kind: 'text_delta', data: { ...data, text: delta.text } }];
    }
    if (event.type === 'content_block_stop') {
      if (block) block.stopped = true;
      if (block && privateBlocks[block.type] === true) return [];
      return [{ kind: 'content_block_stop', data }];
    }
    if (event.type === 'message_stop') {
      this.lanes.delete(lane);
      return [{ kind: 'message_stop', data: { native_message_id: state?.id, parent_tool_use_id: frame.parent_tool_use_id } }];
    }
    if (typeof event.type === 'string' && privateBlocks[event.type] === true || event.type === 'message_delta') return [];
    const summary = safeUnknownEvent(event);
    return summary ? [{ kind: 'native/unsupported', data: summary }] : [];
  }
}
