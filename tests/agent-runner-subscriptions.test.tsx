import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useAgentRunner, type AgentOpenInput, type AgentSession } from '../src/stores/agentRunnerStore';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
const session = (id: string, overrides: Partial<AgentSession> = {}): AgentSession => ({ id, generation: `generation-${id}`, engine: 'codex', native_id: id, workspace: '/tmp/project', status: 'idle', events: [], pending: [], error: null, ...overrides });
const input: AgentOpenInput = { intent: 'create', id: 'old', engine: 'codex', task_id: 'task', project_revision: 1, provider: { provider_id: 'provider', model_id: 'model', provider_revision: 0 }, options: { workspace: '/tmp/project', resume_id: null, executable: null, model: null, trust_workspace: true } };
beforeEach(() => { vi.useFakeTimers(); useAgentRunner.getState().reset(); });
afterEach(() => { useAgentRunner.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });

it('polls background runtimes without selecting them or mixing their approvals and read failures into the foreground', async () => {
  let backgroundFails = false;
  let waiting = false;
  const pending = { id: 'background-approval', kind: 'approval', title: 'Background only', data: null };
  invoke.mockImplementation(async (_command: string, args: { sessionId: string }) => {
    if (args.sessionId === 'background' && backgroundFails) throw 'capability_revoked';
    return session(args.sessionId, args.sessionId === 'background' && waiting ? { status: 'waiting', pending: [pending] } : {});
  });
  await useAgentRunner.getState().attach('background');
  await useAgentRunner.getState().attach('foreground');
  useAgentRunner.getState().markSeen('background');
  useAgentRunner.getState().markSeen('foreground');
  waiting = true;
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().snapshots.background.pending).toEqual([pending]);
  expect(useAgentRunner.getState().unread).toEqual({ background: true, foreground: false });
  expect(useAgentRunner.getState().selected).toBe('foreground');
  expect(useAgentRunner.getState().session?.pending).toEqual([]);
  useAgentRunner.setState({ error: 'request_expired' });
  backgroundFails = true;
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().error).toBe('request_expired');
  expect(useAgentRunner.getState().session?.id).toBe('foreground');
  expect(useAgentRunner.getState().snapshots.background.pending).toEqual([]);
  expect(invoke.mock.calls.every(([command]) => command === 'get_agent_session')).toBe(true);
});

it('keeps read-only attach and unread acknowledgement independent of selection', async () => {
  let seq = 0;
  invoke.mockImplementation(async () => session('one', { events: seq ? [{ seq, kind: 'output', text: 'new output', data: null }] : [] }));
  await useAgentRunner.getState().attach('one');
  expect(useAgentRunner.getState().unread.one).toBe(true);
  useAgentRunner.getState().markSeen('one');
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().unread.one).toBe(false);
  seq = 1;
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().unread.one).toBe(true);
  useAgentRunner.getState().detach();
  await useAgentRunner.getState().attach('one');
  expect(useAgentRunner.getState().unread.one).toBe(true);
  expect(invoke.mock.calls.every(([command]) => command === 'get_agent_session')).toBe(true);
});

it('drops late old-account polls and cancels every runtime timer on reset', async () => {
  const late = Promise.withResolvers<AgentSession>();
  let firstReads = 0;
  invoke.mockImplementation(async (_command: string, args: { sessionId: string }) => args.sessionId === 'one' && ++firstReads > 1 ? late.promise : session(args.sessionId));
  await useAgentRunner.getState().attach('one');
  await useAgentRunner.getState().attach('two');
  await vi.advanceTimersByTimeAsync(600);
  useAgentRunner.getState().reset();
  late.resolve(session('one', { status: 'waiting', pending: [{ id: 'old-account', kind: 'approval', title: 'old', data: null }] }));
  await Promise.resolve(); await Promise.resolve();
  const calls = invoke.mock.calls.length;
  await vi.advanceTimersByTimeAsync(2400);
  expect(invoke.mock.calls.length).toBe(calls);
  expect(useAgentRunner.getState().snapshots).toEqual({});
  expect(useAgentRunner.getState().unread).toEqual({});
  expect(useAgentRunner.getState().live).toEqual({});
  expect(useAgentRunner.getState().session).toBeNull();
});

it('does not attach or retain a late old-account open', async () => {
  const late = Promise.withResolvers<AgentSession>();
  invoke.mockImplementation(async () => late.promise);
  const opening = useAgentRunner.getState().open(input);
  useAgentRunner.getState().reset();
  late.resolve(session('old', { status: 'waiting', pending: [{ id: 'old-approval', kind: 'approval', title: 'old', data: null }] }));
  await opening;
  await vi.advanceTimersByTimeAsync(1200);
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(useAgentRunner.getState().snapshots).toEqual({});
  expect(useAgentRunner.getState().unread).toEqual({});
  expect(useAgentRunner.getState().live).toEqual({});
  expect(useAgentRunner.getState().selected).toBeNull();
});

it('drops an old-account action failure without polling or clearing the new account projection', async () => {
  const receipt = Promise.withResolvers<null>();
  invoke.mockImplementation(async (command: string, args: { sessionId: string }) => command === 'send_agent_message' ? receipt.promise : session(args.sessionId));
  await useAgentRunner.getState().attach('old');
  const sending = useAgentRunner.getState().action('send_agent_message', { input: { session_id: 'old', generation: 'generation-old', request_id: 'old-send', text: 'once' } });
  useAgentRunner.getState().reset();
  await useAgentRunner.getState().attach('new');
  useAgentRunner.setState({ error: 'request_expired' });
  const reads = invoke.mock.calls.filter(([command]) => command === 'get_agent_session').length;
  receipt.reject('capability_revoked');
  expect(await sending).toBe(false);
  expect(invoke.mock.calls.filter(([command]) => command === 'get_agent_session')).toHaveLength(reads);
  expect(useAgentRunner.getState().selected).toBe('new');
  expect(useAgentRunner.getState().error).toBe('request_expired');
  expect(Object.keys(useAgentRunner.getState().snapshots)).toEqual(['new']);
});

it('preserves send single flight across detach/attach and never replays an unknown request receipt', async () => {
  const receipt = Promise.withResolvers<null>();
  invoke.mockImplementation(async (command: string) => command === 'send_agent_message' ? receipt.promise : session('one'));
  await useAgentRunner.getState().attach('one');
  const args = { input: { session_id: 'one', generation: 'generation-one', request_id: 'send-once', text: 'only once' } };
  const sending = useAgentRunner.getState().action('send_agent_message', args);
  useAgentRunner.getState().detach();
  await useAgentRunner.getState().attach('one');
  expect(useAgentRunner.getState().acting).toBe(true);
  expect(await useAgentRunner.getState().action('send_agent_message', args)).toBe(false);
  receipt.reject(new Error('unknown transport receipt'));
  expect(await sending).toBe(false);
  expect(await useAgentRunner.getState().action('send_agent_message', args)).toBe(false);
  expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
});

it('acknowledges a detached send and refreshes its target without changing foreground errors or approvals', async () => {
  const receipt = Promise.withResolvers<null>();
  const backgroundPending = { id: 'background-only', kind: 'approval', title: 'Background', data: null };
  let waiting = false;
  invoke.mockImplementation(async (command: string, args: { sessionId: string }) => command === 'send_agent_message' ? receipt.promise : session(args.sessionId, args.sessionId === 'old' && waiting ? { status: 'waiting', pending: [backgroundPending] } : {}));
  await useAgentRunner.getState().attach('old');
  const sending = useAgentRunner.getState().action('send_agent_message', { input: { session_id: 'old', generation: 'generation-old', request_id: 'send', text: 'once' } });
  await useAgentRunner.getState().attach('new');
  useAgentRunner.setState({ error: 'request_expired' });
  waiting = true;
  receipt.resolve(null);
  expect(await sending).toBe(true);
  expect(useAgentRunner.getState().snapshots.old.pending).toEqual([backgroundPending]);
  expect(useAgentRunner.getState().session?.id).toBe('new');
  expect(useAgentRunner.getState().session?.pending).toEqual([]);
  expect(useAgentRunner.getState().error).toBe('request_expired');
});

it('shows cached transcript without cached approval controls and rejects responses until attach refresh completes', async () => {
  const fresh = Promise.withResolvers<AgentSession>();
  const pending = { id: 'cached-request', kind: 'approval', title: 'Cached approval', data: null };
  const cached = session('one', { status: 'waiting', pending: [pending], events: [{ seq: 1, kind: 'output', text: 'Cached transcript', data: null }] });
  useAgentRunner.setState({ snapshots: { one: cached }, live: { one: 'waiting' } });
  invoke.mockImplementation(async () => fresh.promise);
  const attaching = useAgentRunner.getState().attach('one');
  expect(useAgentRunner.getState().busy).toBe(true);
  expect(useAgentRunner.getState().session?.events).toEqual(cached.events);
  expect(useAgentRunner.getState().session?.pending).toEqual([]);
  expect(useAgentRunner.getState().snapshots.one.pending).toEqual([pending]);
  const args = { input: { session_id: 'one', generation: 'generation-one', request_id: pending.id, response: { decision: 'accept' } } };
  expect(await useAgentRunner.getState().action('respond_agent_request', args)).toBe(false);
  // Even a retained callback with an exposed request cannot bypass the attach boundary.
  useAgentRunner.setState({ session: cached });
  expect(await useAgentRunner.getState().action('respond_agent_request', args)).toBe(false);
  fresh.resolve(session('one', { events: cached.events }));
  await attaching;
  expect(useAgentRunner.getState().busy).toBe(false);
  expect(useAgentRunner.getState().session?.pending).toEqual([]);
  expect(await useAgentRunner.getState().action('respond_agent_request', args)).toBe(false);
  expect(invoke.mock.calls.every(([command]) => command === 'get_agent_session')).toBe(true);
});

it('keeps a detached action failure out of the newly selected projection', async () => {
  const receipt = Promise.withResolvers<null>();
  invoke.mockImplementation(async (command: string, args: { sessionId: string }) => command === 'send_agent_message' ? receipt.promise : session(args.sessionId));
  await useAgentRunner.getState().attach('old');
  const sending = useAgentRunner.getState().action('send_agent_message', { input: { session_id: 'old', generation: 'generation-old', request_id: 'old-send', text: 'once' } });
  useAgentRunner.getState().detach();
  await useAgentRunner.getState().attach('new');
  useAgentRunner.setState({ error: 'request_expired' });
  receipt.reject('capability_revoked');
  expect(await sending).toBe(false);
  expect(useAgentRunner.getState().selected).toBe('new');
  expect(useAgentRunner.getState().session?.pending).toEqual([]);
  expect(useAgentRunner.getState().error).toBe('request_expired');
});

it('blocks fresh send identities until the original unknown send has matching admission evidence', async () => {
 let evidenced = false;
 invoke.mockImplementation(async (command: string) => {
  if (command === 'send_agent_message') throw new Error('lost receipt');
  return session('one', { events: evidenced ? [{ seq: 1, kind: 'message', text: 'admitted', data: null, run: 'original' }] : [] });
 });
 await useAgentRunner.getState().attach('one');
 const input = { session_id: 'one', generation: 'generation-one', request_id: 'original', text: 'once' };
 expect(await useAgentRunner.getState().action('send_agent_message', { input })).toBe(false);
 expect(await useAgentRunner.getState().action('send_agent_message', { input: { ...input, request_id: 'fresh' } })).toBe(false);
 expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
 evidenced = true; await useAgentRunner.getState().attach('one');
 invoke.mockImplementation(async (command: string) => command === 'send_agent_message' ? null : session('one', { events: [{ seq: 1, kind: 'message', text: 'admitted', data: null, run: 'original' }] }));
 expect(await useAgentRunner.getState().action('send_agent_message', { input: { ...input, request_id: 'next', text: 'next turn' } })).toBe(true);
 expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(2);
});
