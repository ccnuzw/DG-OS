import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import AgentWorkbench from '../src/pages/workbench/AgentWorkbench';
import { useAgentRunner } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { chooseContext } from '../src/stores/developmentSelection';
import { useDevelopmentRuntime, type DevelopmentProcess } from '../src/stores/developmentRuntime';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
afterEach(() => { cleanup(); localStorage.clear(); useAgentRunner.getState().reset(); useDevelopment.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });

const workspace = {
 projects: [{ id: 'p1', owner: 'u', name: 'demo', path: '/tmp/project', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: 1 }],
 tasks: [{ id: 't1', owner: 'u', project_id: 'p1', goal: 'Ship', requirements: 'Tests pass', status: 'todo', created_at: 1 }],
 sessions: [], selection: { project_id: 'p1', task_id: 't1', session_id: null }, operations: [],
};
const inspection = { path_state: 'available', kind: 'directory', repository_root: null, branch: null, changes: [], truncated: false, error: null };
const session = { id: 'one', engine: 'codex', native_id: 'native', workspace: '/tmp/project', status: 'idle', events: [], pending: [], error: null };

// `override` returns a value for the commands a test cares about; undefined falls back to the defaults below.
function install(override: (command: string) => unknown = () => undefined) {
 invoke.mockImplementation(async (command: string) => {
  const result = override(command);
  if (result !== undefined) return result;
  if (command === 'read_development_workspace') return workspace;
  if (command === 'read_development_workspaces') return { workspaces: [{ id: 'w1', path: '/tmp/project', kind: 'original' }], selected_workspace_id: 'w1', operation: null };
  if (command === 'inspect_development_project') return inspection;
  if (command === 'open_agent_session' || command === 'get_agent_session') return session;
  if (command === 'read_development_capabilities' || command === 'get_development_session_capabilities') return null;
  if (command === 'list_development_capability_confirmations') return [];
  if (command === 'read_development_capability_catalog') return { providers: [], skills: [], mcp: [] };
  if (command === 'read_development_git') return { kind: 'directory', branch: null, head: null };
  if (command === 'list_development_processes') return [];
  if (command === 'list_development_files') return { entries: [], truncated: false };
  if (command === 'read_development_commands') return { commands: [], discovered: [] };
  return undefined;
 });
}
// Trust and start are explicit clicks: the project directory must be trusted before a runtime can be created.
async function start(state = 'idle') {
 await screen.findByTestId('dev-task-summary');
 fireEvent.click(screen.getByTestId('dev-trust'));
 fireEvent.click(screen.getByTestId('dev-session-new-codex'));
 await screen.findAllByText(`agent.state.${state}`);
}
describe('agent sessions', () => {
 it('requires workspace trust before creating a runtime', async () => {
  install(); render(<AgentWorkbench />);
  const button = await screen.findByTestId('dev-session-new-codex') as HTMLButtonElement;
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
  await waitFor(() => expect(screen.getByTestId('dev-session-cwd').textContent).toContain('/tmp/project'));
  changed = true;
  await act(async () => { await chooseContext({ project_id: 'p1', task_id: 't2', session_id: null }); });
  await act(async () => { resolveWorkspace({ workspaces: [{ id: 'w2', path: '/tmp/task-worktree', kind: 'managed' }], selected_workspace_id: 'w2', operation: null }); });
  await waitFor(() => expect(screen.getByTestId('dev-session-cwd').textContent).toContain('/tmp/task-worktree'));
  expect((screen.getByTestId('dev-trust') as HTMLInputElement).checked).toBe(false);
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
  install(command => (command === 'open_agent_session' || command === 'get_agent_session' ? claude : undefined));
  render(<AgentWorkbench />); await start('waiting');
  fireEvent.click(await screen.findByRole('button', { name: 'agent.deny' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('respond_agent_request', { input: { session_id: 'one', request_id: 'approval', response: { behavior: 'deny' } } }));
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
 await act(async () => { fireEvent.click(screen.getByTestId('dev-toggle-runtime')); });
 expect(screen.getByTestId('dev-development-runtime')).toBeTruthy();
 expect(controller.getSurfaceStatuses()['cli-agent']?.running).toBe(true);
 const reads = (command: string) => invoke.mock.calls.filter(([name]) => name === command).length;
 // One list read each second even while both the runtime and its outer consumer are mounted.
 const visibleHistory = reads('list_development_processes');
 await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
 expect(reads('list_development_processes') - visibleHistory).toBe(2);
 if (!tabMounted) {
  await act(async () => { fireEvent.click(screen.getByTestId('dev-toggle-runtime')); });
  expect(screen.getByTestId('dev-development-runtime').closest('#dev-panel-runtime')?.hasAttribute('hidden')).toBe(true);
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
 await act(async () => { fireEvent.click(screen.getByTestId('dev-toggle-runtime')); });
 fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
 fireEvent.click(screen.getByTestId('dev-process-trust'));
 fireEvent.click(screen.getByTestId('dev-process-start'));
 expect(invoke.mock.calls.filter(([name]) => name === 'start_development_process')).toHaveLength(1);
 const submittedId = invoke.mock.calls.find(([name]) => name === 'start_development_process')![1].input.id;
 await act(async () => { fireEvent.click(screen.getByTestId('dev-toggle-runtime')); });
 expect(screen.getByTestId('dev-development-runtime').closest('#dev-panel-runtime')?.hasAttribute('hidden')).toBe(true);
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
 await act(async () => { fireEvent.click(screen.getByTestId('dev-toggle-runtime')); });
 await act(async () => { fireEvent.click(screen.getByTestId('dev-toggle-runtime')); });
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

it('counts native and host requests in code view, navigates to review and expires revoked or closed requests', async () => {
 vi.useFakeTimers();
 const native = { id: 'native-approval', kind: 'item/commandExecution/requestApproval', title: 'native', data: {} };
 const host = { id: 'host-approval', project_id: 'p1', task_id: 't1', session_id: 'one', run_id: 'r', server_id: 'server', tool_name: 'write', schema_hash: 'schema', arguments: {} };
 let items = [host];
 install(command => command === 'open_agent_session' || command === 'get_agent_session' ? { ...session, status: 'waiting', pending: [native] } : command === 'list_development_capability_confirmations' ? items : undefined);
 render(<AgentWorkbench />);
 await act(async () => {});
 fireEvent.click(screen.getByTestId('dev-trust'));
 await act(async () => { fireEvent.click(screen.getByTestId('dev-session-new-codex')); });
 fireEvent.click(screen.getByRole('button', { name: 'dev.projectTools' }));
 expect(screen.getByTestId('dev-toggle-agent').textContent).toContain('(2)');
 fireEvent.click(screen.getByRole('button', { name: /dev.pendingApprovals/ }));
 expect(screen.getByRole('region', { name: 'capabilities.hostApproval' })).toBeTruthy();
 expect(screen.getByRole('region', { name: 'agent.approval' })).toBeTruthy();
 const staleHostButton = screen.getByRole('region', { name: 'capabilities.hostApproval' }).querySelector('button')!;
 items = [];
 await act(async () => { await vi.advanceTimersByTimeAsync(750); });
 expect(screen.getByTestId('dev-toggle-agent').textContent).toContain('(1)');
 expect(screen.queryByRole('region', { name: 'capabilities.hostApproval' })).toBeNull();
 fireEvent.click(staleHostButton);
 expect(invoke.mock.calls.some(([command]) => command === 'respond_development_capability_confirmation')).toBe(false);
 await act(async () => { useAgentRunner.setState({ session: { ...useAgentRunner.getState().session!, status: 'closed' } }); });
 expect(screen.queryByRole('button', { name: /dev.pendingApprovals/ })).toBeNull();
 expect(screen.queryByRole('button', { name: 'agent.allow' })).toBeNull();
 expect(await useAgentRunner.getState().action('respond_agent_request', { input: { session_id: 'one', request_id: native.id, response: { decision: 'accept' } } })).toBe(false);
 expect(invoke.mock.calls.some(([command]) => command === 'respond_agent_request')).toBe(false);
});

it('offers other-task requests as navigation only and reads fresh host state after navigation', async () => {
 const other = { ...session, id: 'other', status: 'waiting', pending: [{ id: 'other-native', kind: 'item/commandExecution/requestApproval', title: 'Other request', data: {} }] };
 const saved = { id: 'other', owner: 'u', engine: 'codex', native_id: null, workspace: '/tmp/project', executable: null, model: null, project_id: 'p1', task_id: 't2' };
 install(command => command === 'read_development_workspace' ? { ...workspace, tasks: [...workspace.tasks, { ...workspace.tasks[0], id: 't2', goal: 'Other task' }], sessions: [saved] } : command === 'get_agent_session' ? other : undefined);
 useAgentRunner.setState({ live: { other: 'waiting' } });
 render(<AgentWorkbench />);
 const navigation = await screen.findByRole('button', { name: 'dev.otherApprovals' });
 expect(screen.queryByRole('button', { name: 'agent.allow' })).toBeNull();
 fireEvent.click(navigation);
 await screen.findByRole('region', { name: 'agent.approval' });
 expect(useDevelopment.getState().selection.task_id).toBe('t2');
 expect(useAgentRunner.getState().selected).toBe('other');
 expect(invoke.mock.calls.some(([command]) => command === 'respond_agent_request')).toBe(false);
});
