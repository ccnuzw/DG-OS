import { afterEach, expect, it, vi } from 'vitest';
import { searchDevelopmentMessages } from '../src/stores/developmentExplorer';
import type { DevelopmentSession } from '../src/stores/developmentStore';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
afterEach(() => { invoke.mockReset(); });
const session: DevelopmentSession = { id: 'session', owner: 'alice', engine: 'codex', native_id: null, workspace: '/work', executable: null, model: null, project_id: 'project', task_id: 'task' };

it('stops queued task reads when cancelled without pretending to cancel in-flight host commands', async () => {
  const pending = Promise.withResolvers<unknown>();
  invoke.mockReturnValue(pending.promise);
  const sessions = Array.from({ length: 8 }, (_, index) => ({ ...session, id: `s${index}`, task_id: `t${index}` }));
  const controller = new AbortController();
  const search = searchDevelopmentMessages(sessions, 'needle', controller.signal);
  expect(invoke).toHaveBeenCalledTimes(3);
  controller.abort();
  pending.resolve([]);
  const result = await search;
  expect(invoke).toHaveBeenCalledTimes(3);
  expect(result.matches).toEqual([]);
});

it('reports failed and unlinked history while retaining available dialogue matches', async () => {
  invoke.mockImplementation(async (_command: string, { taskId }: { taskId: string }) => {
    if (taskId === 'failed') throw 'history_missing';
    return [{ id: 'run', session_id: 'session', engine: 'codex', truncated: true, events: [
      { seq: 1, kind: 'userMessage', text: 'needle request', data: null, run: 'run' },
      { seq: 2, kind: 'agentMessage', text: 'needle reply', data: null, run: 'run' },
    ] }];
  });
  const result = await searchDevelopmentMessages([session, { ...session, id: 'unlinked', task_id: null }, { ...session, id: 'failed', task_id: 'failed' }], 'needle', new AbortController().signal);
  expect(result).toEqual({ matches: [{ sessionId: 'session', runId: 'run', seq: 1, text: 'needle request' }], truncated: true, unavailable: 2 });
});

it('never searches tool, reasoning, host runs or unrelated session evidence as messages', async () => {
  invoke.mockResolvedValue([
    { id: 'host', session_id: 'session', engine: 'host', truncated: false, events: [{ seq: 1, kind: 'userMessage', text: 'needle host', data: null, run: 'host' }] },
    { id: 'other', session_id: 'other-session', engine: 'codex', truncated: false, events: [{ seq: 2, kind: 'agentMessage', text: 'needle other', data: null, run: 'other' }] },
    { id: 'native', session_id: 'session', engine: 'claude', truncated: false, summary: 'needle summary', events: [
      { seq: 3, kind: 'reasoning', text: 'needle thinking', data: null, run: 'native' },
      { seq: 4, kind: 'message', text: null, data: { type: 'assistant', content: [{ type: 'tool_use', name: 'needle tool' }, { type: 'thinking', thinking: 'needle reasoning' }] }, run: 'native' },
      { seq: 5, kind: 'message', text: null, data: { type: 'user', content: [{ type: 'tool_result', content: 'needle tool result' }] }, run: 'native' },
    ] },
  ]);
  const result = await searchDevelopmentMessages([session], 'needle', new AbortController().signal);
  expect(result.matches).toEqual([]);
});
