import type { ComponentProps } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, expect, it, vi } from 'vitest';
import { ModalHost } from '@dgos/ui';
import { ProjectConversation } from '../src/pages/workbench/ProjectConversation';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import type { CliProviderRevision } from '../src/stores/developmentCapabilities';
import { addConversationReference, captureConversationReferences, clearConversationReferences, getConversationReferences, type ConversationReference } from '../src/stores/developmentReferences';
import { addAgentImages, agentImagesKey, captureAgentImages, useAgentImages } from '../src/stores/agentImages';
import { resetDesktopState } from '../src/stores/developmentDesktop';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { cleanup(); vi.useRealTimers(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); useDevelopmentDrafts.getState().clear(); resetDesktopState(); localStorage.clear(); invoke.mockReset(); });

const project = { id: 'p', owner: 'u', name: 'Project', path: '/project', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: null };
const task = { id: 't', owner: 'u', project_id: 'p', goal: 'Goal', requirements: '', status: 'todo' as const, created_at: 1 };
const workspace = { id: 'w', project_id: 'p', path: '/actual-worktree', directory_identity: 'worktree', kind: 'original' };
const capability: CliProviderRevision = { revision: 4, permission_epoch: 2, configuration: { engine: 'claude', provider: { provider_id: 'provider', model_id: 'model', provider_revision: 0 } }, created_at: 1 };
const selection = { project_id: 'p', task_id: null, session_id: null };
type Start = ComponentProps<typeof ProjectConversation>['onStart'];
const calls = (command: string) => invoke.mock.calls.filter(([name]) => name === command);
const prompt = () => screen.getByLabelText<HTMLTextAreaElement>('agent.prompt');
const send = () => screen.getByTestId<HTMLButtonElement>('direct-conversation-send');

function host(existingTask = false, trusted = true) {
  const state = {
    capability: structuredClone(capability),
    providerRevision: 0,
    workspace: { ...workspace },
    trusted,
    trustIdentity: workspace.directory_identity,
    trustRevision: 1,
    revoked: false,
    taskError: null as string | null,
    openError: null as string | null,
    sendError: null as string | null,
    openGate: null as Promise<void> | null,
    sendGate: null as Promise<void> | null,
    session: null as AgentSession | null,
  };
  useDevelopment.setState({ workspaceOwner: project.owner, loaded: true, projects: [project], tasks: existingTask ? [task] : [], selection: { ...selection, task_id: existingTask ? task.id : null } });
  invoke.mockImplementation(async (command, args) => {
    switch (command) {
      case 'read_development_workspace_trust': {
        if (state.trustIdentity !== state.workspace.directory_identity) throw 'workspace_changed';
        return { path: state.workspace.path, directory_identity: state.workspace.directory_identity, agent: state.trusted, agent_revoked: state.revoked, terminal: false, service: false, revision: state.trustRevision };
      }
      case 'activate_development_workspace_agent': {
        const input = args.input;
        const selected = useDevelopment.getState().selection;
        if (selected.project_id !== input.project_id || selected.task_id !== input.task_id) throw 'invalid_selection';
        if (input.expected_path !== state.workspace.path || input.expected_directory_identity !== state.workspace.directory_identity) throw 'workspace_changed';
        if (input.expected_revision !== state.trustRevision) throw 'workspace_trust_conflict';
        if (!state.trusted && !state.revoked) { state.trusted = true; ++state.trustRevision; }
        return { path: state.workspace.path, directory_identity: state.workspace.directory_identity, agent: state.trusted, agent_revoked: state.revoked, terminal: false, service: false, revision: state.trustRevision };
      }
      case 'set_development_workspace_trust': {
        if (args.input.expected_directory_identity !== state.workspace.directory_identity || args.input.expected_revision !== state.trustRevision) throw 'workspace_changed';
        state.trusted = args.input.granted;
        state.revoked = args.input.scope === 'agent' && !args.input.granted;
        state.trustIdentity = state.workspace.directory_identity;
        ++state.trustRevision;
        return { path: state.workspace.path, directory_identity: state.workspace.directory_identity, agent: state.trusted, agent_revoked: state.revoked, terminal: false, service: false, revision: state.trustRevision };
      }
      case 'read_development_cli_provider': return state.capability;
      case 'read_development_cli_provider_catalog': return { project_id: project.id, engine: args.engine, providers: [{ id: 'provider', name: 'Provider', enabled: true, config_revision: state.providerRevision, availability: { available: true, reason: null }, models: [{ id: 'model', name: 'Model', model_type: 'text' }] }] };
      case 'read_development_workspaces': return { workspaces: [state.workspace], selected_workspace_id: 'w', operation: null, operations: [] };
      case 'save_development_task': if (state.taskError) throw state.taskError; return { ...task, goal: args.input.goal };
      case 'read_development_workspace': return { projects: [project], tasks: [task], sessions: [], selection: useDevelopment.getState().selection, operations: [] };
      case 'open_agent_session': {
        await state.openGate;
        if (state.openError) throw state.openError;
        state.session = { id: args.input.id, generation: 'runtime-generation', engine: args.input.engine, native_id: null, workspace: args.input.options.workspace, status: 'idle', events: [], pending: [], error: null };
        return state.session;
      }
      case 'get_agent_session': if (!state.session) throw 'session_not_found'; return state.session;
      case 'cancel_unstarted_agent_session': return !state.session;
      case 'send_agent_message':
        if (state.sendError) throw state.sendError;
        state.session = { ...state.session!, status: 'running', active_run: args.input.request_id };
        await state.sendGate;
        return null;
      case 'interrupt_agent_session': return null;
      default: throw new Error(`Unexpected conversation IPC: ${command}`);
    }
  });
  // Exercise the real open/action boundary rather than reporting a mocked launch success.
  const start = vi.fn<Start>(async (input, projectId, taskId, dispatch) => {
    await useAgentRunner.getState().open(input);
    const runner = useAgentRunner.getState();
    if (runner.error || !runner.session) return { ok: false, code: runner.error ?? 'unknown' };
    const result = await dispatch(runner.session);
    if (result.ok) useDevelopment.setState({ selection: { project_id: projectId, task_id: taskId, session_id: runner.session.id } });
    return result;
  });
  return { state, start };
}
function conversation(start: Start, taskId: string | null = null, inspect = vi.fn(async () => {}), setSessionDraft = vi.fn()) {
  return <MemoryRouter><ProjectConversation project={project} taskId={taskId} onStart={start} onInspect={inspect} setSessionDraft={setSessionDraft} /><ModalHost /></MemoryRouter>;
}
async function prepare(text = 'Goal\nKeep this complete prompt') {
  fireEvent.change(prompt(), { target: { value: text } });
  await waitFor(() => expect(send().disabled).toBe(false));
}

async function waitForWorkspace() {
  await waitFor(() => expect(calls('read_development_workspace_trust').length).toBeGreaterThan(0));
  await waitFor(() => expect(calls('read_development_workspaces').length).toBeGreaterThan(0));
  await act(async () => {});
}

async function beginAutomaticPreparation(text = 'Goal\nKeep this complete prompt') {
  fireEvent.change(prompt(), { target: { value: text } });
  await waitFor(() => expect(calls('open_agent_session')).toHaveLength(1));
}

async function passPreparationDebounce() {
  const elapsed = Promise.withResolvers<void>();
  window.setTimeout(elapsed.resolve, 250);
  await act(async () => { await elapsed.promise; });
}

it('auto-activates the selected host directory and never sends a retained draft', async () => {
  const { state, start } = host(false, false);
  const setSessionDraft = vi.fn();
  render(conversation(start, null, undefined, setSessionDraft));
  fireEvent.change(prompt(), { target: { value: 'Goal' } });
  await waitFor(() => expect(calls('activate_development_workspace_agent')).toHaveLength(1));
  expect(calls('activate_development_workspace_agent')[0][1].input).toEqual({ project_id: 'p', task_id: null, expected_path: workspace.path, expected_directory_identity: workspace.directory_identity, expected_revision: 1 });
  await waitFor(() => expect(calls('open_agent_session')).toHaveLength(1));
  expect(calls('save_development_task')).toHaveLength(1);
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(screen.queryByTestId('direct-conversation-authorize')).toBeNull();
  expect(start).toHaveBeenCalledTimes(1);
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(state.trusted).toBe(true);
  expect(setSessionDraft).toHaveBeenLastCalledWith(state.session!.id, 'Goal');
});

it('never prepares or sends when host activation rejects a changed directory', async () => {
  const { start } = host(false, false);
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => {
    if (command === 'activate_development_workspace_agent') throw 'workspace_changed';
    return fallback(command, args);
  });
  render(conversation(start));
  fireEvent.change(prompt(), { target: { value: 'Retain this draft on directory replacement' } });
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('dev.error.workspace_changed'));
  await passPreparationDebounce();
  expect(calls('save_development_task')).toHaveLength(0);
  expect(calls('open_agent_session')).toHaveLength(0);
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(calls('set_development_workspace_trust')).toHaveLength(0);
  expect(start).not.toHaveBeenCalled();
  expect(prompt().value).toBe('Retain this draft on directory replacement');
});

it('admits one task, session and first prompt despite double click and preserves edits made during open', async () => {
  const { state, start } = host();
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  const setSessionDraft = vi.fn();
  render(conversation(start, null, undefined, setSessionDraft));
  await prepare();
  fireEvent.click(send());
  await waitFor(() => expect(calls('open_agent_session')).toHaveLength(1));
  fireEvent.click(send());
  fireEvent.keyDown(prompt(), { key: 'Enter', metaKey: true });
  fireEvent.change(prompt(), { target: { value: 'Next prompt' } });
  await act(async () => { opening.resolve(); });
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  const opened = calls('open_agent_session')[0][1].input;
  const sent = calls('send_agent_message')[0][1].input;
  expect(opened).toMatchObject({ intent: 'create', task_id: task.id, project_revision: capability.revision, provider: capability.configuration.provider, options: { workspace: workspace.path, trust_workspace: false } });
  expect(sent).toMatchObject({ session_id: opened.id, generation: 'runtime-generation', text: 'Goal\nKeep this complete prompt' });
  expect(sent.request_id).toEqual(expect.any(String));
  expect(calls('save_development_task')).toHaveLength(1);
  expect(start).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(setSessionDraft).toHaveBeenCalledWith(opened.id, 'Next prompt'));
  await waitFor(() => expect(prompt().value).toBe(''));
});

it('keeps the loaded project selection and task identity through first-send workspace refresh', async () => {
  const { start } = host();
  localStorage.setItem(`dgos.development.conversations:${project.owner}`, JSON.stringify({ tabs: [{ id: 'old-tab', projectId: 'old-project', taskId: null, sessionId: null, title: 'Old workspace' }], activeTabId: 'old-tab' }));
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command, args) => {
    const result = await fallback(command, args);
    return command === 'read_development_workspace' ? { ...result, selection: { project_id: null, task_id: null, session_id: null } } : result;
  });
  render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  expect(useDevelopment.getState().workspaceOwner).toBe(project.owner);
  expect(useDevelopment.getState().selection.project_id).toBe(project.id);
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')[0][1].input.task_id).toBe(task.id);
});

it('can stop a first turn while its send acknowledgement is stalled, without claiming native completion', async () => {
  const { state, start } = host();
  const receipt = Promise.withResolvers<void>(); state.sendGate = receipt.promise;
  render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  state.session = { ...state.session!, persistence_error: 'storage_failed' };
  await act(async () => { await useAgentRunner.getState().attach(state.session!.id); });
  expect(useAgentRunner.getState().acting).toBe(true);
  expect(screen.getByText('agent.storageFailed')).toBeTruthy();
  expect(screen.queryByText('agent.failure')).toBeNull();
  const stop = screen.getByRole<HTMLButtonElement>('button', { name: 'agent.interrupt' });
  expect(stop.disabled).toBe(false);
  fireEvent.click(stop); fireEvent.click(stop);
  await waitFor(() => expect(calls('interrupt_agent_session')).toHaveLength(1));
  expect(calls('interrupt_agent_session')[0][1]).toEqual({ sessionId: state.session!.id, generation: state.session!.generation, runId: state.session!.active_run });
  await screen.findByText('agent.interruptPending');
  expect(screen.getByText('agent.state.running')).toBeTruthy();
  expect(stop.disabled).toBe(true);
  expect(screen.getByRole<HTMLButtonElement>('button', { name: 'agent.close' }).disabled).toBe(false);
  state.session = { ...state.session!, status: 'idle', active_run: null, events: [{ seq: 1, kind: 'turn/completed', text: null, data: { run_status: 'cancelled' }, run: state.session!.active_run }] };
  await act(async () => { await useAgentRunner.getState().attach(state.session!.id); });
  expect(screen.queryByText('agent.interruptPending')).toBeNull();
  expect(screen.getByText('agent.state.idle')).toBeTruthy();
  await act(async () => { receipt.resolve(); });
  await waitFor(() => expect(prompt().value).toBe(''));
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(1);
});

it.each(['unsupported_reasoning_effort', 'unsupported_permission_mode', 'unsupported_collaboration_mode', 'invalid_turn_settings', 'turn_controls_unavailable'])('keeps explicit first-turn control rejection correctable without replay or a new session (%s)', async code => {
  const { state, start } = host(); state.sendError = code;
  render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain(`dev.error.${code}`));
  expect(screen.queryByText('directConversation.send_unknown')).toBeNull();
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  state.sendError = null;
  await waitFor(() => expect(send().disabled).toBe(false));
  expect(calls('send_agent_message')).toHaveLength(1);
  fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(2));
  expect(calls('send_agent_message')[1][1].input.session_id).toBe(calls('send_agent_message')[0][1].input.session_id);
  expect(calls('send_agent_message')[1][1].input.request_id).not.toBe(calls('send_agent_message')[0][1].input.request_id);
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
});

it.each(['cli_incompatible', 'cli_handshake_failed', 'cli_handshake_timeout'])('retains the prompt and task identity after a definite failed start (%s), requiring an explicit fresh session attempt after remount', async failure => {
  const { state, start } = host(); state.openError = failure;
  const view = render(conversation(start));
  await prepare(); fireEvent.click(send());
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain(`dev.error.${failure}`));
  const first = calls('open_agent_session')[0][1].input;
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  view.unmount(); render(conversation(start));
  await waitForWorkspace();
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
  await passPreparationDebounce();
  expect(calls('open_agent_session')).toHaveLength(1);
  state.openError = null;
  await prepare(); fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  expect(calls('open_agent_session')[1][1].input.task_id).toBe(first.task_id);
  expect(calls('open_agent_session')[1][1].input.id).not.toBe(first.id);
  expect(calls('save_development_task')).toHaveLength(1);
});

it('never repeats uncertain task creation and requires explicit selection after host reconciliation', async () => {
  const { state, start } = host(); state.taskError = 'transport lost';
  const view = render(conversation(start));
  await prepare(); fireEvent.click(send());
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  expect(send().disabled).toBe(true);
  expect(calls('save_development_task')).toHaveLength(1);
  view.unmount(); render(conversation(start));
  fireEvent.click(send());
  expect(calls('save_development_task')).toHaveLength(1);
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  const candidate = await screen.findByRole('button', { name: 'directConversation.useTask' });
  expect(start).not.toHaveBeenCalled();
  expect(send().disabled).toBe(true);
  fireEvent.click(candidate);
  expect(start).not.toHaveBeenCalled();
  state.taskError = null;
  await prepare(); fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')[0][1].input.task_id).toBe(task.id);
});

it('blocks an unknown open across remount and only attaches the original identity on explicit reconciliation', async () => {
  const { state, start } = host(true); state.openError = 'unknown';
  const view = render(conversation(start, task.id));
  await prepare(); fireEvent.click(send());
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  const opened = calls('open_agent_session')[0][1].input;
  view.unmount(); render(conversation(start, task.id));
  await waitForWorkspace();
  await passPreparationDebounce();
  fireEvent.click(send());
  expect(send().disabled).toBe(true);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(calls('get_agent_session')).toHaveLength(0);
  state.session = { id: opened.id, generation: 'reconciled-generation', engine: 'claude', native_id: null, workspace: workspace.path, status: 'idle', events: [], pending: [], error: null };
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(screen.queryByRole('button', { name: 'directConversation.recheck' })).toBeNull());
  await waitFor(() => expect(send().disabled).toBe(false));
  expect(calls('get_agent_session')[0][1]).toEqual({ sessionId: opened.id });
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
  fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')[0][1].input.session_id).toBe(opened.id);
});

it.each(['revision', 'permission_epoch'] as const)('rejects a stale project %s before creating a task or opening a session', async field => {
  const { state, start } = host();
  render(conversation(start)); await prepare();
  state.capability = { ...state.capability, [field]: state.capability[field] + 1 };
  fireEvent.click(send());
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('dev.error.capability_conflict'));
  expect(calls('save_development_task')).toHaveLength(0);
  expect(start).not.toHaveBeenCalled();
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
});

it('rejects a changed provider revision instead of silently replacing the selected route', async () => {
  const { state, start } = host();
  render(conversation(start)); await prepare(); state.providerRevision = 1;
  fireEvent.click(send());
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('dev.error.provider_revision_changed'));
  expect(calls('save_development_cli_provider')).toHaveLength(0);
  expect(calls('save_development_task')).toHaveLength(0);
  expect(start).not.toHaveBeenCalled();
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
});

it('revokes directory trust when the resolved workspace identity changes before open', async () => {
  const { state, start } = host();
  render(conversation(start)); await prepare();
  state.workspace = { ...workspace, directory_identity: 'replacement-directory' };
  fireEvent.click(send());
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('dev.error.workspace_changed'));
  expect(screen.queryByTestId('direct-conversation-authorize')).toBeNull();
  await passPreparationDebounce();
  expect(start).not.toHaveBeenCalled();
  expect(calls('send_agent_message')).toHaveLength(0);
});

it('does not automatically open or send again after acknowledged success and remount', async () => {
  const { start } = host();
  const view = render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(prompt().value).toBe(''));
  expect(calls('send_agent_message')).toHaveLength(1);
  view.unmount(); render(conversation(start));
  await waitForWorkspace();
  await passPreparationDebounce();
  fireEvent.click(send());
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(1);
  expect(prompt().value).toBe('');
});

it('never replays an uncertain first send and clears it only after explicit run-identity reconciliation', async () => {
  const { state, start } = host(true); state.sendError = 'transport lost';
  const inspect = vi.fn(async () => {}); const setSessionDraft = vi.fn();
  const view = render(conversation(start, task.id, inspect, setSessionDraft)); await prepare(); fireEvent.click(send());
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  const request = calls('send_agent_message')[0][1].input;
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  view.unmount(); render(conversation(start, task.id, inspect, setSessionDraft));
  fireEvent.click(send());
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(screen.getAllByRole('alert').some(alert => alert.textContent?.includes('dev.error.unknown'))).toBe(true));
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  expect(calls('send_agent_message')).toHaveLength(1);
  state.session = { ...state.session!, active_run: request.request_id };
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(inspect).toHaveBeenCalledWith(request.session_id));
  expect(prompt().value).toBe('');
  expect(setSessionDraft).toHaveBeenCalledWith(request.session_id, '');
  expect(calls('send_agent_message')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
});

it.each(['cli_incompatible', 'provider_revision_changed', 'provider_binding_unconfirmed', 'session_busy', 'bridge_unavailable', 'workspace_busy', 'cleanup_pending', 'session_exists', 'unrecognized transport failure', 'result_unknown'])('preserves the prompt and task after host open rejection without replaying uncertainty (%s)', async failure => {
  const { state, start } = host(true); state.openError = failure;
  render(conversation(start, task.id)); await prepare(); fireEvent.click(send());
  const blocked = !['cli_incompatible', 'provider_revision_changed', 'cleanup_pending'].includes(failure);
  await waitFor(() => expect(screen.getAllByRole('alert').some(alert => alert.textContent?.includes(`dev.error.${['unrecognized transport failure', 'result_unknown'].includes(failure) ? 'unknown' : failure}`))).toBe(true));
  await waitFor(() => expect(send().disabled).toBe(blocked));
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  const first = calls('open_agent_session')[0][1].input;
  fireEvent.click(send());
  if (!blocked) await waitFor(() => expect(calls('open_agent_session')).toHaveLength(2));
  else expect(calls('open_agent_session')).toHaveLength(1);
  if (!blocked) {
    expect(calls('open_agent_session')[1][1].input.task_id).toBe(first.task_id);
    expect(calls('open_agent_session')[1][1].input.id).not.toBe(first.id);
  }
  expect(calls('save_development_task')).toHaveLength(0);
  expect(calls('send_agent_message')).toHaveLength(0);
});

it('keeps a retained failed launch blocked until the host confirms it closed, then preserves the draft for explicit retry', async () => {
  const { state, start } = host(true); state.openError = 'cleanup_pending';
  state.session = { id: 'retained', generation: 'runtime-generation', engine: 'codex', native_id: null, workspace: workspace.path, status: 'idle', events: [], pending: [], error: null };
  render(conversation(start, task.id)); await prepare(); fireEvent.click(send());
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  const first = calls('open_agent_session')[0][1].input;
  state.session = { ...state.session, id: first.id, status: 'closed' };
  expect(send().disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(send().disabled).toBe(false));
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  await passPreparationDebounce();
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
  state.openError = null;
  fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  expect(calls('open_agent_session')[1][1].input.id).not.toBe(first.id);
  expect(calls('open_agent_session')[1][1].input.task_id).toBe(first.task_id);
});

it('does not dispatch a delayed startup after its composer unmounts, and remount keeps it pending verification', async () => {
  const { state, start } = host();
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  const view = render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(calls('open_agent_session')).toHaveLength(1));
  view.unmount();
  await act(async () => { opening.resolve(); });
  render(conversation(start));
  await waitForWorkspace();
  await passPreparationDebounce();
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  expect(send().disabled).toBe(true);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
});

const rootReference: ConversationReference = { project_id: 'p', task_id: null, path: 'a.ts', kind: 'file', expected_hash: 'root-version' };
it('captures root references before task creation and migrates only concurrent additions to the new session', async () => {
  const { state, start } = host();
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  addConversationReference('p', 'p', rootReference);
  render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(calls('open_agent_session')).toHaveLength(1));
  act(() => addConversationReference('p', 'p', { ...rootReference, path: 'later.ts' }));
  await act(async () => { opening.resolve(); });
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  const input = calls('send_agent_message')[0][1].input;
  expect(input.references).toEqual([rootReference]);
  await waitFor(() => expect(getConversationReferences('p', input.session_id)).toEqual([{ ...rootReference, path: 'later.ts' }]));
  expect(getConversationReferences('p', 'p')).toEqual([]);
});

it('preserves root reference versions across a rejected startup and explicit retry after task creation', async () => {
  const { state, start } = host(); state.openError = 'cli_incompatible';
  addConversationReference('p', 'p', rootReference);
  render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(calls('open_agent_session')).toHaveLength(1));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('dev.error.cli_incompatible'));
  expect(getConversationReferences('p', 'p')).toEqual([rootReference]);
  state.openError = null; fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  expect(calls('send_agent_message')[0][1].input.references).toEqual([rootReference]);
  expect(calls('save_development_task')).toHaveLength(1);
});

it('allows correction after definite reference preflight rejection without creating another task or session', async () => {
  const { state, start } = host(); state.sendError = 'file_changed';
  addConversationReference('p', 'p', rootReference);
  render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(screen.getAllByRole('alert').some(alert => alert.textContent?.includes('dev.error.file_changed'))).toBe(true));
  expect(getConversationReferences('p', 'p')).toEqual([rootReference]);
  act(() => {
    clearConversationReferences('p', 'p', captureConversationReferences('p', 'p'));
    addConversationReference('p', 'p', { ...rootReference, expected_hash: 'updated-version' });
  });
  state.sendError = null;
  await waitFor(() => expect(send().disabled).toBe(false)); fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(2));
  expect(calls('send_agent_message')[1][1].input.references).toEqual([{ ...rootReference, expected_hash: 'updated-version' }]);
  expect(calls('send_agent_message')[1][1].input.session_id).toBe(calls('send_agent_message')[0][1].input.session_id);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('save_development_task')).toHaveLength(1);
});

it('keeps uncertain captured references through remount, acknowledging only the original list on run reconciliation', async () => {
  const { state, start } = host(); state.sendError = 'transport lost';
  addConversationReference('p', 'p', rootReference);
  const inspect = vi.fn(async () => {});
  const view = render(conversation(start, null, inspect)); await prepare(); fireEvent.click(send());
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  const input = calls('send_agent_message')[0][1].input;
  act(() => addConversationReference('p', 'p', { ...rootReference, path: 'next.ts' }));
  view.unmount(); render(conversation(start, null, inspect));
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(screen.getAllByRole('alert').some(alert => alert.textContent?.includes('dev.error.unknown'))).toBe(true));
  expect(getConversationReferences('p', 'p')).toHaveLength(2);
  state.session = { ...state.session!, active_run: input.request_id };
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(inspect).toHaveBeenCalledWith(input.session_id));
  expect(getConversationReferences('p', input.session_id)).toEqual([{ ...rootReference, path: 'next.ts' }]);
  expect(getConversationReferences('p', 'p')).toEqual([]);
  expect(calls('send_agent_message')).toHaveLength(1);
});

it('migrates references added after send acknowledgement but before the blank tab finishes session selection', async () => {
  const { start } = host();
  const lateStart: Start = (input, projectId, taskId, dispatch) => start(input, projectId, taskId, async session => {
    const result = await dispatch(session);
    if (result.ok) addConversationReference('p', 'p', rootReference);
    return result;
  });
  render(conversation(lateStart)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(prompt().value).toBe(''));
  const input = calls('send_agent_message')[0][1].input;
  expect(input.references).toBeUndefined();
  expect(getConversationReferences('p', input.session_id)).toEqual([rootReference]);
  expect(getConversationReferences('p', 'p')).toEqual([]);
});

it('keeps a hidden first-send draft and closes composer overlays without dispatching retained gestures', async () => {
  const { start } = host(true);
  addConversationReference('p', 'p', { ...rootReference, task_id: task.id });
  const props = { project, taskId: task.id, onStart: start, onInspect: vi.fn(async () => {}), setSessionDraft: vi.fn() };
  const view = render(<MemoryRouter><ProjectConversation {...props} visible /></MemoryRouter>);
  await prepare();
  const button = send();
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.context' }));
  await screen.findByRole('dialog', { name: 'agentControls.context' });
  view.rerender(<MemoryRouter><ProjectConversation {...props} visible={false} /></MemoryRouter>);
  fireEvent.click(button); fireEvent.keyDown(prompt(), { key: 'Enter', metaKey: true });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(start).not.toHaveBeenCalled();
  expect(calls('send_agent_message')).toHaveLength(0);
  view.rerender(<MemoryRouter><ProjectConversation {...props} visible /></MemoryRouter>);
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  expect(getConversationReferences('p', 'p')).toEqual([{ ...rootReference, task_id: task.id }]);
  expect(screen.queryByRole('dialog')).toBeNull();
});

it.each(['project', 'engine', 'contradictory', 'cli_missing'] as const)('refuses an unverified click-time catalog route (%s)', async mismatch => {
  const { start } = host();
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command, args) => {
    const result = await fallback(command, args);
    if (command !== 'read_development_cli_provider_catalog') return result;
    if (mismatch === 'project') return { ...result, project_id: 'another-project' };
    if (mismatch === 'engine') return { ...result, engine: args.engine === 'claude' ? 'codex' : 'claude' };
    return { ...result, providers: result.providers.map((provider: object) => ({ ...provider, availability: { available: mismatch === 'contradictory', reason: 'cli_incompatible' } })) };
  });
  render(conversation(start)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(screen.getAllByRole('alert').some(alert => alert.textContent?.includes(mismatch === 'project' || mismatch === 'engine' ? 'dev.error.provider_revision_changed' : 'dev.error.cli_incompatible'))).toBe(true));
  expect(calls('save_development_task')).toHaveLength(0);
  expect(start).not.toHaveBeenCalled();
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
});

it('does not send or start on first use without a configured provider', async () => {
  const { start } = host();
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => command === 'read_development_cli_provider' ? Promise.resolve(null) : fallback(command, args));
  render(conversation(start));
  fireEvent.change(prompt(), { target: { value: 'Retained first-use prompt' } });
  await waitForWorkspace();
  await passPreparationDebounce();
  fireEvent.click(send()); fireEvent.keyDown(prompt(), { key: 'Enter', ctrlKey: true });
  expect(send().disabled).toBe(true);
  expect(start).not.toHaveBeenCalled();
  expect(calls('save_development_task')).toHaveLength(0);
  expect(prompt().value).toBe('Retained first-use prompt');
});

it('does not inspect a reconciled ready session when its send finishes after a context switch', async () => {
  const { state, start } = host(true); state.sendError = 'file_changed';
  const inspect = vi.fn(async () => {});
  render(conversation(start, task.id, inspect)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('dev.error.file_changed'));
  const sent = Promise.withResolvers<null>();
  const fallback = invoke.getMockImplementation()!;
  state.sendError = null;
  invoke.mockImplementation((command, args) => command === 'send_agent_message' ? sent.promise : fallback(command, args));
  await waitFor(() => expect(send().disabled).toBe(false)); fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(2));
  act(() => useDevelopment.setState({ selection: { project_id: 'another-project', task_id: null, session_id: null } }));
  await act(async () => { sent.resolve(null); });
  await waitFor(() => expect(useAgentRunner.getState().acting).toBe(false));
  expect(inspect).not.toHaveBeenCalled();
  expect(calls('open_agent_session')).toHaveLength(1);
});

it('migrates prompt edits made between first-send acknowledgement and session selection', async () => {
  const { start } = host();
  const setSessionDraft = vi.fn();
  const lateStart: Start = (input, projectId, taskId, dispatch) => start(input, projectId, taskId, async session => {
    const result = await dispatch(session);
    if (result.ok) fireEvent.change(prompt(), { target: { value: 'Late next-turn edit' } });
    return result;
  });
  render(conversation(lateStart, null, undefined, setSessionDraft)); await prepare(); fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  const id = calls('send_agent_message')[0][1].input.session_id;
  await waitFor(() => expect(setSessionDraft).toHaveBeenLastCalledWith(id, 'Late next-turn edit'));
  expect(calls('send_agent_message')[0][1].input.text).toBe('Goal\nKeep this complete prompt');
});

it('uses a captured reference path as the task title for an explicit reference-only first send', async () => {
  const { start } = host();
  addConversationReference('p', 'p', rootReference);
  render(conversation(start)); await prepare(''); fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  expect(calls('save_development_task')[0][1].input.goal).toBe(rootReference.path);
  expect(calls('send_agent_message')[0][1].input).toMatchObject({ text: '', references: [rootReference] });
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(getConversationReferences('p', 'p')).toEqual([]);
});

it('re-enables an explicitly revoked directory directly and prepares without sending its retained draft', async () => {
  const { state, start } = host(false, false);
  state.revoked = true;
  const setSessionDraft = vi.fn();
  render(conversation(start, null, undefined, setSessionDraft));
  fireEvent.change(prompt(), { target: { value: 'Unsent setup draft' } });
  const authorize = await screen.findByTestId('direct-conversation-authorize');
  await waitFor(() => expect(authorize).toHaveProperty('disabled', false));
  await passPreparationDebounce();
  expect(calls('activate_development_workspace_agent')).toHaveLength(0);
  expect(calls('open_agent_session')).toHaveLength(0);
  fireEvent.click(authorize);
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toEqual(expect.any(String)));
  expect(calls('set_development_workspace_trust')).toHaveLength(1);
  expect(calls('set_development_workspace_trust')[0][1]).toEqual({ input: { project_id: project.id, task_id: null, expected_directory_identity: workspace.directory_identity, expected_revision: 1, scope: 'agent', granted: true } });
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(state.trusted).toBe(true);
  expect(state.revoked).toBe(false);
  expect(setSessionDraft).toHaveBeenLastCalledWith(state.session!.id, 'Unsent setup draft');
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
});

it('waits for the 180ms trusted preparation debounce without treating it as send intent', async () => {
  vi.useFakeTimers();
  const { state, start } = host();
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  render(conversation(start));
  await act(async () => {});
  await act(async () => { await vi.advanceTimersByTimeAsync(179); });
  expect(calls('save_development_task')).toHaveLength(0);
  expect(calls('open_agent_session')).toHaveLength(0);
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
  await act(async () => { opening.resolve(); });
  expect(state.session?.status).toBe('idle');
  expect(calls('send_agent_message')).toHaveLength(0);
});

it('automatically prepares an empty trusted draft as a selected idle session with no model dispatch', async () => {
  const { state, start } = host();
  const setSessionDraft = vi.fn();
  const view = render(conversation(start, null, undefined, setSessionDraft));
  expect(send().disabled).toBe(true);
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toEqual(expect.any(String)));
  const opened = calls('open_agent_session')[0][1].input;
  expect(opened).toMatchObject({ intent: 'create', task_id: task.id, project_revision: capability.revision, provider: capability.configuration.provider, options: { workspace: workspace.path, trust_workspace: false } });
  expect(calls('save_development_task')[0][1].input.goal).toBe('directConversation.defaultTaskGoal');
  expect(calls('save_development_task')).toHaveLength(1);
  expect(start).toHaveBeenCalledTimes(1);
  expect(calls('set_development_workspace_trust')).toHaveLength(0);
  expect(state.session).toMatchObject({ id: opened.id, status: 'idle', pending: [], events: [] });
  expect(state.session?.active_run).toBeFalsy();
  expect(useDevelopment.getState().selection).toEqual({ project_id: project.id, task_id: task.id, session_id: opened.id });
  expect(setSessionDraft).toHaveBeenLastCalledWith(opened.id, '');
  expect(calls('send_agent_message')).toHaveLength(0);
  view.unmount(); render(conversation(start, null, undefined, setSessionDraft));
  await waitForWorkspace(); await passPreparationDebounce();
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
});

it.each(['Revised setup draft', ''])('hands off the latest automatically prepared draft edited during open (%j)', async latest => {
  const { state, start } = host();
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  const setSessionDraft = vi.fn();
  render(conversation(start, null, undefined, setSessionDraft));
  await beginAutomaticPreparation();
  fireEvent.change(prompt(), { target: { value: latest } });
  expect(setSessionDraft).not.toHaveBeenCalled();
  await act(async () => { opening.resolve(); });
  const id = calls('open_agent_session')[0][1].input.id;
  await waitFor(() => expect(setSessionDraft).toHaveBeenLastCalledWith(id, latest));
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toBe(id));
  expect(prompt().value).toBe('');
  expect(state.session?.status).toBe('idle');
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
});

it('queues one explicit first send during automatic preparation and preserves subsequent prompt edits', async () => {
  const { state, start } = host();
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  const setSessionDraft = vi.fn();
  render(conversation(start, null, undefined, setSessionDraft));
  await beginAutomaticPreparation();
  expect(screen.getByRole('status').textContent).toBe('desktop.preparing');
  expect(calls('send_agent_message')).toHaveLength(0);
  fireEvent.change(prompt(), { target: { value: 'Explicit queued prompt' } });
  fireEvent.click(send()); fireEvent.click(send());
  fireEvent.keyDown(prompt(), { key: 'Enter', ctrlKey: true });
  expect(screen.getByRole('status').textContent).toBe('desktop.queuedFirst');
  fireEvent.change(prompt(), { target: { value: 'Next unsent prompt' } });
  await act(async () => { opening.resolve(); });
  const id = calls('open_agent_session')[0][1].input.id;
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toBe(id));
  expect(calls('send_agent_message')).toHaveLength(1);
  expect(calls('send_agent_message')[0][1].input).toMatchObject({ session_id: id, text: 'Explicit queued prompt', generation: 'runtime-generation' });
  expect(setSessionDraft).toHaveBeenLastCalledWith(id, 'Next unsent prompt');
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(start).toHaveBeenCalledTimes(1);
  expect(state.session?.status).toBe('running');
});

it.each(['opening', 'selection'] as const)('migrates all unsent automatically prepared references including additions during %s', async timing => {
  const { state, start } = host();
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  const laterReference = { ...rootReference, path: 'later.ts', expected_hash: 'later-version' };
  addConversationReference('p', 'p', rootReference);
  const lateStart: Start = (input, projectId, taskId, dispatch) => start(input, projectId, taskId, async session => {
    const result = await dispatch(session);
    if (result.ok && timing === 'selection') addConversationReference('p', 'p', laterReference);
    return result;
  });
  render(conversation(lateStart)); await beginAutomaticPreparation();
  await waitFor(() => expect(calls('open_agent_session')).toHaveLength(1));
  if (timing === 'opening') act(() => addConversationReference('p', 'p', laterReference));
  await act(async () => { opening.resolve(); });
  const id = calls('open_agent_session')[0][1].input.id;
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toBe(id));
  await waitFor(() => expect(getConversationReferences('p', id)).toEqual([rootReference, laterReference]));
  expect(getConversationReferences('p', 'p')).toEqual([]);
  expect(state.session?.status).toBe('idle');
  expect(calls('send_agent_message')).toHaveLength(0);
});

it('retains automatic preparation draft and references after definite failure until explicit retry', async () => {
  const failure = 'cli_handshake_failed';
  const { state, start } = host(); state.openError = failure;
  const setSessionDraft = vi.fn();
  addConversationReference('p', 'p', rootReference);
  const view = render(conversation(start, null, undefined, setSessionDraft));
  await beginAutomaticPreparation();
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain(`dev.error.${failure}`));
  const first = calls('open_agent_session')[0][1].input;
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  expect(getConversationReferences('p', 'p')).toEqual([rootReference]);
  expect(setSessionDraft).not.toHaveBeenCalled();
  expect(useDevelopment.getState().selection.session_id).toBeNull();
  view.unmount(); render(conversation(start, null, undefined, setSessionDraft));
  await waitForWorkspace(); await passPreparationDebounce();
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
  state.openError = null;
  fireEvent.click(screen.getByRole('button', { name: 'desktop.retryPrepare' }));
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toEqual(expect.any(String)));
  const second = calls('open_agent_session')[1][1].input;
  expect(second.task_id).toBe(first.task_id);
  expect(second.id).not.toBe(first.id);
  expect(calls('save_development_task')).toHaveLength(1);
  expect(setSessionDraft).toHaveBeenLastCalledWith(second.id, 'Goal\nKeep this complete prompt');
  expect(getConversationReferences('p', second.id)).toEqual([rootReference]);
  expect(calls('send_agent_message')).toHaveLength(0);
});

it('reconciles an uncertain automatic open without reopening, sending, or entering until explicit send', async () => {
  const { state, start } = host(true); state.openError = 'result_unknown';
  const inspect = vi.fn(async (sessionId: string) => { useDevelopment.setState({ selection: { project_id: project.id, task_id: task.id, session_id: sessionId } }); });
  const setSessionDraft = vi.fn();
  addConversationReference('p', 'p', rootReference);
  const view = render(conversation(start, task.id, inspect, setSessionDraft));
  await beginAutomaticPreparation();
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  const opened = calls('open_agent_session')[0][1].input;
  view.unmount(); render(conversation(start, task.id, inspect, setSessionDraft));
  await waitForWorkspace(); await passPreparationDebounce();
  fireEvent.click(send());
  expect(send().disabled).toBe(true);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('get_agent_session')).toHaveLength(0);
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(setSessionDraft).not.toHaveBeenCalled();
  state.session = { id: opened.id, generation: 'reconciled-generation', engine: 'claude', native_id: null, workspace: workspace.path, status: 'idle', events: [], pending: [], error: null };
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(screen.queryByRole('button', { name: 'directConversation.recheck' })).toBeNull());
  await waitFor(() => expect(send().disabled).toBe(false));
  await passPreparationDebounce();
  expect(calls('get_agent_session')[0][1]).toEqual({ sessionId: opened.id });
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(inspect).not.toHaveBeenCalled();
  expect(setSessionDraft).not.toHaveBeenCalled();
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  expect(getConversationReferences('p', 'p')).toEqual([rootReference]);
  expect(useDevelopment.getState().selection.session_id).toBeNull();
  fireEvent.click(send());
  await waitFor(() => expect(inspect).toHaveBeenCalledWith(opened.id));
  expect(calls('send_agent_message')).toHaveLength(1);
  expect(calls('send_agent_message')[0][1].input).toMatchObject({ session_id: opened.id, text: 'Goal\nKeep this complete prompt', references: [rootReference] });
  expect(useDevelopment.getState().selection.session_id).toBe(opened.id);
  expect(getConversationReferences('p', 'p')).toEqual([]);
  expect(calls('open_agent_session')).toHaveLength(1);
});

it('does not replay uncertain automatic task creation through remount or explicit reconciliation', async () => {
  const { state, start } = host(); state.taskError = 'transport lost';
  const view = render(conversation(start));
  fireEvent.change(prompt(), { target: { value: 'Unsent setup draft' } });
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  expect(send().disabled).toBe(true);
  expect(calls('save_development_task')).toHaveLength(1);
  view.unmount(); render(conversation(start));
  await waitForWorkspace(); await passPreparationDebounce();
  fireEvent.click(send());
  expect(calls('save_development_task')).toHaveLength(1);
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  const candidate = await screen.findByRole('button', { name: 'directConversation.useTask' });
  expect(send().disabled).toBe(true);
  expect(start).not.toHaveBeenCalled();
  fireEvent.click(candidate);
  await passPreparationDebounce();
  expect(start).not.toHaveBeenCalled();
  expect(calls('send_agent_message')).toHaveLength(0);
  state.taskError = null;
  await waitFor(() => expect(send().disabled).toBe(false));
  fireEvent.click(send());
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toEqual(expect.any(String)));
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('open_agent_session')[0][1].input.task_id).toBe(task.id);
  expect(calls('send_agent_message')).toHaveLength(1);
  expect(calls('send_agent_message')[0][1].input).toMatchObject({ text: 'Unsent setup draft', session_id: calls('open_agent_session')[0][1].input.id });
});

const firstSendPng = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6bXkAAAAASUVORK5CYII=';
const firstSendFile = (name = 'first-image.png') => new File([Uint8Array.from(atob(firstSendPng), char => char.charCodeAt(0))], name, { type: 'image/png' });
async function selectFirstSendImage() {
  fireEvent.change(await screen.findByTestId('agent-image-picker'), { target: { files: [firstSendFile()] } });
  await waitFor(() => expect(captureAgentImages(agentImagesKey('p', 'p'))).toHaveLength(1));
}

it('sends an image-only first Codex turn once with actual selected bytes and no image data in draft metadata', async () => {
  const { state, start } = host(); state.capability.configuration.engine = 'codex';
  render(conversation(start));
  await selectFirstSendImage(); await prepare('');
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain(firstSendPng);
  fireEvent.click(send()); fireEvent.keyDown(prompt(), { key: 'Enter', metaKey: true });
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  const input = calls('send_agent_message')[0][1].input;
  expect(input).toMatchObject({ text: '', images: [{ mime: 'image/png', base64: firstSendPng }] });
  expect(calls('save_development_task')[0][1].input.goal).toBe('directConversation.defaultTaskGoal');
  await waitFor(() => expect(captureAgentImages(agentImagesKey('p', 'p'))).toEqual([]));
  expect(captureAgentImages(agentImagesKey('p', { id: input.session_id, generation: input.generation }))).toEqual([]);
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain(firstSendPng);
  expect(calls('open_agent_session')).toHaveLength(1);
});

it('prepares without consuming images and moves the same preview identities to the ready session', async () => {
  const { state, start } = host(); state.capability.configuration.engine = 'codex';
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  render(conversation(start));
  await selectFirstSendImage();
  const images = captureAgentImages(agentImagesKey('p', 'p'));
  await waitFor(() => expect(calls('open_agent_session')).toHaveLength(1));
  await act(async () => { opening.resolve(); });
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toEqual(expect.any(String)));
  const id = calls('open_agent_session')[0][1].input.id;
  await waitFor(() => expect(captureAgentImages(agentImagesKey('p', { id, generation: 'runtime-generation' }))).toEqual(images));
  expect(captureAgentImages(agentImagesKey('p', 'p'))).toEqual([]);
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain(firstSendPng);
});

it.each(['acknowledged', 'unknown'] as const)('retains queued image and reference identities through %s admission without replaying later additions', async outcome => {
  const { state, start } = host(); state.capability.configuration.engine = 'codex';
  if (outcome === 'unknown') state.sendError = 'transport lost';
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  const inspect = vi.fn(async (id: string) => { useDevelopment.setState({ selection: { project_id: project.id, task_id: task.id, session_id: id } }); });
  const setSessionDraft = vi.fn();
  await addAgentImages(agentImagesKey('p', 'p'), [firstSendFile()]);
  addConversationReference('p', 'p', rootReference);
  const view = render(conversation(start, null, inspect, setSessionDraft));
  await beginAutomaticPreparation('Queued media prompt');
  const queuedImages = captureAgentImages(agentImagesKey('p', 'p'));
  fireEvent.click(send());
  expect(screen.getByRole('status').textContent).toBe('desktop.queuedFirst');
  const laterReference = { ...rootReference, path: 'later.ts', expected_hash: 'later-version' };
  act(() => addConversationReference('p', 'p', laterReference));
  await act(async () => { await addAgentImages(agentImagesKey('p', 'p'), [firstSendFile('later.png')]); });
  const laterImages = captureAgentImages(agentImagesKey('p', 'p')).filter(image => !queuedImages.some(queued => queued.id === image.id));
  fireEvent.change(prompt(), { target: { value: 'Next media prompt' } });
  await act(async () => { opening.resolve(); });
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  const input = calls('send_agent_message')[0][1].input;
  expect(input).toMatchObject({ text: 'Queued media prompt', references: [rootReference], images: [{ mime: 'image/png', base64: firstSendPng }] });
  expect(input.images).toHaveLength(1);
  if (outcome === 'unknown') {
    await screen.findByRole('button', { name: 'directConversation.recheck' });
    expect(captureAgentImages(agentImagesKey('p', 'p'))).toHaveLength(2);
    expect(getConversationReferences('p', 'p')).toEqual([rootReference, laterReference]);
    view.unmount(); render(conversation(start, null, inspect, setSessionDraft));
    await waitForWorkspace(); await passPreparationDebounce();
    fireEvent.click(send());
    fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
    await waitFor(() => expect(screen.getAllByRole('alert').some(alert => alert.textContent?.includes('dev.error.unknown'))).toBe(true));
    expect(calls('send_agent_message')).toHaveLength(1);
    expect(captureAgentImages(agentImagesKey('p', 'p'))).toHaveLength(2);
    state.session = { ...state.session!, active_run: input.request_id };
    fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
    await waitFor(() => expect(inspect).toHaveBeenCalledWith(input.session_id));
  }
  await waitFor(() => expect(useDevelopment.getState().selection.session_id).toBe(input.session_id));
  await waitFor(() => expect(captureAgentImages(agentImagesKey('p', { id: input.session_id, generation: input.generation }))).toEqual(laterImages));
  expect(getConversationReferences('p', input.session_id)).toEqual([laterReference]);
  expect(getConversationReferences('p', 'p')).toEqual([]);
  expect(captureAgentImages(agentImagesKey('p', 'p'))).toEqual([]);
  expect(setSessionDraft).toHaveBeenLastCalledWith(input.session_id, 'Next media prompt');
  expect(calls('save_development_task')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(1);
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain(firstSendPng);
});

it('does not dispatch a queued first send when automatic preparation finishes after its tab is hidden', async () => {
  const { state, start } = host();
  const opening = Promise.withResolvers<void>(); state.openGate = opening.promise;
  const inspect = vi.fn(async () => {}), setSessionDraft = vi.fn();
  addConversationReference('p', 'p', rootReference);
  const props = { project, taskId: null, onStart: start, onInspect: inspect, setSessionDraft };
  const view = render(<MemoryRouter><ProjectConversation {...props} visible /></MemoryRouter>);
  await beginAutomaticPreparation();
  fireEvent.click(send());
  expect(screen.getByRole('status').textContent).toBe('desktop.queuedFirst');
  view.rerender(<MemoryRouter><ProjectConversation {...props} visible={false} /></MemoryRouter>);
  await act(async () => { opening.resolve(); });
  expect(calls('send_agent_message')).toHaveLength(0);
  expect(inspect).not.toHaveBeenCalled();
  expect(setSessionDraft).not.toHaveBeenCalled();
  expect(useDevelopment.getState().selection.session_id).toBeNull();
  expect(prompt().value).toBe('Goal\nKeep this complete prompt');
  expect(getConversationReferences('p', 'p')).toEqual([rootReference]);
  view.rerender(<MemoryRouter><ProjectConversation {...props} visible /></MemoryRouter>);
  await waitForWorkspace(); await passPreparationDebounce();
  expect(send().disabled).toBe(true);
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')).toHaveLength(0);
});

it('retains unknown image-only first sends through remount and reconciles IDs without replaying bytes', async () => {
  const { state, start } = host(); state.capability.configuration.engine = 'codex'; state.sendError = 'unknown';
  const inspect = vi.fn(async (id: string) => { useDevelopment.setState({ selection: { project_id: 'p', task_id: task.id, session_id: id } }); });
  const view = render(conversation(start, null, inspect));
  await selectFirstSendImage(); await prepare(''); fireEvent.click(send());
  await screen.findByRole('button', { name: 'directConversation.recheck' });
  const input = calls('send_agent_message')[0][1].input;
  expect(screen.getByTestId('agent-image-remove-0')).toHaveProperty('disabled', true);
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain(firstSendPng);
  expect(JSON.stringify(useDevelopmentDrafts.getState().entries)).not.toContain('first-image.png');
  await act(async () => { await addAgentImages(agentImagesKey('p', 'p'), [firstSendFile('later.png')]); });
  view.unmount(); render(conversation(start, null, inspect));
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'directConversation.recheck' })).toHaveProperty('disabled', false));
  expect(useAgentImages.getState().drafts[agentImagesKey('p', 'p')].images).toHaveLength(2);
  expect(calls('send_agent_message')).toHaveLength(1);
  state.session = { ...state.session!, active_run: input.request_id };
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(inspect).toHaveBeenCalledWith(input.session_id));
  await waitFor(() => expect(captureAgentImages(agentImagesKey('p', { id: input.session_id, generation: input.generation })).map(image => image.name)).toEqual(['later.png']));
  expect(captureAgentImages(agentImagesKey('p', 'p'))).toEqual([]);
  expect(calls('send_agent_message')).toHaveLength(1);
  expect(calls('open_agent_session')).toHaveLength(1);
});

it.each(['agent_image_invalid', 'agent_image_type', 'agent_image_total_limit'])('keeps a definite first-send image rejection correctable without creating another native session (%s)', async code => {
  const { state, start } = host(); state.capability.configuration.engine = 'codex'; state.sendError = code;
  render(conversation(start)); await selectFirstSendImage(); await prepare(''); fireEvent.click(send());
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain(`dev.error.${code}`));
  expect(screen.queryByText('directConversation.send_unknown')).toBeNull();
  expect(captureAgentImages(agentImagesKey('p', 'p'))).toHaveLength(1);
  expect(screen.getByTestId('agent-image-remove-0')).toHaveProperty('disabled', false);
  state.sendError = null; fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(2));
  expect(calls('open_agent_session')).toHaveLength(1);
  expect(calls('send_agent_message')[1][1].input.request_id).not.toBe(calls('send_agent_message')[0][1].input.request_id);
});

it('recovers an absent startup only after the host fences the old identity, preserving drafts without replay', async () => {
  const { state, start } = host(true); state.openError = 'unknown';
  render(conversation(start, task.id)); await prepare('Retained startup draft'); fireEvent.click(send());
  await screen.findByText('directConversation.open_unknown');
  const originalId = calls('open_agent_session')[0][1].input.id;
  state.openError = null;
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await screen.findByText('dev.error.session_not_started');
  expect(calls('cancel_unstarted_agent_session')[0][1]).toEqual({ input: { session_id: originalId, task_id: task.id } });
  await passPreparationDebounce();
  expect(prompt().value).toBe('Retained startup draft');
  expect(calls('open_agent_session')).toHaveLength(1); expect(calls('send_agent_message')).toHaveLength(0);
  fireEvent.click(send());
  await waitFor(() => expect(calls('send_agent_message')).toHaveLength(1));
  expect(calls('open_agent_session')[1][1].input.id).not.toBe(originalId);
});

it('keeps an absent startup unknown when its host fence cannot be confirmed', async () => {
  const { state, start } = host(true); state.openError = 'unknown';
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => command === 'cancel_unstarted_agent_session' ? Promise.reject('storage_failed') : original(command, args));
  render(conversation(start, task.id)); await prepare('Keep this draft'); fireEvent.click(send());
  await screen.findByText('directConversation.open_unknown');
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await screen.findByText('dev.error.storage_failed');
  expect(screen.getByText('directConversation.open_unknown')).toBeTruthy();
  expect(send()).toHaveProperty('disabled', true);
  expect(prompt().value).toBe('Keep this draft');
  expect(calls('open_agent_session')).toHaveLength(1); expect(calls('send_agent_message')).toHaveLength(0);
});

it('does not persist startup uncertainty when the parent rejects before dispatch', async () => {
  host(true);
  const start: Start = async () => ({ ok: false, code: 'cancelled', notStarted: true });
  render(conversation(start, task.id)); await prepare('Never dispatched'); fireEvent.click(send());
  await screen.findByText('dev.error.cancelled');
  expect(screen.queryByText('directConversation.open_unknown')).toBeNull();
  expect(prompt().value).toBe('Never dispatched');
  expect(calls('open_agent_session')).toHaveLength(0);
  expect(calls('send_agent_message')).toHaveLength(0);
});
