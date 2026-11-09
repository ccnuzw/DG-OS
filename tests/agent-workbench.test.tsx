import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import AgentWorkbench from '../src/pages/workbench/AgentWorkbench';
import { AgentConversation, EventContent, Request } from '../src/pages/workbench/AgentConversation';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { chooseContext } from '../src/stores/developmentSelection';
import { useDevelopmentRuntime, type DevelopmentProcess } from '../src/stores/developmentRuntime';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
import type { CliProviderRevision } from '../src/stores/developmentCapabilities';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
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
  if (command === 'read_development_git') return { task_id: 't1', kind: 'directory', branch: null, head: null, conflicts: [] };
  if (command === 'list_development_processes') return [];
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
 invoke.mockImplementation(async command => command === 'respond_agent_request' ? response.promise : waiting);
 fireEvent.click(screen.getByRole('button', { name: 'agent.allowTurn' }));
 expect(screen.queryByRole('button', { name: 'agent.allowSession' })).toBeNull();
 expect(invoke.mock.calls.filter(([command]) => command === 'respond_agent_request')).toEqual([['respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: native.id, response: { permissions, scope: 'turn' } } }]]);
 await act(async () => { useAgentRunner.setState({ session: { ...waiting, status: 'unknown', generation: 'replacement' } }); response.resolve(null); });
 expect(screen.queryByRole('button', { name: 'agent.allowTurn' })).toBeNull();
 expect(screen.queryByRole('button', { name: 'agent.allowSession' })).toBeNull();
 expect(useAgentRunner.getState().session?.generation).toBe('replacement');
});

it('never offers session or permanent approval decisions', async () => {
 const submit = vi.fn(async () => true);
 render(<Request engine="codex" sessionId="session" generation="generation" pending={{ id: 'specific-request', kind: 'item/commandExecution/requestApproval', title: 'native', data: { command: 'echo reviewed', availableDecisions: ['acceptForSession', { acceptWithExecpolicyAmendment: { execpolicy_amendment: ['echo'] } }, 'decline'] } }} submit={submit} />);
 expect(screen.getByText('specific-request')).toBeTruthy();
 expect(screen.getByText('generation', { exact: true })).toBeTruthy();
 expect(screen.queryByRole('button', { name: 'agent.allowSession' })).toBeNull();
 expect(screen.queryByRole('button', { name: 'agent.allow', exact: true })).toBeNull();
 fireEvent.click(screen.getByRole('button', { name: 'agent.deny' }));
 await waitFor(() => expect(submit).toHaveBeenCalledExactlyOnceWith({ decision: 'decline' }));
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
