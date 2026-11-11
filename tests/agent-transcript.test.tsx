import { describe, expect, it } from 'vitest';
import type { AgentEvent, Json } from '../src/stores/agentRunnerStore';
import { blocks, createAgentTranscriptProjector, findAgentTranscriptRow, isUser, itemKey, object, projectAgentTranscript, prose } from '../src/components/development/agentTranscript';
import { inspectableAgentEvent } from '../src/components/development/codexTranscript';
import { agentEventFailed } from '../src/components/development/eventOutputReferences';

const event = (seq: number, kind: string, data: Json = null, run: string | null = 'run-a', text: string | null = null): AgentEvent => ({ seq, kind, data, run, text });
const delta = (seq: number, message: string, index: number, value: string, parent: string | null = null, run = 'run-a') => event(seq, 'text_delta', { native_message_id: message, block_index: index, parent_tool_use_id: parent }, run, value);
const message = (seq: number, id: string, content: Json[], indices: Json[], parent: string | null = null, run: string | null = 'run-a') => event(seq, 'message', { type: 'assistant', native_message_id: id, content, block_indices: indices, parent_tool_use_id: parent }, run);
const textBlock = (value: string): Json => ({ type: 'text', text: value });

describe('Claude native transcript identity', () => {
  it('replaces every block independently after stop and retains stable keys, latest seq and original evidence', () => {
    const start = event(1, 'content_block_start', { native_message_id: 'm', block_index: 0, content_block: { type: 'text', text: '' } });
    const first = delta(2, 'm', 0, 'First');
    const stop = event(3, 'content_block_stop', { native_message_id: 'm', block_index: 0 });
    const second = delta(4, 'm', 2, 'Second');
    const messageStop = event(5, 'message_stop', { native_message_id: 'm' });
    const events = [start, first, stop, second, messageStop];
    const before = projectAgentTranscript(events);
    expect(before.rows.map(row => prose(row.event))).toEqual(['First', 'Second']);
    expect(before.rows.map(row => row.event.seq)).toEqual([5, 5]);
    const snapshot = message(6, 'm', [textBlock('First revised'), textBlock('Second revised')], [0, 2]);
    const after = projectAgentTranscript([...events, snapshot]);
    expect(after.rows.map(row => row.key)).toEqual(before.rows.map(row => row.key));
    expect(after.rows.map(row => prose(row.event))).toEqual(['First revised', 'Second revised']);
    expect(after.rows.map(row => row.event.seq)).toEqual([6, 6]);
    expect(after.rows[0].sources).toEqual([start, first, stop, messageStop, snapshot]);
    expect(after.rows[1].sources).toEqual([second, messageStop, snapshot]);
    expect(blocks(snapshot)).toHaveLength(2);
    expect(object(snapshot.data)).not.toHaveProperty('stream_complete');
  });

  it('keeps same-message per-block snapshots, equal text and interleaved tool blocks distinct', () => {
    const events = [
      delta(1, 'm', 0, 'Same'),
      event(2, 'content_block_start', { native_message_id: 'm', block_index: 1, content_block: { type: 'tool_use', id: 'tool', name: 'Read', input: { file_path: 'one.ts' } } }),
      delta(3, 'm', 2, 'Same'),
      message(4, 'm', [textBlock('Same')], [0]),
      message(5, 'm', [{ type: 'tool_use', id: 'tool', name: 'Read', input: { file_path: 'one.ts' } }], [1]),
      event(6, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'tool', content: 'file content' }] }),
      message(7, 'm', [textBlock('Same')], [2]),
    ];
    const projection = projectAgentTranscript(events);
    expect(projection.rows.filter(row => prose(row.event)).map(row => prose(row.event))).toEqual(['Same', 'Same']);
    expect(new Set(projection.rows.map(row => row.key)).size).toBe(projection.rows.length);
    const tools = projection.rows.filter(row => blocks(row.event).some(block => object(block).type === 'tool_use'));
    expect(tools).toHaveLength(1);
    expect(tools[0].event.seq).toBe(6);
    expect(tools[0].sources).toEqual([events[1], events[4], events[5]]);
    expect(projection.rows).toHaveLength(3);
    expect(projection.toolNames.get(itemKey(events[1], 'tool'))).toBe('Read');
    expect(projection.toolResults.get(itemKey(events[1], 'tool'))).toMatchObject({ content: 'file content' });
    expect(isUser(events[5])).toBe(false);
  });

  it('isolates parent lanes, child snapshots and tool namespaces from main assistant content', () => {
    const events = [
      delta(1, 'shared-id', 0, 'Main'),
      delta(2, 'shared-id', 0, 'Child', 'task-tool'),
      message(3, 'shared-id', [textBlock('Child complete')], [0], 'task-tool'),
      message(4, 'shared-id', [textBlock('Main complete')], [0]),
      message(5, 'tool-message', [{ type: 'tool_use', id: 'same-tool', name: 'Read', input: {} }], [0]),
      message(6, 'tool-message', [{ type: 'tool_use', id: 'same-tool', name: 'Bash', input: {} }], [0], 'task-tool'),
      event(7, 'result', { result: 'Child complete' }),
    ];
    const projection = projectAgentTranscript(events);
    expect(projection.rows.slice(0, 2).map(row => prose(row.event))).toEqual(['Main complete', 'Child complete']);
    expect(projection.rows[1].parentToolUseId).toBe('task-tool');
    expect(object(projection.rows[1].event.data).parent_tool_use_id).toBe('task-tool');
    expect(projection.rows.at(-1)?.suppressText).toBe(false);
    expect(projection.toolNames.get(itemKey(events[4], 'same-tool'))).toBe('Read');
    expect(projection.toolNames.get(itemKey(events[5], 'same-tool'))).toBe('Bash');
  });

  it('presents identity-free legacy fragments conservatively without joining equal text across turns', () => {
    const events = [
      event(1, 'text_delta', null, 'run-a', 'Same'),
      event(2, 'text_delta', null, 'run-a', 'Same'),
      event(3, 'message', { type: 'assistant', content: [textBlock('Same')] }),
      event(4, 'userMessage', null, 'run-a', 'Again'),
      event(5, 'message', { type: 'assistant', content: [textBlock('Same')] }),
      event(6, 'text_delta', null, 'run-b', 'Same'),
      message(7, 'unknown-index', [textBlock('Same'), textBlock('Same')], [null, null]),
    ];
    const projection = projectAgentTranscript(events);
    expect(projection.rows.map(row => prose(row.event))).toEqual(['Same', 'Same', 'Same', 'Again', 'Same', 'Same', 'Same', 'Same']);
    expect(new Set(projection.rows.map(row => row.key)).size).toBe(8);
    expect(projection.rows.some(row => row.suppressText)).toBe(false);
    expect(projection.rows.filter(row => row.identityUncertain)).toHaveLength(7);
  });

  it('uses restored native session identity for tools and isolates unrelated history and anonymous events', () => {
    const use = event(1, 'message', { session_id: 'native-a', history: true, type: 'assistant', native_message_id: 'm', block_indices: [0], content: [{ type: 'tool_use', id: 'tool', name: 'Bash', input: {} }] }, null);
    const result = event(2, 'message', { session_id: 'native-a', history: true, type: 'user', content: [{ type: 'tool_result', tool_use_id: 'tool', content: 'done' }] }, null);
    const other = event(3, 'message', { session_id: 'native-b', type: 'user', content: [{ type: 'tool_result', tool_use_id: 'tool', content: 'other' }] }, null);
    const projection = projectAgentTranscript([use, result, other]);
    expect(itemKey(use, 'tool')).toBe(itemKey(result, 'tool'));
    expect(itemKey(other, 'tool')).not.toBe(itemKey(use, 'tool'));
    expect(projection.toolResults.get(itemKey(use, 'tool'))).toMatchObject({ content: 'done' });
    expect(itemKey(event(4, 'message', null, null), 'tool')).not.toBe(itemKey(event(5, 'message', null, null), 'tool'));
    expect(itemKey(use, '')).not.toBe(itemKey(result, ''));
  });

  it('suppresses only a matching whole multi-block terminal echo and never loses the result or error facts', () => {
    const first = message(1, 'm', [textBlock('One')], [0]);
    const second = message(2, 'm', [textBlock('Two')], [2]);
    const terminal = event(3, 'result', { result: 'One\n\nTwo', run_status: 'completed' });
    const projection = projectAgentTranscript([first, second, terminal]);
    expect(projection.rows).toHaveLength(3);
    expect(projection.rows[2]).toMatchObject({ event: terminal, suppressText: true, sources: [terminal] });
    expect(projectAgentTranscript([first, second, event(3, 'result', { result: 'Two' })]).rows.at(-1)?.suppressText).toBe(false);
    expect(projectAgentTranscript([first, second, event(3, 'result', { result: 'One\n\nTwo', is_error: true })]).rows.at(-1)?.suppressText).toBe(false);
    const nextUser = event(4, 'userMessage', null, 'run-a', 'Next');
    expect(projectAgentTranscript([first, second, nextUser, event(5, 'result', { result: 'One\n\nTwo' })]).rows.at(-1)?.suppressText).toBe(false);
    expect(projectAgentTranscript([first, second, event(5, 'result', { result: 'One\n\nTwo' }, 'run-b')]).rows.at(-1)?.suppressText).toBe(false);
    const unknown = event(6, 'result', { result: 'Everything succeeded. Final answer.' });
    expect(projectAgentTranscript([unknown]).rows[0].event).toBe(unknown);
    expect(object(unknown.data)).not.toHaveProperty('run_status');
  });

  it('keeps consecutive paired Read rows adjacent and moves failed result evidence to its tool row', () => {
    const first = message(1, 'first', [{ type: 'tool_use', id: 'read-1', name: 'Read', input: { file_path: 'one.ts' } }], [0]);
    const firstResult = event(2, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'read-1', content: 'one' }] });
    const second = message(3, 'second', [{ type: 'tool_use', id: 'read-2', name: 'Read', input: { file_path: 'two.ts' } }], [0]);
    const failedResult = event(4, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'read-2', is_error: true, content: 'Permission denied' }] });
    const projection = projectAgentTranscript([first, firstResult, second, failedResult]);
    expect(projection.rows).toHaveLength(2);
    expect(projection.rows.map(row => object(blocks(row.event)[0]).id)).toEqual(['read-1', 'read-2']);
    expect(projection.rows[0].sources).toEqual([first, firstResult]);
    expect(projection.rows[1].sources).toEqual([second, failedResult]);
    expect(projection.rows.map(row => row.event.seq)).toEqual([2, 4]);
    expect(projection.toolResults.get(itemKey(projection.rows[1].event, 'read-2'))).toMatchObject({ is_error: true, content: 'Permission denied' });
    const unmatched = event(5, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'missing', content: 'Unknown source' }] });
    expect(projectAgentTranscript([unmatched]).rows).toHaveLength(1);
    const independentError = { ...firstResult, data: { ...object(firstResult.data), error: 'Native envelope error' } };
    expect(projectAgentTranscript([first, independentError]).rows).toHaveLength(2);
    const withProse = { ...firstResult, text: 'Additional user note' };
    expect(projectAgentTranscript([first, withProse]).rows).toHaveLength(2);
  });

  it('coalesces each task lifecycle by run and parent with stable key and complete native evidence', () => {
    const started = event(1, 'task_started', { task_id: 'task', tool_use_id: 'spawn', description: 'Inspect project', status: 'running' });
    const progress = event(2, 'task_progress', { task_id: 'task', summary: 'Reading files' });
    const child = event(3, 'task_started', { task_id: 'task', parent_tool_use_id: 'parent', description: 'Nested task' });
    const otherRun = event(4, 'task_started', { task_id: 'task', description: 'Other turn' }, 'run-b');
    const completed = event(5, 'task_notification', { task_id: 'task', status: 'completed', summary: 'Inspection complete' });
    const before = projectAgentTranscript([started]);
    const projection = projectAgentTranscript([started, progress, child, otherRun, completed]);
    expect(projection.rows).toHaveLength(3);
    expect(projection.rows[0].key).toBe(before.rows[0].key);
    expect(projection.rows[0].event).toMatchObject({ seq: 5, kind: 'task_notification', data: { task_id: 'task', description: 'Inspect project', tool_use_id: 'spawn', status: 'completed', summary: 'Inspection complete' } });
    expect(projection.rows[0].sources).toEqual([started, progress, completed]);
    expect(projection.rows[1].parentToolUseId).toBe('parent');
    expect(projection.rows[2].event.run).toBe('run-b');
    expect(new Set(projection.rows.map(row => row.key)).size).toBe(3);
    const failed = event(6, 'task_notification', { task_id: 'task', status: 'failed', summary: 'engine_failed' });
    expect(projectAgentTranscript([started, progress, failed]).rows[0].event).toMatchObject({ seq: 6, data: { status: 'failed', summary: 'engine_failed' } });
    expect(projectAgentTranscript([event(7, 'task_progress'), event(8, 'task_progress')]).rows).toHaveLength(2);
  });

  it('retains all unscoped completion blocks when exact native indices are unavailable', () => {
    const streamed = delta(1, 'm', 0, 'A');
    const snapshot = message(2, 'm', [textBlock('A'), textBlock('B')], [null, null]);
    const projection = projectAgentTranscript([streamed, snapshot]);
    expect(projection.rows.map(row => prose(row.event))).toEqual(['A', 'A', 'B']);
    expect(projection.rows[1].sources).toEqual([snapshot]);
    expect(projection.rows[2].sources).toEqual([snapshot]);
    expect(projection.rows.map(row => Boolean(row.identityUncertain))).toEqual([false, true, true]);
  });
});

describe('Codex and private native transcript content', () => {
  it('keeps phase and stable stream identity through completion without guessing final from text', () => {
    const first = event(1, 'item/agentMessage/delta', { itemId: 'reply', phase: 'commentary' }, 'run-a', 'Working');
    const complete = event(2, 'agentMessage', { id: 'reply', phase: 'commentary', text: 'Working complete' }, 'run-a', 'Working complete');
    const before = projectAgentTranscript([first]);
    const after = projectAgentTranscript([first, complete]);
    expect(after.rows).toHaveLength(1);
    expect(after.rows[0].key).toBe(before.rows[0].key);
    expect(after.rows[0].event.seq).toBe(2);
    expect(object(after.rows[0].event.data).phase).toBe('commentary');
    expect(after.rows[0].sources).toEqual([first, complete]);
    expect(object(projectAgentTranscript([event(3, 'agentMessage', { id: 'other' }, 'run-a', 'Final: done')]).rows[0].event.data)).not.toHaveProperty('phase');
  });

  it('merges summary parts with stable identity and preserves multiple plan/diff revisions per run', () => {
    const first = event(1, 'item/reasoning/summaryTextDelta', { itemId: 'r', summaryIndex: 0, delta: 'One' }, 'run-a', 'One');
    const second = event(2, 'item/reasoning/summaryTextDelta', { itemId: 'r', summaryIndex: 1, delta: 'Two' }, 'run-a', 'Two');
    const before = projectAgentTranscript([first, second]);
    expect(before.rows).toHaveLength(1);
    expect(before.rows[0].codex).toMatchObject({ kind: 'summary', parts: [{ index: 0, text: 'One' }, { index: 1, text: 'Two' }], seq: 2 });
    const complete = event(3, 'reasoning', { type: 'reasoning', id: 'r', summary: ['Revised'], content: ['PRIVATE'] });
    const plan1 = event(4, 'turn/plan/updated', { plan: [{ step: 'Inspect', status: 'pending' }] });
    const diff1 = event(5, 'turn/diff/updated', { diff: '+one' });
    const plan2 = event(6, 'turn/plan/updated', { explanation: 'Changed', plan: [{ step: 'Inspect', status: 'completed' }] });
    const planOther = event(7, 'turn/plan/updated', { plan: [] }, 'run-b');
    const diff2 = event(8, 'turn/diff/updated', { diff: '+two' });
    const after = projectAgentTranscript([first, second, complete, plan1, diff1, plan2, planOther, diff2]);
    expect(after.rows[0].key).toBe(before.rows[0].key);
    expect(after.rows[0].event.seq).toBe(3);
    expect(after.rows[0].sources).toEqual([first, second, complete]);
    expect(after.rows[0].codex).toMatchObject({ complete: true, parts: [{ text: 'Revised' }] });
    expect(after.rows.find(row => row.event === plan2)?.codex).toMatchObject({ kind: 'revision', revisions: [plan1, plan2] });
    expect(after.rows.find(row => row.event === diff2)?.codex).toMatchObject({ kind: 'revision', revisions: [diff1, diff2] });
    expect(after.rows.find(row => row.event === planOther)?.codex).toMatchObject({ kind: 'revision', revisions: [planOther] });
  });

  it('keeps malformed reasoning separate and retains only allowed summary evidence', () => {
    const first = event(1, 'item/reasoning/summaryTextDelta', { itemId: 'r', summaryIndex: 0, delta: 'Allowed' }, 'run-a', 'Allowed');
    const privateDelta = event(2, 'item/reasoning/textDelta', { itemId: 'r', delta: 'PRIVATE_RAW' }, 'run-a', 'PRIVATE_RAW');
    const malformed = event(3, 'reasoning', { type: 'reasoning', id: 'r', summary: [42] });
    const projection = projectAgentTranscript([first, privateDelta, malformed]);
    expect(projection.rows).toHaveLength(2);
    expect(projection.rows[0].event.seq).toBe(1);
    expect(projection.rows[0].sources).toEqual([first]);
    expect(projection.rows[0].codex).toMatchObject({ parts: [{ text: 'Allowed' }] });
    expect(projection.rows[1].codex).toEqual({ kind: 'malformed' });
    expect(JSON.stringify(projection.rows.map(row => row.codex))).not.toContain('PRIVATE_RAW');
  });

  it('retains a plan row key across revisions and nested completed reasoning snapshots', () => {
    const first = event(1, 'turn/plan/updated', { plan: [{ step: 'One', status: 'pending' }] });
    const next = event(2, 'turn/plan/updated', { plan: [{ step: 'One', status: 'completed' }] });
    const before = projectAgentTranscript([first]);
    const after = projectAgentTranscript([first, next]);
    expect(after.rows[0].key).toBe(before.rows[0].key);
    expect(after.rows[0].event.seq).toBe(2);
    expect(after.rows[0].sources).toEqual([first, next]);
    const nested = event(3, 'item/completed', { item: { type: 'reasoning', id: 'reasoning', summary: ['Nested summary'], content: ['PRIVATE_RAW'] } });
    expect(projectAgentTranscript([nested]).rows[0].codex).toMatchObject({ kind: 'summary', complete: true, parts: [{ text: 'Nested summary' }] });
    expect(JSON.stringify(inspectableAgentEvent(nested))).not.toContain('PRIVATE_RAW');
  });

  it('redacts Claude thinking, signature and redacted payloads recursively while preserving ordinary blocks', () => {
    const native = message(1, 'm', [
      { type: 'thinking', thinking: 'PRIVATE_THINKING', signature: 'PRIVATE_SIGNATURE' },
      { type: 'redacted_thinking', data: 'PRIVATE_REDACTED' },
      textBlock('Public text'),
    ], [0, 1, 2]);
    const inspected = inspectableAgentEvent({ ...native, text: 'PRIVATE_THINKING' });
    expect(JSON.stringify(inspected)).not.toMatch(/PRIVATE_/);
    expect(JSON.stringify(inspected)).toContain('Public text');
    expect(projectAgentTranscript([native]).rows.map(row => prose(row.event))).toEqual(['Public text']);
    for (const type of ['thinking', 'thinking_delta', 'signature', 'signature_delta', 'redacted_thinking']) {
      const raw = event(2, type, { type, text: 'PRIVATE_TEXT', delta: 'PRIVATE_DELTA', thinking: 'PRIVATE_THINKING', signature: 'PRIVATE_SIGNATURE', data: 'PRIVATE_DATA' }, 'run-a', 'PRIVATE_EVENT');
      expect(JSON.stringify(inspectableAgentEvent(raw))).not.toMatch(/PRIVATE_/);
      expect(prose(raw)).toBe('');
      expect(projectAgentTranscript([raw]).rows).toHaveLength(0);
    }
    const nested = event(3, 'stream_event', { event: { delta: { type: 'signature_delta', signature: 'PRIVATE_SIGNATURE' } } }, 'run-a', 'PRIVATE_EVENT');
    expect(JSON.stringify(inspectableAgentEvent(nested))).not.toMatch(/PRIVATE_/);
    expect(JSON.stringify(native)).toContain('PRIVATE_THINKING');
  });

  it('keeps a native execution key through start, output and completion while retaining status evidence', () => {
    const start = event(1, 'item/started', { type: 'commandExecution', id: 'command', status: 'inProgress', command: 'pwd' });
    const output = event(2, 'item/commandExecution/outputDelta', { itemId: 'command' }, 'run-a', '/workspace');
    const complete = event(3, 'commandExecution', { type: 'commandExecution', id: 'command', status: 'completed', aggregatedOutput: '/workspace', exitCode: 0 });
    const before = projectAgentTranscript([start]);
    const streaming = projectAgentTranscript([start, output]);
    const after = projectAgentTranscript([start, output, complete]);
    expect(streaming.rows).toHaveLength(1);
    expect(after.rows).toHaveLength(1);
    expect(streaming.rows[0].key).toBe(before.rows[0].key);
    expect(after.rows[0].key).toBe(before.rows[0].key);
    expect(object(streaming.rows[0].event.data).command).toBe('pwd');
    expect(after.rows[0].event).toMatchObject(complete);
    expect(object(after.rows[0].event.data).command).toBe('pwd');
    expect(after.rows[0].sources).toEqual([start, output, complete]);
  });

  it('coalesces interleaved output by item and preserves source evidence and stable key on completion', () => {
    const first = event(1, 'item/commandExecution/outputDelta', { itemId: 'one' }, 'run-a', 'A');
    const other = event(2, 'item/commandExecution/outputDelta', { itemId: 'two' }, 'run-a', 'B');
    const last = event(3, 'item/commandExecution/outputDelta', { itemId: 'one' }, 'run-a', 'C');
    const complete = event(4, 'commandExecution', { type: 'commandExecution', id: 'one', status: 'completed', aggregatedOutput: 'AC', exitCode: 0 });
    const before = projectAgentTranscript([first, other, last]);
    const after = projectAgentTranscript([first, other, last, complete]);
    expect(after.rows).toHaveLength(2);
    expect(after.rows[0].key).toBe(before.rows[0].key);
    expect(after.rows[0].event).toMatchObject(complete);
    expect(after.rows[0].sources).toEqual([first, last, complete]);
    expect(after.rows[1].event.text).toBe('B');
  });
});

describe('bounded per-run transcript projector', () => {
  it('reuses unchanged run rows and tool map references when an active run appends in the same array', () => {
    const projector = createAgentTranscriptProjector();
    const events = [message(1, 'old', [textBlock('Old answer')], [0]), delta(2, 'active', 0, 'New', null, 'run-b')];
    const first = projector.project(events);
    expect(projector.project(events)).toBe(first);
    events.push(delta(3, 'active', 0, ' answer', null, 'run-b'));
    const second = projector.project(events);
    expect(second.rows[0]).toBe(first.rows[0]);
    expect(second.rows[1].key).toBe(first.rows[1].key);
    expect(prose(second.rows[1].event)).toBe('New answer');
    expect(second.toolResults).toBe(first.toolResults);
    expect(second.toolNames).toBe(first.toolNames);
    expect(prose(first.rows[1].event)).toBe('New');
  });

  it('handles resets, trims, replaced entries, reordered runs and restoration without stale projected content', () => {
    const projector = createAgentTranscriptProjector();
    const events = [delta(1, 'm', 0, 'A'), delta(2, 'm', 0, 'B')];
    const first = projector.project(events);
    expect(prose(first.rows[0].event)).toBe('AB');
    events.shift();
    const trimmed = projector.project(events);
    expect(prose(trimmed.rows[0].event)).toBe('B');
    expect(trimmed.rows[0].key).toBe(first.rows[0].key);
    events[0] = delta(2, 'm', 0, 'Replacement');
    expect(prose(projector.project(events).rows[0].event)).toBe('Replacement');
    events.splice(0, events.length, message(8, 'restored', [textBlock('Restored')], [2], null, 'run-c'));
    expect(projector.project(events).rows.map(row => prose(row.event))).toEqual(['Restored']);
    events.unshift(message(9, 'earlier', [textBlock('Earlier')], [0], null, 'run-b'));
    expect(projector.project(events).rows.map(row => prose(row.event))).toEqual(['Earlier', 'Restored']);
    events.reverse();
    expect(projector.project(events).rows.map(row => prose(row.event))).toEqual(['Restored', 'Earlier']);
    events.length = 0;
    expect(projector.project(events).rows).toEqual([]);
    expect(projector.project(events).toolNames.size).toBe(0);
  });

  it('reconciles a restored snapshot after a trimmed stream without changing the block key', () => {
    const projector = createAgentTranscriptProjector();
    const first = delta(1, 'm', 3, 'Trimmed prefix ');
    const last = delta(2, 'm', 3, 'tail');
    const initial = projector.project([first, last]);
    const restored = message(3, 'm', [textBlock('Restored complete block')], [3]);
    const projection = projector.project([last, restored]);
    expect(projection.rows).toHaveLength(1);
    expect(projection.rows[0].key).toBe(initial.rows[0].key);
    expect(prose(projection.rows[0].event)).toBe('Restored complete block');
    expect(projection.rows[0].sources).toEqual([last, restored]);
  });

  it('invalidates changed tool maps and bounds run memoization without dropping visible history', () => {
    const projector = createAgentTranscriptProjector({ maxRuns: 1 });
    const use = message(1, 'm', [{ type: 'tool_use', id: 'tool', name: 'Read', input: {} }], [0]);
    const result = event(2, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'tool', content: 'one' }] });
    const events = [use, result];
    const first = projector.project(events);
    events.push(message(3, 'other', [textBlock('Other run')], [0], null, 'run-b'));
    const second = projector.project(events);
    expect(second.rows).toHaveLength(2);
    expect(second.toolResults).toBe(first.toolResults);
    const evicted = projector.project([use, result]);
    expect(evicted.rows[0]).not.toBe(first.rows[0]);
    expect(evicted.rows[0].key).toBe(first.rows[0].key);
    events.splice(0, events.length, use, event(2, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'tool', content: 'two' }] }));
    const replaced = projector.project(events);
    expect(replaced.toolResults).not.toBe(evicted.toolResults);
    expect(replaced.toolResults.get(itemKey(use, 'tool'))).toMatchObject({ content: 'two' });
    projector.clear();
    expect(projector.project(events).rows[0]).not.toBe(replaced.rows[0]);
  });
});

describe('REPLY-1 content and evidence regression', () => {
  it.each([
    [{}, 'streamed output'],
    [{ aggregatedOutput: null }, 'streamed output'],
    [{ aggregatedOutput: '' }, ''],
    [{ aggregatedOutput: 'authoritative replacement' }, 'authoritative replacement'],
  ] as [Record<string, Json>, string][])('RA01 preserves absent output and replaces explicit complete output: %j', (snapshot, expected) => {
    const start = event(1, 'item/started', { id: 'cmd', type: 'commandExecution', command: 'run', status: 'inProgress' });
    const output = event(2, 'item/commandExecution/outputDelta', { itemId: 'cmd' }, 'run-a', 'streamed output');
    const complete = event(3, 'commandExecution', { id: 'cmd', type: 'commandExecution', status: 'completed', exitCode: 0, ...snapshot });
    const late = event(4, 'item/commandExecution/outputDelta', { itemId: 'cmd' }, 'run-a', ' ignored late suffix');
    const before = projectAgentTranscript([start, output]);
    const after = projectAgentTranscript([start, output, complete, late]);
    expect(after.rows).toHaveLength(1);
    expect(after.rows[0].key).toBe(before.rows[0].key);
    expect(after.rows[0].event.kind).toBe('commandExecution');
    expect(object(after.rows[0].event.data)).toMatchObject({ aggregatedOutput: expected, status: 'completed', command: 'run' });
    expect(after.rows[0].sources).toEqual([start, output, complete, late]);
    expect(after.rows[0].event.seq).toBe(4);
    expect(object(complete.data)).not.toHaveProperty('command');
  });

  it('RA01 retains absent answer output, honors an empty complete answer and ignores later deltas', () => {
    const first = event(1, 'item/agentMessage/delta', { itemId: 'answer', phase: 'commentary' }, 'run-a', 'Collected answer');
    const missing = event(2, 'agentMessage', { id: 'answer', phase: 'final_answer' });
    const late = event(3, 'item/agentMessage/delta', { itemId: 'answer' }, 'run-a', 'late');
    const projected = projectAgentTranscript([first, missing, late]);
    expect(prose(projected.rows[0].event)).toBe('Collected answer');
    expect(object(projected.rows[0].event.data).phase).toBe('final_answer');
    expect(projected.rows[0].sources).toEqual([first, missing, late]);
    const empty = event(2, 'agentMessage', { id: 'answer', text: '', phase: 'final_answer' });
    expect(projectAgentTranscript([first, empty, late]).rows).toHaveLength(0);
    const replacement = event(2, 'agentMessage', { id: 'answer', text: 'Replacement' });
    expect(prose(projectAgentTranscript([first, replacement, late]).rows[0].event)).toBe('Replacement');
  });

  it('RA02 extracts structured restored input while preserving block order and native payloads', () => {
    const restored = event(1, 'userMessage', { type: 'userMessage', input: [textBlock('First paragraph'), { type: 'image', url: 'local-image-metadata' }, textBlock('Second paragraph')] }, null);
    const projection = projectAgentTranscript([restored]);
    expect(projection.rows).toHaveLength(1);
    expect(prose(projection.rows[0].event)).toBe('First paragraph\n\nSecond paragraph');
    expect(isUser(projection.rows[0].event)).toBe(true);
    expect(blocks(restored)).toHaveLength(3);
    expect(object(restored.data)).not.toHaveProperty('content');
  });

  it('RA02 keeps invisible starts and metadata out of visible rows without losing the later block identity', () => {
    const start = event(1, 'content_block_start', { native_message_id: 'm', block_index: 2, content_block: textBlock('') });
    const metadata = event(2, 'thread/tokenUsage/updated', { totalTokens: 10 });
    const summaryStart = event(3, 'item/reasoning/summaryPartAdded', { itemId: 'r', summaryIndex: 0 });
    expect(projectAgentTranscript([start, metadata, summaryStart]).rows).toHaveLength(0);
    const first = delta(4, 'm', 2, 'Visible');
    const completed = message(5, 'm', [textBlock('Complete')], [2]);
    const before = projectAgentTranscript([start, metadata, first]);
    const after = projectAgentTranscript([start, metadata, first, completed]);
    expect(after.rows).toHaveLength(1);
    expect(after.rows[0].key).toBe(before.rows[0].key);
    expect(after.rows[0].sources).toEqual([start, first, completed]);
    expect(prose(after.rows[0].event)).toBe('Complete');
  });

  it('RA02 merges scoped tool progress and elapsed evidence without replacing the tool or result status', () => {
    const tool = message(1, 'm', [{ type: 'tool_use', id: 'read', name: 'Read', input: {} }], [0]);
    const progress = event(2, 'tool_progress', { tool_use_id: 'read', tool_name: 'Read', task_id: 'task', elapsed_time_seconds: 2 });
    const result = event(3, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'read', content: 'done' }] });
    const late = event(4, 'tool_progress', { tool_use_id: 'read', elapsed_time_seconds: 4 });
    const child = event(5, 'tool_progress', { tool_use_id: 'read', parent_tool_use_id: 'other-parent', elapsed_time_seconds: 7 });
    const otherRun = event(6, 'tool_progress', { tool_use_id: 'read', elapsed_time_seconds: 9 }, 'run-b');
    const projection = projectAgentTranscript([tool, progress, result, late, child, otherRun]);
    expect(projection.rows).toHaveLength(3);
    expect(projection.rows[0].key).toBe(projectAgentTranscript([tool]).rows[0].key);
    expect(projection.rows[0].sources).toEqual([tool, progress, result, late]);
    expect(projection.rows[0].event.kind).toBe('message');
    expect(object(projection.rows[0].event.data)).toMatchObject({ elapsed_time_seconds: 4, task_id: 'task', tool_name: 'Read' });
    expect(projection.toolResults.get(itemKey(tool, 'read'))).toMatchObject({ content: 'done' });
    expect(projection.rows[1].event).toBe(child);
    expect(projection.rows[2].event).toBe(otherRun);
  });

  it('RA02 keeps elapsed progress attributed to each tool in a legacy multi-tool message', () => {
    const tools = event(1, 'message', { type: 'assistant', content: [{ type: 'tool_use', id: 'first', name: 'Read', input: {} }, { type: 'tool_use', id: 'second', name: 'Grep', input: {} }] });
    const first = event(2, 'tool_progress', { tool_use_id: 'first', elapsed_time_seconds: 2 });
    const second = event(3, 'tool_progress', { tool_use_id: 'second', elapsed_time_seconds: 5 });
    const projection = projectAgentTranscript([tools, first, second]);
    expect(projection.rows).toHaveLength(1);
    expect(projection.rows[0].sources).toEqual([tools, first, second]);
    expect(object(projection.rows[0].event.data)).toMatchObject({ tool_progress: { first: { elapsed_time_seconds: 2 }, second: { elapsed_time_seconds: 5 } } });
    expect(object(projection.rows[0].event.data)).not.toHaveProperty('elapsed_time_seconds');
  });

  it('RA02 separates resumed executions of the same native task within a host run', () => {
    const first = event(1, 'task_started', { task_id: 'task', run_id: 'execution-a', status: 'running' });
    const done = event(2, 'task_notification', { task_id: 'task', run_id: 'execution-a', status: 'completed' });
    const resumed = event(3, 'task_started', { task_id: 'task', run_id: 'execution-b', status: 'running' });
    const failed = event(4, 'task_notification', { task_id: 'task', run_id: 'execution-b', status: 'failed' });
    const projection = projectAgentTranscript([first, done, resumed, failed]);
    expect(projection.rows).toHaveLength(2);
    expect(projection.rows[0].sources).toEqual([first, done]);
    expect(projection.rows[1].sources).toEqual([resumed, failed]);
    expect(object(projection.rows[0].event.data).status).toBe('completed');
    expect(object(projection.rows[1].event.data).status).toBe('failed');
    expect(projection.rows[0].key).not.toBe(projection.rows[1].key);
  });

  it('RA02 completes the original background task after a new run begins and does not regress on delayed progress', () => {
    const started = event(1, 'task_started', { task_id: 'task', session_id: 'native', status: 'running' });
    const nextTurn = event(2, 'userMessage', null, 'run-b', 'Next request');
    const unrelated = event(3, 'task_started', { task_id: 'task', session_id: 'native', status: 'running' }, 'run-b');
    const completed = event(4, 'task_notification', { task_id: 'task', session_id: 'native', status: 'completed', summary: 'Done' });
    const delayed = event(5, 'task_progress', { task_id: 'task', session_id: 'native', summary: 'Old progress' });
    const events = [started, nextTurn, unrelated, completed, delayed];
    for (const projection of [projectAgentTranscript(events), createAgentTranscriptProjector().project(events)]) {
      expect(projection.rows).toHaveLength(3);
      expect(projection.rows[0].event).toMatchObject({ run: 'run-a', kind: 'task_notification', data: { status: 'completed', summary: 'Done' } });
      expect(projection.rows[0].sources).toEqual([started, completed, delayed]);
      expect(projection.rows[2].event).toBe(unrelated);
    }
  });

  it('RA02 scopes restored background tasks by native session and parent when no run exists', () => {
    const started = event(1, 'task_started', { task_id: 'task', session_id: 'native-a', status: 'running' }, null);
    const child = event(2, 'task_started', { task_id: 'task', session_id: 'native-a', parent_tool_use_id: 'spawn', status: 'running' }, null);
    const otherSession = event(3, 'task_started', { task_id: 'task', session_id: 'native-b', status: 'running' }, null);
    const completed = event(4, 'task_notification', { task_id: 'task', session_id: 'native-a', status: 'completed' }, null);
    const events = [started, child, otherSession, completed];
    for (const projection of [projectAgentTranscript(events), createAgentTranscriptProjector().project(events)]) {
      expect(projection.rows).toHaveLength(3);
      expect(projection.rows[0].sources).toEqual([started, completed]);
      expect(projection.rows[0].event).toMatchObject({ kind: 'task_notification', data: { status: 'completed' } });
      expect(projection.rows[1].parentToolUseId).toBe('spawn');
      expect(projection.rows[2].event).toBe(otherSession);
    }
  });

  it('RA03 treats nested native MCP business errors as failures even when transport completed', () => {
    const failed = event(1, 'mcpToolCall', { type: 'mcpToolCall', status: 'completed', result: { isError: true, content: [textBlock('Rejected by tool')] } });
    expect(agentEventFailed(failed)).toBe(true);
    expect(agentEventFailed(event(2, 'mcpToolCall', { status: 'completed', result: { isError: false } }))).toBe(false);
    expect(agentEventFailed(event(3, 'message', { type: 'user', tool_use_result: { isError: true } }))).toBe(true);
  });

  it('RA03 attaches unique envelope failure and structured content to its scoped tool result without mutating native evidence', () => {
    const tool = message(1, 'm', [{ type: 'tool_use', id: 'mcp', name: 'mcp__data__query', input: {} }], [0]);
    const content: Json[] = [textBlock('Business response')];
    const structuredContent: Json = { records: [{ id: 'one', value: 3 }] };
    const block: Json = { type: 'tool_result', tool_use_id: 'mcp', content };
    const result = event(2, 'message', { type: 'user', is_error: true, content: [block], tool_use_result: { structuredContent } });
    const projection = projectAgentTranscript([tool, result]);
    expect(projection.rows).toHaveLength(1);
    expect(projection.rows[0].sources).toEqual([tool, result]);
    const projectedResult = object(projection.toolResults.get(itemKey(tool, 'mcp')));
    expect(projectedResult).toMatchObject({ type: 'tool_result', tool_use_id: 'mcp', is_error: true });
    expect(projectedResult.content).toBe(content);
    expect(projectedResult.structuredContent).toBe(structuredContent);
    expect(projectedResult).not.toBe(block);
    expect(object(block)).not.toHaveProperty('is_error');
    expect(object(block)).not.toHaveProperty('structuredContent');
    const otherRun = { ...result, run: 'run-b' };
    const isolated = projectAgentTranscript([tool, otherRun]);
    expect(isolated.toolResults.has(itemKey(tool, 'mcp'))).toBe(false);
    expect(isolated.rows).toHaveLength(2);
  });

  it('RA03 preserves an ambiguous envelope failure separately and never assigns its structured content to multiple tools', () => {
    const tools = message(1, 'm', [{ type: 'tool_use', id: 'first', name: 'mcp__one', input: {} }, { type: 'tool_use', id: 'second', name: 'mcp__two', input: {} }], [0, 1]);
    const first: Json = { type: 'tool_result', tool_use_id: 'first', content: 'First response' };
    const second: Json = { type: 'tool_result', tool_use_id: 'second', content: 'Second response' };
    const result = event(2, 'message', { type: 'user', is_error: true, content: [first, second], tool_use_result: { structuredContent: { records: ['ambiguous'] } } });
    const projection = projectAgentTranscript([tools, result]);
    expect(projection.rows).toHaveLength(3);
    expect(projection.rows.at(-1)?.event).toBe(result);
    expect(projection.toolResults.get(itemKey(tools, 'first'))).toBe(first);
    expect(projection.toolResults.get(itemKey(tools, 'second'))).toBe(second);
    expect(object(projection.toolResults.get(itemKey(tools, 'first')))).not.toHaveProperty('structuredContent');
    expect(object(projection.toolResults.get(itemKey(tools, 'second')))).not.toHaveProperty('is_error');
  });

  it('RA04 locates exact original evidence before latest display sequences and never crosses runs', () => {
    const first = delta(1, 'm', 0, 'First');
    const second = delta(2, 'm', 2, 'Second');
    const stop = event(20, 'message_stop', { native_message_id: 'm' });
    const otherRun = delta(2, 'm', 0, 'Other run', null, 'run-b');
    const { rows } = projectAgentTranscript([first, second, stop, otherRun]);
    expect(rows.slice(0, 2).map(row => row.event.seq)).toEqual([20, 20]);
    expect(findAgentTranscriptRow(rows, { run: 'run-a', seq: 2 })).toBe(rows[1]);
    expect(findAgentTranscriptRow(rows, { run: 'run-b', seq: 2 })).toBe(rows[2]);
    expect(findAgentTranscriptRow(rows, { run: 'run-a', seq: 3 })).toBe(rows[1]);
    expect(findAgentTranscriptRow(rows, { run: 'missing', seq: 2 })).toBeUndefined();
  });
});

describe('native notification projection boundary', () => {
  it('keeps routine notifications out of the last 100 reply rows without changing source evidence', () => {
    const replies = Array.from({ length: 120 }, (_, index) => event(index + 1, 'agentMessage', { id: `reply-${index}` }, 'run-a', `Reply ${index + 1}`));
    const routineTypes = ['hook/started', 'hook/completed', 'account/rateLimits/updated', 'turn/moderationMetadata', 'thread/status/changed', 'thread/readState/updated', 'thread/settings/updated', 'mcpServer/startupStatus/updated', 'future/nativeType'];
    const notifications = Array.from({ length: 116 }, (_, index) => event(index + 121, 'native/unsupported', { diagnostic: 'unsupported_native_event', native_type: index >= 114 ? 'warning' : routineTypes[index % routineTypes.length], payload_omitted: true }));
    const events = [...replies, ...notifications];
    const original = JSON.stringify(events);
    for (const projection of [projectAgentTranscript(events), createAgentTranscriptProjector().project(events)]) {
      expect(projection.rows).toHaveLength(122);
      expect(projection.rows.slice(-100).filter(row => row.event.kind === 'agentMessage')).toHaveLength(98);
      expect(projection.rows.slice(-100)[0].event).toBe(replies[22]);
      expect(projection.rows.filter(row => row.event.kind === 'native/unsupported').map(row => row.event)).toEqual(notifications.slice(-2));
      expect(projection.rows.at(-1)?.sources[0]).toBe(notifications.at(-1));
      expect(Object.keys(projection).sort()).toEqual(['rows', 'toolNames', 'toolResults']);
    }
    expect(JSON.stringify(events)).toBe(original);
    expect(events[120]).toBe(notifications[0]);
    expect(events).toHaveLength(236);
  });

  it.each([
    { native_type: 'error' },
    { native_type: 'hook/failed' },
    { native_type: 'failure' },
    { native_type: 'future/type', error: 'Native error' },
    { native_type: 'future/type', status: 'error' },
    { native_type: 'future/type', status: 'failure' },
    { native_type: 'future/type', errors: ['Native error'] },
    { native_type: 'mcpServer/startupStatus/updated', status: 'failed' },
    { native_type: 'future/type', run_status: 'failed' },
    { native_type: 'future/type', success: false },
    { native_type: 'future/type', is_error: true },
    { native_type: 'future/type', isError: true },
    { native_type: 'future/type', exitCode: 1 },
    { native_type: 'future/type', result: { isError: true } },
    { native_type: 'future/type', tool_use_result: { is_error: true } },
  ] as Json[])('retains explicit native failure evidence in the visible projection: %j', data => {
    const failed = event(1, 'native/unsupported', data);
    const projection = projectAgentTranscript([failed]);
    expect(projection.rows).toHaveLength(1);
    expect(projection.rows[0].event).toBe(failed);
    expect(projection.rows[0].sources).toEqual([failed]);
  });

  it('applies the same boundary to restored legacy records and cached appended notifications', () => {
    const reply = event(1, 'agentMessage', { id: 'old', history: true }, null, 'Restored answer');
    const ordinary = event(2, 'native/unsupported', { native_type: 'hook/completed', payload_omitted: true }, null);
    const unknown = event(3, 'native/unsupported', { native_type: 'future/event', status: 'completed' }, null);
    const warning = event(4, 'native/unsupported', { native_type: 'warning', payload_omitted: true }, null);
    const events = [reply, ordinary, unknown, warning];
    const projector = createAgentTranscriptProjector();
    expect(projectAgentTranscript(events).rows.map(row => row.event)).toEqual([reply, warning]);
    expect(projector.project(events).rows.map(row => row.event)).toEqual([reply, warning]);
    const failed = event(5, 'native/unsupported', { native_type: 'future/event', status: 'failed' }, null);
    events.push(failed);
    expect(projector.project(events).rows.map(row => row.event)).toEqual([reply, warning, failed]);
  });
});

describe('failure evidence retained through projection', () => {
  it.each([
    { isError: true },
    { tool_use_result: { isError: true } },
    { tool_use_result: { is_error: true } },
  ] as Record<string, Json>[])('attaches a uniquely scoped envelope failure to its tool result: %j', envelope => {
    const tool = message(1, 'm', [{ type: 'tool_use', id: 'read', name: 'Read', input: {} }], [0]);
    const block: Json = { type: 'tool_result', tool_use_id: 'read', content: 'Native failure' };
    const result = event(2, 'message', { type: 'user', content: [block], ...envelope });
    const projection = projectAgentTranscript([tool, result]);
    expect(projection.rows).toHaveLength(1);
    expect(projection.rows[0].sources).toEqual([tool, result]);
    expect(projection.toolResults.get(itemKey(tool, 'read'))).toMatchObject({ is_error: true, content: 'Native failure' });
    expect(object(block)).not.toHaveProperty('is_error');
  });

  it.each([
    { errors: ['Envelope failure'] },
    { status: 'error' },
    { result: { is_error: true } },
    { tool_use_result: { isError: true } },
  ] as Record<string, Json>[])('retains an independent or ambiguous envelope failure: %j', envelope => {
    const tool = message(1, 'm', [{ type: 'tool_use', id: 'one', name: 'Read', input: {} }, { type: 'tool_use', id: 'two', name: 'Read', input: {} }], [0, 1]);
    const result = event(2, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'one', content: 'One' }, { type: 'tool_result', tool_use_id: 'two', content: 'Two' }], ...envelope });
    const projection = projectAgentTranscript([tool, result]);
    expect(projection.rows).toHaveLength(3);
    expect(projection.rows.at(-1)?.event).toBe(result);
    expect(agentEventFailed(result)).toBe(true);
    expect(object(projection.toolResults.get(itemKey(tool, 'one')))).not.toHaveProperty('is_error');
  });

  it.each([
    { errors: ['Native failure'] },
    { status: 'failure' },
    { isError: true },
  ] as Record<string, Json>[])('does not suppress failure terminal prose matching an assistant answer: %j', failure => {
    const answer = message(1, 'answer', [textBlock('Repeated native evidence')], [0]);
    const terminal = event(2, 'result', { result: 'Repeated native evidence', ...failure });
    expect(projectAgentTranscript([answer, terminal]).rows.at(-1)?.suppressText).toBe(false);
  });
});
