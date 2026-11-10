import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import CodexOperations from '../src/components/development/CodexOperations';
import { codexOperationsKey, useCodexOperations } from '../src/stores/codexOperations';
import { useDevelopment, type DevelopmentSession, type DevelopmentTask, type Project, type ProjectOperation, type Selection } from '../src/stores/developmentStore';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import type { CodexOperationAction, CodexOperationRequest, CodexOperationScope, CodexOperationsSnapshot } from '../src/components/development/codexOperationsService';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ t: (key: string, values?: Record<string, string>) => values ? `${key}:${Object.values(values).join(',')}` : key, useLocale: () => ({ locale: 'en-US' }) }));
const session: AgentSession = { id: 'session-a', generation: 'generation-a', engine: 'codex', native_id: 'thread-a', workspace: '/workspace', status: 'ready', events: [], pending: [], error: null };
// First-account hosts keep default-user as the native owner; the UI account key is account:<username>.
const workspace = {
  projects: [{ id: 'project-a', owner: 'default-user', name: 'Project A', path: '/workspace', directory_identity: 'directory-a', archived: false, created_at: 1, last_opened_at: null }],
  sessions: [{ id: session.id, owner: 'default-user', engine: 'codex', native_id: 'thread-a', workspace: '/workspace', executable: null, model: null, project_id: 'project-a', task_id: 'task-a' }],
  tasks: [{ id: 'task-a', owner: 'default-user', project_id: 'project-a', goal: 'Task', requirements: '', status: 'in_progress', created_at: 1 }],
  selection: { project_id: 'project-a', task_id: 'task-a', session_id: session.id },
  operations: [],
} satisfies { projects: Project[]; sessions: DevelopmentSession[]; tasks: DevelopmentTask[]; selection: Selection; operations: ProjectOperation[] };
let scope: CodexOperationScope;
let snapshot: CodexOperationsSnapshot;
const controlCalls = () => invoke.mock.calls.filter(([command]) => command === 'control_codex_operation');
const entry = () => useCodexOperations.getState().entries[codexOperationsKey(scope)];
beforeEach(() => {
  invoke.mockReset();
  useAgentRunner.setState({ session, snapshots: { [session.id]: session }, selected: session.id, live: {} });
  useDevelopment.setState(state => ({ epoch: state.epoch + 1, workspaceOwner: 'account:alice', loaded: true, ...structuredClone(workspace) }));
  useCodexOperations.getState().reset();
  scope = { epoch: useDevelopment.getState().epoch, accountKey: 'account:alice', owner: 'default-user', projectId: 'project-a', taskId: 'task-a', sessionId: session.id, generation: session.generation };
  snapshot = { session_id: session.id, generation: session.generation, children: [{ thread_id: 'child-a', parent_thread_id: 'thread-a', status: 'running', turn_id: 'child-turn-a' }], terminals: [{ process_id: 'process-a', command: 'sleep 10', status: 'running' }], queue: [{ id: 'queue-a', text: 'First' }, { id: 'queue-b', text: 'Second' }], turns: [{ id: 'turn-a', status: 'completed', items: [] }], unsupported: [], incomplete: false };
  invoke.mockImplementation(async (command: string, args: { input?: CodexOperationRequest }) => {
    if (command === 'read_development_workspace') return structuredClone(workspace);
    if (command === 'observe_codex_operations') return structuredClone(snapshot);
    if (command === 'control_codex_operation') return { session_id: session.id, generation: session.generation, request_id: args.input!.request_id, state: 'applied' };
    if (command === 'read_codex_child_history') return { thread_id: 'child-a', turns: [{ id: 'child-turn-a', status: 'completed', items: [{ id: 'message-a', type: 'agentMessage', text: 'Native child response' }] }], incomplete: false };
    if (command === 'get_agent_session') return session;
    throw 'unexpected_command';
  });
});
afterEach(cleanup);
async function openPanel(props: Partial<Parameters<typeof CodexOperations>[0]> = {}) {
  const view = render(<CodexOperations session={session} {...props} />);
  await act(async () => { const panel = screen.getByTestId('codex-operations') as HTMLDetailsElement; panel.open = true; fireEvent(panel, new Event('toggle')); });
  // Explicit observation is the same production button path; it does not start a run.
  if (!entry()?.snapshot) await act(async () => { fireEvent.click(screen.getByTestId('codex-operations-refresh')); });
  await waitFor(() => expect(entry()?.snapshot?.generation).toBe(session.generation));
  return view;
}

describe('Codex operations ownership and uncertainty', () => {
  it('mounts with host-owned metadata loaded through the production signed-in workspace path', async () => {
    useDevelopment.getState().reset();
    expect(await useDevelopment.getState().load('alice')).toBe(true);
    scope = { ...scope, epoch: useDevelopment.getState().epoch };
    expect(useDevelopment.getState().workspaceOwner).toBe('account:alice');
    expect(useDevelopment.getState().sessions[0].owner).toBe('default-user');
    await openPanel();
    expect(screen.getByTestId('codex-operations').getAttribute('data-scope-ready')).toBe('true');
    expect(screen.getByTestId('codex-operations-refresh').hasAttribute('disabled')).toBe(false);
    expect(screen.getByTestId('codex-compact').hasAttribute('disabled')).toBe(false);
    const observation = invoke.mock.calls.find(([command]) => command === 'observe_codex_operations')!;
    expect(observation[1]).toEqual({ input: { session_id: session.id, generation: session.generation } });
    expect(JSON.stringify(observation[1])).not.toContain('account:alice');
    expect(JSON.stringify(observation[1])).not.toContain('default-user');
  });
  it('rejects mismatched project and session host owners despite a valid UI account key', async () => {
    useDevelopment.setState({ projects: [{ ...workspace.projects[0], owner: 'foreign-host-owner' }] });
    render(<CodexOperations session={session} />);
    expect(screen.getByTestId('codex-operations-unavailable').getAttribute('data-scope-ready')).toBe('false');
    await useCodexOperations.getState().observe(scope);
    expect(invoke.mock.calls.some(([command]) => command === 'observe_codex_operations')).toBe(false);
  });
  it('drops a late observation across account replacement without exposing private state', async () => {
    const deferred = Promise.withResolvers<CodexOperationsSnapshot>();
    invoke.mockReturnValue(deferred.promise);
    const read = useCodexOperations.getState().observe(scope);
    useDevelopment.setState(state => ({ epoch: state.epoch + 1, workspaceOwner: 'bob' }));
    deferred.resolve({ ...snapshot, queue: [{ id: 'private', text: 'Alice private message' }] });
    await read;
    expect(useCodexOperations.getState().entries).toEqual({});
  });
  it('fails closed for malformed native snapshot envelopes', async () => {
    invoke.mockResolvedValue({ ...snapshot, session_id: 'foreign-session', queue: [{ id: 'private', text: 'Foreign private queue' }] });
    await useCodexOperations.getState().observe(scope);
    expect(entry().snapshot).toBeNull();
    expect(entry().error).toBe('unavailable');
  });
  it('releases a lost request when inspection proves a pre-dispatch refusal, without clearing its draft', async () => {
    await useCodexOperations.getState().observe(scope);
    useCodexOperations.getState().draft(scope, 'queue:new', 'Keep draft');
    invoke.mockRejectedValue('transport_lost');
    await useCodexOperations.getState().act(scope, { kind: 'queue_add', text: 'Keep draft' });
    invoke.mockRejectedValue('codex_queue_membership_changed');
    await useCodexOperations.getState().inspect(scope);
    expect(entry().pending).toBeNull();
    expect(entry().drafts['queue:new']).toBe('Keep draft');
    expect(entry().error).toBe('codex_queue_membership_changed');
    expect(controlCalls()).toHaveLength(1);
  });
  it.each(['__proto__', 'constructor', 'toString'])('does not mistake inherited object keys for proved refusals: %s', async error => {
    await useCodexOperations.getState().observe(scope);
    invoke.mockRejectedValue(error);
    await useCodexOperations.getState().act(scope, { kind: 'compact' });
    expect(entry().pending).not.toBeNull();
    expect(entry().error).toBe('unknown');
  });
  it('rejects a late receipt from a replaced runtime generation', async () => {
    await useCodexOperations.getState().observe(scope);
    const deferred = Promise.withResolvers<unknown>();
    invoke.mockImplementation((command: string) => command === 'control_codex_operation' ? deferred.promise : Promise.resolve(snapshot));
    const flight = useCodexOperations.getState().act(scope, { kind: 'compact' });
    const request = entry().pending!;
    useAgentRunner.setState({ session: { ...session, generation: 'generation-b' } });
    deferred.resolve({ session_id: session.id, generation: session.generation, request_id: request.request_id, state: 'applied' });
    expect(await flight).toBeNull();
    expect(entry().receipt).toBeNull();
  });
  it('retains unknown identity across remount and inspects without replay', async () => {
    let original: CodexOperationRequest;
    invoke.mockImplementation(async (command: string, args: { input: CodexOperationRequest }) => {
      if (command === 'observe_codex_operations') return snapshot;
      if (command === 'control_codex_operation') { original = args.input; throw 'transport_lost'; }
      if (command === 'inspect_codex_operation') return { session_id: session.id, generation: session.generation, request_id: original.request_id, state: 'applied' };
      throw 'unexpected_command';
    });
    const view = await openPanel();
    await act(async () => { fireEvent.click(screen.getByTestId('codex-compact')); });
    expect(entry().pending?.action.kind).toBe('compact');
    view.unmount(); await openPanel();
    expect(screen.getByTestId('codex-compact').hasAttribute('disabled')).toBe(true);
    await act(async () => { fireEvent.click(screen.getByTestId('codex-operation-inspect')); });
    expect(entry().pending).toBeNull();
    expect(controlCalls()).toHaveLength(1);
    const inspection = invoke.mock.calls.find(([command]) => command === 'inspect_codex_operation')!;
    expect(inspection[1]).toEqual({ input: { session_id: session.id, generation: session.generation, request_id: original!.request_id } });
  });
  it('does not unblock a malformed inspection receipt or leak error details', async () => {
    await useCodexOperations.getState().observe(scope);
    invoke.mockRejectedValue('secret-native-error-content');
    await useCodexOperations.getState().act(scope, { kind: 'compact' });
    const original = entry().pending;
    invoke.mockResolvedValue({ session_id: session.id, generation: 'other-generation', request_id: original!.request_id, state: 'applied' });
    await useCodexOperations.getState().inspect(scope);
    expect(entry().pending).toBe(original);
    expect(entry().error).toBe('unknown');
    expect(JSON.stringify(entry())).not.toContain('secret-native-error-content');
  });
  it.each(['session_busy', 'codex_operation_unsupported', 'engine_rejected', 'method_unsupported'])('clears only a proved refusal (%s) without auto-retry', async error => {
    await useCodexOperations.getState().observe(scope);
    invoke.mockRejectedValue(error);
    await useCodexOperations.getState().act(scope, { kind: 'compact' });
    expect(entry().pending).toBeNull();
    expect(entry().error).toBe(error);
    expect(controlCalls()).toHaveLength(1);
  });
  it.each<CodexOperationAction>([
    { kind: 'send_child', child_thread_id: 'foreign-thread', text: 'Hello' },
    { kind: 'interrupt_child', child_thread_id: 'child-a', turn_id: 'old-turn' },
    { kind: 'terminate_terminal', process_id: 'host-pty-or-foreign-process' },
    { kind: 'queue_update', submission_id: 'foreign-message', text: 'Changed' },
    { kind: 'queue_reorder', submission_ids: ['queue-a', 'queue-a'] },
    { kind: 'queue_reorder', submission_ids: ['queue-b'] },
    { kind: 'revert', before_turn_id: 'foreign-turn' },
  ])('never dispatches resources absent from the observed scope: $kind', async action => {
    await useCodexOperations.getState().observe(scope);
    expect(await useCodexOperations.getState().act(scope, action)).toBeNull();
    expect(controlCalls()).toHaveLength(0);
    expect(entry().error).toBe('invalid_codex_operation');
  });
  it('prevents child history races from replacing post-mutation state', async () => {
    await useCodexOperations.getState().observe(scope);
    const deferred = Promise.withResolvers<unknown>();
    invoke.mockImplementation(async (command: string, args: { input: CodexOperationRequest }) => {
      if (command === 'read_codex_child_history') return deferred.promise;
      if (command === 'control_codex_operation') return { session_id: session.id, generation: session.generation, request_id: args.input.request_id, state: 'applied' };
      return snapshot;
    });
    const read = useCodexOperations.getState().history(scope, 'child-a');
    await useCodexOperations.getState().act(scope, { kind: 'compact' });
    deferred.resolve({ thread_id: 'child-a', turns: [{ id: 'stale-private-turn' }], incomplete: false });
    await read;
    expect(entry().histories).toEqual({});
  });
});

describe('Codex native operations controls', () => {
  it('keeps reads separate from review execution and sends the explicit native review target', async () => {
    await openPanel();
    expect(controlCalls()).toHaveLength(0);
    fireEvent.click(screen.getByTestId('codex-review-open'));
    expect(screen.getByRole('dialog').textContent).toContain('codexOperations.reviewNotice');
    await act(async () => { fireEvent.click(screen.getByTestId('codex-review-start')); });
    expect(controlCalls()).toHaveLength(1);
    expect(controlCalls()[0][1].input.action).toEqual({ kind: 'review', target: { type: 'uncommittedChanges' } });
    expect(screen.getByRole('status').textContent).toContain('codexOperations.accepted');
  });
  it('requires explicit revert confirmation and sends only an observed turn identity', async () => {
    await openPanel();
    fireEvent.click(screen.getByTestId('codex-revert-open'));
    expect(screen.getByRole('dialog').textContent).toContain('codexOperations.revertNotice');
    expect(screen.getByTestId('codex-revert-confirm').hasAttribute('disabled')).toBe(true);
    fireEvent.click(screen.getByRole('combobox', { name: 'codexOperations.beforeTurn' }));
    fireEvent.click(await screen.findByRole('option', { name: 'turn-a · completed' }));
    expect(controlCalls()).toHaveLength(0);
    await act(async () => { fireEvent.click(screen.getByTestId('codex-revert-confirm')); });
    expect(controlCalls()[0][1].input.action).toEqual({ kind: 'revert', before_turn_id: 'turn-a' });
  });
  it('leaves source selected after fork and navigates only through the explicit parent callback', async () => {
    const open = vi.fn();
    invoke.mockImplementation(async (command: string, args: { input: CodexOperationRequest }) => command === 'control_codex_operation' ? { session_id: session.id, generation: session.generation, request_id: args.input.request_id, state: 'applied', fork_session_id: 'fork-session' } : snapshot);
    await openPanel({ onOpenSession: open });
    fireEvent.click(screen.getByTestId('codex-fork'));
    expect(controlCalls()).toHaveLength(0);
    await act(async () => { fireEvent.click(screen.getByTestId('codex-fork-confirm')); });
    expect(open).not.toHaveBeenCalled();
    expect(useAgentRunner.getState().selected).toBe(session.id);
    fireEvent.click(screen.getByTestId('codex-fork-open'));
    expect(open).toHaveBeenCalledWith('fork-session');
    expect(invoke.mock.calls.some(([command]) => command === 'open_agent_session')).toBe(false);
  });
  it('reorders the full native queue with keyboard-operable buttons and starts the selected entry', async () => {
    await openPanel();
    const row = screen.getByText('1. First').closest('li')!;
    await act(async () => { fireEvent.click(within(row).getByTestId('codex-queue-down')); });
    expect(controlCalls()[0][1].input.action).toEqual({ kind: 'queue_reorder', submission_ids: ['queue-b', 'queue-a'] });
    const second = screen.getByText('2. Second').closest('li')!;
    await act(async () => { fireEvent.click(within(second).getByTestId('codex-queue-start')); });
    expect(controlCalls()[1][1].input.action).toEqual({ kind: 'queue_start', submission_id: 'queue-b' });
  });
  it('preserves a queue draft edited after dispatch instead of clearing newer text', async () => {
    await openPanel();
    const deferred = Promise.withResolvers<unknown>();
    let request: CodexOperationRequest;
    invoke.mockImplementation((command: string, args: { input: CodexOperationRequest }) => { if (command === 'control_codex_operation') { request = args.input; return deferred.promise; } return Promise.resolve(snapshot); });
    fireEvent.change(screen.getByTestId('codex-queue-text'), { target: { value: 'Original text' } });
    fireEvent.click(screen.getByTestId('codex-queue-add'));
    fireEvent.change(screen.getByTestId('codex-queue-text'), { target: { value: 'Newer draft' } });
    await act(async () => { deferred.resolve({ session_id: session.id, generation: session.generation, request_id: request!.request_id, state: 'applied' }); });
    expect((screen.getByTestId('codex-queue-text') as HTMLTextAreaElement).value).toBe('Newer draft');
    expect(request!.action).toEqual({ kind: 'queue_add', text: 'Original text' });
  });
  it('reads selected child history without switching main session and binds interrupt to its current turn', async () => {
    await openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'child-a' }));
    await act(async () => { fireEvent.click(screen.getByTestId('codex-agent-history')); });
    expect(screen.getByText('Native child response')).toBeTruthy();
    expect(useAgentRunner.getState().selected).toBe(session.id);
    await act(async () => { fireEvent.click(screen.getByTestId('codex-agent-interrupt')); });
    expect(controlCalls()[0][1].input.action).toEqual({ kind: 'interrupt_child', child_thread_id: 'child-a', turn_id: 'child-turn-a' });
  });
  it('stops only an observed native process, never a host terminal', async () => {
    await openPanel();
    await act(async () => { fireEvent.click(screen.getByTestId('codex-background-stop')); });
    expect(controlCalls()[0][1].input.action).toEqual({ kind: 'terminate_terminal', process_id: 'process-a' });
  });
  it('disables only unsupported actions while leaving independent operations available', async () => {
    snapshot.unsupported = ['close_child', 'queue_start'];
    await openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'child-a' }));
    expect(screen.getByTestId('codex-agent-close').hasAttribute('disabled')).toBe(true);
    expect(screen.getByTestId('codex-agent-interrupt').hasAttribute('disabled')).toBe(false);
    expect(screen.getByTestId('codex-compact').hasAttribute('disabled')).toBe(false);
    expect(screen.getAllByTestId('codex-queue-start').every(button => button.hasAttribute('disabled'))).toBe(true);
  });
  it('edits and deletes the exact native queue entry rather than sending a conversation turn', async () => {
    await openPanel();
    let row = screen.getByText('1. First').closest('li')!;
    fireEvent.click(within(row).getByTestId('codex-queue-edit'));
    fireEvent.change(within(row).getByTestId('codex-queue-edit-text'), { target: { value: 'Edited first' } });
    await act(async () => { fireEvent.click(within(row).getByTestId('codex-queue-save')); });
    expect(controlCalls()[0][1].input.action).toEqual({ kind: 'queue_update', submission_id: 'queue-a', text: 'Edited first' });
    row = screen.getByText('2. Second').closest('li')!;
    await act(async () => { fireEvent.click(within(row).getByTestId('codex-queue-delete')); });
    expect(controlCalls()[1][1].input.action).toEqual({ kind: 'queue_delete', submission_id: 'queue-b' });
    expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
  });
  it('keeps an unknown queue draft until the original request is proven applied', async () => {
    let request: CodexOperationRequest;
    invoke.mockImplementation(async (command: string, args: { input: CodexOperationRequest }) => {
      if (command === 'observe_codex_operations') return snapshot;
      if (command === 'control_codex_operation') { request = args.input; return { session_id: session.id, generation: session.generation, request_id: request.request_id, state: 'unknown' }; }
      if (command === 'inspect_codex_operation') return { session_id: session.id, generation: session.generation, request_id: request!.request_id, state: 'applied' };
      throw 'unexpected_command';
    });
    await openPanel();
    fireEvent.change(screen.getByTestId('codex-queue-text'), { target: { value: 'Retain until known' } });
    await act(async () => { fireEvent.click(screen.getByTestId('codex-queue-add')); });
    expect((screen.getByTestId('codex-queue-text') as HTMLTextAreaElement).value).toBe('Retain until known');
    await act(async () => { fireEvent.click(screen.getByTestId('codex-operation-inspect')); });
    expect((screen.getByTestId('codex-queue-text') as HTMLTextAreaElement).value).toBe('');
    expect(controlCalls()).toHaveLength(1);
  });
  it('hides generation-bound draft and observations when the same session is replaced', async () => {
    const view = await openPanel();
    fireEvent.change(screen.getByTestId('codex-queue-text'), { target: { value: 'Old generation draft' } });
    view.rerender(<CodexOperations session={{ ...session, generation: 'generation-b' }} />);
    expect((screen.getByTestId('codex-queue-text') as HTMLTextAreaElement).value).toBe('');
    expect(screen.queryByText('1. First')).toBeNull();
    expect(screen.getByTestId('codex-compact').hasAttribute('disabled')).toBe(true);
  });
  it.each<{ draftId: string; action: CodexOperationAction }>([
    { draftId: 'queue:new', action: { kind: 'queue_add', text: 'Submitted text' } },
    { draftId: 'queue:queue-a', action: { kind: 'queue_update', submission_id: 'queue-a', text: 'Submitted text' } },
    { draftId: 'child:child-a', action: { kind: 'send_child', child_thread_id: 'child-a', text: 'Submitted text' } },
  ])('acknowledges $draftId after its panel unmounts without keeping an empty draft entry', async ({ draftId, action }) => {
    const view = await openPanel();
    const deferred = Promise.withResolvers<unknown>();
    let request: CodexOperationRequest;
    useCodexOperations.getState().draft(scope, draftId, 'Submitted text');
    invoke.mockImplementation((command: string, args: { input: CodexOperationRequest }) => {
      if (command === 'control_codex_operation') { request = args.input; return deferred.promise; }
      return Promise.resolve(snapshot);
    });
    const operation = useCodexOperations.getState().act(scope, action);
    view.unmount();
    await act(async () => { deferred.resolve({ session_id: session.id, generation: session.generation, request_id: request!.request_id, state: 'applied' }); await operation; });
    expect(Object.prototype.hasOwnProperty.call(entry().drafts, draftId)).toBe(false);
    expect(entry().pending).toBeNull();
    await openPanel();
    expect(controlCalls()).toHaveLength(1);
    if (draftId === 'queue:new') expect((screen.getByTestId('codex-queue-text') as HTMLTextAreaElement).value).toBe('');
  });
  it('reconciles the original receipt while runtime status is unknown but still rejects mutations', async () => {
    const view = await openPanel();
    let request: CodexOperationRequest;
    invoke.mockImplementation(async (command: string, args: { input: CodexOperationRequest }) => {
      if (command === 'observe_codex_operations') return snapshot;
      if (command === 'control_codex_operation') { request = args.input; return { session_id: session.id, generation: session.generation, request_id: request.request_id, state: 'unknown' }; }
      if (command === 'inspect_codex_operation') return { session_id: session.id, generation: session.generation, request_id: request!.request_id, state: 'applied' };
      throw 'unexpected_command';
    });
    await act(async () => { fireEvent.click(screen.getByTestId('codex-compact')); });
    const uncertain = { ...session, status: 'unknown' };
    await act(async () => { useAgentRunner.setState({ session: uncertain }); view.rerender(<CodexOperations session={uncertain} disabled />); });
    expect(screen.getByTestId('codex-operation-inspect').hasAttribute('disabled')).toBe(false);
    expect(screen.getByRole('status').textContent).toContain('codexOperations.unknown');
    await act(async () => { fireEvent.click(screen.getByTestId('codex-operation-inspect')); });
    expect(entry().pending).toBeNull();
    expect(entry().receipt?.state).toBe('applied');
    expect(screen.getByTestId('codex-compact').hasAttribute('disabled')).toBe(true);
    expect(await useCodexOperations.getState().act(scope, { kind: 'compact' })).toBeNull();
    expect(controlCalls()).toHaveLength(1);
  });
  it('reopens acknowledged queue edits from refreshed native text but preserves intentionally emptied drafts', async () => {
    await openPanel();
    fireEvent.click(within(screen.getByText('1. First').closest('li')!).getByTestId('codex-queue-edit'));
    fireEvent.change(screen.getByTestId('codex-queue-edit-text'), { target: { value: 'Edited first' } });
    snapshot.queue[0].text = 'Edited first';
    await act(async () => { fireEvent.click(screen.getByTestId('codex-queue-save')); });
    expect(Object.prototype.hasOwnProperty.call(entry().drafts, 'queue:queue-a')).toBe(false);
    fireEvent.click(within(screen.getByText('1. Edited first').closest('li')!).getByTestId('codex-queue-edit'));
    expect((screen.getByTestId('codex-queue-edit-text') as HTMLTextAreaElement).value).toBe('Edited first');
    fireEvent.change(screen.getByTestId('codex-queue-edit-text'), { target: { value: '' } });
    fireEvent.click(screen.getByRole('button', { name: 'codexOperations.cancel' }));
    fireEvent.click(within(screen.getByText('1. Edited first').closest('li')!).getByTestId('codex-queue-edit'));
    expect((screen.getByTestId('codex-queue-edit-text') as HTMLTextAreaElement).value).toBe('');
    expect(Object.prototype.hasOwnProperty.call(entry().drafts, 'queue:queue-a')).toBe(true);
    expect(screen.getByTestId('codex-queue-save').hasAttribute('disabled')).toBe(true);
  });
  it('preserves an explicitly empty edit made while an earlier queue update is in flight', async () => {
    await useCodexOperations.getState().observe(scope);
    useCodexOperations.getState().draft(scope, 'queue:queue-a', 'Submitted edit');
    const deferred = Promise.withResolvers<unknown>();
    invoke.mockReturnValue(deferred.promise);
    const flight = useCodexOperations.getState().act(scope, { kind: 'queue_update', submission_id: 'queue-a', text: 'Submitted edit' });
    const request = entry().pending!;
    useCodexOperations.getState().draft(scope, 'queue:queue-a', '');
    deferred.resolve({ session_id: session.id, generation: session.generation, request_id: request.request_id, state: 'applied' });
    await flight;
    expect(entry().drafts['queue:queue-a']).toBe('');
    expect(Object.prototype.hasOwnProperty.call(entry().drafts, 'queue:queue-a')).toBe(true);
  });
});
