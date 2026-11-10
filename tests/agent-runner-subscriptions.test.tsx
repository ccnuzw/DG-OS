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

it('appends each event once, drops evicted history and refreshes approvals without transcript output', async () => {
  const event = (seq: number) => ({ seq, kind: 'output', text: `message-${seq}`, data: null });
  const pending = { id: 'native-request', kind: 'approval', title: 'Approve command', data: null };
  invoke.mockResolvedValueOnce(session('one', { events: [event(1), event(2)] }))
    .mockResolvedValueOnce(session('one', { events: [event(3)], event_sync: { reset: false, after_seq: 2, first_seq: 2 } }))
    .mockResolvedValueOnce(session('one', { events: [], status: 'waiting', pending: [pending], event_sync: { reset: false, after_seq: 3, first_seq: 2 } }));
  await useAgentRunner.getState().attach('one');
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().session?.events.map(row => row.seq)).toEqual([2, 3]);
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().session?.events.map(row => row.text)).toEqual(['message-2', 'message-3']);
  expect(useAgentRunner.getState().session?.pending).toEqual([pending]);
  expect(invoke.mock.calls[2][1]).toEqual({ sessionId: 'one', cursor: { generation: 'generation-one', after_seq: 3 } });
});

it('recovers an event gap through a read-only full snapshot rather than displaying partial history', async () => {
  const event = (seq: number) => ({ seq, kind: 'output', text: `message-${seq}`, data: null });
  invoke.mockResolvedValueOnce(session('one', { events: [event(1)] }))
    .mockResolvedValueOnce(session('one', { events: [event(3)], event_sync: { reset: false, after_seq: 1, first_seq: 1 } }))
    .mockResolvedValueOnce(session('one', { events: [event(1), event(2), event(3)], event_sync: { reset: true, after_seq: 0, first_seq: 1 } }));
  await useAgentRunner.getState().attach('one');
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().session?.events.map(row => row.seq)).toEqual([1, 2, 3]);
  expect(invoke.mock.calls.at(-1)).toEqual(['get_agent_session', { sessionId: 'one' }]);
  expect(invoke.mock.calls.every(([command]) => command === 'get_agent_session')).toBe(true);
});

it('replaces a restored generation without retaining old messages or approvals', async () => {
  const old = { seq: 1, kind: 'output', text: 'old runtime', data: null };
  const fresh = { seq: 1, kind: 'output', text: 'restored history', data: null };
  invoke.mockResolvedValueOnce(session('one', { events: [old], pending: [{ id: 'old', kind: 'approval', title: 'Old', data: null }] }))
    .mockResolvedValueOnce(session('one', { generation: 'restored', events: [fresh], event_sync: { reset: true, after_seq: 0, first_seq: 1 } }));
  await useAgentRunner.getState().attach('one');
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().session?.generation).toBe('restored');
  expect(useAgentRunner.getState().session?.events).toEqual([fresh]);
  expect(useAgentRunner.getState().session?.pending).toEqual([]);
});

it('keeps interrupt pending beyond its acknowledgement and across detach until native terminal observation', async () => {
  let native = session('one', { status: 'running', active_run: 'first' });
  invoke.mockImplementation(async (command: string) => command === 'interrupt_agent_session' ? null : native);
  await useAgentRunner.getState().attach('one');
  const args = { sessionId: 'one', generation: native.generation, runId: 'first' };
  expect(await useAgentRunner.getState().action('interrupt_agent_session', args)).toBe(true);
  expect(useAgentRunner.getState().session?.status).toBe('running');
  expect(useAgentRunner.getState().interrupts.one).toEqual({ generation: native.generation, runId: 'first' });
  useAgentRunner.getState().detach();
  await useAgentRunner.getState().attach('one');
  expect(await useAgentRunner.getState().action('interrupt_agent_session', args)).toBe(false);
  expect(invoke.mock.calls.filter(([name]) => name === 'interrupt_agent_session')).toHaveLength(1);
  native = { ...native, status: 'idle', active_run: null, events: [{ seq: 1, kind: 'turn/completed', text: null, data: { run_status: 'cancelled' }, run: 'first' }] };
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().interrupts.one).toBeUndefined();
  expect(useAgentRunner.getState().session?.status).toBe('idle');
  native = { ...native, status: 'running', active_run: 'second' };
  await useAgentRunner.getState().attach('one');
  expect(await useAgentRunner.getState().action('interrupt_agent_session', args)).toBe(false);
  expect(await useAgentRunner.getState().action('interrupt_agent_session', { ...args, runId: 'second' })).toBe(true);
  expect(invoke.mock.calls.filter(([name]) => name === 'interrupt_agent_session')).toHaveLength(2);
});

it('isolates late interrupt failure from a replacement runtime and rejects old generation and run callbacks', async () => {
  const oldReceipt = Promise.withResolvers<null>();
  let native = session('one', { status: 'running', active_run: 'reused-run' });
  let interrupts = 0;
  invoke.mockImplementation(async (command: string) => command === 'interrupt_agent_session' ? ++interrupts === 1 ? oldReceipt.promise : null : native);
  await useAgentRunner.getState().attach('one');
  const stale = { sessionId: 'one', generation: native.generation, runId: 'reused-run' };
  const stopping = useAgentRunner.getState().action('interrupt_agent_session', stale);
  native = { ...native, generation: 'replacement' };
  await useAgentRunner.getState().attach('one');
  expect(useAgentRunner.getState().interrupts.one).toBeUndefined();
  expect(await useAgentRunner.getState().action('interrupt_agent_session', stale)).toBe(false);
  expect(await useAgentRunner.getState().action('interrupt_agent_session', { ...stale, generation: native.generation, runId: 'old-run' })).toBe(false);
  expect(await useAgentRunner.getState().action('interrupt_agent_session', { ...stale, generation: native.generation })).toBe(true);
  useAgentRunner.setState({ error: 'request_expired' });
  const reads = invoke.mock.calls.filter(([name]) => name === 'get_agent_session').length;
  oldReceipt.reject('bridge_unavailable');
  expect(await stopping).toBe(false);
  expect(invoke.mock.calls.filter(([name]) => name === 'get_agent_session')).toHaveLength(reads);
  expect(useAgentRunner.getState().error).toBe('request_expired');
  expect(useAgentRunner.getState().interrupts.one).toEqual({ generation: 'replacement', runId: 'reused-run' });
  expect(useAgentRunner.getState().session?.status).toBe('running');
});

it('retains uncertain interrupt admission without replay until native run evidence changes', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'interrupt_agent_session') throw new Error('lost acknowledgement');
    return session('one', { status: 'running', active_run: 'run' });
  });
  await useAgentRunner.getState().attach('one');
  const args = { sessionId: 'one', generation: 'generation-one', runId: 'run' };
  expect(await useAgentRunner.getState().action('interrupt_agent_session', args)).toBe(false);
  expect(await useAgentRunner.getState().action('interrupt_agent_session', args)).toBe(false);
  expect(useAgentRunner.getState().interrupts.one).toEqual({ generation: 'generation-one', runId: 'run' });
  expect(useAgentRunner.getState().session?.status).toBe('running');
  expect(invoke.mock.calls.filter(([name]) => name === 'interrupt_agent_session')).toHaveLength(1);
});

it('observes storage degradation independently of native status and blocks only new sends until recovery', async () => {
  let native = session('one', { status: 'running', active_run: 'run' });
  invoke.mockImplementation(async (command: string) => command === 'interrupt_agent_session' || command === 'send_agent_message' ? null : command === 'close_agent_session' ? true : native);
  await useAgentRunner.getState().attach('one');
  useAgentRunner.getState().markSeen('one');
  native = { ...native, persistence_error: 'storage_failed' };
  await vi.advanceTimersByTimeAsync(150);
  expect(useAgentRunner.getState().unread.one).toBe(true);
  expect(useAgentRunner.getState().session).toMatchObject({ status: 'running', error: null, persistence_error: 'storage_failed' });
  expect(await useAgentRunner.getState().action('interrupt_agent_session', { sessionId: 'one', generation: native.generation, runId: 'run' })).toBe(true);
  native = { ...native, status: 'idle', active_run: null };
  await useAgentRunner.getState().attach('one');
  const failure = vi.fn();
  const args = { input: { session_id: 'one', generation: native.generation, request_id: 'next', text: 'Keep this draft' } };
  expect(await useAgentRunner.getState().action('send_agent_message', args, failure)).toBe(false);
  expect(failure).toHaveBeenCalledWith('storage_failed');
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message')).toBe(false);
  native = { ...native, persistence_error: null };
  await useAgentRunner.getState().attach('one');
  expect(await useAgentRunner.getState().action('send_agent_message', args)).toBe(true);
  native = { ...native, persistence_error: 'storage_failed' };
  await useAgentRunner.getState().attach('one');
  expect(await useAgentRunner.getState().action('close_agent_session', { sessionId: 'one' })).toBe(true);
  expect(useAgentRunner.getState().session?.status).toBe('closed');
});
