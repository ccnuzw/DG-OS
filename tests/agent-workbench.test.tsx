import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Modal, toast } from '@dgos/ui';
import * as chatAttachments from '../src/components/chat/Attachments';
import { addAgentImages, agentImagesKey, captureAgentImages, useAgentImages } from '../src/stores/agentImages';
import AgentWorkbench from '../src/pages/workbench/AgentWorkbench';
import { AgentConversation } from '../src/pages/workbench/AgentConversation';
import { Request } from '../src/components/development/AgentNativeRequest';
import { EventContent } from '../src/components/development/AgentReplyContent';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { chooseContext } from '../src/stores/developmentSelection';
import { useDevelopmentRuntime, type DevelopmentProcess } from '../src/stores/developmentRuntime';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
import type { CliProviderRevision } from '../src/stores/developmentCapabilities';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { MemoryRouter, useLocation, useNavigate } from 'react-router-dom';
import { useWorkspaceTabs } from '../src/workspace/useWorkspaceTabs';
import { setDesktopSlot } from '../src/stores/developmentDesktop';
import { agentChatDisplayKey, agentChatPreferences } from '../src/stores/agentChatPreferences';
import { openDevelopmentDocument, useDevelopmentEditor } from '../src/stores/developmentEditor';
import { captureConversationReferences } from '../src/stores/developmentReferences';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
beforeEach(() => { let id = 0; vi.spyOn(crypto, 'randomUUID').mockImplementation(() => `00000000-0000-4000-8000-${String(++id).padStart(12, '0')}`); });
afterEach(() => { cleanup(); localStorage.clear(); useAgentRunner.getState().reset(); useDevelopment.getState().reset(); useDevelopmentDrafts.getState().clear(); invoke.mockReset(); vi.restoreAllMocks(); vi.useRealTimers(); });

const workspace = {
 projects: [{ id: 'p1', owner: 'u', name: 'demo', path: '/tmp/project', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: 1 }],
 tasks: [{ id: 't1', owner: 'u', project_id: 'p1', goal: 'Ship', requirements: 'Tests pass', status: 'todo', created_at: 1 }],
 sessions: [], selection: { project_id: 'p1', task_id: 't1', session_id: null }, operations: [],
};
const inspection = { path_state: 'available', kind: 'directory', repository_root: null, branch: null, changes: [], truncated: false, error: null };
const session = { id: '00000000-0000-4000-8000-000000000001', generation: 'runtime-generation-1', engine: 'codex', native_id: 'native', workspace: '/tmp/project', status: 'idle', events: [], pending: [], error: null };
const providerRevision: CliProviderRevision = { revision: 4, permission_epoch: 2, configuration: { engine: 'codex', provider: { provider_id: 'provider1', model_id: 'model1', provider_revision: 3 } }, created_at: 1 };

// `override` returns a value for the commands a test cares about; undefined falls back to the defaults below.
function install(override: (command: string, args?: { input?: unknown; sessionId?: string }) => unknown = () => undefined) {
 let selected = workspace.selection;
 const grants = new Map<string, { path: string; directory_identity: string; agent: boolean; agent_revoked: boolean; terminal: boolean; service: boolean; revision: number }>();
 invoke.mockImplementation(async (command: string, args?: { input?: unknown; sessionId?: string }) => {
  const result = override(command, args);
  if (result !== undefined) return result;
  if (command === 'read_development_desktop_state') return { revision: 0, value: {} };
  if (command === 'save_development_desktop_state') { const input = args!.input as { expected_revision: number; value: Record<string, unknown> }; return { revision: input.expected_revision + 1, value: structuredClone(input.value) }; }
  if (['read_development_workspace_trust', 'set_development_workspace_trust', 'activate_development_workspace_agent'].includes(command)) {
   const input = args!.input as { project_id: string; task_id: string | null; scope?: 'agent' | 'terminal' | 'service'; granted?: boolean; expected_revision: number; expected_path?: string; expected_directory_identity?: string };
   const key = JSON.stringify([input.project_id, input.task_id === 't2' ? '/tmp/task-worktree' : '/tmp/project']);
   const before = grants.get(key) ?? { path: input.task_id === 't2' ? '/tmp/task-worktree' : '/tmp/project', directory_identity: 'd', agent: false, agent_revoked: false, terminal: false, service: false, revision: 0 };
   let value = before;
   if (command === 'activate_development_workspace_agent') {
    const active = useDevelopment.getState().selection;
    if (active.project_id !== input.project_id || active.task_id !== input.task_id) throw 'invalid_selection';
    if (input.expected_revision !== before.revision) throw 'workspace_trust_conflict';
    if (input.expected_directory_identity !== before.directory_identity || input.expected_path !== before.path) throw 'workspace_changed';
    if (!before.agent && !before.agent_revoked) value = { ...before, agent: true, revision: before.revision + 1 };
   } else if (command === 'set_development_workspace_trust') {
    value = { ...before, [input.scope!]: input.granted, agent_revoked: input.scope === 'agent' ? !input.granted : before.agent_revoked, revision: before.revision + 1 };
   }
   grants.set(key, value);
   return value;
  }
  if (command === 'read_development_workspace') return { ...workspace, sessions: Object.values(useAgentRunner.getState().snapshots).map(view => ({ id: view.id, owner: 'u', project_id: 'p1', task_id: 't1', engine: view.engine, native_id: view.native_id, workspace: view.workspace, model: null, executable: null })), selection: selected };
  if (command === 'select_development_context') { selected = args!.input as typeof selected; return null; }
  if (command === 'read_development_workspaces') return { workspaces: [{ id: 'w1', path: '/tmp/project', directory_identity: 'd', kind: 'original' }], selected_workspace_id: 'w1', operation: null, operations: [] };
  if (command === 'inspect_development_project') return inspection;
  if (command === 'open_agent_session' || command === 'get_agent_session') return { ...session, id: args?.sessionId ?? session.id };
  if (command === 'read_development_cli_provider') return providerRevision;
  if (command === 'read_development_cli_provider_catalog') return { project_id: 'p1', engine: 'codex', providers: [{ id: 'provider1', name: 'Provider One', enabled: true, config_revision: 3, availability: { available: true, reason: null }, models: [{ id: 'model1', name: 'Model One', model_type: 'chat' }] }] };
  if (command === 'get_development_session_configuration') return { format: 'provider_bound', read_only: false, snapshot: { binding_id: 'binding1', engine: 'codex', ...providerRevision.configuration.provider, revision: 4, permission_epoch: 2 } };
  if (command === 'read_development_git') return { task_id: 't1', kind: 'directory', path: '/tmp/project', branch: null, head: null, files: [], conflicts: [], snapshot_token: 'snapshot', merge_heads: [], initial_staged_paths: [], file_versions: {}, baseline_version: '', review_token: 'review', index: [] };
  if (command === 'list_development_processes') return [];
  if (command === 'read_development_session_summaries') return [];
  if (command === 'read_development_project_git') return { kind: 'directory', path: '/tmp/project', branch: null, head: null, files: [], conflicts: [], snapshot_token: 'snapshot', merge_heads: [] };
  if (command === 'list_development_files') return { entries: [], truncated: false };
  if (command === 'list_development_runs') return [];
  if (command === 'read_development_review') return { task_id: 't1', captured_at: null, baseline_late: false, truncated: false, fingerprint: '', changes: [], runs_stale: false };
  if (command === 'read_development_commands') return { commands: [], discovered: [] };
  if (command === 'discover_native_mcp') return { instances: [], incomplete: false };
  if (command === 'discover_native_skills') return { instances: [], readonly_entries: [], missing: false, incomplete: false };
  if (command === 'discover_native_skill_rules') return { rules: [], coverage: 'selected_settings_layer' };
  if (command === 'list_native_sessions' || command === 'list_native_skill_sources') return [];
  if (command === 'search_native_skill_library') return { sources: [], results: [] };
  return undefined;
 });
}
async function waitForAgentTrust(projectId = 'p1', taskId: string | null = 't1') {
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('activate_development_workspace_agent', { input: expect.objectContaining({ project_id: projectId, task_id: taskId, expected_directory_identity: 'd', expected_path: '/tmp/project' }) }));
}
async function openGoals() {
 const projects = screen.getByTestId('dev-tool-projects');
 if (projects.getAttribute('aria-expanded') !== 'true') fireEvent.click(projects);
 fireEvent.pointerDown(screen.getByRole('button', { name: 'workbenchNavigation.manageProject: demo' }), { button: 0, ctrlKey: false });
 await act(async () => { fireEvent.click(screen.getByRole('menuitem', { name: 'desktop.goals' })); });
}
// Existing-session scenarios start through the production runner; first-send admission
// and orchestration are exercised separately below and in project-conversation coverage.
async function start(state = 'idle') {
 await act(async () => {});
 await act(async () => {
  const configuration = await invoke('read_development_cli_provider', { projectId: 'p1' }) as CliProviderRevision;
  await useAgentRunner.getState().open({ intent: 'create', id: session.id, engine: configuration.configuration.engine, task_id: 't1', project_revision: configuration.revision, provider: configuration.configuration.provider, options: { workspace: '/tmp/project', resume_id: null, executable: null, model: null, trust_workspace: true } });
  await useDevelopment.getState().load();
  await chooseContext({ project_id: 'p1', task_id: 't1', session_id: session.id });
 });
 expect(useAgentRunner.getState().session?.status).toBe(state);
 expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toBeTruthy();
}
describe('agent sessions', () => {
 it('automatically trusts the selected active workspace without sending a draft on typing/focus', async () => {
  install(command => command === 'read_development_cli_provider' ? null : undefined); render(<AgentWorkbench />);
  const prompt = await screen.findByRole('textbox', { name: 'agent.prompt' });
  await waitForAgentTrust();
  vi.useFakeTimers();
  fireEvent.change(prompt, { target: { value: 'First request does not start a runtime' } });
  fireEvent.focus(prompt);
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message')).toBe(false);
  expect(screen.queryByTestId('direct-conversation-authorize')).toBeNull();
  expect(prompt).toHaveProperty('value', 'First request does not start a runtime');
 });
 it('shows the selected task worktree after context settlement', async () => {
  let resolveWorkspace!: (value: unknown) => void;
  const selectedWorkspace = new Promise(resolve => { resolveWorkspace = resolve; });
  let changed = false;
  install(command => {
   if (command === 'read_development_workspace') return { ...workspace, tasks: [...workspace.tasks, { ...workspace.tasks[0], id: 't2', goal: 'Isolated work' }] };
   if (command === 'read_development_workspaces' && changed) return selectedWorkspace;
   return undefined;
  });
  render(<AgentWorkbench />);
  await waitFor(() => expect(screen.getByTestId('dev-context-details').textContent).toContain('/tmp/project'));
  changed = true;
  await act(async () => { await chooseContext({ project_id: 'p1', task_id: 't2', session_id: null }); });
  await act(async () => { resolveWorkspace({ workspaces: [{ id: 'w2', path: '/tmp/task-worktree', directory_identity: 'd', kind: 'managed' }], selected_workspace_id: 'w2', operation: null }); });
  await waitFor(() => expect(screen.getByTestId('dev-context-details').textContent).toContain('/tmp/task-worktree'));
  expect(invoke).toHaveBeenCalledWith('read_development_workspace_trust', { input: { project_id: 'p1', task_id: 't2' } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('activate_development_workspace_agent', { input: expect.objectContaining({ project_id: 'p1', task_id: 't2', expected_path: '/tmp/task-worktree' }) }));
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message')).toBe(false);
 });
 it('keeps the host session alive when its page unmounts', async () => {
  install(); const view = render(<AgentWorkbench />); await start(); view.unmount();
  expect(invoke.mock.calls.some(([name]) => name === 'close_agent_session' || name === 'interrupt_agent_session')).toBe(false);
 });
 it('retains unsent text on submission failure and does not retry', async () => {
  install(); render(<AgentWorkbench />); await start();
  install(command => { if (command === 'send_agent_message') throw Error('private-token'); return undefined; });
  fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'change file' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await screen.findAllByRole('alert');
  expect((screen.getByLabelText('agent.prompt') as HTMLTextAreaElement).value).toBe('change file');
  expect(document.body.textContent).not.toContain('private-token');
  expect(invoke.mock.calls.filter(([name]) => name === 'send_agent_message')).toHaveLength(1);
 });
 it('returns a Claude denial without translating it to a Codex decision', async () => {
  const claude = { ...session, engine: 'claude', status: 'waiting', pending: [{ id: 'approval', kind: 'claude/toolPermission', title: 'Bash', data: { input: { command: 'echo hello' } } }] };
  install(command => command === 'read_development_cli_provider' ? { ...providerRevision, configuration: { ...providerRevision.configuration, engine: 'claude' } } : command === 'open_agent_session' || command === 'get_agent_session' ? claude : undefined);
  render(<AgentWorkbench />); await start('waiting');
  fireEvent.click(await screen.findByRole('button', { name: 'agent.deny' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: 'approval', response: { behavior: 'deny' } } }));
 });
});

it('opens CLI plugins in place and keeps the live session and unsent draft when management closes', async () => {
 install(command => {
  if (command === 'open_agent_session' || command === 'get_agent_session') return { ...session, status: 'running', active_run: 'run-in-progress' };
  if (command === 'discover_native_mcp') return { instances: [], incomplete: false };
  if (command === 'discover_native_skills') return { instances: [], readonly_entries: [], missing: false, incomplete: false };
  if (command === 'discover_native_skill_rules') return { rules: [], coverage: 'selected_settings_layer' };
  if (command === 'list_native_sessions') return [];
  return undefined;
 });
 render(<AgentWorkbench />); await start('running');
 fireEvent.change(screen.getByRole('textbox', { name: 'agent.prompt' }), { target: { value: 'Keep this conversation draft' } });
 fireEvent.click(screen.getByTestId('dev-tool-plugins'));
 const summary = await screen.findByRole('region', { name: 'nativeExtensions.configuredServers' });
 const manage = within(summary).getByRole('button', { name: 'nativeExtensions.manage' });
 manage.focus(); fireEvent.click(manage);
 const dialog = await screen.findByRole('dialog', { name: 'nativeExtensions.mcp' });
 expect(within(dialog).getByRole('combobox', { name: 'nativeExtensions.target' }).textContent).toContain('Codex');
 expect(within(dialog).getByRole('combobox', { name: 'nativeExtensions.scope' }).textContent).toContain('nativeExtensions.scope.ProjectShared');
 fireEvent.keyDown(dialog, { key: 'Escape' });
 await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
 expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toHaveProperty('value', 'Keep this conversation draft');
 expect(document.activeElement).toBe(manage);
 expect(useAgentRunner.getState().session?.status).toBe('running');
 expect(invoke.mock.calls.some(([name]) => ['close_agent_session', 'interrupt_agent_session', 'send_agent_message'].includes(name))).toBe(false);
});

it('keeps stop usable during a stalled send and waits for the native terminal before clearing stop pending', async () => {
 const receipt = Promise.withResolvers<null>();
 let native: AgentSession = { ...session, engine: 'codex' };
 install(command => {
  if (command === 'open_agent_session' || command === 'get_agent_session') return native;
  if (command === 'send_agent_message') { native = { ...native, status: 'running', active_run: 'stalled-run' }; return receipt.promise; }
  return undefined;
 });
 render(<AgentWorkbench />); await start();
 fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Send once, keep this until acknowledgement' } });
 fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
 await waitFor(() => expect(invoke.mock.calls.filter(([name]) => name === 'send_agent_message')).toHaveLength(1));
 await act(async () => { await useAgentRunner.getState().attach(session.id); });
 expect(useAgentRunner.getState().acting).toBe(true);
 const stop = screen.getByRole<HTMLButtonElement>('button', { name: 'agent.interrupt' });
 expect(stop.disabled).toBe(false);
 fireEvent.click(stop); fireEvent.click(stop);
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('interrupt_agent_session', { sessionId: session.id, generation: session.generation, runId: 'stalled-run' }));
 await screen.findByText('agent.interruptPending');
 expect(screen.getByText('agent.state.running')).toBeTruthy();
 expect(stop.disabled).toBe(true);
 expect(screen.getByRole<HTMLButtonElement>('button', { name: 'agent.close' }).disabled).toBe(false);
 expect(screen.getByLabelText('agent.prompt')).toHaveProperty('value', 'Send once, keep this until acknowledgement');
 native = { ...native, status: 'idle', active_run: null, events: [{ seq: 1, kind: 'turn/completed', text: null, run: 'stalled-run', data: { run_status: 'cancelled' } }] };
 await act(async () => { await useAgentRunner.getState().attach(session.id); });
 expect(screen.queryByText('agent.interruptPending')).toBeNull();
 expect(screen.getByText('agent.state.idle')).toBeTruthy();
 await act(async () => receipt.resolve(null));
 expect(invoke.mock.calls.filter(([name]) => name === 'interrupt_agent_session')).toHaveLength(1);
 expect(invoke.mock.calls.filter(([name]) => name === 'send_agent_message')).toHaveLength(1);
});

it('shows a separate storage warning without native failure and keeps stop and close available', async () => {
 let native: AgentSession = { ...session, engine: 'codex', status: 'running', active_run: 'run', persistence_error: 'storage_failed' };
 install(command => command === 'open_agent_session' || command === 'get_agent_session' ? native : undefined);
 render(<AgentWorkbench />); await start('running');
 expect(screen.getAllByText('agent.storageFailed').length).toBeGreaterThan(0);
 expect(screen.getByText('agent.state.running')).toBeTruthy();
 expect(screen.queryByText('agent.failure')).toBeNull();
 expect(screen.getByRole<HTMLButtonElement>('button', { name: 'agent.interrupt' }).disabled).toBe(false);
 expect(screen.getByRole<HTMLButtonElement>('button', { name: 'agent.close' }).disabled).toBe(false);
 native = { ...native, status: 'idle', active_run: null };
 await act(async () => { await useAgentRunner.getState().attach(session.id); });
 fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Keep my next draft' } });
 expect(screen.getByRole<HTMLButtonElement>('button', { name: 'agent.send' }).disabled).toBe(true);
 fireEvent.keyDown(screen.getByLabelText('agent.prompt'), { key: 'Enter', ctrlKey: true });
 expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message')).toBe(false);
 native = { ...native, persistence_error: null };
 await act(async () => { await useAgentRunner.getState().attach(session.id); });
 expect(screen.queryByText('agent.storageFailed')).toBeNull();
 expect(screen.getByLabelText('agent.prompt')).toHaveProperty('value', 'Keep my next draft');
 expect(screen.getByRole<HTMLButtonElement>('button', { name: 'agent.send' }).disabled).toBe(false);
});

it('starts from a taskless project through Goals without losing the draft or duplicating admission', async () => {
 const state = { ...workspace, tasks: [] as typeof workspace.tasks, sessions: [] as { id: string; owner: string; project_id: string; task_id: string; engine: string; native_id: string; workspace: string; model: null; executable: null }[], selection: { project_id: 'p1', task_id: null as string | null, session_id: null as string | null } };
 let native = { ...session };
 const provider = Promise.withResolvers<CliProviderRevision>();
 install((command, args) => {
  const input = args?.input as Record<string, any>;
  if (command === 'read_development_cli_provider') return provider.promise;
  if (command === 'read_development_workspace') return structuredClone(state);
  if (command === 'select_development_context') { state.selection = input as typeof state.selection; return null; }
  if (command === 'prepare_agent_task') { const task = { ...workspace.tasks[0], ...input, id: input.id, requirements: '', status: 'todo' }; const existing = state.tasks.find(item => item.id === input.id); if (!existing) state.tasks.push(task); return existing ?? task; }
  if (command === 'open_agent_session') {
   native = { ...session, id: input.id };
   state.sessions.push({ id: input.id, owner: 'u', project_id: 'p1', task_id: input.task_id, engine: input.engine, native_id: 'native', workspace: '/tmp/project', model: null, executable: null });
   return native;
  }
  if (command === 'get_agent_session') return native;
  if (command === 'send_agent_message') { native = { ...native, status: 'running' }; return null; }
  return undefined;
 });
 render(<AgentWorkbench />);
 await waitForAgentTrust('p1', null);
 const tab = useDevelopment.getState().activeTabId;
 const prompt = screen.getByRole('textbox', { name: 'agent.prompt' });
 fireEvent.change(prompt, { target: { value: 'Explain this project without a task form' } });
 await openGoals();
 fireEvent.click(screen.getByTestId('dev-task-conversation'));
 await waitFor(() => expect(document.activeElement).toBe(prompt));
 expect(prompt).toHaveProperty('value', 'Explain this project without a task form');
 expect(useDevelopment.getState().activeTabId).toBe(tab);
 expect(state.tasks).toEqual([]);
 expect(invoke.mock.calls.some(([name]) => ['prepare_agent_task', 'open_agent_session', 'send_agent_message'].includes(name))).toBe(false);
 await act(async () => { provider.resolve(providerRevision); });
 await waitFor(() => expect(screen.getByTestId('direct-conversation-send')).toHaveProperty('disabled', false));
 const send = screen.getByTestId('direct-conversation-send');
 await act(async () => { fireEvent.click(send); fireEvent.click(send); });
 await waitFor(() => expect(useDevelopment.getState().selection.session_id).toBe(native.id));
 expect(state.tasks).toEqual([expect.objectContaining({ project_id: 'p1', goal: 'Explain this project without a task form', requirements: '' })]);
 expect(useDevelopment.getState().activeTabId).toBe(tab);
 expect(useDevelopment.getState().tabs.find(item => item.id === tab)?.sessionId).toBe(native.id);
 for (const command of ['prepare_agent_task', 'open_agent_session', 'send_agent_message']) expect(invoke.mock.calls.filter(([name]) => name === command)).toHaveLength(1);
 expect(invoke.mock.calls.some(([name]) => ['close_agent_session', 'interrupt_agent_session'].includes(name))).toBe(false);
});
it('prepares file editing once, retains open tabs and the draft, and does not start CLI after context remount', async () => {
 const state = { ...workspace, tasks: [] as typeof workspace.tasks, selection: { project_id: 'p1', task_id: null as string | null, session_id: null as string | null } };
 const provider = Promise.withResolvers<CliProviderRevision>(), selecting = Promise.withResolvers<null>();
 install((command, args) => {
  const input = args?.input as Record<string, any>;
  if (command === 'read_development_cli_provider') return provider.promise;
  if (command === 'read_development_workspace') return structuredClone(state);
  if (command === 'save_development_task') { const task = { ...workspace.tasks[0], ...input, id: 'file-task' }; state.tasks.push(task); return task; }
  if (command === 'select_development_context') { state.selection = input as typeof state.selection; return selecting.promise; }
  if (command === 'read_development_file') return { path: input.path, text: 'base', hash: 'disk-hash', size: 4, reason: null };
  return undefined;
 });
 render(<AgentWorkbench />);
 const prompt = await screen.findByRole('textbox', { name: 'agent.prompt' });
 const tabId = useDevelopment.getState().activeTabId;
 fireEvent.change(prompt, { target: { value: 'Retain the conversation draft' } });
 act(() => { for (const path of ['a.ts', 'b.ts']) openDevelopmentDocument({ projectId: 'p1', taskId: null, workspacePath: '/tmp/project', path, mode: 'file', source: 'worktree' }); });
 await waitFor(() => expect(screen.getAllByTestId('dev-file-editor')).toHaveLength(2));
 fireEvent.click(screen.getAllByTestId('dev-file-edit').find(button => !button.closest('[hidden]'))!);
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('select_development_context', { input: { project_id: 'p1', task_id: 'file-task', session_id: null } }));
 expect(screen.getByTestId('dev-file-workspace').isConnected).toBe(true);
 await act(async () => { selecting.resolve(null); });
 await waitFor(() => expect(screen.getByRole('textbox', { name: 'direct.source' })).toHaveProperty('readOnly', false));
 expect(useDevelopment.getState().activeTabId).toBe(tabId);
 expect(within(screen.getByTestId('dev-file-workspace')).getAllByRole('tab')).toHaveLength(2);
 expect(useDevelopmentEditor.getState().tabs.every(tab => tab.taskId === 'file-task')).toBe(true);
 expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toHaveProperty('value', 'Retain the conversation draft');
 expect(invoke.mock.calls.filter(([command]) => command === 'save_development_task')).toHaveLength(1);
 vi.useFakeTimers();
 await act(async () => { provider.resolve(providerRevision); });
 await act(async () => { await vi.advanceTimersByTimeAsync(500); });
 expect(invoke.mock.calls.some(([command]) => ['prepare_agent_task', 'open_agent_session', 'send_agent_message', 'start_development_process'].includes(command))).toBe(false);
});

it('does not replay an unknown file-edit task creation', async () => {
 const state = { ...workspace, tasks: [], selection: { project_id: 'p1', task_id: null, session_id: null } };
 install((command, args) => {
  if (command === 'read_development_cli_provider') return null;
  if (command === 'read_development_workspace') return state;
  if (command === 'save_development_task') throw 'unknown';
  if (command === 'read_development_file') { const input = args?.input; return { path: input && typeof input === 'object' && 'path' in input ? input.path : '', text: 'base', hash: 'disk-hash', size: 4, reason: null }; }
  return undefined;
 });
 render(<AgentWorkbench />);
 await screen.findByRole('textbox', { name: 'agent.prompt' });
 act(() => openDevelopmentDocument({ projectId: 'p1', taskId: null, workspacePath: '/tmp/project', path: 'a.ts', mode: 'file', source: 'worktree' }));
 await screen.findByTestId('dev-file-editor');
 fireEvent.click(screen.getByTestId('dev-file-edit'));
 await screen.findByText('workbenchFiles.editFailed');
 fireEvent.click(screen.getByTestId('dev-file-edit'));
 await act(async () => {});
 expect(invoke.mock.calls.filter(([command]) => command === 'save_development_task')).toHaveLength(1);
 expect(screen.getByTestId('dev-file-editor')).toHaveProperty('readOnly', true);
 expect(invoke.mock.calls.some(([command]) => ['prepare_agent_task', 'open_agent_session', 'send_agent_message'].includes(command))).toBe(false);
});

it('adds a file reference, retains draft and chips, and focuses without automatic CLI startup', async () => {
 const provider = Promise.withResolvers<CliProviderRevision>();
 const notify = vi.spyOn(toast, 'success');
 install((command, args) => {
  if (command === 'read_development_cli_provider') return provider.promise;
  if (command === 'read_development_file') { const input = args?.input; return { path: input && typeof input === 'object' && 'path' in input ? input.path : '', text: 'base', hash: 'disk-hash', size: 4, reason: null }; }
  return undefined;
 });
 render(<AgentWorkbench />);
 const prompt = await screen.findByRole('textbox', { name: 'agent.prompt' });
 const tabId = useDevelopment.getState().activeTabId!;
 fireEvent.change(prompt, { target: { value: 'Keep this unsent draft' } });
 act(() => openDevelopmentDocument({ projectId: 'p1', taskId: 't1', workspacePath: '/tmp/project', path: 'a.ts', mode: 'file', source: 'worktree' }));
 await screen.findByTestId('dev-file-editor');
 fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.addConversation' }));
 await waitFor(() => expect(document.activeElement).toBe(prompt));
 expect(prompt).toHaveProperty('value', 'Keep this unsent draft');
 expect(captureConversationReferences('p1', tabId)).toEqual([expect.objectContaining({ reference: { project_id: 'p1', task_id: 't1', path: 'a.ts', kind: 'file', expected_hash: 'disk-hash' } })]);
 expect(screen.getByRole('list', { name: 'workbenchReferences.selected' }).textContent).toContain('a.ts');
 expect(notify).toHaveBeenCalledWith('workbenchFiles.referenceAdded');
 fireEvent.keyDown(screen.getByRole('button', { name: 'workbenchFiles.addOptions' }), { key: 'Enter' });
 fireEvent.click(await screen.findByRole('menuitem', { name: 'workbenchFiles.pathOnly' }));
 await waitFor(() => expect(document.activeElement).toBe(prompt));
 expect(captureConversationReferences('p1', tabId)).toHaveLength(2);
 expect(prompt).toHaveProperty('value', 'Keep this unsent draft');
 vi.useFakeTimers();
 await act(async () => { provider.resolve(providerRevision); });
 await act(async () => { await vi.advanceTimersByTimeAsync(500); });
 expect(invoke.mock.calls.some(([command]) => ['prepare_agent_task', 'open_agent_session', 'send_agent_message'].includes(command))).toBe(false);
});
it('focuses a saved conversation after adding a file without triggering focus preparation', async () => {
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install((command, args) => {
  if (command === 'read_development_workspace') return { ...workspace, sessions: [saved], selection: { project_id: 'p1', task_id: 't1', session_id: saved.id } };
  if (command === 'read_development_file') { const input = args?.input; return { path: input && typeof input === 'object' && 'path' in input ? input.path : '', text: 'base', hash: 'disk-hash', size: 4, reason: null }; }
  return undefined;
 });
 render(<AgentWorkbench />);
 const composer = await screen.findByTestId('saved-conversation-composer');
 const prompt = within(composer).getByRole('textbox', { name: 'agent.prompt' });
 await waitForAgentTrust();
 fireEvent.change(prompt, { target: { value: 'Keep saved history draft' } });
 act(() => openDevelopmentDocument({ projectId: 'p1', taskId: 't1', workspacePath: '/tmp/project', path: 'a.ts', mode: 'file', source: 'worktree' }));
 await screen.findByTestId('dev-file-editor');
 fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.addConversation' }));
 await waitFor(() => expect(document.activeElement).toBe(prompt));
 expect(prompt).toHaveProperty('value', 'Keep saved history draft');
 expect(captureConversationReferences('p1', saved.id)).toHaveLength(1);
 expect(invoke.mock.calls.some(([command]) => ['open_agent_session', 'send_agent_message'].includes(command))).toBe(false);
});



it.each([[true, false], [false, true], [false, false]])('clears the Agent running marker when development is hidden (runtime visible: %s, surface active: %s)', async (runtimeVisible, active) => {
 vi.useFakeTimers();
 let running = true;
 const process = (): DevelopmentProcess => ({ id: 'host-process', project_id: 'p1', task_id: 't1', kind: 'command', source: 'host', status: running ? 'running' : 'exited', output: 'host output', truncated: false, exit_code: running ? null : 0, reason: null, started_at: 1, finished_at: running ? null : 2, program: 'echo', args: [], cwd: '.', fingerprint: '', start_fingerprint: '', verification_status: 'not_requested', verification_complete: false, mutation_observed: false, preview_url: null });
 install(command => command === 'list_development_processes' ? [process()] : command === 'read_development_process' ? process() : undefined);
 const controller = createWorkspaceController();
 const tree = (surfaceActive: boolean) => <SurfaceScope id="cli-agent" active={surfaceActive} controller={controller}><AgentWorkbench /></SurfaceScope>;
 const view = render(tree(true));
 await act(async () => {});
 await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.runtime', exact: true })); });
 expect(screen.getByTestId('dev-development-runtime')).toBeTruthy();
 expect(controller.getSurfaceStatuses()['cli-agent']?.running).toBe(true);
 const reads = (command: string) => invoke.mock.calls.filter(([name]) => name === command).length;
 // One list read each second even while both the runtime and its outer consumer are mounted.
 const visibleHistory = reads('list_development_processes');
 await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
 expect(reads('list_development_processes') - visibleHistory).toBe(2);
 if (!runtimeVisible) {
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.conversation', exact: true })); });
  expect(screen.getByTestId('dev-panel-runtime').hasAttribute('hidden')).toBe(true);
 }
 view.rerender(tree(active));
 await act(async () => {});
 const details = reads('read_development_process');
 running = false;
 await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
 expect(useDevelopmentRuntime.getState().processes[0]?.status).toBe('exited');
 expect(controller.getSurfaceStatuses()['cli-agent']?.running).toBe(false);
 expect(reads('read_development_process')).toBe(details);
 const history = reads('list_development_processes');
 await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
 expect(reads('list_development_processes')).toBe(history);
 expect(invoke.mock.calls.some(([name]) => name === 'start_development_process' || name === 'stop_development_process')).toBe(false);
});

it('tracks a successful host start acknowledged after leaving the development tab, then clears its running marker', async () => {
 vi.useFakeTimers();
 const started = Promise.withResolvers<DevelopmentProcess>();
 let current: DevelopmentProcess | null = null;
 install(command => command === 'read_development_workspace_trust' ? { path: '/tmp/project', directory_identity: 'd', agent: false, terminal: true, service: false, revision: 1 } : command === 'list_development_processes' ? current ? [current] : [] : command === 'start_development_process' ? started.promise : undefined);
 const controller = createWorkspaceController();
 render(<SurfaceScope id="cli-agent" active controller={controller}><AgentWorkbench /></SurfaceScope>);
 await act(async () => {});
 await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.runtime', exact: true })); });
 fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
 fireEvent.click(screen.getByTestId('dev-process-start'));
 expect(invoke.mock.calls.filter(([name]) => name === 'start_development_process')).toHaveLength(1);
 const submittedId = invoke.mock.calls.find(([name]) => name === 'start_development_process')![1].input.id;
 await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.conversation', exact: true })); });
 expect(screen.getByTestId('dev-panel-runtime').hasAttribute('hidden')).toBe(true);
 current = { id: submittedId, project_id: 'p1', task_id: 't1', kind: 'command', source: 'host', status: 'running', output: '', truncated: false, exit_code: null, reason: null, started_at: 1, finished_at: null, program: 'echo', args: [], cwd: '.', fingerprint: '', start_fingerprint: '', verification_status: 'not_requested', verification_complete: false, mutation_observed: false, preview_url: null };
 await act(async () => { started.resolve(current!); });
 expect(controller.getSurfaceStatuses()['cli-agent']?.running).toBe(true);
 expect(useDevelopmentRuntime.getState().processes[0]?.id).toBe(submittedId);
 current = { ...current, status: 'exited', finished_at: 2, exit_code: 0 };
 await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
 expect(controller.getSurfaceStatuses()['cli-agent']?.running).toBe(false);
 expect(invoke.mock.calls.some(([name]) => name === 'read_development_process' || name === 'stop_development_process')).toBe(false);
});

it('keeps the hidden running marker qualified through read failure and unknown cleanup', async () => {
 vi.useFakeTimers();
 let status = 'running'; let failed = false;
 const process = (): DevelopmentProcess => ({ id: 'run', project_id: 'p1', task_id: 't1', kind: 'command', source: 'host_command', status, output: '', truncated: false, exit_code: null, reason: null, started_at: 1, finished_at: status === 'running' ? null : 2, program: 'echo', args: [], cwd: '', fingerprint: '', start_fingerprint: '', verification_status: 'not_requested', verification_complete: false, mutation_observed: false, preview_url: null });
 install(command => { if (command === 'list_development_processes') { if (failed) throw 'storage_failed'; return [process()]; } if (command === 'read_development_process') return process(); });
 const controller = createWorkspaceController();
 const tree = (active: boolean) => <SurfaceScope id="cli-agent" active={active} controller={controller}><AgentWorkbench /></SurfaceScope>;
 const view = render(tree(true)); await act(async () => {});
 await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.runtime', exact: true })); });
 await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.conversation', exact: true })); });
 view.rerender(tree(false)); failed = true;
 await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
 expect(controller.getSurfaceStatuses()['cli-agent']).toMatchObject({ running: true, runningLabel: 'direct.marker.lastKnown' });
 failed = false; status = 'unknown';
 await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
 expect(controller.getSurfaceStatuses()['cli-agent']).toMatchObject({ running: true, runningLabel: 'direct.status.unknown' });
 status = 'cleanup_pending';
 await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
 expect(controller.getSurfaceStatuses()['cli-agent']).toMatchObject({ running: true, runningLabel: 'direct.status.cleanup_pending' });
 status = 'cancelled';
 await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
 expect(controller.getSurfaceStatuses()['cli-agent']?.running).toBe(false);
});

it('keeps identical answers in separate turns while suppressing repeated terminal prose', async () => {
 const events = ['first', 'second'].flatMap((run, index) => [
  { seq: index * 3 + 1, kind: 'userMessage', text: `Request ${run}`, data: null, run },
  { seq: index * 3 + 2, kind: 'message', text: 'Finished the requested change.', data: { type: 'assistant', content: [{ type: 'text', text: 'Finished the requested change.' }] }, run },
  { seq: index * 3 + 3, kind: 'result', text: 'Finished the requested change.', data: { run_status: 'completed' }, run },
 ]);
 install(command => command === 'open_agent_session' || command === 'get_agent_session' ? { ...session, events } : undefined);
 render(<AgentWorkbench />); await start();
 expect(screen.getAllByText('Finished the requested change.')).toHaveLength(2);
 expect(screen.getByText('Request first')).toBeTruthy();
 expect(screen.getByText('Request second')).toBeTruthy();
});

it('sends once while the shortcut request is pending and preserves text edited during it', async () => {
 install(); render(<AgentWorkbench />); await start();
 const pending = Promise.withResolvers<null>();
 install(command => command === 'send_agent_message' ? pending.promise : undefined);
 const prompt = screen.getByLabelText('agent.prompt') as HTMLTextAreaElement;
 fireEvent.change(prompt, { target: { value: 'Original request' } });
 fireEvent.keyDown(prompt, { key: 'Enter', ctrlKey: true });
 fireEvent.keyDown(prompt, { key: 'Enter', metaKey: true });
 expect(invoke.mock.calls.filter(([name]) => name === 'send_agent_message')).toHaveLength(1);
 fireEvent.change(prompt, { target: { value: 'Next request drafted while sending' } });
 await act(async () => { pending.resolve(null); });
 expect(prompt.value).toBe('Next request drafted while sending');
});

it('counts only native requests and rejects expired or closed-session approval callbacks', async () => {
 const native = { id: 'native-approval', kind: 'item/commandExecution/requestApproval', title: 'native', data: { availableDecisions: ['accept', 'decline'] } };
 let pending = [native];
 install(command => command === 'open_agent_session' || command === 'get_agent_session' ? { ...session, status: 'waiting', pending } : undefined);
 render(<AgentWorkbench />); await start('waiting');
 const staleButton = within(screen.getByRole('region', { name: 'agent.approval' })).getByRole('button', { name: 'agent.allow' });
 pending = [];
 await act(async () => { await useAgentRunner.getState().attach(session.id); });
 expect(screen.queryByRole('region', { name: 'agent.approval' })).toBeNull();
 fireEvent.click(staleButton);
 expect(await useAgentRunner.getState().action('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { decision: 'accept' } } })).toBe(false);
 await act(async () => { useAgentRunner.setState({ session: { ...useAgentRunner.getState().session!, status: 'closed', pending: [native] } }); });
 expect(screen.queryByRole('button', { name: 'agent.allow' })).toBeNull();
 expect(await useAgentRunner.getState().action('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { decision: 'accept' } } })).toBe(false);
 expect(invoke.mock.calls.some(([command]) => command === 'respond_agent_request')).toBe(false);
});

it('continues the task session without starting another runtime and rejects the previous session approval', async () => {
 const first = { ...session, status: 'waiting', pending: [{ id: 'first-native', kind: 'item/commandExecution/requestApproval', title: 'First request', data: { availableDecisions: ['accept', 'decline'] } }] };
 const other = { ...session, id: 'other', status: 'waiting', pending: [{ id: 'other-native', kind: 'item/commandExecution/requestApproval', title: 'Other request', data: { availableDecisions: ['accept', 'decline'] } }] };
 const saved = { id: 'other', owner: 'u', engine: 'codex', native_id: 'other-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't2' };
 install(command => command === 'read_development_workspace' ? { ...workspace, tasks: [...workspace.tasks, { ...workspace.tasks[0], id: 't2', goal: 'Other task' }], sessions: [saved] } : command === 'open_agent_session' ? first : command === 'get_agent_session' ? useAgentRunner.getState().selected === 'other' ? other : first : undefined);
 useAgentRunner.setState({ live: { other: 'waiting' } });
 render(<AgentWorkbench />); await start('waiting');
 const staleButton = screen.getByRole('button', { name: 'agent.allow' });
 await openGoals();
 await act(async () => { fireEvent.click(screen.getByTestId('dev-task-item-t2')); });
 expect(screen.getByRole('region', { name: 'agent.approval' }).textContent).toContain('Other request');
 expect(invoke.mock.calls.some(([command]) => command === 'respond_agent_request')).toBe(false);
 expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1);
 expect(useDevelopment.getState().selection.task_id).toBe('t2');
 expect(useAgentRunner.getState().selected).toBe('other');
 fireEvent.click(staleButton);
 expect(await useAgentRunner.getState().action('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: 'first-native', response: { decision: 'accept' } } })).toBe(false);
 expect(invoke.mock.calls.some(([command]) => command === 'respond_agent_request')).toBe(false);
 await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'agent.allow' })); });
 expect(invoke).toHaveBeenCalledWith('respond_agent_request', { input: { session_id: 'other', generation: session.generation, request_id: 'other-native', response: { decision: 'accept' } } });
});

it('keeps the conversation, draft and reading position while drawers switch, resize and overlay', async () => {
 let width = 1400;
 const callbacks: ResizeObserverCallback[] = [];
 const frames: FrameRequestCallback[] = [];
 vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => width);
 vi.stubGlobal('ResizeObserver', class { constructor(callback: ResizeObserverCallback) { callbacks.push(callback); } observe() {} unobserve() {} disconnect() {} });
 vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
 vi.stubGlobal('cancelAnimationFrame', () => {});
 const settle = () => act(() => { callbacks.forEach(callback => callback([], {} as ResizeObserver)); frames.splice(0).forEach(callback => callback(0)); });
 try {
  install(); render(<AgentWorkbench />); await start(); settle();
  const prompt = screen.getByLabelText('agent.prompt');
  const transcript = screen.getByRole('region', { name: 'agent.messages' });
  fireEvent.change(prompt, { target: { value: 'Keep this actual draft' } });
  transcript.scrollTop = 120; fireEvent.scroll(transcript);
  fireEvent.click(screen.getByTestId('dev-tool-files'));
  fireEvent.keyDown(screen.getByTestId('dev-drawer-resizer'), { key: 'ArrowRight' });
  await openGoals();
  fireEvent.click(screen.getByTestId('dev-tool-projects'));
  fireEvent.click(screen.getByTestId('dev-tool-projects'));
  expect(document.getElementById('dev-navigation')?.hasAttribute('hidden')).toBe(true);
  expect(screen.getByLabelText('agent.prompt')).toBe(prompt);
  expect(prompt).toHaveProperty('value', 'Keep this actual draft');
  expect(screen.getByRole('region', { name: 'agent.messages' })).toBe(transcript);
  expect(transcript.scrollTop).toBe(120);
  width = 500; settle();
  fireEvent.click(screen.getByTestId('dev-tool-files')); settle();
  fireEvent.keyDown(document.getElementById('dev-navigation')!, { key: 'Escape' }); settle();
  expect(document.activeElement).toBe(screen.getByTestId('dev-tool-files'));
  expect(screen.getByLabelText('agent.prompt')).toBe(prompt);
  expect(prompt).toHaveProperty('value', 'Keep this actual draft');
  width = 1400; settle();
  expect(screen.getByRole('region', { name: 'agent.messages' })).toBe(transcript);
  expect(invoke.mock.calls.filter(([name]) => name === 'open_agent_session')).toHaveLength(1);
  expect(invoke.mock.calls.some(([name]) => ['close_agent_session', 'interrupt_agent_session', 'send_agent_message'].includes(name))).toBe(false);
 } finally { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); }
});

it.each(['codex', 'claude'] as const)('preserves the %s runtime and next draft through management and never replays a pending send', async engine => {
 const response = Promise.withResolvers<null>();
 const native = { ...session, engine };
 let persistedSelection: typeof workspace.selection | { project_id: string; task_id: string; session_id: string } = workspace.selection;
 install((command, args) => {
  if (command === 'select_development_context') { persistedSelection = args!.input as typeof persistedSelection; return null; }
  if (command === 'read_development_workspace') return { ...workspace, sessions: Object.values(useAgentRunner.getState().snapshots).map(view => ({ id: view.id, owner: 'u', project_id: 'p1', task_id: 't1', engine: view.engine, native_id: view.native_id, workspace: view.workspace, model: null, executable: null })), selection: persistedSelection };
  return command === 'read_development_cli_provider' ? { ...providerRevision, configuration: { ...providerRevision.configuration, engine } } : command === 'open_agent_session' || command === 'get_agent_session' ? native : command === 'send_agent_message' ? response.promise : undefined;
 });
 function Shell() {
  const tabs = useWorkspaceTabs();
  const location = useLocation(), navigate = useNavigate();
  return <>
   {tabs.surfaces.some(item => item.id === 'cli-agent') && <div hidden={tabs.backgroundSurfaceId !== 'cli-agent'}><SurfaceScope id="cli-agent" active={tabs.activeSurfaceId === 'cli-agent'} controller={tabs.controller}><AgentWorkbench /></SurfaceScope></div>}
   <output data-testid="management-entry">{location.pathname}</output>
   <button onClick={() => navigate(-1)}>Return to conversation</button>
  </>;
 }
 render(<MemoryRouter initialEntries={['/tasks/agent']}><Shell /></MemoryRouter>);
 await start();
 const prompt = screen.getByLabelText('agent.prompt') as HTMLTextAreaElement;
 fireEvent.change(prompt, { target: { value: 'One explicit request' } });
 fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
 fireEvent.change(prompt, { target: { value: 'Keep next request' } });
 const selection = useDevelopment.getState().selection;
 for (const label of ['nav.skills', 'nav.mcp']) {
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.more' }));
  fireEvent.click(within(screen.getByRole('navigation', { name: 'agentControls.management' })).getByRole('button', { name: label }));
  const manager = await screen.findByRole('dialog', { name: label === 'nav.skills' ? 'nativeExtensions.skills' : 'nativeExtensions.mcp' });
  expect(screen.getByTestId('management-entry').textContent).toBe('/tasks/agent');
  fireEvent.keyDown(manager, { key: 'Escape' });
  await waitFor(() => expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toBe(prompt));
 }
 fireEvent.click(screen.getByRole('button', { name: 'agentControls.more' }));
 fireEvent.click(within(screen.getByRole('dialog', { name: 'agentControls.more' })).getByRole('link', { name: 'capabilities.provider' }));
 await waitFor(() => expect(screen.getByTestId('management-entry').textContent).toContain('/settings/providers'));
 fireEvent.click(screen.getByRole('button', { name: 'Return to conversation' }));
 await waitFor(() => expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toBe(prompt));
 await act(async () => response.resolve(null));
 expect(prompt.value).toBe('Keep next request');
 expect(useDevelopment.getState().selection).toEqual(selection);
 expect(invoke.mock.calls.filter(([name]) => name === 'open_agent_session')).toHaveLength(1);
 expect(invoke.mock.calls.filter(([name]) => name === 'send_agent_message')).toHaveLength(1);
 expect(invoke.mock.calls.some(([name]) => name === 'close_agent_session' || name === 'interrupt_agent_session' || name === 'native_session_action')).toBe(false);
});

it('rejects old runtime approvals and sends when a restored session reuses native request ids', async () => {
 const native = { id: 'reused-request', kind: 'item/commandExecution/requestApproval', title: 'native', data: { command: 'echo current', availableDecisions: ['accept', 'decline', 'cancel'] } };
 install(command => command === 'open_agent_session' || command === 'get_agent_session' ? { ...session, status: 'waiting', pending: [native] } : undefined);
 render(<AgentWorkbench />); await start('waiting');
 const old = screen.getByRole('button', { name: 'agent.allow' });
 await act(async () => { useAgentRunner.setState({ session: { ...useAgentRunner.getState().session!, generation: 'replacement-runtime' } }); });
 fireEvent.click(old);
 expect(await useAgentRunner.getState().action('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { decision: 'accept' } } })).toBe(false);
 expect(await useAgentRunner.getState().action('send_agent_message', { input: { session_id: session.id, generation: session.generation, request_id: 'late-send', text: 'stale' } })).toBe(false);
 expect(invoke.mock.calls.some(([command]) => command === 'respond_agent_request' || command === 'send_agent_message')).toBe(false);
});

it('keeps the consumed approval absent when a pre-response poll arrives late', async () => {
 vi.useFakeTimers();
 const native = { id: 'native-request', kind: 'item/commandExecution/requestApproval', title: 'native', data: {} };
 const stale = Promise.withResolvers<unknown>();
 let reads = 0;
 invoke.mockImplementation(async (command: string) => {
  if (command === 'get_agent_session') { reads += 1; return reads === 2 ? stale.promise : { ...session, status: reads === 1 ? 'waiting' : 'running', pending: reads === 1 ? [native] : [] }; }
  return null;
 });
 await useAgentRunner.getState().attach(session.id);
 await vi.advanceTimersByTimeAsync(600);
 expect(await useAgentRunner.getState().action('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { decision: 'accept' } } })).toBe(true);
 stale.resolve({ ...session, status: 'waiting', pending: [native] });
 await Promise.resolve(); await Promise.resolve();
 expect(useAgentRunner.getState().session?.pending).toEqual([]);
 expect(useAgentRunner.getState().session?.status).toBe('running');
});

it('deduplicates an in-flight approval across callback instances without combining later turns', async () => {
 const native = { id: 'request', kind: 'item/commandExecution/requestApproval', title: 'native', data: {} };
 const response = Promise.withResolvers<null>();
 invoke.mockImplementation(async (command: string) => command === 'respond_agent_request' ? response.promise : { ...session, status: 'running', pending: [] });
 useAgentRunner.setState({ session: { ...session, engine: 'codex', status: 'waiting', pending: [native] }, selected: session.id });
 const args = { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { decision: 'accept' } } };
 const first = useAgentRunner.getState().action('respond_agent_request', args);
 expect(await useAgentRunner.getState().action('respond_agent_request', args)).toBe(false);
 expect(invoke.mock.calls.filter(([command]) => command === 'respond_agent_request')).toHaveLength(1);
 response.resolve(null); expect(await first).toBe(true);
});

it.each(['codex', 'claude'] as const)('keeps the %s session through terminal observation and allows the same text in the next turn', async engine => {
 let status = 'idle';
 const native = () => ({ ...session, engine, status });
 install(command => command === 'read_development_cli_provider' ? { ...providerRevision, configuration: { ...providerRevision.configuration, engine } } : command === 'open_agent_session' || command === 'get_agent_session' ? native() : undefined);
 render(<AgentWorkbench />); await start();
 for (let turn = 0; turn < 2; turn += 1) {
  fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Repeat this explicit request' } });
  status = 'running';
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'agent.send' })); });
  expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', true);
  expect(useAgentRunner.getState().session?.status).toBe('running');
  status = 'idle';
  await act(async () => { await useAgentRunner.getState().attach(session.id); });
 }
 expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(2);
 expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1);
});


it.each(['accept', 'decline', 'cancel'])('shows native command context and sends the exact Codex %s decision', async decision => {
 const native = { id: 'decision', kind: 'item/commandExecution/requestApproval', title: 'native', data: { command: 'echo reviewed-command', cwd: '/tmp/reviewed-directory', reason: 'Native permission reason', availableDecisions: ['accept', 'decline', 'cancel'] } };
 install(command => command === 'open_agent_session' || command === 'get_agent_session' ? { ...session, status: 'waiting', pending: [native] } : undefined);
 render(<AgentWorkbench />); await start('waiting');
 expect(screen.getByText('echo reviewed-command')).toBeTruthy();
 expect(screen.getByText('Native permission reason')).toBeTruthy();
 fireEvent.click(screen.getByRole('button', { name: decision === 'accept' ? 'agent.allow' : decision === 'decline' ? 'agent.deny' : 'common.cancel' }));
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { decision } } }));
});

it('dispatches one new session for repeated clicks before the open acknowledgement', async () => {
 const opened = Promise.withResolvers<unknown>();
 install(command => command === 'open_agent_session' ? opened.promise : undefined);
 render(<AgentWorkbench />);
 await waitForAgentTrust();
 fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Send exactly once' } });
 const create = screen.getByTestId('direct-conversation-send');
 await act(async () => { fireEvent.click(create); fireEvent.click(create); });
 await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1));
 const openedId = invoke.mock.calls.find(([command]) => command === 'open_agent_session')![1].input.id;
 await act(async () => opened.resolve({ ...session, id: openedId }));
 expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
});

it.each(['accept', 'decline', 'cancel'])('returns the native MCP elicitation %s action without a Codex decision', async action => {
 const submit = vi.fn(async () => true);
 render(<Request engine="codex" pending={{ id: 'elicitation', kind: 'mcpServer/elicitation/request', title: 'MCP', data: { mode: 'url', message: 'Review the native request', url: 'https://example.invalid/review' } }} submit={submit} />);
 fireEvent.click(screen.getByRole('button', { name: action === 'accept' ? 'agent.allow' : action === 'decline' ? 'agent.deny' : 'common.cancel' }));
 await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ action, content: null }));
});

it('submits Claude original tool permission and question answers in its native response shape', async () => {
 const submit = vi.fn(async () => true);
 render(<Request engine="claude" pending={{ id: 'question', kind: 'claude/toolPermission', title: 'AskUserQuestion', data: { input: { questions: [{ question: 'Choose a branch', options: [{ label: 'main' }, { label: 'release' }] }] } } }} submit={submit} />);
 fireEvent.click(screen.getByRole('button', { name: 'release' }));
 fireEvent.click(screen.getByRole('button', { name: 'agent.answer' }));
 await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ behavior: 'allow', answers: { 'Choose a branch': 'release' } }));
});

it('revalidates a closed Provider binding on authorized input focus without creating a conversation or sending a turn', async () => {
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install(command => command === 'read_development_workspace' ? { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } } : command === 'read_development_workspace_trust' ? { path: '/tmp/project', directory_identity: 'd', agent: true, terminal: false, service: false, revision: 1 } : command === 'get_development_session_configuration' ? { format: 'provider_bound', read_only: true, snapshot: { binding_id: 'closed-binding', engine: 'codex', ...providerRevision.configuration.provider, revision: 4, permission_epoch: 2 } } : undefined);
 render(<AgentWorkbench />);
 const composer = await screen.findByTestId('saved-conversation-composer');
 await act(async () => {});
 expect(invoke.mock.calls.some(([command]) => command === 'open_agent_session' || command === 'send_agent_message')).toBe(false);
 const prompt = within(composer).getByRole('textbox', { name: 'agent.prompt' });
 expect(prompt).toHaveProperty('disabled', false);
 fireEvent.focus(prompt);
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('open_agent_session', { input: { intent: 'resume', session_id: saved.id, trust_workspace: false } }));
 expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1);
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
 expect(useAgentRunner.getState().session?.id).toBe(saved.id);
});

it('queues one explicit saved-history send during focus preparation and preserves edits made afterward', async () => {
 const opened = Promise.withResolvers<AgentSession>();
 const accepted = Promise.withResolvers<null>();
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install(command => command === 'read_development_workspace' ? { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } } : command === 'read_development_workspace_trust' ? { path: '/tmp/project', directory_identity: 'd', agent: true, terminal: false, service: false, revision: 1 } : command === 'open_agent_session' ? opened.promise : command === 'send_agent_message' ? accepted.promise : command === 'read_development_cli_provider' ? { ...providerRevision, configuration: { engine: 'claude', provider: { provider_id: 'different-provider', model_id: 'new-model', provider_revision: 9 } } } : undefined);
 render(<AgentWorkbench />);
 const composer = await screen.findByTestId('saved-conversation-composer');
 await act(async () => {});
 const prompt = within(composer).getByRole('textbox', { name: 'agent.prompt' });
 fireEvent.focus(prompt);
 await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1));
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
 fireEvent.change(prompt, { target: { value: 'Continue the saved conversation' } });
 const send = within(composer).getByRole('button', { name: /desktop.continue|directConversation.preparing/ });
 expect(send).toHaveProperty('disabled', false);
 fireEvent.click(send); fireEvent.click(send);
 fireEvent.change(prompt, { target: { value: 'Keep this newer draft' } });
 await act(async () => opened.resolve({ ...session, engine: 'codex' }));
 await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1));
 expect(invoke).toHaveBeenCalledWith('open_agent_session', { input: { intent: 'resume', session_id: saved.id, trust_workspace: false } });
 expect(invoke.mock.calls.find(([command]) => command === 'send_agent_message')![1].input).toMatchObject({ session_id: saved.id, generation: session.generation, text: 'Continue the saved conversation' });
 await act(async () => accepted.resolve(null));
 await waitFor(() => expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toHaveProperty('value', 'Keep this newer draft'));
 expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1);
 expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
});

it('does not grant again when sending an already trusted saved conversation', async () => {
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install(command => command === 'read_development_workspace' ? { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } } : command === 'read_development_workspace_trust' ? { path: '/tmp/project', directory_identity: 'd', agent: true, agent_revoked: false, terminal: false, service: false, revision: 1 } : undefined);
 render(<AgentWorkbench />);
 const composer = await screen.findByTestId('saved-conversation-composer');
 const prompt = within(composer).getByRole('textbox', { name: 'agent.prompt' });
 fireEvent.change(prompt, { target: { value: 'Continue without another grant' } });
 fireEvent.keyDown(prompt, { key: 'Enter', ctrlKey: true });
 await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1));
 expect(invoke.mock.calls.some(([command]) => command === 'set_development_workspace_trust')).toBe(false);
 expect(invoke.mock.calls.some(([command]) => command === 'activate_development_workspace_agent')).toBe(false);
 expect(invoke).toHaveBeenCalledWith('open_agent_session', { input: { intent: 'resume', session_id: saved.id, trust_workspace: false } });
});

it('keeps an explicitly revoked saved conversation denied on attempted continuation', async () => {
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install(command => command === 'read_development_workspace' ? { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } } : command === 'read_development_workspace_trust' ? { path: '/tmp/project', directory_identity: 'd', agent: false, agent_revoked: true, terminal: false, service: false, revision: 2 } : undefined);
 render(<AgentWorkbench />);
 const composer = await screen.findByTestId('saved-conversation-composer');
 expect(await within(composer).findByText('desktop.trust.disabled')).toBeTruthy();
 expect(within(composer).getByRole('button', { name: 'desktop.trust.start' })).toBeTruthy();
 const prompt = within(composer).getByRole('textbox', { name: 'agent.prompt' });
 fireEvent.change(prompt, { target: { value: 'Do not silently regrant' } });
 fireEvent.keyDown(prompt, { key: 'Enter', ctrlKey: true });
 await act(async () => {});
 expect(invoke.mock.calls.some(([command]) => ['open_agent_session', 'send_agent_message', 'set_development_workspace_trust', 'activate_development_workspace_agent'].includes(command))).toBe(false);
 expect(prompt).toHaveProperty('value', 'Do not silently regrant');
});

it.each(['selection', 'account'] as const)('drops a queued saved continuation after its %s owner changes', async boundary => {
 const opened = Promise.withResolvers<AgentSession>();
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install(command => command === 'read_development_workspace' ? { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } } : command === 'read_development_workspace_trust' ? { path: '/tmp/project', directory_identity: 'd', agent: true, terminal: false, service: false, revision: 1 } : command === 'open_agent_session' ? opened.promise : undefined);
 const view = render(<AgentWorkbench />);
 const composer = await screen.findByTestId('saved-conversation-composer');
 await act(async () => {});
 const prompt = within(composer).getByRole('textbox', { name: 'agent.prompt' });
 fireEvent.focus(prompt);
 await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1));
 fireEvent.change(prompt, { target: { value: 'Only send to this original conversation' } });
 fireEvent.keyDown(prompt, { key: 'Enter', ctrlKey: true });
 if (boundary === 'account') {
  view.unmount();
  act(() => { useDevelopment.getState().reset(); useAgentRunner.getState().reset(); });
 } else await act(async () => { await chooseContext({ project_id: null, task_id: null, session_id: null }); });
 await act(async () => opened.resolve({ ...session, engine: 'codex' }));
 expect(invoke.mock.calls.some(([command]) => ['send_agent_message', 'interrupt_agent_session', 'close_agent_session'].includes(command))).toBe(false);
 if (boundary === 'account') {
  expect(useAgentRunner.getState().snapshots).toEqual({});
  expect(useDevelopmentDrafts.getState().entries).toEqual({});
 } else {
  expect(useDevelopment.getState().selection.session_id).toBeNull();
  expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', 'p1', saved.id, 'message')]?.value).toBe('Only send to this original conversation');
 }
});

it.each(['pending-send', 'pending-steer'] as const)('does not replay saved continuation while its %s receipt is unknown', async pending => {
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install(command => command === 'read_development_workspace' ? { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } } : command === 'read_development_workspace_trust' ? { path: '/tmp/project', directory_identity: 'd', agent: true, terminal: false, service: false, revision: 1 } : undefined);
 useAgentRunner.setState({ live: { [session.id]: 'unknown' }, snapshots: { [session.id]: { ...session, engine: 'codex', status: 'unknown' } } });
 const key = developmentDraftKey('conversation', 'p1', session.id, session.generation, pending);
 const receipt = { requestId: 'unconfirmed', generation: session.generation, runId: 'previous-run', text: 'Unconfirmed instruction', references: [], skills: [], images: [] };
 useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'conversation', value: receipt, baseline: null } } });
 render(<AgentWorkbench />);
 const prompt = await screen.findByRole('textbox', { name: 'agent.prompt' });
 await act(async () => {});
 fireEvent.change(prompt, { target: { value: 'A newer draft must not replay the pending instruction' } });
 fireEvent.focus(prompt);
 fireEvent.keyDown(prompt, { key: 'Enter', ctrlKey: true });
 const send = screen.queryByRole('button', { name: 'agent.send', exact: true }) ?? screen.getByRole('button', { name: /desktop.continue|directConversation.preparing/ });
 fireEvent.click(send);
 await act(async () => {});
 expect(invoke.mock.calls.some(([command]) => command === 'open_agent_session' || command === 'send_agent_message')).toBe(false);
 expect(useDevelopmentDrafts.getState().entries[key]?.value).toEqual(receipt);
 expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toHaveProperty('value', 'A newer draft must not replay the pending instruction');
});

it('detaches the workbench only after saving without stopping the runtime or deleting its draft', async () => {
 const native: AgentSession = { ...session, engine: 'codex', status: 'running', active_run: 'active-run' };
 install(command => command === 'open_agent_session' || command === 'get_agent_session' ? native : undefined);
 const controller = createWorkspaceController();
 render(<SurfaceScope id="cli-agent" active controller={controller}><AgentWorkbench /></SurfaceScope>);
 await start('running');
 fireEvent.change(screen.getByRole('textbox', { name: 'agent.prompt' }), { target: { value: 'Keep the next instruction' } });
 const key = developmentDraftKey('conversation', 'p1', session.id, 'message');
 const confirm = vi.spyOn(Modal, 'confirm');
 await act(async () => { expect(await controller.canLeave('cli-agent')).toBe(true); });
 expect(useAgentRunner.getState().session).toBeNull();
 expect(useAgentRunner.getState().selected).toBeNull();
 expect(useAgentRunner.getState().live[session.id]).toBe('running');
 expect(useAgentRunner.getState().snapshots[session.id]).toEqual(native);
 expect(useDevelopmentDrafts.getState().entries[key]?.value).toBe('Keep the next instruction');
 expect(invoke.mock.calls.some(([command]) => command === 'save_development_desktop_state')).toBe(true);
 expect(invoke.mock.calls.some(([command]) => ['close_agent_session', 'interrupt_agent_session', 'send_agent_message'].includes(command))).toBe(false);
 expect(confirm).not.toHaveBeenCalled();
});

it('drops a late action acknowledgement after the same session receives a replacement generation', async () => {
 const acknowledged = Promise.withResolvers<null>();
 invoke.mockImplementation(async (command: string) => command === 'send_agent_message' ? acknowledged.promise : session);
 useAgentRunner.setState({ session: { ...session, engine: 'codex' }, selected: session.id });
 const sending = useAgentRunner.getState().action('send_agent_message', { input: { session_id: session.id, generation: session.generation, request_id: 'original-send', text: 'Original request' } });
 useAgentRunner.setState({ session: { ...session, engine: 'codex', generation: 'replacement-generation', status: 'waiting' }, error: 'request_expired' });
 acknowledged.resolve(null);
 expect(await sending).toBe(false);
 expect(useAgentRunner.getState().session?.generation).toBe('replacement-generation');
 expect(useAgentRunner.getState().session?.status).toBe('waiting');
 expect(useAgentRunner.getState().error).toBe('request_expired');
 expect(invoke.mock.calls.some(([command]) => command === 'get_agent_session')).toBe(false);
});

it('keeps permission expansion scoped and cannot approve an unknown replacement runtime', async () => {
 const permissions = { network: { enabled: true }, fileSystem: { read: ['/tmp/reviewed-directory/input'], write: null } };
 const native = { id: 'permission-request', kind: 'item/permissions/requestApproval', title: 'native', data: { threadId: 'thread', turnId: 'turn', itemId: 'item', startedAtMs: 1, cwd: '/tmp/reviewed-directory', permissions } };
 const waiting = { ...session, engine: 'codex' as const, status: 'waiting', pending: [native] };
 install(command => command === 'open_agent_session' || command === 'get_agent_session' ? waiting : undefined);
 render(<AgentWorkbench />); await start('waiting');
 expect(screen.queryByRole('button', { name: 'agent.allow', exact: true })).toBeNull();
 expect(screen.getByText('/tmp/reviewed-directory/input', { exact: true })).toBeTruthy();
 const response = Promise.withResolvers<unknown>();
 install(command => command === 'respond_agent_request' ? response.promise : command === 'get_agent_session' || command === 'open_agent_session' ? waiting : undefined);
 fireEvent.click(screen.getByRole('button', { name: 'agent.allowTurn' }));
 expect(invoke.mock.calls.filter(([command]) => command === 'respond_agent_request')).toEqual([['respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { permissions, scope: 'turn' } } }]]);
 await act(async () => { useAgentRunner.setState({ session: { ...waiting, status: 'unknown', generation: 'replacement' } }); response.resolve(null); });
 expect(screen.queryByRole('button', { name: 'agent.allowTurn' })).toBeNull();
 expect(screen.queryByRole('button', { name: 'agent.allowSession' })).toBeNull();
 expect(useAgentRunner.getState().session?.generation).toBe('replacement');
});

it('offers only native-listed decisions and preserves the selected session scope', async () => {
 const submit = vi.fn(async () => true);
 render(<Request engine="codex" sessionId="session" generation="generation" pending={{ id: 'specific-request', kind: 'item/commandExecution/requestApproval', title: 'native', data: { command: 'echo reviewed', availableDecisions: ['acceptForSession', 'decline'] } }} submit={submit} />);
 expect(screen.queryByRole('button', { name: 'agent.allow', exact: true })).toBeNull();
 expect(screen.queryByRole('button', { name: 'agent.applyPolicy' })).toBeNull();
 fireEvent.click(screen.getByRole('button', { name: 'agent.allowSession' }));
 await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ decision: 'acceptForSession' }));
});

it('does not treat a future terminal reason as success or failure', () => {
 render(<EventContent event={{ seq: 1, run: 'run', kind: 'result', text: null, data: { run_status: 'unknown', is_error: true, terminal_reason: 'future_reason' } }} />);
 expect(screen.getByText('agentExecution.unknown')).toBeTruthy();
 expect(screen.queryByText('agentExecution.replyCompleted')).toBeNull();
 expect(screen.queryByText('agentExecution.failed')).toBeNull();
});

it.each(['completed', 'cancelled', 'failed', 'unknown'])('keeps unfinished operations separate from the %s reply outcome', status => {
 const native: AgentSession = { ...session, engine: 'codex', configuration: null, events: [
  { seq: 1, run: 'run', kind: 'item/started', text: null, data: { id: 'command', type: 'commandExecution', command: 'echo incomplete' } },
  { seq: 2, run: 'run', kind: 'turn/completed', text: null, data: { run_status: status } },
 ] };
 useAgentRunner.setState({ session: native, selected: native.id });
 render(<AgentConversation session={native} drafts={{}} setDraft={vi.fn()} busy={false} error={false} />);
 expect(within(screen.getByTestId('agent-event-1')).getByText(status === 'cancelled' ? 'agentExecution.stopped' : 'agentExecution.unknown')).toBeTruthy();
 expect(within(screen.getByTestId('agent-event-2')).getByText(`agentExecution.${status === 'completed' ? 'replyCompleted' : status === 'cancelled' ? 'stopped' : status}`)).toBeTruthy();
 expect(screen.getByTestId('agent-event-1').querySelector('details')?.open).toBe(false);
});

it('labels command success only as operation completion and reveals output explicitly', async () => {
 render(<EventContent event={{ seq: 1, run: 'run', kind: 'commandExecution', text: null, data: { type: 'commandExecution', command: 'echo native-output', exitCode: 0, aggregatedOutput: 'native output body' } }} />);
 expect(screen.getByText('agentExecution.toolCompleted')).toBeTruthy();
 expect(screen.queryByText('agentExecution.replyCompleted')).toBeNull();
 expect(screen.queryByText('native output body')).toBeNull();
 fireEvent.click(screen.getByText('direct.command'));
 await screen.findByText('native output body');
});

it('keeps a reply unread while the conversation is hidden behind runtime tools', async () => {
 install(); render(<AgentWorkbench />); await start();
 fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.runtime' }));
 await act(async () => {
  const updated = { ...useAgentRunner.getState().session!, events: [{ seq: 1, kind: 'agentMessage', text: 'background reply', data: null }] };
  useAgentRunner.setState({ session: updated, snapshots: { [session.id]: updated }, unread: { [session.id]: true } });
 });
 expect(useAgentRunner.getState().unread[session.id]).toBe(true);
 fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.conversation' }));
 await waitFor(() => expect(useAgentRunner.getState().unread[session.id]).toBe(false));
});

it('shows confirmed supplementary text as user dialogue and never mistakes a steering receipt for a reply', () => {
 render(<><EventContent event={{ seq: 1, run: 'active-run', kind: 'userSteering', text: 'Also keep the public API unchanged.', data: { requestId: 'request', run: 'active-run' } }} /><EventContent event={{ seq: 2, run: 'active-run', kind: 'steeringReceipt', text: 'Native receipt, not an answer', data: { requestId: 'request' } }} /></>);
 expect(screen.getByText('agent.you')).toBeTruthy();
 expect(screen.getByText('Also keep the public API unchanged.')).toBeTruthy();
 expect(screen.queryByText('agent.assistant')).toBeNull();
 expect(screen.queryByText('Native receipt, not an answer')).toBeNull();
});

it('never routes raw reasoning through the failed-operation text fallback', () => {
 const view = render(<EventContent event={{ seq: 1, run: 'run', kind: 'item/reasoning/textDelta', text: 'PRIVATE RAW REASONING', data: { itemId: 'r', error: 'native error', status: 'failed' } }} />);
 expect(view.container.textContent).toBe('');
});

it('opens immutable commit details between tools and the mounted conversation, then restores its draft, scroll and focus', async () => {
 const oid = 'a'.repeat(40);
 install(command => {
  if (command === 'read_development_project_git') return { kind: 'git', path: '/tmp/project', branch: 'main', head: oid, files: [], conflicts: [], snapshot_token: 'snapshot', merge_heads: [] };
  if (command === 'read_development_git_details') return { branches: ['main'], remotes: [], remote_branches: {}, remote_push_targets: {}, remote_fetch_targets: {}, upstream: null, ahead: null, behind: null, files: [], history: [{ oid, subject: 'Inspect immutable history', author: 'Alice', timestamp: 1700000000, parents: [] }], history_truncated: false };
  if (command === 'read_development_git_commit') return { oid, author: 'Alice', author_email: 'alice@example.test', timestamp: 1700000000, message: 'Inspect immutable history\n\nComplete historical body', parents: [], files: [], truncated: false };
  return undefined;
 });
 render(<AgentWorkbench />); await start();
 const composer = screen.getByLabelText('agent.prompt'); fireEvent.change(composer, { target: { value: 'Unsent conversation draft survives commit inspection' } });
 const conversation = screen.getByTestId('dev-panel-agent'); conversation.scrollTop = 73;
 fireEvent.click(screen.getByTestId('dev-tool-files')); fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.git', exact: true })); await act(async () => {});
 const row = screen.getByRole('button', { name: /Inspect immutable history/ }); row.focus(); fireEvent.click(row); await act(async () => {});
 const detail = screen.getByTestId('git-commit-detail'); expect(within(detail).getByText('Complete historical body')).toBeTruthy();
 expect(screen.getByTestId('dev-panel-agent')).toBe(conversation); expect(conversation.scrollTop).toBe(73); expect(composer).toHaveProperty('value', 'Unsent conversation draft survives commit inspection');
 const column = screen.getByTestId('dev-commit-column'); expect(column.previousElementSibling?.id).toBe('dev-navigation'); expect(column.nextElementSibling?.tagName).toBe('MAIN');
 fireEvent.click(screen.getByTestId('git-commit-close')); await waitFor(() => expect(screen.queryByTestId('git-commit-detail')).toBeNull());
 await waitFor(() => expect(document.activeElement).toBe(row));
 expect(screen.getByLabelText('agent.prompt')).toBe(composer); expect(composer).toHaveProperty('value', 'Unsent conversation draft survives commit inspection'); expect(conversation.scrollTop).toBe(73);
 expect(invoke).toHaveBeenCalledWith('read_development_git_commit', { input: { project_id: 'p1', task_id: 't1', oid } });
 expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git' || command === 'create_development_task')).toBe(false);
});

const tabImageFile = () => new File(['local-image-bytes'], 'tab.png', { type: 'image/png' });

it.each([
 { taskless: false, text: 'Inspect the selected image' },
 { taskless: false, text: '' },
 { taskless: true, text: 'Inspect the selected image' },
 { taskless: true, text: '' },
])('hands prepared images to the session before the real parent transition (taskless=$taskless, text="$text")', async ({ taskless, text }) => {
 const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC';
 const selecting = Promise.withResolvers<null>();
 const accepted = Promise.withResolvers<null>();
 const state = { ...workspace, tasks: taskless ? [] : [...workspace.tasks], sessions: [] as { id: string; owner: string; project_id: string; task_id: string; engine: string; native_id: string; workspace: string; model: null; executable: null }[], selection: { project_id: 'p1', task_id: taskless ? null as string | null : 't1', session_id: null as string | null } };
 let native = { ...session };
 const activation = Promise.withResolvers<{ path: string; directory_identity: string; agent: boolean; agent_revoked: boolean; terminal: boolean; service: boolean; revision: number }>();
 install((command, args) => {
  const input = args?.input as Record<string, any>;
  if (command === 'activate_development_workspace_agent') return activation.promise;
  if (command === 'read_development_workspace') return structuredClone(state);
  if (command === 'select_development_context') { state.selection = input as typeof state.selection; return selecting.promise; }
  if (command === 'prepare_agent_task') { const task = { ...workspace.tasks[0], ...input, id: input.id, requirements: '', status: 'todo' }; const existing = state.tasks.find(item => item.id === input.id); if (!existing) state.tasks.push(task); return existing ?? task; }
  if (command === 'open_agent_session') {
   native = { ...session, id: input.id };
   state.sessions.push({ id: input.id, owner: 'u', project_id: 'p1', task_id: input.task_id, engine: input.engine, native_id: 'native', workspace: '/tmp/project', model: null, executable: null });
   return native;
  }
  if (command === 'get_agent_session') return native;
  if (command === 'send_agent_message') return accepted.promise;
  return undefined;
 });
 vi.spyOn(chatAttachments, 'fileBase64').mockResolvedValue(png);
 render(<AgentWorkbench />);
 await screen.findByTestId('direct-conversation-send');
 const tabId = useDevelopment.getState().activeTabId!;
 const source = agentImagesKey('p1', tabId);
 fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: text } });
 fireEvent.change(await screen.findByTestId('agent-image-picker'), { target: { files: [tabImageFile()] } });
 await screen.findByTestId('agent-image-preview-0');
 const captured = captureAgentImages(source);
 expect(captured).toHaveLength(1);
 await waitForAgentTrust('p1', taskless ? null : 't1');
 await act(async () => { activation.resolve({ path: '/tmp/project', directory_identity: 'd', agent: true, agent_revoked: false, terminal: false, service: false, revision: 1 }); });
 await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'select_development_context')).toHaveLength(1));
 const target = agentImagesKey('p1', native);
 // Production chooseContext has already unmounted ProjectConversation while its
 // selection receipt is pending. The live-session owner must already hold bytes.
 await waitFor(() => expect(screen.queryByTestId('dev-project-conversation')).toBeNull());
 expect(captureAgentImages(source)).toEqual([]);
 expect(captureAgentImages(target)).toEqual(captured);
 expect(useDevelopmentDrafts.getState().entries[target]).toBeDefined();
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
 await act(async () => { selecting.resolve(null); });
 await screen.findByRole('button', { name: 'agent.send' });
 expect(useDevelopment.getState().activeTabId).toBe(tabId);
 expect(screen.getByLabelText('agent.prompt')).toHaveProperty('value', text);
 expect(screen.getByTestId('agent-image-preview-0')).toBeTruthy();
 fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
 await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1));
 expect(invoke.mock.calls.find(([command]) => command === 'send_agent_message')![1].input).toMatchObject({ session_id: native.id, generation: native.generation, text, images: [{ mime: 'image/png', base64: png }] });
 expect(captureAgentImages(target)).toEqual(captured);
 await act(async () => { accepted.resolve(null); });
 await waitFor(() => expect(captureAgentImages(target)).toEqual([]));
 expect(useDevelopmentDrafts.getState().entries[target]).toBeUndefined();
 expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1);
 expect(invoke.mock.calls.filter(([command]) => command === 'prepare_agent_task')).toHaveLength(taskless ? 1 : 0);
});

it.each(['draft', 'session'] as const)('hides an image-only %s tab without discarding either image owner or stopping its runtime', async kind => {
 install(); render(<AgentWorkbench />);
 await screen.findByTestId('direct-conversation-send');
 if (kind === 'session') await start();
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 const owner = agentImagesKey('p1', kind === 'session' ? { id: session.id, generation: session.generation } : tab.id);
 const otherOwner = agentImagesKey('p1', 'another-unsent-tab');
 await act(async () => { await addAgentImages(owner, [tabImageFile()]); await addAgentImages(otherOwner, [tabImageFile()]); });
 const captured = captureAgentImages(owner);
 const confirm = vi.spyOn(Modal, 'confirm');
 await act(async () => { fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' }); });
 await waitFor(() => expect(screen.queryByTestId(`conversation-tab-${tab.id}`)).toBeNull());
 expect(confirm).not.toHaveBeenCalled();
 expect(captureAgentImages(owner)).toEqual(captured);
 expect(useDevelopmentDrafts.getState().entries[owner]).toBeDefined();
 expect(captureAgentImages(otherOwner)).toHaveLength(1);
 if (kind === 'session') expect(useAgentRunner.getState().snapshots[session.id]).toMatchObject({ generation: session.generation, status: 'idle' });
 await act(async () => {
  await chooseContext({ project_id: tab.projectId, task_id: tab.taskId, session_id: tab.sessionId });
  if (kind === 'draft') useDevelopment.getState().activateTab(tab.id);
  else useDevelopment.getState().openTab(useDevelopment.getState().sessions.find(item => item.id === session.id)!);
 });
 expect(screen.getByTestId('agent-image-preview-0')).toBeTruthy();
 expect(captureAgentImages(owner)).toEqual(captured);
 expect(invoke.mock.calls.some(([command]) => ['send_agent_message', 'close_agent_session', 'interrupt_agent_session'].includes(command))).toBe(false);
});

it('retains a pending image import when hiding its draft and accepts the late bytes for the same owner', async () => {
 install(); render(<AgentWorkbench />); await screen.findByTestId('direct-conversation-send');
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 const owner = agentImagesKey('p1', tab.id);
 const read = Promise.withResolvers<string>();
 vi.spyOn(chatAttachments, 'fileBase64').mockReturnValueOnce(read.promise);
 let importing!: Promise<void>;
 act(() => { importing = addAgentImages(owner, [tabImageFile()]); });
 expect(useAgentImages.getState().drafts[owner].busy).toBe(true);
 const confirm = vi.spyOn(Modal, 'confirm');
 await act(async () => { fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' }); });
 await waitFor(() => expect(screen.queryByTestId(`conversation-tab-${tab.id}`)).toBeNull());
 expect(confirm).not.toHaveBeenCalled();
 await act(async () => { read.resolve('aW1hZ2U='); await importing; });
 expect(captureAgentImages(owner)).toHaveLength(1);
 expect(useDevelopmentDrafts.getState().entries[owner]).toBeDefined();
 expect(useDevelopment.getState().tabs.find(item => item.id === tab.id)?.hidden).toBe(true);
});

it('retains images added while a tab close is awaiting host persistence', async () => {
 const saved = Promise.withResolvers<{ revision: number; value: Record<string, unknown> }>();
 let closing = false;
 let snapshot!: { expected_revision: number; value: Record<string, unknown> };
 install((command, args) => { if (command === 'save_development_desktop_state' && closing) { snapshot = args!.input as typeof snapshot; return saved.promise; } });
 render(<AgentWorkbench />); await screen.findByTestId('direct-conversation-send');
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 const owner = agentImagesKey('p1', tab.id);
 await act(async () => { await addAgentImages(owner, [tabImageFile()]); });
 closing = true;
 fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' });
 await waitFor(() => expect(snapshot).toBeDefined());
 expect(screen.getByTestId(`conversation-tab-${tab.id}`)).toBeTruthy();
 await act(async () => { await addAgentImages(owner, [tabImageFile()]); closing = false; saved.resolve({ revision: snapshot.expected_revision + 1, value: snapshot.value }); });
 await waitFor(() => expect(screen.queryByTestId(`conversation-tab-${tab.id}`)).toBeNull());
 expect(captureAgentImages(owner)).toHaveLength(2);
 expect(useDevelopmentDrafts.getState().entries[owner]).toBeDefined();
});

it('hides an unknown image send without losing its receipt or permitting replay when reopened', async () => {
 install(); render(<AgentWorkbench />); await start();
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 const owner = agentImagesKey('p1', { id: session.id, generation: session.generation });
 await act(async () => { await addAgentImages(owner, [tabImageFile()]); });
 const key = developmentDraftKey('conversation', 'p1', session.id, session.generation, 'pending-send');
 act(() => useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { requestId: 'unconfirmed-request', generation: session.generation, text: '', references: [], skills: [], imageOwner: owner, images: captureAgentImages(owner).map(image => ({ id: image.id })) }, baseline: null } } })));
 const confirm = vi.spyOn(Modal, 'confirm');
 await act(async () => { fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' }); });
 expect(confirm).not.toHaveBeenCalled();
 await waitFor(() => expect(screen.queryByTestId(`conversation-tab-${tab.id}`)).toBeNull());
 expect(captureAgentImages(owner)).toHaveLength(1);
 expect(useDevelopmentDrafts.getState().entries[key]).toBeDefined();
 await act(async () => { await chooseContext({ project_id: 'p1', task_id: 't1', session_id: session.id }); });
 const prompt = screen.getByRole('textbox', { name: 'agent.prompt' });
 fireEvent.change(prompt, { target: { value: 'Do not replay the unknown image turn' } });
 fireEvent.keyDown(prompt, { key: 'Enter', ctrlKey: true });
 expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', true);
 expect(captureAgentImages(owner)).toHaveLength(1);
 expect(useDevelopmentDrafts.getState().entries[key]).toBeDefined();
 expect(invoke.mock.calls.some(([command]) => ['send_agent_message', 'close_agent_session', 'interrupt_agent_session'].includes(command))).toBe(false);
});

it('keeps unknown steering protected from resend while allowing its tab to hide and reopen', async () => {
 install(); render(<AgentWorkbench />); await start();
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Unconfirmed additional instruction' } });
 const key = developmentDraftKey('conversation', 'p1', session.id, session.generation, 'pending-steer');
 act(() => useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { requestId: 'steer-request', runId: 'previous-run', text: 'Unconfirmed additional instruction' }, baseline: null } } })));
 expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', true);
 fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
 const confirm = vi.spyOn(Modal, 'confirm');
 await act(async () => { fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' }); });
 expect(confirm).not.toHaveBeenCalled();
 await waitFor(() => expect(screen.queryByTestId(`conversation-tab-${tab.id}`)).toBeNull());
 await act(async () => { await chooseContext({ project_id: 'p1', task_id: 't1', session_id: session.id }); });
 expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toHaveProperty('value', 'Unconfirmed additional instruction');
 expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', true);
 fireEvent.keyDown(screen.getByRole('textbox', { name: 'agent.prompt' }), { key: 'Enter', ctrlKey: true });
 expect(useDevelopmentDrafts.getState().entries[key]).toBeDefined();
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
});

it('preserves latest input and display preferences during a model replacement without carrying full automation into the new session', async () => {
 const closed = Promise.withResolvers<boolean>();
 install((command, args) => {
  if (command === 'close_agent_session') return closed.promise;
  if (command === 'read_development_cli_provider_catalog' && args && 'engine' in args) return { project_id: 'p1', engine: args.engine, providers: [{ id: 'provider1', name: 'Provider One', enabled: true, config_revision: 3, availability: { available: true, reason: null }, models: [{ id: 'model1', name: 'Model One' }, { id: 'model2', name: 'Model Two' }] }] };
  return undefined;
 });
 render(<MemoryRouter><AgentWorkbench /></MemoryRouter>); await start();
 const key = developmentDraftKey('conversation', 'p1', session.id, 'turn-settings');
 act(() => {
  useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { permission_mode: 'full_auto', reasoning_effort: 'high' }, baseline: {} } } }));
  setDesktopSlot(agentChatDisplayKey('p1', session.id), { stream_response: false, code_highlighting: false });
 });
 fireEvent.change(screen.getByRole('textbox', { name: 'agent.prompt' }), { target: { value: 'Original draft' } });
 fireEvent.click(screen.getByTestId('composer-model-picker-trigger'));
 fireEvent.click(await screen.findByRole('button', { name: 'Model Two', exact: true }));
 await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'close_agent_session')).toBe(true));
 fireEvent.change(screen.getByRole('textbox', { name: 'agent.prompt' }), { target: { value: 'Latest draft during close' } });
 await act(async () => closed.resolve(true));
 const tabId = useDevelopment.getState().activeTabId!;
 expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', 'p1', tabId, 'turn-settings')]?.value).toEqual({ permission_mode: 'default', reasoning_effort: 'high' });
 expect(agentChatPreferences({ projectId: 'p1', draftId: tabId })).toMatchObject({ stream_response: false, code_highlighting: false });
 expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toHaveProperty('value', 'Latest draft during close');
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
});

it('does not navigate back to a stale model replacement after the source context changes during close', async () => {
 const closed = Promise.withResolvers<boolean>();
 install((command, args) => {
  if (command === 'close_agent_session') return closed.promise;
  if (command === 'read_development_cli_provider_catalog' && args && 'engine' in args) return { project_id: 'p1', engine: args.engine, providers: [{ id: 'provider1', name: 'Provider One', enabled: true, config_revision: 3, availability: { available: true, reason: null }, models: [{ id: 'model2', name: 'Model Two' }] }] };
  return undefined;
 });
 render(<MemoryRouter><AgentWorkbench /></MemoryRouter>); await start();
 fireEvent.click(screen.getByTestId('composer-model-picker-trigger'));
 fireEvent.click(await screen.findByRole('button', { name: 'Model Two', exact: true }));
 await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'close_agent_session')).toBe(true));
 await act(async () => { await chooseContext({ project_id: null, task_id: null, session_id: null }); });
 const tabs = useDevelopment.getState().tabs;
 await act(async () => closed.resolve(true));
 expect(useDevelopment.getState().selection).toEqual({ project_id: null, task_id: null, session_id: null });
 expect(useDevelopment.getState().tabs).toBe(tabs);
});

it('prepares a saved session after explicit agent re-enablement observes the confirmed grant', async () => {
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 let authorized = false;
 install(command => {
  if (command === 'read_development_workspace') return { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } };
  if (command === 'set_development_workspace_trust') authorized = true;
  if (command === 'read_development_workspace_trust' || command === 'set_development_workspace_trust') return { path: '/tmp/project', directory_identity: 'd', agent: authorized, agent_revoked: !authorized, terminal: false, service: false, revision: authorized ? 3 : 2 };
  return undefined;
 });
 render(<AgentWorkbench />);
 const composer = await screen.findByTestId('saved-conversation-composer');
 fireEvent.click(await within(composer).findByRole('button', { name: 'desktop.trust.start' }));
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('open_agent_session', { input: { intent: 'resume', session_id: saved.id, trust_workspace: false } }));
 expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1);
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message' || command === 'activate_development_workspace_agent')).toBe(false);
});

it('keeps a queued saved message as a draft when its work surface hides before resume completes', async () => {
 const opened = Promise.withResolvers<AgentSession>();
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install(command => command === 'read_development_workspace' ? { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } } : command === 'read_development_workspace_trust' ? { path: '/tmp/project', directory_identity: 'd', agent: true, agent_revoked: false, terminal: false, service: false, revision: 1 } : command === 'open_agent_session' ? opened.promise : undefined);
 const controller = createWorkspaceController();
 const tree = (active: boolean) => <SurfaceScope id="cli-agent" active={active} controller={controller}><AgentWorkbench /></SurfaceScope>;
 const view = render(tree(true));
 const composer = await screen.findByTestId('saved-conversation-composer');
 const prompt = within(composer).getByRole('textbox', { name: 'agent.prompt' });
 fireEvent.focus(prompt);
 await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1));
 fireEvent.change(prompt, { target: { value: 'Keep until I return' } });
 fireEvent.keyDown(prompt, { key: 'Enter', ctrlKey: true });
 view.rerender(tree(false));
 await act(async () => opened.resolve({ ...session, engine: 'codex' }));
 expect(invoke.mock.calls.some(([command]) => ['send_agent_message', 'interrupt_agent_session', 'close_agent_session'].includes(command))).toBe(false);
 expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', 'p1', session.id, 'message')]?.value).toBe('Keep until I return');
});

it('does not open a replaced reply resource after its host resolver returns late, but opens the current resource', async () => {
 const resource = (name: string): AgentSession['events'][number] => ({ seq: 1, kind: 'mcpToolCall', run: 'resource-run', text: null, data: { type: 'mcpToolCall', tool: 'Artifact tool', status: 'completed', result: { content: [{ type: 'resource_link', uri: `file:///tmp/project/${name}` }] } } });
 let native: AgentSession = { ...session, engine: 'codex', events: [resource('old.txt')] };
 const resolved = Promise.withResolvers<{ path: string; line: number | null }>();
 install((command, args) => {
  if (command === 'open_agent_session' || command === 'get_agent_session') return native;
  if (command === 'resolve_development_file_reference') {
   const input = args!.input as { path: string };
   return input.path.endsWith('/old.txt') ? resolved.promise : { path: 'new.txt', line: null };
  }
  if (command === 'read_development_file') { const input = args!.input as { path: string }; return { path: input.path, text: 'Current resource', hash: 'resource-hash', size: 16, reason: null }; }
  return undefined;
 });
 render(<MemoryRouter><AgentWorkbench /></MemoryRouter>);
 await start();
 fireEvent.click(screen.getByText('Artifact tool'));
 fireEvent.click(await screen.findByRole('button', { name: 'agentResources.openFile' }));
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('resolve_development_file_reference', { input: expect.objectContaining({ session_id: session.id, path: '/tmp/project/old.txt', mode: 'file' }) }));
 const documents = useDevelopmentEditor.getState().tabs;
 native = { ...native, events: [resource('new.txt')] };
 act(() => useAgentRunner.setState({ session: native, snapshots: { [native.id]: native } }));
 expect(await screen.findByText('file:///tmp/project/new.txt')).toBeTruthy();
 await act(async () => resolved.resolve({ path: 'old.txt', line: null }));
 expect(useDevelopmentEditor.getState().tabs).toBe(documents);
 expect(invoke.mock.calls.some(([command, args]) => command === 'read_development_file' && args.input.path === 'old.txt')).toBe(false);
 fireEvent.click(screen.getByRole('button', { name: 'agentResources.openFile' }));
 await waitFor(() => expect(useDevelopmentEditor.getState().tabs.some(tab => tab.path === 'new.txt')).toBe(true));
 expect(invoke).toHaveBeenCalledWith('read_development_file', { input: expect.objectContaining({ path: 'new.txt' }) });
 expect(useDevelopmentEditor.getState().tabs.some(tab => tab.path === 'old.txt')).toBe(false);
});
