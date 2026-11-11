import { describe, expect, test } from 'bun:test';
import { NativeRunBindings, NativeTranscriptProjection, safeNativeDetails, safeUnknownEvent } from './native-transcript';

const stream = (event: Record<string, unknown>, parent: string | null = null) => ({ type: 'stream_event', session_id: 'session', parent_tool_use_id: parent, event });
const assistant = (id: string, uuid: string, content: unknown[], parent: string | null = null) => ({ type: 'assistant', session_id: 'session', parent_tool_use_id: parent, uuid, message: { id, model: 'model', content, usage: { input_tokens: 10, output_tokens: 999 } } });

describe('Claude native transcript boundaries', () => {
  test('keeps background tool results and resumed task runs in their original sends', () => {
    const bindings = new NativeRunBindings();
    expect(bindings.resolve(assistant('api-a', 'a', [{ type: 'tool_use', id: 'spawn-a', name: 'Agent', input: {} }]), 'send-a')).toBe('send-a');
    expect(bindings.resolve({ type: 'system', subtype: 'task_started', task_id: 'task', run_id: '001', tool_use_id: 'spawn-a' }, 'send-a')).toBe('send-a');
    expect(bindings.resolve(stream({ type: 'message_start', message: { id: 'child-api' } }, 'spawn-a'), 'send-b')).toBe('send-a');
    expect(bindings.resolve({ type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: 'spawn-a', content: 'done' }] } }, 'send-b')).toBe('send-a');
    bindings.resolve(assistant('api-b', 'b', [{ type: 'tool_use', id: 'spawn-b', name: 'Agent', input: {} }]), 'send-b');
    expect(bindings.resolve({ type: 'system', subtype: 'task_started', task_id: 'task', run_id: '002', tool_use_id: 'spawn-b' }, 'send-b')).toBe('send-b');
    expect(bindings.resolve({ type: 'system', subtype: 'task_notification', task_id: 'task', run_id: '001' }, 'send-b')).toBe('send-a');
    expect(bindings.resolve({ type: 'system', subtype: 'task_progress', task_id: 'task', run_id: '002' }, 'send-c')).toBe('send-b');
    expect(bindings.resolve({ type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: 'missing' }] } }, 'send-c')).toBeUndefined();
    expect(bindings.resolve({ type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: 'spawn-a' }, { type: 'tool_result', tool_use_id: 'spawn-b' }] } }, 'send-c')).toBeUndefined();
    expect(bindings.resolve({ type: 'system', subtype: 'task_notification', task_id: 'unknown' }, 'send-c')).toBeUndefined();
    expect(bindings.resolve({ type: 'tool_progress', tool_use_id: 'unknown' }, 'send-c')).toBeUndefined();
    bindings.clear();
    expect(bindings.resolve(stream({ type: 'message_stop' }, 'spawn-a'), 'send-c')).toBeUndefined();
  });

  test('keeps unknown event type diagnostics without retaining any arbitrary body', () => {
    const projection = new NativeTranscriptProjection();
    const value = { type: 'system', subtype: 'future_event', credentials: 'SECRET', thinking: 'SECRET', payload: { apiKey: 'SECRET' } };
    expect(projection.project(value)).toEqual([{ kind: 'native/unsupported', data: { diagnostic: 'unsupported_native_event', native_type: 'system', native_subtype: 'future_event', payload_omitted: true } }]);
    for (const subtype of ['auth_status', 'thinking_tokens', 'login_callback']) expect(safeUnknownEvent({ type: 'system', subtype, payload: 'SECRET' })).toBeUndefined();
    const unknown = projection.project(assistant('api', 'unknown', [{ type: 'future_block', data: 'SECRET', thinking: 'SECRET' }]));
    expect(unknown[0].data.content).toEqual([{ type: 'future_block', diagnostic: 'unsupported_native_event', payload_omitted: true }]);
    expect(JSON.stringify(unknown)).not.toContain('SECRET');
    expect(JSON.stringify(projection.project({ type: 'x'.repeat(1000), payload: 'SECRET' }))).not.toContain('SECRET');
    expect(projection.project(assistant('api', 'untyped', [{ payload: 'SECRET' }]))[0].data.content).toEqual([{ type: 'unknown', diagnostic: 'unsupported_native_event', payload_omitted: true }]);
  });

  test('passes native MCP resources, structured data, image bytes and search hits to the host boundary', () => {
    const projection = new NativeTranscriptProjection();
    const content = [{ type: 'tool_result', tool_use_id: 'mcp', content: [{ type: 'text', text: 'answer' }, { type: 'resource', resource: { uri: 'file:///workspace/report.txt', mimeType: 'text/plain', text: 'report' } }, { type: 'image', mimeType: 'image/png', data: 'existing-native-bytes' }] }];
    const tool_use_result = { structuredContent: { type: 'business_record', value: 42, content: [{ type: 'domain_record', value: 7 }] }, results: [{ tool_use_id: 'native-search', content: [{ title: 'Native source', url: 'https://example.org' }] }] };
    const event = projection.project({ type: 'user', message: { content }, tool_use_result })[0];
    expect(event.data.content).toEqual(content);
    expect(event.data.tool_use_result).toEqual(tool_use_result);
    const failed = projection.project({ type: 'user', message: { content }, tool_use_result: { isError: true, content: 'SECRET' } })[0];
    expect(failed.data.tool_use_result).toBe('engine_failed');
    expect(failed.data.is_error).toBe(true);
    expect(failed.data.content).toEqual([{ type: 'tool_result', tool_use_id: 'mcp', is_error: true, content: 'engine_failed' }]);
    expect(JSON.stringify(failed)).not.toContain('SECRET');
  });

  test('preserves native search hit fields while omitting unknown payloads and malformed hits', () => {
    const projected = safeNativeDetails({ tool_use_result: { query: 'native query', results: [{ tool_use_id: 'server-search', content: [{ title: 'Native source', url: 'https://example.org', snippet: 'Public excerpt', token: 'SECRET', thinking: 'SECRET' }, { title: 'Missing URL', payload: 'SECRET' }] }, { payload: 'SECRET' }] } });
    expect(projected).toEqual({ tool_use_result: { query: 'native query', results: [{ tool_use_id: 'server-search', content: [{ title: 'Native source', url: 'https://example.org', snippet: 'Public excerpt' }, { diagnostic: 'unsupported_native_event', native_type: 'search_hit', payload_omitted: true }] }, { diagnostic: 'unsupported_native_event', native_type: 'search_result_group', payload_omitted: true }] } });
    expect(JSON.stringify(projected)).not.toContain('SECRET');
    const unknown = new NativeTranscriptProjection().project(assistant('api', 'not-a-search-hit', [{ title: 'Native source', url: 'https://example.org', payload: 'SECRET' }]));
    expect(unknown[0].data.content).toEqual([{ type: 'unknown', diagnostic: 'unsupported_native_event', payload_omitted: true }]);
  });

  test('retains equal text blocks from one API message and associates tool blocks by native id', () => {
    const projection = new NativeTranscriptProjection();
    projection.project(stream({ type: 'message_start', message: { id: 'api' } }));
    for (const index of [1, 3]) {
      projection.project(stream({ type: 'content_block_start', index, content_block: { type: 'text', text: 'same' } }));
      const complete = projection.project(assistant('api', `text-${index}`, [{ type: 'text', text: 'same' }]));
      expect(complete[0].data.block_indices).toEqual([index]);
      projection.project(stream({ type: 'content_block_stop', index }));
    }
    for (const [index, id] of [[5, 'read-one'], [6, 'read-two']] as const) {
      projection.project(stream({ type: 'content_block_start', index, content_block: { type: 'tool_use', id, name: 'Read', input: {} } }));
    }
    const tools = projection.project(assistant('api', 'tools', [{ type: 'tool_use', id: 'read-two', name: 'Read', input: { file_path: 'two' } }, { type: 'tool_use', id: 'read-one', name: 'Read', input: { file_path: 'one' } }]));
    expect(tools[0].data.block_indices).toEqual([6, 5]);
  });

  test('main turn completion preserves a still-running background child stream', () => {
    const projection = new NativeTranscriptProjection();
    for (const parent of [null, 'spawn']) {
      projection.project(stream({ type: 'message_start', message: { id: parent ? 'child-api' : 'main-api' } }, parent));
      projection.project(stream({ type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } }, parent));
    }
    projection.clearMain();
    const child = projection.project(stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'background complete' } }, 'spawn'));
    expect(child[0].data.native_message_id).toBe('child-api');
    expect(projection.project(assistant('child-api', 'background', [{ type: 'text', text: 'background complete' }], 'spawn'))[0].data.block_indices).toEqual([0]);
    const orphan = projection.project(stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'late main' } }));
    expect(orphan[0].data.native_message_id).toBeUndefined();
    projection.clear();
    expect(projection.project(assistant('child-api', 'after-close', [{ type: 'text', text: 'background complete' }], 'spawn'))[0].data.block_indices).toEqual([null]);
  });

  test('links separate completed blocks sharing an API id across both native stop orderings', () => {
    const projection = new NativeTranscriptProjection();
    projection.project(stream({ type: 'message_start', message: { id: 'api' } }));
    projection.project(stream({ type: 'content_block_start', index: 1, content_block: { type: 'text', text: '' } }));
    const delta = projection.project(stream({ type: 'content_block_delta', index: 1, delta: { type: 'text_delta', text: 'First' } }));
    expect(delta).toEqual([{ kind: 'text_delta', data: { native_message_id: 'api', block_index: 1, parent_tool_use_id: null, text: 'First' } }]);
    // Some native paths deliver the complete block before its stream stop.
    const first = assistant('api', 'first-block', [{ type: 'text', text: 'First' }]);
    expect(projection.project(first)[0].data).toMatchObject({ block_indices: [1], usage_final: false, usage_source: 'native_response_input' });
    projection.project(stream({ type: 'content_block_stop', index: 1 }));
    projection.project(stream({ type: 'content_block_start', index: 4, content_block: { type: 'text', text: '' } }));
    projection.project(stream({ type: 'content_block_delta', index: 4, delta: { type: 'text_delta', text: 'Second' } }));
    projection.project(stream({ type: 'content_block_stop', index: 4 }));
    expect(projection.project(assistant('api', 'second-block', [{ type: 'text', text: 'Second' }]))[0].data).toMatchObject({ native_message_id: 'api', content: [{ type: 'text', text: 'Second' }], block_indices: [4] });
    expect(projection.project(first)).toEqual([]);
    expect(projection.project(stream({ type: 'message_stop' }))).toEqual([{ kind: 'message_stop', data: { native_message_id: 'api', parent_tool_use_id: null } }]);
  });

  test('retains distinct completed payloads even when a producer reuses a wrapper UUID', () => {
    const projection = new NativeTranscriptProjection();
    expect(projection.project(assistant('same-api', 'same-wrapper', [{ type: 'text', text: 'one' }]))).toHaveLength(1);
    expect(projection.project(assistant('same-api', 'same-wrapper', [{ type: 'text', text: 'two' }]))).toHaveLength(1);
    expect(projection.project(assistant('same-api', 'same-wrapper', [{ type: 'text', text: 'two' }]))).toEqual([]);
  });

  test('keeps interleaved parent streams independent and forwards complete child text', () => {
    const projection = new NativeTranscriptProjection();
    for (const parent of [null, 'spawn']) {
      projection.project(stream({ type: 'message_start', message: { id: 'shared-id' } }, parent));
      projection.project(stream({ type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } }, parent));
    }
    const child = projection.project(stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'child text' } }, 'spawn'));
    expect(child[0].data).toMatchObject({ parent_tool_use_id: 'spawn', native_message_id: 'shared-id', block_index: 0 });
    projection.project(stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'main text' } }));
    expect(projection.project({ ...assistant('shared-id', 'child', [{ type: 'text', text: 'child text' }], 'spawn'), agent_id: 'native-agent' })[0].data).toMatchObject({ parent_tool_use_id: 'spawn', agent_id: 'native-agent', content: [{ type: 'text', text: 'child text' }], block_indices: [0] });
    expect(projection.project(assistant('shared-id', 'main', [{ type: 'text', text: 'main text' }]))[0].data.block_indices).toEqual([0]);
  });

  test('never invents missing or ambiguous block identities after resume and cutoff', () => {
    const projection = new NativeTranscriptProjection();
    const historical = assistant('api', 'history', [{ type: 'text', text: 'saved' }]);
    expect(projection.project(historical, true)[0].data.block_indices).toEqual([null]);
    projection.project(stream({ type: 'message_start', message: { id: 'api' } }));
    for (const index of [2, 7]) {
      projection.project(stream({ type: 'content_block_start', index, content_block: { type: 'text', text: 'same' } }));
      projection.project(stream({ type: 'content_block_stop', index }));
    }
    expect(projection.project(assistant('api', 'ambiguous', [{ type: 'text', text: 'same' }]))[0].data.block_indices).toEqual([null]);
    projection.clear();
    const orphan = projection.project(stream({ type: 'content_block_delta', index: 2, delta: { type: 'text_delta', text: 'late' } }));
    expect(orphan[0].data.native_message_id).toBeUndefined();
    expect(orphan[0].data.block_index).toBe(2);
    expect(projection.project({ ...assistant('api', 'aborted', [{ type: 'text', text: 'same' }]), aborted: true })[0].data).toMatchObject({ block_indices: [null], aborted: true });
    projection.project(stream({ type: 'message_start', message: { id: 'next-api' } }));
    const next = projection.project(stream({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'next' } }));
    expect(next[0].data.native_message_id).toBe('next-api');
  });

  test('filters thinking, signatures, redaction and error bodies before any display payload', () => {
    const projection = new NativeTranscriptProjection();
    projection.project(stream({ type: 'message_start', message: { id: 'api', content: [{ type: 'thinking', thinking: 'SECRET' }] } }));
    expect(projection.project(stream({ type: 'content_block_start', index: 0, content_block: { type: 'thinking', thinking: 'SECRET', signature: 'SECRET' } }))).toEqual([]);
    expect(projection.project(stream({ type: 'content_block_delta', index: 0, delta: { type: 'thinking_delta', thinking: 'SECRET' } }))).toEqual([]);
    expect(projection.project(stream({ type: 'content_block_delta', index: 0, delta: { type: 'signature_delta', signature: 'SECRET' } }))).toEqual([]);
    expect(projection.project(stream({ type: 'content_block_stop', index: 0 }))).toEqual([]);
    projection.project(stream({ type: 'content_block_start', index: 3, content_block: { type: 'text', text: 'Visible' } }));
    const visible = projection.project(assistant('api', 'safe', [{ type: 'thinking', thinking: 'SECRET', signature: 'SECRET' }, { type: 'redacted_thinking', data: 'SECRET' }, { type: 'text', text: 'Visible', signature: 'SECRET' }]))[0];
    expect(visible.data.content).toEqual([{ type: 'text', text: 'Visible' }]);
    expect(visible.data.block_indices).toEqual([3]);
    expect(JSON.stringify(visible)).not.toContain('SECRET');
    const failure = projection.project({ ...assistant('api', 'failed', [{ type: 'text', text: 'SECRET' }]), error: 'SECRET', tool_use_result: { error: 'SECRET' } });
    expect(JSON.stringify(failure)).not.toContain('SECRET');
    expect(failure[0].data).toMatchObject({ content: 'engine_failed', error: 'engine_failed', tool_use_result: 'engine_failed' });
    const tool = projection.project({ type: 'user', uuid: 'tool-result', message: { content: [{ type: 'tool_result', tool_use_id: 'tool', is_error: true, content: 'SECRET', extra: 'SECRET' }] } });
    expect(JSON.stringify(tool)).not.toContain('SECRET');
    expect(safeNativeDetails({ errors: ['SECRET'], tool_use_result: { output: 'SECRET' } })).toEqual({ errors: ['engine_failed'], tool_use_result: 'engine_failed' });
    expect(safeNativeDetails({ tool_use_result: { isError: true, content: 'SECRET' } })).toEqual({ tool_use_result: 'engine_failed', is_error: true });
    const childFailure = projection.project({ ...assistant('api', 'child-failure', [{ type: 'text', text: 'SECRET' }], 'spawn'), isApiErrorMessage: true });
    expect(childFailure[0].data).toMatchObject({ parent_tool_use_id: 'spawn', content: 'engine_failed', error: 'engine_failed' });
    expect(JSON.stringify(childFailure)).not.toContain('SECRET');
  });

  test('preserves native task lifecycle evidence using the shared whitelist only', () => {
    const projection = new NativeTranscriptProjection();
    for (const subtype of ['task_started', 'task_progress', 'task_notification']) {
      const projected = projection.project({ type: 'system', subtype, task_id: 'task', tool_use_id: 'spawn', parent_tool_use_id: 'outer', agent_id: 'agent', status: 'completed', description: 'Review', summary: 'Finished', output_file: '/private', prompt: 'SECRET', usage: { total_tokens: 999 }, control: 'stop' });
      expect(projected).toEqual([{ kind: subtype, data: { task_id: 'task', tool_use_id: 'spawn', parent_tool_use_id: 'outer', agent_id: 'agent', status: 'completed', description: 'Review', summary: 'Finished' } }]);
    }
    expect(projection.project({ type: 'system', subtype: 'task_notification', task_id: 'task', status: 'failed', summary: 'SECRET' })[0].data.summary).toBe('engine_failed');
    expect(projection.project({ type: 'tool_progress', tool_use_id: 'tool', parent_tool_use_id: 'spawn', task_id: 'task', tool_name: 'Read', elapsed_time_seconds: 2 })[0].data).toMatchObject({ parent_tool_use_id: 'spawn', task_id: 'task', tool_use_id: 'tool' });
    const resource = projection.project({ type: 'system', subtype: 'task_notification', task_id: 'task', run_id: '001', tool_use_id: 'spawn', status: 'completed', resource_links: [{ uri: 'native://report', name: 'report', mimeType: 'text/plain', size: 12, _meta: { token: 'SECRET' }, annotations: { token: 'SECRET' } }] })[0];
    expect(resource.data).toMatchObject({ run_id: '001', resource_links: [{ uri: 'native://report', name: 'report', mimeType: 'text/plain', size: 12 }] });
    expect(JSON.stringify(resource)).not.toContain('SECRET');
  });
});
