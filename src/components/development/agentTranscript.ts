import type { AgentEvent, Json } from '../../stores/agentRunnerStore';
import { isReasoningEvent, projectCodexTranscript, type CodexTranscriptEntry } from './codexTranscript';

export function object(value: Json | undefined): Record<string, Json> { return value !== null && typeof value === 'object' && !Array.isArray(value) ? value : {}; }
export function text(value: Json | undefined): string { return typeof value === 'string' ? value : ''; }
export function blocks(event: AgentEvent): Json[] { const data = object(event.data), content = data.content ?? data.input; return Array.isArray(content) ? content : []; }
export function prose(event: AgentEvent): string {
  if (isReasoningEvent(event)) return '';
  const data = object(event.data);
  return event.text ?? (typeof data.text === 'string' ? data.text : typeof data.content === 'string' ? data.content : blocks(event).filter(block => object(block).type === 'text').map(block => text(object(block).text)).join('\n\n') || (event.kind === 'result' ? text(data.result) : ''));
}
export function isUser(event: AgentEvent): boolean {
  return ['userMessage', 'userSteering'].includes(event.kind) || object(event.data).type === 'user' && (Boolean(prose(event)) || !blocks(event).some(block => object(block).type === 'tool_result'));
}

// A native session is the shared namespace for restored Claude tool-use/result pairs.
// Without any native namespace an event is deliberately isolated rather than guessed.
function eventScope(event: AgentEvent): Json[] {
  const data = object(event.data);
  if (event.run) return ['run', event.run];
  if (text(data.turnId)) return ['turn', text(data.threadId), text(data.turnId)];
  if (text(data.session_id)) return ['session', text(data.session_id)];
  if (text(data.threadId)) return ['thread', text(data.threadId)];
  return ['event', event.seq];
}
export function itemKey(event: AgentEvent, id: string): string {
  const data = object(event.data);
  return JSON.stringify([eventScope(event), text(data.parent_tool_use_id), id || ['event', event.seq]]);
}

export interface AgentTranscriptRow {
  event: AgentEvent;
  key: string;
  suppressText?: boolean;
  codex?: CodexTranscriptEntry;
  parentToolUseId?: string;
  blockIndex?: number;
  /** Native message/block identity was insufficient to reconcile this evidence. */
  identityUncertain?: boolean;
  /** Unmodified native evidence, including deltas and the complete message snapshot. */
  sources: readonly AgentEvent[];
}
export interface AgentTranscriptProjection {
  rows: AgentTranscriptRow[];
  toolResults: ReadonlyMap<string, Json>;
  toolNames: ReadonlyMap<string, string>;
}

/** Match original evidence before considering the row's latest display sequence.
 * Search sequences are only meaningful inside their original run. */
export function findAgentTranscriptRow(rows: readonly AgentTranscriptRow[], source: { run: string | null; seq: number }): AgentTranscriptRow | undefined {
  let nearest: AgentTranscriptRow | undefined;
  let distance = Infinity;
  for (const row of rows) {
    for (const event of row.sources) {
      if ((event.run ?? null) !== source.run) continue;
      if (event.seq === source.seq) return row;
      const nextDistance = Math.abs(event.seq - source.seq);
      if (nextDistance < distance) { nearest = row; distance = nextDistance; }
    }
  }
  return nearest;
}
interface WorkingRow extends AgentTranscriptRow { order: number; complete?: boolean; sources: AgentEvent[]; }
const executionTypes = ['commandExecution', 'fileChange', 'mcpToolCall', 'dynamicToolCall', 'webSearch'];
const blockIndex = (value: Json | undefined): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const metadataKinds: Record<string, true> = { initialized: true, 'native/initialized': true, 'native/capabilities': true, 'native/unavailable': true, controls: true, history: true, 'thread/tokenUsage/updated': true, steeringReceipt: true };
function visibleRow(row: WorkingRow): boolean {
  if (Object.prototype.hasOwnProperty.call(metadataKinds, row.event.kind)) return false;
  if (row.codex?.kind === 'summary') return row.codex.parts.some(part => part.text.trim() !== '');
  if (row.codex) return true;
  const data = object(row.event.data);
  if (row.event.kind === 'agentMessage' || data.type === 'assistant' || isUser(row.event)) {
    return prose(row.event).trim() !== '' || blocks(row.event).some(block => !['text', 'thinking', 'redacted_thinking', 'signature', 'thinking_delta', 'signature_delta', 'reasoning'].includes(text(object(block).type))) || Array.isArray(data.images) && data.images.length > 0 || Boolean(data.error) || Array.isArray(data.errors) && data.errors.length > 0 || data.is_error === true || data.success === false;
  }
  return true;
}

/** Pure projection. Input events and their payloads are never changed. */
export function projectAgentTranscript(events: readonly AgentEvent[]): AgentTranscriptProjection {
  const rows: WorkingRow[] = [];
  const eventOrder = new Map(events.map((event, index) => [event, index]));
  const keyed = new Map<string, WorkingRow>();
  const codexEvents = projectCodexTranscript(events);
  const summaryEvidence = new Map<string, AgentEvent[]>();
  const toolResults = new Map<string, Json>();
  const toolNames = new Map<string, string>();
  const answers = new Map<string, { message: string; parts: Map<string, string> }>();
  const answerEpochs = new Map<string, number>();
  const laneKey = (event: AgentEvent) => JSON.stringify([eventScope(event), text(object(event.data).parent_tool_use_id)]);
  const scopedKey = (event: AgentEvent, kind: string, id: string | number) => JSON.stringify([itemKey(event, String(id)), kind]);
  const add = (event: AgentEvent, key: string, order: number, extra: Partial<WorkingRow> = {}): WorkingRow => {
    const parent = text(object(event.data).parent_tool_use_id);
    const row: WorkingRow = { event, key, order, sources: [event], ...(parent ? { parentToolUseId: parent } : {}), ...extra };
    rows.push(row); keyed.set(key, row); return row;
  };
  const update = (row: WorkingRow, event: AgentEvent, rendered = event) => {
    row.event = rendered;
    row.sources.push(event);
  };
  const rememberAnswer = (event: AgentEvent, part: string, source: string, message: string) => {
    if (!source || text(object(event.data).parent_tool_use_id)) return;
    const lane = laneKey(event);
    const identity = message || `legacy:${answerEpochs.get(lane) ?? 0}`;
    let answer = answers.get(lane);
    if (!answer || answer.message !== identity) { answer = { message: identity, parts: new Map() }; answers.set(lane, answer); }
    answer.parts.set(part, source);
  };
  for (const event of events) {
    const data = object(event.data);
    const type = text(data.type);
    const content = blocks(event);
    const resultCount = content.reduce<number>((count, block) => count + (object(block).type === 'tool_result' ? 1 : 0), 0);
    const nativeResult = object(data.tool_use_result);
    for (const block of content) {
      const value = object(block);
      if (value.type === 'tool_result' && text(value.tool_use_id)) {
        const failed = resultCount === 1 && data.is_error === true && value.is_error !== true;
        const structured = resultCount === 1 && value.structuredContent === undefined && nativeResult.structuredContent !== undefined;
        toolResults.set(itemKey(event, text(value.tool_use_id)), failed || structured ? { ...value, ...(failed ? { is_error: true } : {}), ...(structured ? { structuredContent: nativeResult.structuredContent } : {}) } : block);
      }
      if (type === 'assistant' && value.type === 'tool_use' && text(value.id)) toolNames.set(itemKey(event, text(value.id)), text(value.name));
    }
  }
  for (const [order, event] of events.entries()) {
    const data = object(event.data);
    const lane = laneKey(event);
    const sourceKey = scopedKey(event, 'event', event.seq);
    if (isUser(event)) {
      answers.delete(lane);
      answerEpochs.set(lane, (answerEpochs.get(lane) ?? 0) + 1);
    }
    if (codexEvents.has(event)) {
      const entry = codexEvents.get(event);
      if (entry?.kind === 'revision') {
        add(event, scopedKey(event, event.kind, 'revisions'), order, { codex: entry, sources: [...entry.revisions] });
      } else if (isReasoningEvent(event)) {
        const value = object(data.item ?? data);
        const id = text(data.itemId) || text(value.id);
        const key = id ? scopedKey(event, 'reasoning', id) : sourceKey;
        if (entry?.kind === 'malformed') { add(event, sourceKey, order, { codex: entry }); continue; }
        const summaryEvent = event.kind === 'item/reasoning/summaryTextDelta' || event.kind === 'item/reasoning/summaryPartAdded' || event.kind === 'reasoning' || event.kind === 'item/completed' && object(data.item ?? data).type === 'reasoning';
        if (!summaryEvent) continue;
        const sources = summaryEvidence.get(key) ?? [];
        sources.push(event); summaryEvidence.set(key, sources);
        if (entry?.kind === 'summary') {
          const row = keyed.get(key);
          if (!row) add(event, key, order, { codex: { ...entry, parts: entry.parts.map(part => ({ ...part })) }, sources: [...sources] });
          else if (row.codex?.kind === 'summary') {
            const parts = new Map(row.codex.parts.map(part => [part.index, part]));
            for (const part of entry.parts) parts.set(part.index, { ...part });
            row.codex = { ...entry, parts: [...parts.values()].sort((left, right) => left.index - right.index) };
          }
        }
      }
      continue;
    }
    if (['task_started', 'task_progress', 'task_notification'].includes(event.kind) && text(data.task_id)) {
      const key = scopedKey(event, `task:${text(data.run_id)}`, text(data.task_id));
      const row = keyed.get(key);
      if (row) {
        update(row, event, row.complete && event.kind !== 'task_notification' ? { ...row.event, seq: event.seq } : { ...event, data: { ...object(row.event.data), ...data } });
        row.complete ||= ['completed', 'failed', 'stopped', 'cancelled'].includes(text(data.status));
      } else add(event, key, order, { complete: ['completed', 'failed', 'stopped', 'cancelled'].includes(text(data.status)) });
      continue;
    }
    const nativeId = text(data.native_message_id);
    const index = data.block_index;
    const claudeKey = nativeId && blockIndex(index) ? scopedKey(event, `claude-block:${nativeId}`, index) : '';
    if (event.kind === 'content_block_start') {
      const block = object(data.content_block);
      if (claudeKey && ['text', 'tool_use'].includes(text(block.type))) {
        let row = keyed.get(claudeKey);
        const rendered: AgentEvent = { ...event, kind: 'message', text: null, data: { ...data, type: 'assistant', content: [block], block_indices: [index as number] } };
        if (!row) row = add(rendered, claudeKey, order, { blockIndex: index as number, sources: [event] });
        else update(row, event, { ...row.event, seq: event.seq });
        if (block.type === 'text') rememberAnswer(event, claudeKey, text(block.text), nativeId);
        if (block.type === 'tool_use' && text(block.id)) toolNames.set(itemKey(event, text(block.id)), text(block.name));
      }
      continue;
    }
    if (event.kind === 'content_block_stop' || event.kind === 'message_stop' || event.kind === 'message_start') {
      if (event.kind === 'content_block_stop' && claudeKey) {
        const row = keyed.get(claudeKey);
        if (row) update(row, event, { ...row.event, seq: event.seq });
      } else if (event.kind === 'message_stop' && nativeId) {
        for (const row of rows) if (object(row.event.data).native_message_id === nativeId && laneKey(row.event) === lane) update(row, event, { ...row.event, seq: event.seq });
      }
      continue;
    }
    if (event.kind === 'text_delta' || event.kind === 'item/agentMessage/delta') {
      const codexId = text(data.itemId);
      // Missing Claude message/block identity is never an empty shared stream.
      const key = event.kind === 'text_delta' ? claudeKey || sourceKey : codexId ? scopedKey(event, 'codex-message', codexId) : sourceKey;
      let row = keyed.get(key);
      if (row?.complete) { update(row, event, { ...row.event, seq: event.seq }); continue; }
      const previous = row ? prose(row.event) : '';
      const source = previous + (event.text ?? text(data.delta));
      const rendered: AgentEvent = { ...event, kind: 'agentMessage', text: source, data: { ...object(row?.event.data), ...data, ...(event.kind === 'text_delta' ? { type: 'assistant', content: [{ type: 'text', text: source }], ...(blockIndex(index) ? { block_indices: [index] } : {}) } : {}) } };
      if (!row) row = add(rendered, key, order, { sources: [event], identityUncertain: event.kind === 'text_delta' && !claudeKey || event.kind === 'item/agentMessage/delta' && !codexId, ...(blockIndex(index) ? { blockIndex: index } : {}) });
      else update(row, event, rendered);
      rememberAnswer(event, key, prose(row.event), nativeId || codexId);
      continue;
    }
    if (event.kind.endsWith('/outputDelta')) {
      const id = text(data.itemId);
      const key = id ? scopedKey(event, 'output', id) : sourceKey;
      const row = keyed.get(key);
      if (row?.complete) { update(row, event, { ...row.event, seq: event.seq }); continue; }
      const output = (row?.event.text ?? text(object(row?.event.data).aggregatedOutput)) + (event.text ?? text(data.delta));
      const rendered = { ...event, text: output, data: { ...object(row?.event.data), ...data, aggregatedOutput: output } };
      if (row) update(row, event, rendered);
      else add(rendered, key, order, { sources: [event] });
      continue;
    }
    const terminal = event.kind === 'result' || event.kind === 'turn/completed';
    const assistant = event.kind === 'agentMessage' || data.type === 'assistant';
    if (assistant && nativeId && blocks(event).length) {
      const indices = Array.isArray(data.block_indices) ? data.block_indices : [];
      for (const [position, block] of blocks(event).entries()) {
        if (['thinking', 'signature', 'redacted_thinking'].includes(text(object(block).type))) continue;
        const nativeIndex = indices[position];
        const key = blockIndex(nativeIndex) ? scopedKey(event, `claude-block:${nativeId}`, nativeIndex) : `${sourceKey}:block:${position}`;
        const row = keyed.get(key);
        const rendered: AgentEvent = { ...event, text: null, data: { ...object(row?.event.data), ...data, content: [block], block_indices: [blockIndex(nativeIndex) ? nativeIndex : null], ...(blockIndex(nativeIndex) ? { block_index: nativeIndex } : {}) } };
        if (row) { update(row, event, rendered); row.complete = true; }
        else add(rendered, key, order, { complete: true, sources: [event], identityUncertain: !blockIndex(nativeIndex), ...(blockIndex(nativeIndex) ? { blockIndex: nativeIndex } : {}) });
        if (object(block).type === 'text') rememberAnswer(event, key, text(object(block).text), nativeId);
      }
      continue;
    }
    if (assistant || terminal) {
      const id = text(data.id);
      const key = assistant && event.kind === 'agentMessage' && id ? scopedKey(event, 'codex-message', id) : sourceKey;
      const source = prose(event);
      const answer = answers.get(lane);
      const failed = Boolean(data.error) || data.run_status === 'failed' || data.status === 'failed' || data.success === false || data.is_error === true || typeof data.exitCode === 'number' && data.exitCode !== 0 || blocks(event).some(block => object(block).type === 'tool_result' && object(block).is_error === true);
      const repeated = terminal && !failed && source.trim() !== '' && Boolean(answer && [...answer.parts.values()].join('\n\n').trim() === source.trim());
      const row = keyed.get(key);
      if (row) {
        const hasText = typeof event.text === 'string' || typeof data.text === 'string' || typeof data.content === 'string' || Array.isArray(data.content);
        update(row, event, { ...event, text: hasText ? source : prose(row.event), data: { ...object(row.event.data), ...data } }); row.complete = true;
      }
      else add(event, key, order, { complete: assistant, ...(assistant && !id ? { identityUncertain: true } : {}), ...(terminal ? { suppressText: repeated } : {}) });
      if (assistant) {
        if (!id && blocks(event).some(block => object(block).type === 'tool_use')) answerEpochs.set(lane, (answerEpochs.get(lane) ?? 0) + 1);
        rememberAnswer(event, key, source, id);
      }
      if (terminal) {
        answers.delete(lane);
        answerEpochs.set(lane, (answerEpochs.get(lane) ?? 0) + 1);
      }
      continue;
    }
    const execution = executionTypes.includes(event.kind) || event.kind === 'item/started' && executionTypes.includes(text(data.type));
    if (execution && text(data.id)) {
      const key = scopedKey(event, 'output', text(data.id));
      const row = keyed.get(key);
      if (!row) add(event, key, order, { complete: event.kind !== 'item/started' && !['inProgress', 'in_progress', 'running'].includes(text(data.status)) });
      else if (event.kind === 'item/started' && row.complete) update(row, event, { ...row.event, seq: event.seq });
      else {
        const previous = object(row.event.data);
        const output = typeof data.aggregatedOutput === 'string' ? data.aggregatedOutput : typeof previous.aggregatedOutput === 'string' ? previous.aggregatedOutput : row.event.text;
        // Complete native output (including an explicit empty string) replaces the
        // stream. A missing/null snapshot keeps the collected output renderable.
        update(row, event, { ...event, text: typeof data.aggregatedOutput === 'string' ? event.text : row.event.text ?? event.text, data: { ...previous, ...data, ...(typeof output === 'string' ? { aggregatedOutput: output } : {}) } });
        row.complete = event.kind !== 'item/started' && !['inProgress', 'in_progress', 'running'].includes(text(data.status));
      }
      answerEpochs.set(lane, (answerEpochs.get(lane) ?? 0) + 1);
      continue;
    }
    add(event, sourceKey, order);
    if (executionTypes.includes(event.kind) || blocks(event).some(block => object(block).type === 'tool_result' || object(block).type === 'tool_use')) answerEpochs.set(lane, (answerEpochs.get(lane) ?? 0) + 1);
  }
  // Summary entries are anchored at their first part, but use the latest native seq/evidence.
  for (const row of rows) if (row.codex?.kind === 'summary') {
    const sources = summaryEvidence.get(row.key) ?? row.sources;
    row.sources = [...sources];
    row.event = sources[sources.length - 1];
    row.codex = { ...row.codex, seq: row.event.seq };
  }
  const toolRows = new Map<string, WorkingRow[]>();
  for (const row of rows) for (const block of blocks(row.event)) {
    const value = object(block);
    if (value.type !== 'tool_use' || !text(value.id)) continue;
    const key = itemKey(row.event, text(value.id));
    const owners = toolRows.get(key) ?? [];
    owners.push(row); toolRows.set(key, owners);
  }
  const pairedResults = new Set<WorkingRow>();
  for (const row of rows) {
    const content = blocks(row.event);
    if (row.event.kind === 'tool_progress') {
      const data = object(row.event.data);
      const owners = toolRows.get(itemKey(row.event, text(data.tool_use_id))) ?? [];
      // Missing/ambiguous native identity remains its own visible evidence.
      if (owners.length === 1) {
        const owner = owners[0];
        owner.sources.push(...row.sources);
        const ownerData = object(owner.event.data);
        const singleTool = blocks(owner.event).filter(block => object(block).type === 'tool_use').length === 1;
        owner.event = { ...owner.event, data: { ...ownerData, tool_progress: { ...object(ownerData.tool_progress), [text(data.tool_use_id)]: data }, ...(singleTool && typeof data.elapsed_time_seconds === 'number' && Number.isFinite(data.elapsed_time_seconds) && data.elapsed_time_seconds >= 0 ? { elapsed_time_seconds: data.elapsed_time_seconds } : {}), ...(singleTool && text(data.task_id) ? { task_id: data.task_id } : {}), ...(singleTool && text(data.tool_name) ? { tool_name: data.tool_name } : {}) } };
        pairedResults.add(row);
      }
      continue;
    }
    const resultBlocks = content.filter(block => object(block).type === 'tool_result');
    if (!resultBlocks.length) continue;
    for (const block of resultBlocks) {
      const owners = toolRows.get(itemKey(row.event, text(object(block).tool_use_id))) ?? [];
      for (const owner of owners) {
        for (const source of row.sources) if (!owner.sources.includes(source)) owner.sources.push(source);
      }
    }
    const data = object(row.event.data);
    const independentFailure = Boolean(data.error) || Array.isArray(data.errors) && data.errors.length > 0 || data.status === 'failed' || data.run_status === 'failed' || data.success === false || data.is_error === true && (resultBlocks.length !== 1 || !text(object(resultBlocks[0]).tool_use_id));
    if (data.type === 'user' && !prose(row.event) && !independentFailure && resultBlocks.length === content.length && resultBlocks.every(block => toolRows.has(itemKey(row.event, text(object(block).tool_use_id))))) pairedResults.add(row);
  }
  for (const owners of toolRows.values()) for (const owner of owners) {
    owner.sources.sort((left, right) => (eventOrder.get(left) ?? 0) - (eventOrder.get(right) ?? 0));
    const latest = owner.sources[owner.sources.length - 1];
    if (owner.event.seq !== latest.seq) owner.event = { ...owner.event, seq: latest.seq };
  }
  rows.sort((left, right) => (eventOrder.get(left.sources[0]) ?? left.order) - (eventOrder.get(right.sources[0]) ?? right.order));
  return { rows: rows.filter(row => !pairedResults.has(row) && visibleRow(row)).map(({ order: _order, complete: _complete, ...row }) => row), toolResults, toolNames };
}

export interface AgentTranscriptProjector {
  project(events: readonly AgentEvent[]): AgentTranscriptProjection;
  clear(): void;
}

/** Bounded per-run cache for immutable event entries/payloads. The array may be mutated
 * by append, reset or trim: copied references/seqs detect those changes without scanning
 * or copying long payloads. Replace an edited event; clear() on session/generation changes. */
export function createAgentTranscriptProjector({ maxRuns = 32 }: { maxRuns?: number } = {}): AgentTranscriptProjector {
  const capacity = Number.isSafeInteger(maxRuns) && maxRuns > 0 ? maxRuns : 32;
  const cache = new Map<string, { events: AgentEvent[]; seqs: number[]; projection: AgentTranscriptProjection }>();
  let previous: AgentTranscriptProjection | undefined;
  return {
    clear() { cache.clear(); previous = undefined; },
    project(events) {
      const groups = new Map<string, AgentEvent[]>();
      const positions = new Map<AgentEvent, number>();
      for (const [index, event] of events.entries()) {
        const key = event.run ? `run:${event.run}` : JSON.stringify(eventScope(event));
        let group = groups.get(key);
        if (!group) { group = []; groups.set(key, group); }
        group.push(event);
        positions.set(event, index);
      }
      const rows: AgentTranscriptRow[] = [];
      let toolResults = new Map<string, Json>();
      let toolNames = new Map<string, string>();
      for (const [key, group] of groups) {
        let entry = cache.get(key);
        if (!entry || entry.events.length !== group.length || group.some((event, index) => event !== entry!.events[index] || event.seq !== entry!.seqs[index])) {
          entry = { events: group, seqs: group.map(event => event.seq), projection: projectAgentTranscript(group) };
        }
        cache.delete(key); cache.set(key, entry);
        rows.push(...entry.projection.rows);
        for (const [id, result] of entry.projection.toolResults) toolResults.set(id, result);
        for (const [id, name] of entry.projection.toolNames) toolNames.set(id, name);
      }
      for (const key of cache.keys()) if (!groups.has(key)) cache.delete(key);
      while (cache.size > capacity) cache.delete(cache.keys().next().value!);
      rows.sort((left, right) => (positions.get(left.sources[0]) ?? 0) - (positions.get(right.sources[0]) ?? 0));
      const sameRows = previous && rows.length === previous.rows.length && rows.every((row, index) => row === previous!.rows[index]);
      if (previous && toolResults.size === previous.toolResults.size && [...toolResults].every(([key, value]) => previous!.toolResults.get(key) === value)) toolResults = previous.toolResults as Map<string, Json>;
      if (previous && toolNames.size === previous.toolNames.size && [...toolNames].every(([key, value]) => previous!.toolNames.get(key) === value)) toolNames = previous.toolNames as Map<string, string>;
      if (sameRows && toolResults === previous!.toolResults && toolNames === previous!.toolNames) return previous!;
      previous = { rows, toolResults, toolNames };
      return previous;
    },
  };
}
