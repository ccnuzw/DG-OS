import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Modal } from '@dgos/ui';
import * as chatAttachments from '../src/components/chat/Attachments';
import { addAgentImages, agentImagesKey, captureAgentImages, useAgentImages } from '../src/stores/agentImages';
import AgentWorkbench from '../src/pages/workbench/AgentWorkbench';
import { AgentConversation, EventContent, Request } from '../src/pages/workbench/AgentConversation';
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
import { useCapabilityEntry } from '../src/components/chat/RuntimeNavigation';
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
 invoke.mockImplementation(async (command: string, args?: { input?: unknown; sessionId?: string }) => {
  const result = override(command, args);
  if (result !== undefined) return result;
  if (command === 'read_development_workspace') return { ...workspace, sessions: Object.values(useAgentRunner.getState().snapshots).map(view => ({ id: view.id, owner: 'u', project_id: 'p1', task_id: 't1', engine: view.engine, native_id: view.native_id, workspace: view.workspace, model: null, executable: null })), selection: selected };
  if (command === 'select_development_context') { selected = args!.input as typeof selected; return null; }
  if (command === 'read_development_workspaces') return { workspaces: [{ id: 'w1', path: '/tmp/project', directory_identity: 'd', kind: 'original' }], selected_workspace_id: 'w1', operation: null, operations: [] };
  if (command === 'inspect_development_project') return inspection;
  if (command === 'open_agent_session' || command === 'get_agent_session') return { ...session, id: args?.sessionId ?? session.id };
  if (command === 'read_development_cli_provider') return providerRevision;
  if (command === 'read_development_cli_provider_catalog') return { project_id: 'p1', engine: 'codex', providers: [{ id: 'provider1', name: 'Provider One', enabled: true, config_revision: 3, availability: { available: true, reason: null }, models: [{ id: 'model1', name: 'Model One', model_type: 'chat' }] }] };
  if (command === 'get_development_session_configuration') return { format: 'provider_bound', read_only: false, snapshot: { binding_id: 'binding1', engine: 'codex', ...providerRevision.configuration.provider, revision: 4, permission_epoch: 2 } };
  if (command === 'read_development_git') return { task_id: 't1', kind: 'directory', branch: null, head: null, files: [], conflicts: [], staged_diff: '', worktree_diff: '', initial_staged_paths: [], file_versions: {}, baseline_version: '' };
  if (command === 'list_development_processes') return [];
  if (command === 'read_development_session_summaries') return [];
  if (command === 'read_development_project_git') return { kind: 'directory', branch: null, head: null, files: [], conflicts: [], staged_diff: '', worktree_diff: '' };
  if (command === 'list_development_files') return { entries: [], truncated: false };
  if (command === 'list_development_runs') return [];
  if (command === 'read_development_review') return { task_id: 't1', captured_at: null, baseline_late: false, truncated: false, fingerprint: '', changes: [], runs_stale: false };
  if (command === 'read_development_commands') return { commands: [], discovered: [] };
  return undefined;
 });
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
 it('requires workspace trust before creating a runtime', async () => {
  install(); render(<AgentWorkbench />);
  const button = await screen.findByTestId('direct-conversation-send') as HTMLButtonElement;
  fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'First request requires explicit trust' } });
  expect(button.disabled).toBe(true);
  expect(invoke.mock.calls.some(([name]) => name === 'open_agent_session')).toBe(false);
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
  await act(async () => { resolveWorkspace({ workspaces: [{ id: 'w2', path: '/tmp/task-worktree', kind: 'managed' }], selected_workspace_id: 'w2', operation: null }); });
  await waitFor(() => expect(screen.getByTestId('dev-context-details').textContent).toContain('/tmp/task-worktree'));
  expect((screen.getByTestId('direct-conversation-trust') as HTMLInputElement).checked).toBe(false);
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

it('starts from a taskless project through the task drawer without losing the draft or duplicating admission', async () => {
 const state = { ...workspace, tasks: [] as typeof workspace.tasks, sessions: [] as { id: string; owner: string; project_id: string; task_id: string; engine: string; native_id: string; workspace: string; model: null; executable: null }[], selection: { project_id: 'p1', task_id: null as string | null, session_id: null as string | null } };
 let native = { ...session };
 install((command, args) => {
  const input = args?.input as Record<string, any>;
  if (command === 'read_development_workspace') return structuredClone(state);
  if (command === 'select_development_context') { state.selection = input as typeof state.selection; return null; }
  if (command === 'save_development_task') { const task = { ...workspace.tasks[0], ...input, id: 'automatic-task' }; state.tasks.push(task); return task; }
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
 await waitFor(() => expect(screen.getByTestId('direct-conversation-trust')).toHaveProperty('disabled', false));
 const tab = useDevelopment.getState().activeTabId;
 const prompt = screen.getByRole('textbox', { name: 'agent.prompt' });
 fireEvent.change(prompt, { target: { value: 'Explain this project without a task form' } });
 fireEvent.click(screen.getByTestId('dev-tool-tasks'));
 fireEvent.click(screen.getByTestId('dev-task-conversation'));
 await waitFor(() => expect(document.activeElement).toBe(prompt));
 expect(prompt).toHaveProperty('value', 'Explain this project without a task form');
 expect(useDevelopment.getState().activeTabId).toBe(tab);
 expect(state.tasks).toEqual([]);
 expect(invoke.mock.calls.some(([name]) => ['save_development_task', 'open_agent_session', 'send_agent_message'].includes(name))).toBe(false);
 fireEvent.click(screen.getByTestId('direct-conversation-trust'));
 await waitFor(() => expect(screen.getByTestId('direct-conversation-send')).toHaveProperty('disabled', false));
 const send = screen.getByTestId('direct-conversation-send');
 await act(async () => { fireEvent.click(send); fireEvent.click(send); });
 await waitFor(() => expect(useDevelopment.getState().selection.session_id).toBe(native.id));
 expect(state.tasks).toEqual([expect.objectContaining({ project_id: 'p1', goal: 'Explain this project without a task form', requirements: '' })]);
 expect(useDevelopment.getState().activeTabId).toBe(tab);
 expect(useDevelopment.getState().tabs.find(item => item.id === tab)?.sessionId).toBe(native.id);
 for (const command of ['save_development_task', 'open_agent_session', 'send_agent_message']) expect(invoke.mock.calls.filter(([name]) => name === command)).toHaveLength(1);
 expect(invoke.mock.calls.some(([name]) => ['close_agent_session', 'interrupt_agent_session'].includes(name))).toBe(false);
});

it.each([[true, false], [false, true], [false, false]])('clears the Agent running marker when development is hidden (tab mounted: %s, surface active: %s)', async (tabMounted, active) => {
 vi.useFakeTimers();
 let running = true;
 const process = (): DevelopmentProcess => ({ id: 'host-process', project_id: 'p1', task_id: 't1', kind: 'command', source: 'host', status: running ? 'running' : 'exited', output: 'host output', truncated: false, exit_code: running ? null : 0, reason: null, started_at: 1, finished_at: running ? null : 2, program: 'echo', args: [], cwd: '.', fingerprint: '', start_fingerprint: '', verification_status: 'not_requested', verification_complete: false, mutation_observed: false, preview_url: null });
 install(command => command === 'list_development_processes' ? [process()] : command === 'read_development_process' ? process() : undefined);
 const controller = createWorkspaceController();
 const tree = (surfaceActive: boolean) => <SurfaceScope id="cli-agent" active={surfaceActive} controller={controller}><AgentWorkbench /></SurfaceScope>;
 const view = render(tree(true));
 await act(async () => {});
 await act(async () => { fireEvent.click(screen.getByTestId('dev-tool-tasks')); });
 expect(screen.getByTestId('dev-development-runtime')).toBeTruthy();
 expect(controller.getSurfaceStatuses()['cli-agent']?.running).toBe(true);
 const reads = (command: string) => invoke.mock.calls.filter(([name]) => name === command).length;
 // One list read each second even while both the runtime and its outer consumer are mounted.
 const visibleHistory = reads('list_development_processes');
 await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
 expect(reads('list_development_processes') - visibleHistory).toBe(2);
 if (!tabMounted) {
  await act(async () => { fireEvent.click(screen.getByTestId('dev-tool-tasks')); });
  expect(document.getElementById('dev-navigation')?.hasAttribute('hidden')).toBe(true);
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
 install(command => command === 'list_development_processes' ? current ? [current] : [] : command === 'start_development_process' ? started.promise : undefined);
 const controller = createWorkspaceController();
 render(<SurfaceScope id="cli-agent" active controller={controller}><AgentWorkbench /></SurfaceScope>);
 await act(async () => {});
 await act(async () => { fireEvent.click(screen.getByTestId('dev-tool-tasks')); });
 fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
 fireEvent.click(screen.getByTestId('dev-process-trust'));
 fireEvent.click(screen.getByTestId('dev-process-start'));
 expect(invoke.mock.calls.filter(([name]) => name === 'start_development_process')).toHaveLength(1);
 const submittedId = invoke.mock.calls.find(([name]) => name === 'start_development_process')![1].input.id;
 await act(async () => { fireEvent.click(screen.getByTestId('dev-tool-tasks')); });
 expect(document.getElementById('dev-navigation')?.hasAttribute('hidden')).toBe(true);
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
 await act(async () => { fireEvent.click(screen.getByTestId('dev-tool-tasks')); });
 await act(async () => { fireEvent.click(screen.getByTestId('dev-tool-tasks')); });
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
 await act(async () => { fireEvent.click(screen.getByTestId('dev-tool-tasks')); });
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
  fireEvent.click(screen.getByTestId('dev-tool-tasks'));
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
  const skills = useCapabilityEntry('/skills'), mcp = useCapabilityEntry('/mcp');
  const entry = location.pathname === '/skills' ? skills : mcp;
  return <>
   {tabs.surfaces.some(item => item.id === 'cli-agent') && <div hidden={tabs.backgroundSurfaceId !== 'cli-agent'}><SurfaceScope id="cli-agent" active={tabs.activeSurfaceId === 'cli-agent'} controller={tabs.controller}><AgentWorkbench /></SurfaceScope></div>}
   <output data-testid="management-entry">{JSON.stringify({ path: location.pathname, runtime: entry.runtime, selection: entry.nativeSelection, sessionId: entry.sessionId })}</output>
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
 for (const [label, path] of [['nav.skills', '/skills'], ['nav.mcp', '/mcp']] as const) {
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.more' }));
  fireEvent.click(within(screen.getByRole('navigation', { name: 'agentControls.management' })).getByRole('link', { name: label }));
  await waitFor(() => expect(screen.getByTestId('management-entry').textContent).toBe(JSON.stringify({ path, runtime: 'native', selection: { target: engine === 'codex' ? 'Codex' : 'Claude', scope: 'ProjectShared', project_id: 'p1' }, sessionId: session.id })));
  fireEvent.click(screen.getByRole('button', { name: 'Return to conversation' }));
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
 expect(screen.getByText('agent.requestDirectory: /tmp/reviewed-directory')).toBeTruthy();
 expect(screen.getByText('Native permission reason')).toBeTruthy();
 fireEvent.click(screen.getByRole('button', { name: decision === 'accept' ? 'agent.allow' : decision === 'decline' ? 'agent.deny' : 'common.cancel' }));
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { decision } } }));
});

it('dispatches one new session for repeated clicks before the open acknowledgement', async () => {
 const opened = Promise.withResolvers<unknown>();
 install(command => command === 'open_agent_session' ? opened.promise : undefined);
 render(<AgentWorkbench />);
 await waitFor(() => expect(screen.getByTestId('direct-conversation-trust')).toHaveProperty('disabled', false));
 fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Send exactly once' } });
 fireEvent.click(screen.getByTestId('direct-conversation-trust'));
 const create = screen.getByTestId('direct-conversation-send');
 await act(async () => { fireEvent.click(create); fireEvent.click(create); });
 expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1);
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

it('restores a saved native conversation explicitly without creating or sending a new session', async () => {
 const saved = { id: session.id, owner: 'u', engine: 'codex', native_id: 'saved-native', workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't1' };
 install(command => command === 'read_development_workspace' ? { ...workspace, sessions: [saved], selection: { ...workspace.selection, session_id: saved.id } } : undefined);
 render(<AgentWorkbench />);
 await screen.findByTestId('dev-resume-entry');
 expect(invoke.mock.calls.some(([command]) => command === 'open_agent_session' || command === 'send_agent_message')).toBe(false);
 fireEvent.click(within(screen.getByTestId('dev-resume-entry')).getByRole('checkbox', { name: 'dev.trustProject' }));
 await act(async () => { fireEvent.click(within(screen.getByTestId('dev-resume-entry')).getByRole('button', { name: 'dev.trustResume' })); });
 expect(invoke).toHaveBeenCalledWith('open_agent_session', { input: { intent: 'resume', session_id: saved.id, trust_workspace: true } });
 expect(invoke.mock.calls.filter(([command]) => command === 'open_agent_session')).toHaveLength(1);
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
 expect(useAgentRunner.getState().session?.id).toBe(saved.id);
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
  if (command === 'read_development_git') return { task_id: 't1', kind: 'git', path: '/tmp/project', branch: 'main', head: oid, files: [], conflicts: [], staged_diff: '', worktree_diff: '', initial_staged_paths: [], file_versions: {}, baseline_version: '' };
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
 install((command, args) => {
  const input = args?.input as Record<string, any>;
  if (command === 'read_development_workspace') return structuredClone(state);
  if (command === 'select_development_context') { state.selection = input as typeof state.selection; return selecting.promise; }
  if (command === 'save_development_task') { const task = { ...workspace.tasks[0], ...input, id: 'image-task' }; state.tasks.push(task); return task; }
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
 await waitFor(() => expect(screen.getByTestId('direct-conversation-trust')).toHaveProperty('disabled', false));
 const tabId = useDevelopment.getState().activeTabId!;
 const source = agentImagesKey('p1', tabId);
 fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: text } });
 fireEvent.change(screen.getByTestId('agent-image-picker'), { target: { files: [tabImageFile()] } });
 await screen.findByTestId('agent-image-preview-0');
 const captured = captureAgentImages(source);
 expect(captured).toHaveLength(1);
 fireEvent.click(screen.getByTestId('direct-conversation-trust'));
 await waitFor(() => expect(screen.getByTestId('direct-conversation-prepare')).toHaveProperty('disabled', false));
 fireEvent.click(screen.getByTestId('direct-conversation-prepare'));
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
 expect(invoke.mock.calls.filter(([command]) => command === 'save_development_task')).toHaveLength(taskless ? 1 : 0);
});

it.each(['draft', 'session'] as const)('confirms image-only %s tab discard and clears only the approved image owner', async kind => {
 install(); render(<AgentWorkbench />);
 await screen.findByTestId('direct-conversation-send');
 if (kind === 'session') await start();
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 const owner = agentImagesKey('p1', kind === 'session' ? { id: session.id, generation: session.generation } : tab.id);
 const otherOwner = agentImagesKey('p1', 'another-unsent-tab');
 await act(async () => { await addAgentImages(owner, [tabImageFile()]); await addAgentImages(otherOwner, [tabImageFile()]); });
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'image-close', close: () => {} }; });
 fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' });
 await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
 expect(useDevelopment.getState().tabs.some(item => item.id === tab.id)).toBe(true);
 expect(captureAgentImages(owner)).toHaveLength(1);
 expect(useDevelopmentDrafts.getState().entries[owner]).toBeDefined();
 confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'image-close', close: () => {} }; });
 await act(async () => { fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' }); });
 await waitFor(() => expect(useDevelopment.getState().tabs.some(item => item.id === tab.id)).toBe(false));
 expect(useAgentImages.getState().drafts[owner]).toBeUndefined();
 expect(useDevelopmentDrafts.getState().entries[owner]).toBeUndefined();
 expect(captureAgentImages(otherOwner)).toHaveLength(1);
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message' || command === 'close_agent_session')).toBe(false);
});

it('confirms and cancels an image import when closing its draft without allowing a late read to recreate it', async () => {
 install(); render(<AgentWorkbench />); await screen.findByTestId('direct-conversation-send');
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 const owner = agentImagesKey('p1', tab.id);
 const read = Promise.withResolvers<string>();
 vi.spyOn(chatAttachments, 'fileBase64').mockReturnValueOnce(read.promise);
 let importing!: Promise<void>;
 act(() => { importing = addAgentImages(owner, [tabImageFile()]); });
 expect(useAgentImages.getState().drafts[owner].busy).toBe(true);
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'image-close', close: () => {} }; });
 await act(async () => { fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' }); });
 await waitFor(() => expect(useDevelopment.getState().tabs.some(item => item.id === tab.id)).toBe(false));
 expect(confirm).toHaveBeenCalledTimes(1);
 await act(async () => { read.resolve('aW1hZ2U='); await importing; });
 expect(useAgentImages.getState().drafts[owner]).toBeUndefined();
 expect(useDevelopmentDrafts.getState().entries[owner]).toBeUndefined();
});

it('retains images added while a tab discard confirmation is open', async () => {
 install(); render(<AgentWorkbench />); await screen.findByTestId('direct-conversation-send');
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 const owner = agentImagesKey('p1', tab.id);
 await act(async () => { await addAgentImages(owner, [tabImageFile()]); });
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(() => ({ id: 'image-close', close: () => {} }));
 fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' });
 await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
 await act(async () => { await addAgentImages(owner, [tabImageFile()]); confirm.mock.calls[0][0].onConfirm?.(); });
 expect(useDevelopment.getState().tabs.some(item => item.id === tab.id)).toBe(true);
 expect(captureAgentImages(owner)).toHaveLength(2);
});

it('keeps an unknown session image send protected from tab discard', async () => {
 install(); render(<AgentWorkbench />); await start();
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 const owner = agentImagesKey('p1', { id: session.id, generation: session.generation });
 await act(async () => { await addAgentImages(owner, [tabImageFile()]); });
 const key = developmentDraftKey('conversation', 'p1', session.id, session.generation, 'pending-send');
 act(() => useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { requestId: 'unconfirmed-request', generation: session.generation, text: '', references: [], skills: [], imageOwner: owner, images: captureAgentImages(owner).map(image => ({ id: image.id })) }, baseline: null } } })));
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'image-close', close: () => {} }; });
 await act(async () => { fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' }); });
 expect(confirm).not.toHaveBeenCalled();
 expect(useDevelopment.getState().tabs.some(item => item.id === tab.id)).toBe(true);
 expect(captureAgentImages(owner)).toHaveLength(1);
 expect(useDevelopmentDrafts.getState().entries[key]).toBeDefined();
});

it('blocks normal resend and tab discard while steering has no authoritative receipt', async () => {
 install(); render(<AgentWorkbench />); await start();
 const tab = useDevelopment.getState().tabs.find(item => item.id === useDevelopment.getState().activeTabId)!;
 fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Unconfirmed additional instruction' } });
 const key = developmentDraftKey('conversation', 'p1', session.id, session.generation, 'pending-steer');
 act(() => useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { requestId: 'steer-request', runId: 'previous-run', text: 'Unconfirmed additional instruction' }, baseline: null } } })));
 expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', true);
 fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'steer-close', close: () => {} }; });
 await act(async () => { fireEvent.keyDown(screen.getByTestId(`conversation-tab-${tab.id}`), { key: 'Delete' }); });
 expect(confirm).not.toHaveBeenCalled();
 expect(useDevelopment.getState().tabs.some(item => item.id === tab.id)).toBe(true);
 expect(useDevelopmentDrafts.getState().entries[key]).toBeDefined();
 expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
});
