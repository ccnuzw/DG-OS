import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { DevelopmentRuntime } from '../src/components/development/DevelopmentRuntime';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentRuntime, type DevelopmentCommand, type DevelopmentProcess, type StartProcessInput } from '../src/stores/developmentRuntime';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
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
  invoke.mockImplementation(async (command: string, args: { input?: StartProcessInput & { commands: DevelopmentCommand[] }; id?: string }) => {
    if (command === 'read_development_commands') return { commands, discovered: [script] };
    if (command === 'list_development_processes') return processes;
    if (command === 'read_development_process') return processes.find(item => item.id === args.id);
    if (command === 'save_development_commands') { commands = args.input!.commands; return { commands, discovered: [script] }; }
    if (command === 'start_development_process') { const input = args.input!; const started = { ...process(input.id, input.kind), program: input.program, args: input.args, cwd: input.cwd }; processes = [started, ...processes]; return started; }
    return null;
  });
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });

it('exposes catalog and explicit actions while configuration is closed; selecting a script only fills the retained draft', async () => {
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  expect((screen.getByTestId('dev-runtime-configuration') as HTMLDetailsElement).open).toBe(false);
  expect(screen.getByRole('button', { name: 'runtimeWorkflow.openTerminal' })).toBeTruthy();
  fireEvent.change(screen.getByRole('combobox', { name: 'direct.discovered' }), { target: { value: script.id } }); await flush();
  expect(screen.getByTestId('dev-runtime-draft-summary').textContent).toContain('["npm","run","test"]');
  expect(calls('start_development_process')).toHaveLength(0);
  fireEvent.click(screen.getByTestId('dev-process-start')); await flush();
  expect(screen.getByText('direct.validation.trust')).toBeTruthy();
  expect(calls('start_development_process')).toHaveLength(0);
  fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-process-start')); await flush();
  expect(calls('start_development_process')).toHaveLength(1);
  expect(calls('start_development_process')[0][1].input).toMatchObject({ kind: 'command', program: 'npm', args: ['run', 'test'], trust_workspace: true });
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

it('opens a trusted terminal explicitly without replacing the command draft and shares the unknown-outcome block', async () => {
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.change(screen.getByRole('combobox', { name: 'direct.discovered' }), { target: { value: script.id } }); await flush();
  fireEvent.click(screen.getByTestId('dev-open-terminal')); await flush();
  expect(calls('start_development_process')).toHaveLength(0);
  fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-open-terminal')); await flush();
  expect(calls('start_development_process')).toHaveLength(1);
  expect(calls('start_development_process')[0][1].input).toMatchObject({ kind: 'terminal', program: '/bin/sh', args: [], trust_workspace: true });
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
  fireEvent.click(screen.getByTestId('dev-process-trust')); fireEvent.click(screen.getByTestId('dev-process-start')); await flush();
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

it('requires fresh trust when the working directory changes and retains the configured command', async () => {
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.change(screen.getByRole('combobox', { name: 'direct.discovered' }), { target: { value: script.id } }); await flush();
  fireEvent.click(screen.getByTestId('dev-process-trust'));
  configure();
  fireEvent.change(screen.getByTestId('dev-process-cwd'), { target: { value: 'packages/app' } }); await flush();
  expect((screen.getByTestId('dev-process-trust') as HTMLInputElement).checked).toBe(false);
  fireEvent.click(screen.getByTestId('dev-open-terminal')); await flush();
  expect(calls('start_development_process')).toHaveLength(0);
  expect(screen.getByTestId('dev-runtime-draft-summary').textContent).toContain('["npm","run","test"]');
  fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-open-terminal')); await flush();
  expect(calls('start_development_process')[0][1].input).toMatchObject({ kind: 'terminal', cwd: 'packages/app', trust_workspace: true });
});
