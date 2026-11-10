import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { DevelopmentRuntime } from '../src/components/development/DevelopmentRuntime';
import { useDevelopment } from '../src/stores/developmentStore';
import { commandIntent, runtimeOutputExcerpt, useDevelopmentRuntime, type DevelopmentCommand, type DevelopmentProcess, type StartProcessInput } from '../src/stores/developmentRuntime';
import { useDesktopPersistence } from '../src/stores/developmentDesktop';

const { invoke, trustState, grant, confirm } = vi.hoisted(() => ({ invoke: vi.fn(), trustState: { path: '/workspace', terminal: true, service: true }, grant: vi.fn(), confirm: vi.fn() }));
vi.mock('../src/stores/developmentTrust', () => ({ useWorkspaceTrust: () => ({ value: trustState, loading: false, error: null, grant, revoke: vi.fn(), refresh: vi.fn() }) }));
vi.mock('../src/components/development/shared', () => ({ confirmAction: confirm, errorText: (code: string) => code }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
vi.mock('../src/components/development/DevelopmentTerminal', () => ({ DevelopmentTerminal: ({ process }: { process: DevelopmentProcess }) => <div>{process.output}</div> }));
vi.mock('../src/components/development/ManualTakeover', () => ({ ManualTakeover: ({ onReleased }: { onReleased: () => void }) => <button onClick={onReleased}>Release directory</button> }));

const script: DevelopmentCommand = { id: 'discovered:test', name: 'test', program: 'npm', args: ['run', 'test'], cwd: '', env_refs: {} };
const saved: DevelopmentCommand = { ...script, id: 'saved-command', name: 'Check project' };
const process = (id: string, kind = 'command', status = 'running'): DevelopmentProcess => ({ id, project_id: 'p', task_id: 't', kind, source: `host_${kind}`, status, output: `${id} real output`, truncated: false, exit_code: null, reason: null, started_at: 1, finished_at: null, program: 'npm', args: ['run', 'test'], cwd: '', fingerprint: '', start_fingerprint: '', verification_status: 'not_requested', verification_complete: false, mutation_observed: false, preview_url: status === 'ready' ? 'http://127.0.0.1:3000' : null });
const calls = (name: string) => invoke.mock.calls.filter(([command]) => command === name);
async function flush() { await act(async () => {}); }
function configure(open = true) { const details = screen.getByTestId('dev-runtime-configuration') as HTMLDetailsElement; details.open = open; fireEvent(details, new Event('toggle')); }
let commands: DevelopmentCommand[];
let processes: DevelopmentProcess[];
beforeEach(() => {
  vi.useFakeTimers(); useDevelopment.getState().reset(); commands = [saved]; processes = [];
  useDesktopPersistence.setState({ value: {}, dirty: false });
  trustState.terminal = true; trustState.service = true; grant.mockResolvedValue(true); confirm.mockResolvedValue(true);
  invoke.mockImplementation(async (command: string, args: { input?: StartProcessInput & { commands: DevelopmentCommand[] }; id?: string }) => {
    if (command === 'read_development_commands') return { commands, discovered: [script] };
    if (command === 'list_development_processes') return processes;
    if (command === 'read_development_process') return processes.find(item => item.id === args.id);
    if (command === 'save_development_commands') { commands = args.input!.commands; return { commands, discovered: [script] }; }
    if (command === 'read_development_workspace_trust') return trustState;
    if (command === 'stop_development_process') { const stopped = { ...processes.find(item => item.id === args.id)!, status: 'cancelled', finished_at: 2 }; processes = processes.map(item => item.id === args.id ? stopped : item); return stopped; }
    if (command === 'start_development_process') { const input = args.input!; const started = { ...process(input.id, input.kind), program: input.program, args: input.args, cwd: `${trustState.path}${input.cwd ? `/${input.cwd}` : ''}` }; processes = [started, ...processes]; return started; }
    return null;
  });
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });

it('runs a chosen real test script directly with verification and no configuration form', async () => {
  commands = [];
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  expect((screen.getByTestId('dev-runtime-configuration') as HTMLDetailsElement).open).toBe(false);
  expect(calls('start_development_process')).toHaveLength(0);
  fireEvent.click(screen.getByRole('button', { name: 'runtimeWorkflow.test' })); await flush();
  expect(calls('start_development_process')).toHaveLength(1);
  expect(calls('start_development_process')[0][1].input).toMatchObject({ kind: 'command', program: 'npm', args: ['run', 'test'], trust_workspace: false, verification: true });
  expect(screen.getByText(`${processes[0].id} real output`)).toBeTruthy();
});

it('edits a saved command without allocating another command identity', async () => {
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.change(screen.getByRole('combobox', { name: 'direct.savedCommands' }), { target: { value: saved.id } }); await flush();
  configure();
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'Updated check' } });
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' })); await flush();
  expect(commands).toEqual([{ ...saved, name: 'Updated check' }]);
  expect(calls('start_development_process')).toHaveLength(0);
});

it('requests shell permission only when needed and blocks unknown starts without changing the command draft', async () => {
  trustState.terminal = false; grant.mockResolvedValue(false);
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.change(screen.getByRole('combobox', { name: 'direct.discovered' }), { target: { value: script.id } }); await flush();
  fireEvent.click(screen.getByTestId('dev-open-terminal')); await flush();
  expect(grant).toHaveBeenCalledWith('terminal'); expect(calls('start_development_process')).toHaveLength(0);
  grant.mockResolvedValue(true);
  fireEvent.click(screen.getByTestId('dev-open-terminal')); await flush();
  expect(calls('start_development_process')[0][1].input).toMatchObject({ kind: 'terminal', program: '/bin/sh', args: [], cwd: '', env_refs: {}, trust_workspace: false });
  expect(screen.getByTestId('dev-runtime-draft-summary').textContent).toContain('["npm","run","test"]');
  await act(async () => { useDevelopmentRuntime.setState(state => ({ operations: state.operations.map(operation => ({ ...operation, phase: 'unknown' as const })) })); });
  fireEvent.click(screen.getByTestId('dev-open-terminal')); fireEvent.click(screen.getByTestId('dev-process-start'));
  expect(calls('start_development_process')).toHaveLength(1);
});

it('retains the blocked draft after directory cleanup and waits for explicit resubmission', async () => {
  const implementation = invoke.getMockImplementation()!; let busy = true;
  invoke.mockImplementation(async (command: string, args: unknown) => { if (command === 'start_development_process' && busy) throw 'workspace_busy'; return implementation(command, args); });
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.change(screen.getByRole('combobox', { name: 'direct.discovered' }), { target: { value: script.id } }); await flush();
  fireEvent.click(screen.getByTestId('dev-process-start')); await flush();
  expect(calls('start_development_process')).toHaveLength(1);
  busy = false; fireEvent.click(screen.getByRole('button', { name: 'Release directory' })); await flush();
  expect(screen.getByText('runtimeWorkflow.resubmit')).toBeTruthy();
  expect(screen.getByTestId('dev-runtime-draft-summary').textContent).toContain('["npm","run","test"]');
  expect(calls('start_development_process')).toHaveLength(1);
  fireEvent.click(screen.getByTestId('dev-process-start')); await flush();
  expect(calls('start_development_process')).toHaveLength(2);
});

it('shows actual service statuses and output selection without starting or refreshing history again', async () => {
  processes = [process('service', 'service', 'ready'), process('recent')];
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  const historyReads = calls('list_development_processes').length;
  const services = screen.getByRole('region', { name: 'runtimeWorkflow.services' });
  expect(within(services).getByText('direct.status.ready')).toBeTruthy();
  fireEvent.click(screen.getByTestId('dev-process-history-recent')); await flush();
  expect(screen.getByText('recent real output')).toBeTruthy();
  expect(calls('list_development_processes')).toHaveLength(historyReads);
  expect(calls('start_development_process')).toHaveLength(0);
});

it('opens a terminal at the actual task workspace independently of advanced command overrides', async () => {
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.change(screen.getByRole('combobox', { name: 'direct.discovered' }), { target: { value: script.id } }); await flush();
  configure();
  fireEvent.change(screen.getByTestId('dev-process-cwd'), { target: { value: 'packages/app' } }); await flush();
  fireEvent.click(screen.getByTestId('dev-open-terminal')); await flush();
  expect(calls('start_development_process')[0][1].input).toMatchObject({ kind: 'terminal', cwd: '', env_refs: {}, trust_workspace: false });
  expect(screen.getByTestId('dev-runtime-draft-summary').textContent).toContain('packages/app');
});

it('restores manual selection per project/task, excludes native runs and rejects stale cross-task selection', async () => {
  const taskA = [process('native'), process('latest'), process('chosen')];
  taskA[0] = { ...taskA[0], source: 'native_agent', kind: 'terminal' };
  const taskB = [{ ...process('other-latest'), project_id: 'other-project', task_id: 'other-task' }, { ...process('other-chosen'), project_id: 'other-project', task_id: 'other-task' }];
  invoke.mockImplementation(async (_command: string, args: { taskId: string }) => args.taskId === 't' ? taskA : taskB);
  await useDevelopmentRuntime.getState().refresh('t');
  expect(useDevelopmentRuntime.getState().processes.map(item => item.id)).toEqual(['latest', 'chosen']);
  useDevelopmentRuntime.getState().select('chosen');
  await useDevelopmentRuntime.getState().refresh('other-task');
  expect(useDevelopmentRuntime.getState().selected).toBe('other-latest');
  useDevelopmentRuntime.getState().select('chosen');
  expect(useDevelopmentRuntime.getState().selected).toBe('other-latest');
  useDevelopmentRuntime.getState().select('other-chosen');
  await useDevelopmentRuntime.getState().refresh('t');
  expect(useDevelopmentRuntime.getState().selected).toBe('chosen');
  await useDevelopmentRuntime.getState().refresh('other-task');
  expect(useDevelopmentRuntime.getState().selected).toBe('other-chosen');
  useDevelopment.getState().reset();
  expect(useDevelopmentRuntime.getState().selections).toEqual({});
  expect(useDevelopmentRuntime.getState().processViews).toEqual({});
  expect(useDevelopmentRuntime.getState().details).toEqual({});
});

it('keeps clear display separate from panel collapse, host history and explicit stop through a slow remount', async () => {
  processes = [process('latest'), process('chosen')];
  const view = render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.click(screen.getByTestId('dev-process-history-chosen')); await flush();
  fireEvent.click(screen.getByTestId('dev-output-clear')); await flush();
  expect(screen.queryByText('chosen real output')).toBeNull();
  expect(screen.getByText('workbenchRuntime.displayCleared')).toBeTruthy();
  expect(processes[1].output).toBe('chosen real output');
  view.rerender(<DevelopmentRuntime projectId="p" taskId="t" visible={false} />); await flush();
  view.unmount();
  const detail = Promise.withResolvers<DevelopmentProcess>();
  const implementation = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args: unknown) => command === 'read_development_process' ? detail.promise : implementation(command, args));
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  expect(screen.getByTestId('dev-process-history-chosen').getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByText('workbenchRuntime.displayCleared')).toBeTruthy();
  expect(screen.queryByText('chosen real output')).toBeNull();
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
  await act(async () => { detail.resolve({ ...processes[1], output: 'chosen real output\nnew captured output' }); });
  expect(screen.getByLabelText('direct.output').textContent).toBe('\nnew captured output');
});

it('shows project and host working directory without treating a native terminal-shaped record as a manual PTY', async () => {
  useDevelopment.setState({ projects: [{ id: 'p', owner: 'owner', name: 'Local project', path: '/tmp/local-project', directory_identity: 'local-directory', archived: false, created_at: 1, last_opened_at: null }] });
  processes = [{ ...process('native', 'terminal'), source: 'native_agent' }, { ...process('manual'), cwd: '/tmp/local-project/task-root' }];
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  const context = screen.getByTestId('dev-runtime-context');
  expect(within(context).getByText('Local project')).toBeTruthy();
  expect(within(context).getByText('/tmp/local-project/task-root')).toBeTruthy();
  expect(within(context).getByText('direct.status.running')).toBeTruthy();
  expect(screen.queryByTestId('dev-process-history-native')).toBeNull();
  expect(useDevelopmentRuntime.getState().selected).toBe('manual');
  expect(calls('start_development_process')).toHaveLength(0);
});

it('remembers a selected real script without running it when returning to the panel', async () => {
  const view = render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.change(screen.getByRole('combobox', { name: 'runtimeWorkflow.test' }), { target: { value: saved.id } }); await flush();
  view.unmount();
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  expect((screen.getByRole('combobox', { name: 'runtimeWorkflow.test' }) as HTMLSelectElement).value).toBe(saved.id);
  expect(calls('start_development_process')).toHaveLength(0);
});

it('selects an already-ready service instead of starting another copy', async () => {
  const dev = { ...saved, id: 'dev', name: 'Preview', args: ['run', 'dev'] };
  commands = [dev];
  processes = [{ ...process('service', 'service', 'ready'), args: dev.args, cwd: '/workspace' }];
  useDesktopPersistence.setState({ value: { 'runtime:p:t': { choices: {}, readiness: { dev: { port: 3000, path: '/', timeout_ms: 10000 } } } } });
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'runtimeWorkflow.preview' })); await flush();
  expect(useDevelopmentRuntime.getState().selected).toBe('service');
  expect(calls('start_development_process')).toHaveLength(0);
  expect(screen.getByTestId('dev-service-preview').hasAttribute('disabled')).toBe(false);
});

it.each([true, false])('restarts only after the old service cleanup has been confirmed (%s)', async confirmed => {
  const dev = { ...saved, id: 'dev', name: 'Preview', args: ['run', 'dev'] };
  commands = [dev]; processes = [{ ...process('service', 'service', 'ready'), args: dev.args, cwd: '/workspace' }];
  useDesktopPersistence.setState({ value: { 'runtime:p:t': { choices: {}, readiness: { dev: { port: 3000, path: '/', timeout_ms: 10000 } } } } });
  const implementation = invoke.getMockImplementation()!;
  if (!confirmed) invoke.mockImplementation(async (command: string, args: unknown) => { if (command === 'stop_development_process') throw 'unknown'; return implementation(command, args); });
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'runtimeWorkflow.restart' })); await flush();
  expect(confirm).toHaveBeenCalled(); expect(calls('stop_development_process')).toHaveLength(1);
  expect(calls('start_development_process')).toHaveLength(confirmed ? 1 : 0);
  if (confirmed) {
    const order = invoke.mock.calls.map(([command]) => command);
    expect(order.indexOf('start_development_process')).toBeGreaterThan(order.indexOf('stop_development_process'));
    expect(calls('start_development_process')[0][1].input).toMatchObject({ kind: 'service', readiness: { port: 3000, path: '/', timeout_ms: 10000 }, trust_workspace: false });
  } else expect(screen.getByRole('button', { name: 'direct.operation.confirm' })).toBeTruthy();
});

it('requires real readiness settings before launching an unconfigured preview', async () => {
  commands = [{ ...saved, args: ['run', 'dev'] }];
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'runtimeWorkflow.preview' })); await flush();
  expect((screen.getByTestId('dev-runtime-configuration') as HTMLDetailsElement).open).toBe(true);
  expect((screen.getByTestId('dev-process-kind') as HTMLSelectElement).value).toBe('service');
  expect(calls('start_development_process')).toHaveLength(0);
});

it('previews bounded redacted failure output and hands back only the user-reviewed draft', async () => {
  const onUseOutput = vi.fn();
  processes = [{ ...process('failure', 'command', 'failed'), exit_code: 1, finished_at: 2, output: 'token=private-value\nfailed assertion' }];
  render(<DevelopmentRuntime projectId="p" taskId="t" onUseOutput={onUseOutput} />); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'runtimeWorkflow.reviewOutput' })); await flush();
  const field = screen.getByRole('textbox', { name: 'runtimeWorkflow.outputDraft' }) as HTMLTextAreaElement;
  expect(field.value).not.toContain('private-value'); expect(field.value).toContain('failed assertion');
  expect(onUseOutput).not.toHaveBeenCalled();
  fireEvent.change(field, { target: { value: 'Reviewed failure excerpt' } });
  fireEvent.click(screen.getByRole('button', { name: 'runtimeWorkflow.useOutput' }));
  expect(onUseOutput).toHaveBeenCalledWith('Reviewed failure excerpt');
  expect(calls('start_development_process')).toHaveLength(0);
});

it('keeps command context while bounding and redacting the shared output tail', () => {
  const excerpt = runtimeOutputExcerpt({ ...process('failure'), program: 'runner', args: ['--token', 'argument-secret'], output: `${'x'.repeat(15000)}\n{"password":"json-secret"}\nBearer bearer-secret\nhttps://user:url-secret@example.test\nsk-1234567890123456\n\x1b[31mvisible\x1b[0m` });
  expect(excerpt.length).toBeLessThanOrEqual(12000); expect(excerpt).toContain('runner'); expect(excerpt).toContain('visible');
  for (const secret of ['argument-secret', 'json-secret', 'bearer-secret', 'url-secret', 'sk-1234567890123456', '\x1b']) expect(excerpt).not.toContain(secret);
  expect(commandIntent({ ...saved, program: 'cargo', args: ['test'] })).toBe('test');
  expect(commandIntent({ ...saved, program: 'cargo', args: ['build'] })).toBe('build');
});

it('treats a conversation result action as navigation only, never a dispatch', async () => {
  commands = [];
  const view = render(<DevelopmentRuntime projectId="p" taskId="t" requestedAction={{ kind: 'test', nonce: 1 }} />); await flush();
  view.rerender(<DevelopmentRuntime projectId="p" taskId="t" requestedAction={{ kind: 'terminal', nonce: 2 }} />); await flush();
  expect(calls('start_development_process')).toHaveLength(0);
});

it('replaces a previous pass with the host stale result when current code changes', async () => {
  processes = [{ ...process('verified', 'command', 'exited'), finished_at: 2, exit_code: 0, verification_status: 'observed_stable', verification_complete: true, fingerprint: 'first', start_fingerprint: 'first' }];
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  expect(screen.getByText('direct.verification: direct.verification.observed_stable')).toBeTruthy();
  processes = [{ ...processes[0], verification_status: 'stale', fingerprint: 'changed' }];
  await act(async () => { await vi.advanceTimersByTimeAsync(800); });
  expect(screen.queryByText('direct.verification: direct.verification.observed_stable')).toBeNull();
  expect(screen.getByText('direct.verification: direct.verification.stale')).toBeTruthy();
});

it('offers configuration and Agent drafting rather than inventing a command in an empty workspace', async () => {
  const implementation = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'read_development_commands' ? { commands: [], discovered: [] } : implementation(command, args));
  const onUseOutput = vi.fn();
  render(<DevelopmentRuntime projectId="p" taskId="t" onUseOutput={onUseOutput} />); await flush();
  expect(screen.getByText('runtimeWorkflow.noScripts')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'runtimeWorkflow.askAgent' }));
  expect(onUseOutput).toHaveBeenCalledWith('runtimeWorkflow.helpPrompt');
  expect(calls('start_development_process')).toHaveLength(0);
});
