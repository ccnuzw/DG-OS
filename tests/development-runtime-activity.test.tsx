import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { DevelopmentRuntime } from '../src/components/development/DevelopmentRuntime';
import { useDevelopment } from '../src/stores/developmentStore';
import { processActive, readDevelopmentProcess, retryRuntimeOperation, runtimeOperationBlocked, submitRuntimeOperation, useDevelopmentRuntime, type DevelopmentProcess } from '../src/stores/developmentRuntime';
import { chooseContext } from '../src/stores/developmentSelection';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
const process = (id = 'run', status = 'exited', output = 'first snapshot'): DevelopmentProcess => ({ id, project_id: 'p', task_id: 't', kind: 'command', source: 'host', status, output, truncated: false, exit_code: status === 'running' ? null : 0, reason: null, started_at: 1, finished_at: status === 'running' ? null : 2, program: 'echo', args: [], cwd: '.', fingerprint: '', start_fingerprint: '', verification_status: 'not_requested', verification_complete: false, mutation_observed: false, preview_url: null });
const calls = (command: string) => invoke.mock.calls.filter(([name]) => name === command).length;
async function flush() { await act(async () => {}); }
async function advance(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }
function expectReceipt(action: 'save' | 'start' | 'stop', phase: 'pending' | 'accepted' | 'succeeded' | 'failed' | 'unknown', targetId?: string) {
  const operation = useDevelopmentRuntime.getState().operations.find(item => item.action === action);
  expect(operation).toMatchObject({ phase, projectId: 'p', taskId: 't', ...(targetId ? { targetId } : {}) });
  const receipts = screen.getByRole('region', { name: 'direct.operation.receipts' });
  const identity = within(receipts).getByText(operation!.targetId, { selector: 'code' });
  const receipt = identity.closest('details')!.parentElement!;
  const announcement = within(receipt).getByRole(phase === 'failed' || phase === 'unknown' ? 'alert' : 'status');
  expect(announcement.textContent).toContain(`direct.operation.${action}.${phase}`);
}
function openConfiguration() { const disclosure = screen.getByTestId('dev-runtime-configuration') as HTMLDetailsElement; disclosure.open = true; fireEvent(disclosure, new Event('toggle')); }
function show() {
  const controller = createWorkspaceController();
  const tree = (active: boolean, taskId = 't') => <SurfaceScope id="agent" active={active} controller={controller}><DevelopmentRuntime projectId="p" taskId={taskId} /></SurfaceScope>;
  const view = render(tree(true));
  openConfiguration();
  return { async activity(active: boolean, taskId = 't') { view.rerender(tree(active, taskId)); await flush(); } };
}
beforeEach(() => {
  vi.useFakeTimers(); useDevelopment.getState().reset();
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process()] : process());
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });
it('pauses idle history and detail while covered, resumes with latest output and preserves a failed snapshot', async () => {
  const view = show(); await flush();
  await advance(4000);
  const visible = { history: calls('list_development_processes'), detail: calls('read_development_process') };
  await view.activity(false);
  await advance(4000);
  console.log('runtime 4000ms visible then hidden', visible, { history: calls('list_development_processes') - visible.history, detail: calls('read_development_process') - visible.detail });
  expect(calls('list_development_processes')).toBe(visible.history);
  expect(calls('read_development_process')).toBe(visible.detail);
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process()] : process('run', 'exited', 'latest output'));
  await view.activity(true);
  expect(screen.getByText('latest output')).toBeTruthy();
  invoke.mockRejectedValue('temporarily unavailable'); await advance(1000);
  expect(screen.getByText('latest output')).toBeTruthy();
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process()] : process('run', 'exited', 'recovered'));
  await view.activity(false); await view.activity(true);
  expect(screen.getByText('recovered')).toBeTruthy();
});
it('coalesces manual history reads and rejects late task and account snapshots', async () => {
  const old = Promise.withResolvers<DevelopmentProcess[]>();
  invoke.mockImplementation((command: string) => command === 'list_development_processes' ? old.promise : Promise.resolve({ commands: [], discovered: [] }));
  const view = show(); await flush();
  fireEvent.click(screen.getByRole('button', { name: 'review.refresh' }));
  fireEvent.click(screen.getByRole('button', { name: 'review.refresh' }));
  expect(calls('list_development_processes')).toBe(1);
  await view.activity(false); await view.activity(true); await view.activity(false); await view.activity(true);
  expect(calls('list_development_processes')).toBe(1);
  await view.activity(true, 'other');
  await act(async () => { old.resolve([process()]); });
  expect(screen.queryByText('first snapshot')).toBeNull();
  await act(async () => { useDevelopment.getState().reset(); });
  expect(useDevelopmentRuntime.getState().processes).toEqual([]);
});
it('accepts an owned detail while inactive, bounds quick switching and discards a different process response', async () => {
  const old = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process(), process('second')] : old.promise);
  const view = show(); await flush();
  await view.activity(false); await view.activity(true); await view.activity(false);
  expect(calls('read_development_process')).toBe(1);
  await act(async () => { old.resolve(process('run', 'exited', 'owned hidden output')); });
  expect(screen.getByText('owned hidden output')).toBeTruthy();
  const stale = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockImplementation(async (command: string, args: { id?: string }) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process(), process('second')] : args.id === 'run' ? stale.promise : process('second', 'exited', 'second output'));
  await view.activity(true);
  fireEvent.click(screen.getByTestId('dev-process-history-second')); await flush();
  await act(async () => { stale.resolve(process('run', 'exited', 'stale first output')); });
  expect(screen.getByText('second output')).toBeTruthy();
  expect(screen.queryByText('stale first output')).toBeNull();
});
it('reads after a mutation even when an earlier history snapshot is still in flight', async () => {
  const old = Promise.withResolvers<DevelopmentProcess[]>(); let reads = 0;
  invoke.mockImplementation(async () => ++reads === 1 ? old.promise : [process('started', 'running')]);
  const first = useDevelopmentRuntime.getState().refresh('t');
  const after = useDevelopmentRuntime.getState().refresh('t', true);
  expect(reads).toBe(1); old.resolve([]); await Promise.all([first, after]);
  expect(reads).toBe(2); expect(useDevelopmentRuntime.getState().processes[0].id).toBe('started');
});
it('rejects an old account detail and bounds slow catalog restoration to one follow-up read', async () => {
  const detail = Promise.withResolvers<DevelopmentProcess>();
  const catalog = Promise.withResolvers<{ commands: []; discovered: [] }>();
  let catalogs = 0;
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? ++catalogs === 1 ? catalog.promise : { commands: [], discovered: [] } : command === 'list_development_processes' ? [process()] : detail.promise);
  const view = show(); await flush();
  for (let index = 0; index < 5; index++) { await view.activity(false); await view.activity(true); }
  expect(catalogs).toBe(1); expect(calls('read_development_process')).toBe(1);
  await act(async () => { catalog.resolve({ commands: [], discovered: [] }); });
  expect(catalogs).toBe(2);
  // New-account reads must not reuse the fixture's old-account host result.
  invoke.mockImplementation(async (command: string) => command === 'list_development_processes' ? [] : { commands: [], discovered: [] });
  await act(async () => { useDevelopment.getState().reset(); detail.resolve(process('run', 'exited', 'PRIVATE_OLD_ACCOUNT')); });
  expect(screen.queryByText('PRIVATE_OLD_ACCOUNT')).toBeNull();
});

it('keeps a stop acknowledgement when an older detail finishes afterwards', async () => {
  const old = Promise.withResolvers<DevelopmentProcess>(); let detailReads = 0;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'list_development_processes') return [process('run', 'running')];
    if (command === 'stop_development_process') return process('run', 'cancelled', 'stopped output');
    return ++detailReads === 1 ? process('run', 'running') : old.promise;
  });
  show(); await flush(); await advance(800);
  fireEvent.click(screen.getByTestId('dev-process-stop')); await flush();
  expect(screen.getByText('stopped output')).toBeTruthy();
  await act(async () => { old.resolve(process('run', 'running', 'stale running output')); });
  expect(screen.getByText('stopped output')).toBeTruthy();
  expect(screen.queryByText('stale running output')).toBeNull();
});

it('never confirms empty history before a successful read and offers retry after first failure', async () => {
  const history = Promise.withResolvers<DevelopmentProcess[]>();
  invoke.mockImplementation(async (command: string) => command === 'list_development_processes' ? history.promise : { commands: [], discovered: [] });
  show(); await flush();
  expect(screen.queryByText('direct.noHistory')).toBeNull();
  expect(screen.getAllByRole('status').some(node => node.textContent?.includes('patterns.loading'))).toBe(true);
  await act(async () => { history.reject('history unavailable'); });
  expect(screen.queryByText('direct.noHistory')).toBeNull();
  expect(screen.getByRole('button', { name: 'direct.retryHistory' })).toBeTruthy();
  invoke.mockImplementation(async (command: string) => command === 'list_development_processes' ? [] : { commands: [], discovered: [] });
  fireEvent.click(screen.getByRole('button', { name: 'direct.retryHistory' })); await flush();
  expect(screen.getByText('direct.noHistory')).toBeTruthy();
});

it('keeps selection, draft and last-known output through refresh failure until a successful retry', async () => {
  show(); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'draft program' } });
  const history = Promise.withResolvers<DevelopmentProcess[]>();
  const detail = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockImplementation(async (command: string) => command === 'list_development_processes' ? history.promise : detail.promise);
  fireEvent.click(screen.getByRole('button', { name: 'review.refresh' })); await advance(800);
  expect(screen.getByText('first snapshot')).toBeTruthy();
  expect(screen.getByTestId('dev-process-history-run').getAttribute('aria-pressed')).toBe('true');
  expect(screen.getAllByRole('status').some(node => node.textContent?.includes('patterns.refreshing'))).toBe(true);
  await act(async () => { history.reject('history unavailable'); detail.reject('detail unavailable'); });
  expect(screen.getByText('first snapshot')).toBeTruthy();
  expect(screen.getByText('direct.lastKnown')).toBeTruthy();
  expect((screen.getByTestId('dev-process-program') as HTMLInputElement).value).toBe('draft program');
  const next = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockImplementation(async (command: string) => command === 'list_development_processes' ? [process()] : next.promise);
  fireEvent.click(screen.getByRole('button', { name: 'direct.retryDetail' })); await flush();
  expect(screen.getAllByRole('alert').filter(node => node.textContent?.includes('patterns.refreshFailed'))).toHaveLength(2);
  await act(async () => { next.resolve(process('run', 'exited', 'fresh confirmed output')); });
  expect(screen.getByText('fresh confirmed output')).toBeTruthy();
  expect(screen.queryByText('direct.lastKnown')).toBeNull();
});

it('shows a retryable first detail failure without inventing output or clearing history', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'list_development_processes') return [process()];
    throw 'detail unavailable';
  });
  show(); await flush();
  expect(screen.getByTestId('dev-process-history-run')).toBeTruthy();
  expect(screen.queryByTestId('dev-process-detail')).toBeNull();
  expect(screen.queryByText('direct.noHistory')).toBeNull();
  invoke.mockResolvedValue(process('run', 'exited', 'retry output'));
  fireEvent.click(screen.getByRole('button', { name: 'direct.retryDetail' })); await flush();
  expect(screen.getByText('retry output')).toBeTruthy();
});

it('rejects a detail response belonging to a different target', async () => {
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process()] : process('other', 'exited', 'WRONG_TARGET_OUTPUT'));
  show(); await flush();
  expect(screen.queryByText('WRONG_TARGET_OUTPUT')).toBeNull();
  expect(screen.getByText('direct.targetMismatch')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'direct.retryDetail' })).toBeTruthy();
});

it('resumes with retained output and draft while the refresh is slow without starting another process', async () => {
  const view = show(); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'retained draft' } });
  await view.activity(false);
  const detail = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process()] : detail.promise);
  await view.activity(true);
  expect(screen.getByText('first snapshot')).toBeTruthy();
  expect(screen.queryByText('patterns.loading')).toBeNull();
  expect((screen.getByTestId('dev-process-program') as HTMLInputElement).value).toBe('retained draft');
  expect(calls('start_development_process')).toBe(0);
  await act(async () => { detail.resolve(process('run', 'exited', 'restored output')); });
  expect(screen.getByText('restored output')).toBeTruthy();
});

it('retries the command catalog independently from successful history and detail reads', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_commands') throw 'catalog unavailable';
    return command === 'list_development_processes' ? [process()] : process();
  });
  show(); await flush();
  expect(screen.getByText('first snapshot')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'direct.retryCatalog' })).toBeTruthy();
  expect(screen.queryByRole('combobox', { name: 'direct.savedCommands' })).toBeNull();
  invoke.mockResolvedValue({ commands: [{ id: 'saved', name: 'Restored command', program: 'echo', args: [], cwd: '', env_refs: {} }], discovered: [] });
  fireEvent.click(screen.getByRole('button', { name: 'direct.retryCatalog' })); await flush();
  expect(screen.getByRole('option', { name: 'Restored command' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'direct.retryCatalog' })).toBeNull();
});

it('explains invalid runtime fields next to inputs before dispatching', async () => {
  show(); await flush();
  fireEvent.click(screen.getByTestId('dev-process-start'));
  expect(screen.getByText('direct.validation.program')).toBeTruthy();
  expect(screen.getByText('direct.validation.trust')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' }));
  expect(screen.getByText('direct.validation.name')).toBeTruthy();
  expect(calls('start_development_process')).toBe(0);
  expect(calls('save_development_commands')).toBe(0);
});

it('retains an owned start receipt across unmount and separates a failed history read', async () => {
  const start = Promise.withResolvers<DevelopmentProcess>();
  let startedId = '';
  invoke.mockImplementation(async (command: string, args: { input?: { id: string } }) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'start_development_process') { startedId = args.input!.id; return start.promise; }
    if (command === 'list_development_processes') { if (startedId) throw 'storage_failed'; return []; }
    return process(startedId, 'running');
  });
  const view = render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
  fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-process-start'));
  expectReceipt('start', 'pending', startedId);
  fireEvent.click(screen.getByTestId('dev-process-start'));
  expect(calls('start_development_process')).toBe(1);
  view.unmount();
  await act(async () => { start.resolve(process(startedId, 'running')); });
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'start')?.phase).toBe('accepted');
  expectReceipt('start', 'accepted', startedId);
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'start')?.refreshError).toBe('storage_failed');
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process(startedId, 'running')] : process(startedId, 'running'));
  fireEvent.click(screen.getByRole('button', { name: 'direct.operation.retryRead' })); await flush();
  expect(calls('start_development_process')).toBe(1);
  expect(screen.queryByRole('button', { name: 'direct.operation.retryRead' })).toBeNull();
});

it('keeps a changed command draft and selection when a hidden save completes', async () => {
  const save = Promise.withResolvers<{ commands: { id: string; name: string; program: string; args: string[]; cwd: string; env_refs: {} }[]; discovered: [] }>();
  const view = show(); await flush();
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'Original' } });
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
  invoke.mockImplementation(async (command: string) => command === 'save_development_commands' ? save.promise : command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process()] : process());
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' }));
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'New draft' } });
  await view.activity(false);
  const submitted = invoke.mock.calls.find(([command]) => command === 'save_development_commands')![1].input.commands;
  await act(async () => { save.resolve({ commands: submitted, discovered: [] }); });
  await view.activity(true);
  expect((screen.getByRole('textbox', { name: 'direct.commandName' }) as HTMLInputElement).value).toBe('New draft');
  expectReceipt('save', 'succeeded', submitted[0].id);
  expect(calls('save_development_commands')).toBe(1);
});

it('keeps an unknown start blocked and reconciles only its original process ID', async () => {
  let id = ''; let visible = false;
  invoke.mockImplementation(async (command: string, args: { input?: { id: string }; id?: string }) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'list_development_processes') return [];
    if (command === 'start_development_process') { id = args.input!.id; throw 'transport disconnected'; }
    if (command === 'read_development_process') { expect(args.id).toBe(id); if (!visible) throw 'process_not_found'; return process(id, 'running'); }
    throw 'unexpected command';
  });
  show(); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } }); fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-process-start')); await flush();
  expectReceipt('start', 'unknown', id);
  fireEvent.click(screen.getByTestId('dev-process-start')); expect(calls('start_development_process')).toBe(1);
  visible = true; fireEvent.click(screen.getByRole('button', { name: 'direct.operation.confirm' })); await flush();
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'start')?.phase).toBe('accepted');
  expectReceipt('start', 'accepted', id);
  expect(calls('start_development_process')).toBe(1);
});

it('reports a definite rejected save and preserves the editable draft', async () => {
  show(); await flush();
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'Keep me' } });
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
  invoke.mockRejectedValue('invalid_command');
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' })); await flush();
  expect((screen.getByRole('textbox', { name: 'direct.commandName' }) as HTMLInputElement).value).toBe('Keep me');
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'save')).toMatchObject({ phase: 'failed', error: 'invalid_command' });
  expectReceipt('save', 'failed');
});

it.each(['running', 'cleanup_pending', 'unknown'])('does not confirm a %s stop acknowledgement or repeat the stop while checking', async status => {
  let stopped = false;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'list_development_processes') return [process('run', stopped ? 'cancelled' : status)];
    return process('run', stopped ? 'cancelled' : status);
  });
  // The stop starts from an active snapshot; its receipt may remain uncertain.
  const handler = invoke.getMockImplementation()!;
  let initialRead = true;
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === 'list_development_processes' && initialRead) return [process('run', 'running')];
    if (command === 'read_development_process' && initialRead) { initialRead = false; return process('run', 'running'); }
    return handler(command, args);
  });
  show(); await flush();
  fireEvent.click(screen.getByTestId('dev-process-stop')); await flush();
  expect(useDevelopmentRuntime.getState().operations[0]?.phase).toBe(status === 'unknown' ? 'unknown' : 'accepted');
  expectReceipt('stop', status === 'unknown' ? 'unknown' : 'accepted', 'run');
  fireEvent.click(screen.getByTestId('dev-process-stop')); expect(calls('stop_development_process')).toBe(1);
  stopped = true; fireEvent.click(screen.getByRole('button', { name: 'direct.operation.confirm' })); await flush();
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'stop')?.phase).toBe('succeeded');
  expectReceipt('stop', 'succeeded', 'run');
  expect(calls('stop_development_process')).toBe(1);
});

it('keeps a late start receipt on the original task and removes it at the account boundary', async () => {
  const start = Promise.withResolvers<DevelopmentProcess>(); let id = '';
  invoke.mockImplementation(async (command: string, args: { input?: { id: string }; taskId?: string }) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'start_development_process') { id = args.input!.id; return start.promise; }
    if (command === 'list_development_processes') return [];
    return process(id, 'running');
  });
  const view = show(); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } }); fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-process-start'));
  await view.activity(true, 'other');
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'other task draft' } });
  await act(async () => { start.resolve(process(id, 'running')); });
  expect(useDevelopmentRuntime.getState().operations.filter(item => item.taskId === (useDevelopmentRuntime.getState().taskId ?? '') && item.action === 'start' && item.phase === 'accepted')).toEqual([]);
  expect(useDevelopmentRuntime.getState().taskId).toBe('other');
  expect((screen.getByTestId('dev-process-program') as HTMLInputElement).value).toBe('other task draft');
  await view.activity(true);
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'start')?.phase).toBe('accepted');
  await act(async () => { useDevelopment.getState().reset(); });
  expect(useDevelopmentRuntime.getState().operations.filter(item => item.taskId === (useDevelopmentRuntime.getState().taskId ?? '') && item.action === 'start' && item.phase === 'accepted')).toEqual([]);
});

it('confirms a committed save by catalog after a post-write error without resending it', async () => {
  let commands: { id: string; name: string; program: string; args: string[]; cwd: string; env_refs: {} }[] = [];
  invoke.mockImplementation(async (command: string, args: { input?: { commands: typeof commands } }) => {
    if (command === 'read_development_commands') return { commands, discovered: [] };
    if (command === 'save_development_commands') { commands = args.input!.commands; throw 'file_access_denied'; }
    return command === 'list_development_processes' ? [] : process();
  });
  show(); await flush();
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'Committed' } });
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' })); await flush();
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'save')?.phase).toBe('succeeded');
  expectReceipt('save', 'succeeded', commands[0].id);
  expect(calls('save_development_commands')).toBe(1);
});

it('takes the runtime operation lock synchronously before React can rerender', async () => {
  const start = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockReturnValue(start.promise);
  const input = { action: 'start' as const, projectId: 'p', taskId: 't', input: { id: 'first', task_id: 't', kind: 'command' as const, program: 'echo', args: [], cwd: '', env_refs: {}, trust_workspace: true, verification: false, readiness: null } };
  const first = submitRuntimeOperation(input);
  const duplicate = submitRuntimeOperation({ ...input, input: { ...input.input, id: 'second' } });
  expect(calls('start_development_process')).toBe(1);
  expect(await duplicate).toBeNull();
  start.resolve(process('first', 'running')); await first;
  expect(useDevelopmentRuntime.getState().operations[0].phase).toBe('accepted');
});

it('validates environment references and each service readiness field without sending a start', async () => {
  show(); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
  fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByRole('button', { name: 'direct.addEnvironment' }));
  fireEvent.change(screen.getByTestId('dev-process-kind'), { target: { value: 'service' } });
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.probePath' }), { target: { value: '//wrong' } });
  fireEvent.change(screen.getByRole('spinbutton', { name: 'direct.timeout' }), { target: { value: '5' } });
  fireEvent.click(screen.getByTestId('dev-process-start'));
  expect(screen.getByText('direct.validation.env')).toBeTruthy();
  expect(screen.getByRole('spinbutton', { name: 'direct.port' }).getAttribute('aria-invalid')).toBe('true');
  expect(screen.getByRole('textbox', { name: 'direct.probePath' }).getAttribute('aria-invalid')).toBe('true');
  expect(screen.getByRole('spinbutton', { name: 'direct.timeout' }).getAttribute('aria-invalid')).toBe('true');
  expect(calls('start_development_process')).toBe(0);
});

it('keeps a slow stop receipt after process selection changes without replacing the new detail', async () => {
  const stop = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockImplementation(async (command: string, args: { id?: string }) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'list_development_processes') return [process('run', 'running'), process('second', 'running', 'second output')];
    if (command === 'stop_development_process') return stop.promise;
    return process(args.id, 'running', args.id === 'second' ? 'second output' : 'first output');
  });
  show(); await flush();
  fireEvent.click(screen.getByTestId('dev-process-stop'));
  expectReceipt('stop', 'pending', 'run');
  fireEvent.click(screen.getByTestId('dev-process-stop')); expect(calls('stop_development_process')).toBe(1);
  fireEvent.click(screen.getByTestId('dev-process-history-second')); await flush();
  await act(async () => { stop.resolve(process('run', 'cancelled', 'late old stop output')); });
  expectReceipt('stop', 'succeeded', 'run');
  expect(screen.getByText('second output')).toBeTruthy();
  expect(screen.queryByText('late old stop output')).toBeNull();
  expect(useDevelopmentRuntime.getState().selected).toBe('second');
});

it('drops a pending operation receipt if the original account ends before acknowledgement', async () => {
  const start = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockReturnValue(start.promise);
  const pending = submitRuntimeOperation({ action: 'start', projectId: 'p', taskId: 't', input: { id: 'old-account-run', task_id: 't', kind: 'command', program: 'echo', args: [], cwd: '', env_refs: {}, trust_workspace: true, verification: false, readiness: null } });
  useDevelopment.getState().reset();
  start.resolve(process('old-account-run', 'running')); await pending;
  expect(useDevelopmentRuntime.getState().operations).toEqual([]);
  expect(useDevelopmentRuntime.getState().processes).toEqual([]);
});

it('preserves task-wide exclusion between save and stop and project-wide catalog exclusion', async () => {
  const save = Promise.withResolvers<{ commands: []; discovered: [] }>();
  invoke.mockReturnValue(save.promise);
  const command = { id: 'command', name: 'Name', program: 'echo', args: [], cwd: '', env_refs: {} };
  const first = submitRuntimeOperation({ action: 'save', projectId: 'p', taskId: 't', command, commands: [command] });
  expect(await submitRuntimeOperation({ action: 'stop', projectId: 'p', taskId: 't', processId: 'run' })).toBeNull();
  expect(await submitRuntimeOperation({ action: 'save', projectId: 'p', taskId: 'other', command, commands: [command] })).toBeNull();
  expect(calls('save_development_commands')).toBe(1);
  expect(calls('stop_development_process')).toBe(0);
  save.resolve({ commands: [], discovered: [] }); await first;
});

it('does not replay a retained catalog receipt over a fresh catalog after remount', async () => {
  const command = { id: 'saved', name: 'Old name', program: 'echo', args: [], cwd: '', env_refs: {} };
  invoke.mockResolvedValue({ commands: [command], discovered: [] });
  await submitRuntimeOperation({ action: 'save', projectId: 'p', taskId: 't', command, commands: [command] });
  invoke.mockImplementation(async (name: string) => name === 'read_development_commands' ? { commands: [{ ...command, name: 'Fresh external name' }], discovered: [] } : name === 'list_development_processes' ? [] : process());
  show(); await flush();
  expect(screen.getByRole('option', { name: 'Fresh external name' })).toBeTruthy();
  expect(screen.queryByRole('option', { name: 'Old name' })).toBeNull();
  expectReceipt('save', 'succeeded', command.id);
});

it('keeps a mismatched start acknowledgement unknown without projecting its output', async () => {
  invoke.mockImplementation(async (name: string) => name === 'read_development_commands' ? { commands: [], discovered: [] } : name === 'list_development_processes' ? [] : { ...process('wrong', 'running', 'WRONG_RECEIPT'), task_id: 'other' });
  show(); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } }); fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-process-start')); await flush();
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'start')?.phase).toBe('unknown');
  expectReceipt('start', 'unknown');
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'start')?.error).toBe('target_mismatch');
  expect(useDevelopmentRuntime.getState().operations.filter(item => item.taskId === (useDevelopmentRuntime.getState().taskId ?? '') && item.action === 'start' && item.phase === 'accepted')).toEqual([]);
  expect(screen.queryByText('WRONG_RECEIPT')).toBeNull();
  expect(useDevelopmentRuntime.getState().processes).toEqual([]);
});

it('retains an unmatched save as unknown and retries only the catalog read', async () => {
  let commands: { id: string; name: string; program: string; args: string[]; cwd: string; env_refs: {} }[] = [];
  let submitted = commands;
  invoke.mockImplementation(async (name: string, args: { input?: { commands: typeof commands } }) => {
    if (name === 'read_development_commands') return { commands, discovered: [] };
    if (name === 'save_development_commands') { submitted = args.input!.commands; throw 'storage_failed'; }
    return name === 'list_development_processes' ? [] : process();
  });
  show(); await flush();
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'Unconfirmed' } });
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' })); await flush();
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'save')?.phase).toBe('unknown');
  expectReceipt('save', 'unknown', submitted[0].id);
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' })); expect(calls('save_development_commands')).toBe(1);
  commands = submitted;
  fireEvent.click(screen.getByRole('button', { name: 'direct.operation.confirm' })); await flush();
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'save')?.phase).toBe('succeeded');
  expectReceipt('save', 'succeeded', submitted[0].id);
  expect(calls('save_development_commands')).toBe(1);
});

it('converges a nonterminal stop from background history without resending it', async () => {
  let ended = false;
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process('run', ended ? 'cancelled' : 'running')] : process('run', 'running'));
  const view = show(); await flush();
  fireEvent.click(screen.getByTestId('dev-process-stop')); await flush();
  expect((screen.getByTestId('dev-process-start') as HTMLButtonElement).disabled).toBe(true);
  await view.activity(false);
  ended = true;
  await act(async () => { await useDevelopmentRuntime.getState().refresh('t'); });
  expect(useDevelopmentRuntime.getState().operations.find(item => item.action === 'stop')?.phase).toBe('succeeded');
  await view.activity(true);
  expect((screen.getByTestId('dev-process-start') as HTMLButtonElement).disabled).toBe(false);
  expect(calls('stop_development_process')).toBe(1);
});

it('does not infer a terminal unknown process from its finished timestamp', async () => {
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? [process('run', 'unknown')] : process('run', 'unknown'));
  show(); await flush();
  fireEvent.click(screen.getByTestId('dev-process-stop')); await flush();
  expect(useDevelopmentRuntime.getState().operations[0]?.phase).not.toBe('succeeded');
  expect((screen.getByTestId('dev-process-start') as HTMLButtonElement).disabled).toBe(true);
});

it('keeps acceptance separate from a later launch failure and distinguishes confirmed exits', async () => {
  let current: DevelopmentProcess | null = null;
  invoke.mockImplementation(async (command: string, args: { input?: { id: string } }) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'start_development_process') { current = { ...process(args.input!.id, 'starting'), kind: 'service', finished_at: null }; return current; }
    if (command === 'list_development_processes') return current ? [current] : [];
    return current;
  });
  show(); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'missing-program' } }); fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-process-start')); await flush();
  expect(useDevelopmentRuntime.getState().operations[0]?.phase).toBe('accepted');
  expect(screen.getByTestId('dev-process-detail').textContent).toContain('direct.status.starting');
  current = { ...current!, status: 'failed', reason: 'process_spawn_failed', finished_at: 2 };
  await advance(1000);
  expect(screen.getByTestId('dev-process-detail').textContent).toContain('direct.status.failed');
  expect(useDevelopmentRuntime.getState().operations[0]?.phase).toBe('accepted');
  expect(calls('start_development_process')).toBe(1);
});

it('rejects pre-stop reads and wrong-project terminal snapshots until a new matching read', async () => {
  const old = Promise.withResolvers<DevelopmentProcess>();
  invoke.mockImplementation(async (command: string) => command === 'read_development_process' ? old.promise : process('run', 'running'));
  const reading = readDevelopmentProcess('run');
  await submitRuntimeOperation({ action: 'stop', projectId: 'p', taskId: 't', processId: 'run' });
  old.resolve(process('run', 'cancelled')); await reading;
  expect(runtimeOperationBlocked(useDevelopmentRuntime.getState().operations, 'p', 't', 'start')).toBe(true);
  invoke.mockResolvedValue({ ...process('run', 'cancelled'), project_id: 'other-project' });
  await readDevelopmentProcess('run');
  expect(runtimeOperationBlocked(useDevelopmentRuntime.getState().operations, 'p', 't', 'start')).toBe(true);
  invoke.mockResolvedValue(process('run', 'cancelled'));
  await retryRuntimeOperation(useDevelopmentRuntime.getState().operations[0].id);
  expect(runtimeOperationBlocked(useDevelopmentRuntime.getState().operations, 'p', 't', 'start')).toBe(false);
  expect(calls('stop_development_process')).toBe(1);
});

it.each(['exited', 'failed', 'cancelled', 'unknown', 'cleanup_pending'])('retains the host %s outcome when a history record disappears', async status => {
  let missing = false;
  invoke.mockImplementation(async (command: string) => command === 'read_development_commands' ? { commands: [], discovered: [] } : command === 'list_development_processes' ? missing ? [] : [process('run', status)] : process('run', status));
  show(); await flush();
  expect(processActive(process('run', status))).toBe(['unknown', 'cleanup_pending'].includes(status));
  missing = true; fireEvent.click(screen.getByRole('button', { name: 'review.refresh' })); await flush();
  expect(calls('stop_development_process')).toBe(0);
});

it('retains command drafts across internal unmount and task changes without carrying trust to another target', async () => {
  const view = render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  openConfiguration();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'retained-command-draft' } });
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'Retained name' } });
  view.unmount();
  const restored = render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  expect((screen.getByTestId('dev-process-program') as HTMLInputElement).value).toBe('retained-command-draft');
  restored.rerender(<DevelopmentRuntime projectId="p" taskId="other" />); await flush();
  expect((screen.getByTestId('dev-process-program') as HTMLInputElement).value).toBe('');
  restored.rerender(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  openConfiguration();
  expect((screen.getByRole('textbox', { name: 'direct.commandName' }) as HTMLInputElement).value).toBe('Retained name');
  expect((screen.getByTestId('dev-process-trust') as HTMLInputElement).checked).toBe(false);
  await act(async () => { useDevelopment.getState().reset(); });
  expect((screen.getByTestId('dev-process-program') as HTMLInputElement).value).toBe('');
  expect(calls('start_development_process')).toBe(0);
});

it.each([false, true])('selects a new accepted start unless the user changed process while waiting (%s)', async changed => {
  const start = Promise.withResolvers<DevelopmentProcess>(); let id = '';
  invoke.mockImplementation(async (command: string, args: { input?: { id: string } }) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'start_development_process') { id = args.input!.id; return start.promise; }
    if (command === 'list_development_processes') return [process('old'), process('other'), ...(id ? [process(id, 'running')] : [])];
    return process(useDevelopmentRuntime.getState().selected ?? 'old');
  });
  show(); await flush();
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } }); fireEvent.click(screen.getByTestId('dev-process-trust'));
  fireEvent.click(screen.getByTestId('dev-process-start'));
  if (changed) { fireEvent.click(screen.getByTestId('dev-process-history-other')); await flush(); }
  await act(async () => { start.resolve(process(id, 'running')); });
  expect(useDevelopmentRuntime.getState().selected).toBe(changed ? 'other' : id);
  expect(calls('start_development_process')).toBe(1);
});

it('does not reuse an old context detail when returning to the same process', async () => {
  const old = Promise.withResolvers<DevelopmentProcess>();
  let reads = 0;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_commands') return { commands: [], discovered: [] };
    if (command === 'list_development_processes') return [process()];
    if (command === 'read_development_process') return ++reads === 1 ? old.promise : process('run', 'exited', 'fresh restored context');
    return null;
  });
  show(); await flush();
  expect(reads).toBe(1);
  await act(async () => { await chooseContext({ project_id: 'p', task_id: 'other', session_id: null }); });
  await act(async () => { await chooseContext({ project_id: 'p', task_id: 't', session_id: null }); });
  expect(screen.getByText('fresh restored context')).toBeTruthy();
  await act(async () => { old.resolve(process('run', 'exited', 'old context output')); });
  expect(screen.getByText('fresh restored context')).toBeTruthy();
  expect(screen.queryByText('old context output')).toBeNull();
  expect(calls('start_development_process')).toBe(0);
});

it('merges a late save into the original task draft and updates that command after returning', async () => {
  const pending = Promise.withResolvers<unknown>();
  let commands: { id: string; name: string; program: string; args: string[]; cwd: string; env_refs: {} }[] = [];
  let saves = 0;
  invoke.mockImplementation(async (command: string, args: any) => {
    if (command === 'read_development_commands') return { commands, discovered: [] };
    if (command === 'list_development_processes') return [];
    if (command === 'save_development_commands') { expect(args.input.task_id).toBe('t'); commands = args.input.commands; return ++saves === 1 ? pending.promise : { commands, discovered: [] }; }
    return null;
  });
  const first = render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  openConfiguration();
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'Original command' } });
  fireEvent.change(screen.getByTestId('dev-process-program'), { target: { value: 'echo' } });
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' })); first.unmount();
  await act(async () => { pending.resolve({ commands, discovered: [] }); });
  render(<DevelopmentRuntime projectId="p" taskId="t" />); await flush();
  openConfiguration();
  const originalId = commands[0].id;
  fireEvent.change(screen.getByRole('textbox', { name: 'direct.commandName' }), { target: { value: 'Updated command' } });
  fireEvent.click(screen.getByRole('button', { name: 'direct.saveCommand' })); await flush();
  expect(commands).toEqual([{ id: originalId, name: 'Updated command', program: 'echo', args: [], cwd: '', env_refs: {} }]);
});
