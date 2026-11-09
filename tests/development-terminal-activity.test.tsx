import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi, type Mock } from 'vitest';
import { DevelopmentTerminal } from '../src/components/development/DevelopmentTerminal';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentRuntime, type DevelopmentProcess } from '../src/stores/developmentRuntime';
import { chooseContext } from '../src/stores/developmentSelection';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';

const originalFontsDescriptor = Object.getOwnPropertyDescriptor(document, 'fonts');
interface TerminalDouble {
  rows: number; cols: number; options: Record<string, unknown>; chunks: string[]; resets: number; disposed: boolean;
  refreshes: Array<[number, number]>; resizeCalls: Array<[number, number]>; rendered: string; accessible: string; focus: Mock;
  data: (value: string) => void; key: () => boolean;
}
const harness = vi.hoisted(() => ({ invoke: vi.fn(), instances: [] as TerminalDouble[], observers: [] as Array<{ connected: boolean; callback: () => void }>, width: 640, height: 320, cols: 80, rows: 24, fits: 0, metrics: true, measurable: true, deferRestore: false, restored: [] as Array<() => void> }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: harness.invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }) }));
vi.mock('@xterm/xterm', () => ({ Terminal: class {
  rows = 24; cols = 80; options: Record<string, unknown>; chunks: string[] = []; resets = 0; disposed = false;
  refreshes: Array<[number, number]> = []; resizeCalls: Array<[number, number]> = []; rendered = ''; accessible = ''; focus = vi.fn();
  data: (data: string) => void = () => {}; key: () => boolean = () => true;
  constructor(options: Record<string, unknown>) { this.options = options; this.rows = Number(options.rows ?? 24); this.cols = Number(options.cols ?? 80); harness.instances.push(this); }
  loadAddon() {} open() {} write(value: string, callback?: () => void) { this.chunks.push(value); if (callback) { if (harness.deferRestore) harness.restored.push(callback); else callback(); } } reset() { this.resets++; }
  resize(cols: number, rows: number) { this.resizeCalls.push([cols, rows]); this.cols = cols; this.rows = rows; if (harness.measurable) harness.metrics = true; }
  refresh(start: number, end: number) { this.refreshes.push([start, end]); this.rendered = this.accessible = this.chunks.join(''); }
  onData(callback: (data: string) => void) { this.data = callback; return { dispose() {} }; }
  attachCustomKeyEventHandler(callback: () => boolean) { this.key = callback; }
  dispose() { this.disposed = true; }
} }));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class {
  proposeDimensions() { harness.fits++; return harness.metrics ? { cols: harness.cols, rows: harness.rows } : undefined; }
  fit() { harness.fits++; }
} }));
const process = (output = 'one\r\n', status = 'running'): DevelopmentProcess => ({ id: 'pty', project_id: 'p', task_id: 't', kind: 'terminal', source: 'host_terminal', status, output, truncated: false, exit_code: null, reason: null, started_at: 1, finished_at: status === 'running' ? null : 2, program: '/bin/sh', args: [], cwd: '', fingerprint: '', start_fingerprint: '', verification_status: 'not_requested', verification_complete: false, mutation_observed: false, preview_url: null });
const calls = (command: string) => harness.invoke.mock.calls.filter(([name]) => name === command);
async function advance(ms = 32) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }
function show() {
  const controller = createWorkspaceController();
  const tree = (visible = true, active = visible, value = process(), mounted = true) => <div hidden={!visible}><SurfaceScope id="agent" visible={visible} active={active} controller={controller}>{mounted && <DevelopmentTerminal process={value} />}</SurfaceScope></div>;
  const view = render(tree());
  return { async update(visible = true, active = visible, value = process(), mounted = true) { view.rerender(tree(visible, active, value, mounted)); await advance(); } };
}
beforeEach(() => {
  vi.useFakeTimers(); useDevelopment.getState().reset();
  useDevelopmentRuntime.setState({ taskId: 't', selected: 'pty', processes: [process()], operations: [] });
  harness.width = 640; harness.height = 320; harness.cols = 80; harness.rows = 24; harness.fits = 0; harness.metrics = true; harness.measurable = true;
  harness.deferRestore = false; harness.restored.length = 0;
  harness.invoke.mockResolvedValue(null);
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => harness.width);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(() => harness.height);
  vi.stubGlobal('ResizeObserver', class {
    connected = false; constructor(public callback: () => void) { harness.observers.push(this); }
    observe() { this.connected = true; } disconnect() { this.connected = false; }
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); if (originalFontsDescriptor) Object.defineProperty(document, 'fonts', originalFontsDescriptor); else Reflect.deleteProperty(document, 'fonts'); harness.invoke.mockReset(); harness.instances.length = 0; harness.observers.length = 0; vi.useRealTimers(); });
it('stops hidden measurements and idle retries, maintains covered size, restores without reconnecting or input replay', async () => {
  const view = show(); await advance(); const instance = harness.instances[0];
  instance.data('first\r'); await advance();
  const initial = { fits: harness.fits, resize: calls('resize_development_terminal').length };
  await advance(4600);
  expect(harness.fits).toBe(initial.fits); expect(calls('resize_development_terminal')).toHaveLength(initial.resize);
  await view.update(false); const fits = harness.fits; const resizes = calls('resize_development_terminal').length;
  harness.cols = 100; harness.rows = 30; instance.data('hidden\r');
  await advance(4600);
  expect(harness.fits).toBe(fits); expect(calls('resize_development_terminal')).toHaveLength(resizes);
  expect(harness.observers.filter(row => row.connected)).toHaveLength(0); expect(instance.key()).toBe(false);
  await view.update(true, false); expect(instance.key()).toBe(false); instance.data('covered\r'); await advance();
  expect(calls('resize_development_terminal').at(-1)?.[1]).toEqual({ input: { id: 'pty', rows: 30, cols: 100 } });
  harness.cols = 110; harness.observers.filter(row => row.connected).forEach(row => row.callback()); await advance();
  expect(calls('resize_development_terminal').at(-1)?.[1].input.cols).toBe(110);
  for (let i = 0; i < 5; i++) { await view.update(false); await view.update(true); }
  instance.data('last\r'); await advance();
  expect(harness.instances).toHaveLength(1); expect(harness.observers.filter(row => row.connected)).toHaveLength(1);
  expect(calls('write_development_terminal').map(([, args]) => args.input.data)).toEqual(['first\r', 'last\r']);
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
});
it('waits for valid layout and retries only terminal startup, ending after host success or hiding', async () => {
  harness.height = 0;
  const view = show(); await advance(); expect(calls('resize_development_terminal')).toHaveLength(0);
  harness.height = 320; harness.cols = Number.NaN;
  harness.observers.filter(row => row.connected).forEach(row => row.callback()); await advance();
  expect(calls('resize_development_terminal')).toHaveLength(0);
  harness.cols = 80; let requests = 0;
  harness.invoke.mockImplementation(async (command: string) => { if (command === 'resize_development_terminal' && ++requests === 1) throw 'terminal_starting'; return null; });
  harness.observers.filter(row => row.connected).forEach(row => row.callback()); await advance();
  await advance(1550); expect(calls('resize_development_terminal')).toHaveLength(2);
  await advance(4600); expect(calls('resize_development_terminal')).toHaveLength(2);
  harness.cols = 90; harness.invoke.mockRejectedValue('terminal_starting');
  harness.observers.filter(row => row.connected).forEach(row => row.callback()); await advance();
  await view.update(false); const count = calls('resize_development_terminal').length; await advance(4600);
  expect(calls('resize_development_terminal')).toHaveLength(count);
  harness.invoke.mockResolvedValue(null); await view.update();
  expect(calls('resize_development_terminal')).toHaveLength(count + 1);
  expect(screen.queryByTestId('dev-terminal-size-feedback')).toBeNull();
});
it('preserves snapshot append/reset and exit/stop history across hiding and internal panel remount', async () => {
  const view = show(); await advance(); const first = harness.instances[0];
  await view.update(false, false, process('one\r\ntwo\r\n'));
  await view.update(true, true, process('one\r\ntwo\r\nthree\r\n'));
  expect(first.chunks.join('')).toBe('one\r\ntwo\r\nthree\r\n');
  const bounded = 'retained snapshot\r\n';
  await view.update(true, true, { ...process(bounded, 'exited'), truncated: true });
  expect(first.resets).toBe(1); expect(first.chunks.at(-1)).toBe(bounded);
  const count = calls('resize_development_terminal').length;
  first.data('after-exit'); await advance(4600); expect(calls('resize_development_terminal')).toHaveLength(count);
  await view.update(true, true, process(bounded, 'exited'), false); expect(first.disposed).toBe(true);
  await view.update(true, true, process(bounded, 'exited'));
  expect(harness.instances[1].chunks).toEqual([bounded]); expect(calls('write_development_terminal')).toHaveLength(0);
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
});
it('keeps repeated real TICK lines while appending the accessibility reflow snapshots exactly once', async () => {
  const first = `${'P'.repeat(148)}\r\nMARK\r\nTICK:1\r\n`;
  const second = `${first}TICK:2\r\nTICK:2\r\n`;
  const view = show(); await advance(); const instance = harness.instances[0];
  await view.update(true, true, process(first));
  const reset = instance.resets;
  const boundary = instance.chunks.length;
  harness.cols = 57;
  harness.observers.filter(row => row.connected).forEach(row => row.callback()); await advance();
  await view.update(true, true, process(second));
  await view.update(true, true, process(second));
  expect(instance.options.screenReaderMode).toBe(true);
  expect(instance.cols).toBe(57);
  expect(instance.resets).toBe(reset);
  expect(instance.chunks.slice(boundary)).toEqual(['TICK:2\r\nTICK:2\r\n']);
  expect(instance.chunks.at(-2) + instance.chunks.at(-1)).toBe(second);
  expect(calls('write_development_terminal')).toHaveLength(0);
});

it('serializes a slow resize across repeated hide/restore and sends only the latest dimensions', async () => {
  let finish!: () => void;
  harness.invoke.mockImplementation(() => new Promise<null>(resolve => { finish = () => resolve(null); }));
  const view = show(); await advance();
  expect(calls('resize_development_terminal')).toHaveLength(1);
  harness.cols = 120;
  for (let i = 0; i < 5; i++) { await view.update(false); await view.update(true); }
  expect(calls('resize_development_terminal')).toHaveLength(1);
  harness.invoke.mockResolvedValue(null);
  await act(async () => { finish(); }); await advance();
  expect(calls('resize_development_terminal')).toHaveLength(2);
  expect(calls('resize_development_terminal').at(-1)?.[1].input.cols).toBe(120);
  await advance(4600); expect(calls('resize_development_terminal')).toHaveLength(2);
});

it('keeps a terminal-starting view local while the process remains running, then clears it on size success', async () => {
  let resizes = 0;
  harness.invoke.mockImplementation(async (command: string) => {
    if (command === 'resize_development_terminal' && ++resizes === 1) throw 'terminal_starting';
    return null;
  });
  const view = show(); await advance();
  const feedback = screen.getByTestId('dev-terminal-size-feedback');
  expect(within(feedback).getByRole('status')).toBeTruthy();
  expect(useDevelopmentRuntime.getState().processes[0]).toMatchObject({ status: 'running', finished_at: null });
  expect(useDevelopmentRuntime.getState().operations).toEqual([]);
  expect(screen.queryByRole('alert')).toBeNull();
  await advance(1400); expect(calls('resize_development_terminal')).toHaveLength(1);
  await advance(150); expect(calls('resize_development_terminal')).toHaveLength(2);
  expect(screen.queryByTestId('dev-terminal-size-feedback')).toBeNull();
  await view.update(false); await view.update(true); await advance(4600);
  expect(calls('resize_development_terminal')).toHaveLength(2);
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
});

it('recovers a size-only failure through a gated size retry without changing process state or output', async () => {
  harness.invoke.mockImplementation(async (command: string) => { if (command === 'resize_development_terminal') throw 'unknown'; return null; });
  const view = show(); await advance(); const instance = harness.instances[0];
  const feedback = screen.getByTestId('dev-terminal-size-feedback');
  expect(within(feedback).getByRole('alert')).toBeTruthy();
  await advance(4600); expect(calls('resize_development_terminal')).toHaveLength(1);
  await view.update(true, false);
  expect((within(feedback).getByRole('button') as HTMLButtonElement).disabled).toBe(true);
  await view.update();
  harness.invoke.mockResolvedValue(null);
  await act(async () => { fireEvent.click(within(feedback).getByRole('button')); }); await advance();
  expect(calls('resize_development_terminal')).toHaveLength(2);
  expect(screen.queryByTestId('dev-terminal-size-feedback')).toBeNull();
  expect(instance.chunks).toEqual(['one\r\n']); expect(instance.resets).toBe(0);
  expect(useDevelopmentRuntime.getState().processes[0]).toMatchObject({ status: 'running', finished_at: null });
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
});

it.each(['unknown', 'terminal_starting'])('never replays %s input through size recovery, visibility restoration, or dismissing its warning', async failure => {
  harness.invoke.mockImplementation(async (command: string) => {
    if (command === 'write_development_terminal') throw failure;
    if (command === 'resize_development_terminal') throw 'unknown';
    return null;
  });
  const view = show(); await advance(); const instance = harness.instances[0];
  instance.data('possibly-sent\r'); await advance();
  expect(within(screen.getByTestId('dev-terminal-input-feedback')).getByRole('alert')).toBeTruthy();
  harness.invoke.mockResolvedValue(null);
  await act(async () => { fireEvent.click(within(screen.getByTestId('dev-terminal-size-feedback')).getByRole('button')); }); await advance();
  expect(screen.queryByTestId('dev-terminal-size-feedback')).toBeNull();
  expect(screen.getByTestId('dev-terminal-input-feedback')).toBeTruthy();
  await view.update(false); await advance(4600); await view.update();
  await act(async () => { fireEvent.click(within(screen.getByTestId('dev-terminal-input-feedback')).getByRole('button')); });
  expect(screen.queryByTestId('dev-terminal-input-feedback')).toBeNull();
  instance.data('new-input\r'); await advance();
  expect(calls('write_development_terminal').map(([, args]) => args.input.data)).toEqual(['possibly-sent\r', 'new-input\r']);
  expect(harness.instances).toHaveLength(1);
  expect(useDevelopmentRuntime.getState().processes[0]).toMatchObject({ status: 'running', finished_at: null });
});

it.each(['process', 'account', 'context'] as const)('discards late size and input failures after changing %s ownership', async change => {
    const reject: Array<(reason: string) => void> = [];
    harness.invoke.mockImplementation((command: string) => {
      if (command !== 'resize_development_terminal' && command !== 'write_development_terminal') return Promise.resolve(null);
      const pending = Promise.withResolvers<null>();
      reject.push(pending.reject);
      return pending.promise;
    });
    const view = show(); await advance();
    harness.instances.at(-1)!.data('old-owner\r'); await advance();
    expect(reject).toHaveLength(2);
    harness.invoke.mockResolvedValue(null);
    if (change === 'process') await view.update(true, true, { ...process(), id: 'other-pty' });
    else if (change === 'account') { await act(async () => { useDevelopment.getState().reset(); }); await advance(); }
    else { await act(async () => { await chooseContext({ project_id: 'other-p', task_id: null, session_id: null }); }); await advance(); }
    await act(async () => { reject.forEach(fail => fail('unknown')); }); await advance();
    expect(screen.queryByTestId('dev-terminal-size-feedback')).toBeNull();
    expect(screen.queryByTestId('dev-terminal-input-feedback')).toBeNull();
    expect(calls('write_development_terminal').filter(([, args]) => args.input.data === 'old-owner\r')).toHaveLength(1);
});

it('bootstraps zero xterm metrics before fit and restores both projections without replaying a snapshot', async () => {
  harness.metrics = false;
  const view = show(); await advance(); const instance = harness.instances[0];
  expect(instance.resizeCalls[0]).toEqual([80, 24]);
  expect(calls('resize_development_terminal')).toHaveLength(1);
  expect(instance.rendered).toBe('one\r\n'); expect(instance.accessible).toBe(instance.rendered);
  await view.update(false, false, process('one\r\ntwo\r\n'));
  instance.rendered = instance.accessible = '';
  const writes = instance.chunks.length;
  await view.update(true, true, process('one\r\ntwo\r\n'));
  expect(instance.rendered).toBe('one\r\ntwo\r\n'); expect(instance.accessible).toBe(instance.rendered);
  expect(instance.chunks).toHaveLength(writes); expect(instance.resets).toBe(0);
  expect(instance.focus).not.toHaveBeenCalled(); expect(harness.instances).toHaveLength(1);
});

it('recovers background activation at unchanged dimensions and accepts only fresh input once', async () => {
  const view = show(); await advance(); const instance = harness.instances[0];
  const outside = document.createElement('button'); document.body.appendChild(outside); outside.focus();
  const hidden = vi.spyOn(document, 'hidden', 'get');
  hidden.mockReturnValue(true);
  act(() => { document.dispatchEvent(new Event('visibilitychange')); });
  expect(instance.options.disableStdin).toBe(true); expect(instance.key()).toBe(false);
  instance.data('background\r');
  await view.update(true, true, process('one\r\nbackground output\r\n'));
  instance.rendered = instance.accessible = '';
  const chunks = instance.chunks.length; const resizes = calls('resize_development_terminal').length;
  hidden.mockReturnValue(false);
  act(() => { document.dispatchEvent(new Event('visibilitychange')); window.dispatchEvent(new Event('focus')); window.dispatchEvent(new Event('pageshow')); });
  await advance();
  expect(instance.rendered).toBe('one\r\nbackground output\r\n'); expect(instance.accessible).toBe(instance.rendered);
  expect(instance.options.disableStdin).toBe(false); expect(instance.key()).toBe(true);
  expect(document.activeElement).toBe(outside); expect(instance.focus).not.toHaveBeenCalled();
  expect(instance.chunks).toHaveLength(chunks); expect(calls('resize_development_terminal')).toHaveLength(resizes);
  instance.data('fresh\r'); await advance();
  expect(calls('write_development_terminal').map(([, args]) => args.input.data)).toEqual(['fresh\r']);
  expect(harness.instances).toHaveLength(1); expect(calls('start_development_process')).toHaveLength(0);
  outside.remove();
});

it('bounds unavailable metric retries, cancels them when hidden and permits a later activation to recover', async () => {
  harness.metrics = false; harness.measurable = false;
  const view = show(); await advance(); const instance = harness.instances[0];
  await advance(1000); expect(harness.fits).toBe(4);
  await advance(4600); expect(harness.fits).toBe(4);
  act(() => { window.dispatchEvent(new Event('focus')); }); await advance();
  await view.update(false); const fits = harness.fits;
  await advance(1000); expect(harness.fits).toBe(fits);
  harness.measurable = true; await view.update();
  expect(instance.accessible).toBe('one\r\n'); expect(calls('resize_development_terminal')).toHaveLength(1);
  expect(instance.chunks).toEqual(['one\r\n']); expect(instance.resets).toBe(0);
});

it('opens while the app is hidden and waits for activation before fitting or accepting input', async () => {
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
  harness.metrics = false;
  show(); await advance(); const instance = harness.instances[0];
  expect(harness.fits).toBe(0); expect(instance.options.disableStdin).toBe(true);
  instance.data('must-not-send\r'); await advance();
  hidden.mockReturnValue(false);
  act(() => { document.dispatchEvent(new Event('visibilitychange')); }); await advance();
  expect(instance.rendered).toBe('one\r\n'); expect(instance.accessible).toBe(instance.rendered);
  expect(instance.chunks).toEqual(['one\r\n']); expect(instance.options.disableStdin).toBe(false);
  expect(calls('resize_development_terminal')).toHaveLength(1);
  expect(calls('write_development_terminal')).toHaveLength(0);
});

it('reveals a selected terminal once after hidden activation and preserves later user scrolling and focus', async () => {
  const scroll = vi.spyOn(Element.prototype, 'scrollIntoView');
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
  const outside = document.createElement('button'); document.body.appendChild(outside); outside.focus();
  const view = show(); await advance();
  expect(scroll).not.toHaveBeenCalled();
  hidden.mockReturnValue(false);
  act(() => { document.dispatchEvent(new Event('visibilitychange')); }); await advance();
  expect(scroll).toHaveBeenCalledTimes(1); expect(document.activeElement).toBe(outside);
  act(() => { window.dispatchEvent(new Event('focus')); window.dispatchEvent(new Event('pageshow')); }); await advance();
  expect(scroll).toHaveBeenCalledTimes(1);
  await view.update(false); await view.update(true);
  expect(scroll).toHaveBeenCalledTimes(2); expect(document.activeElement).toBe(outside);
  expect(harness.instances).toHaveLength(1); expect(calls('write_development_terminal')).toHaveLength(0);
  outside.remove();
});

it('updates the existing terminal from theme variables and remeasures loaded fonts without replaying input or output', async () => {
  const fonts = new EventTarget();
  Object.defineProperty(fonts, 'ready', { value: Promise.resolve() });
  Object.defineProperty(document, 'fonts', { configurable: true, get: () => fonts as FontFaceSet });
  let appearance = { family: 'System Mono, monospace', size: '12px', background: '#101010', foreground: '#eeeeee' };
  const computed = window.getComputedStyle.bind(window);
  vi.spyOn(window, 'getComputedStyle').mockImplementation((element, pseudo) => {
    if (!(element instanceof HTMLElement) || !element.classList.contains('dev-terminal')) return computed(element, pseudo);
    const variables: Record<string, string> = { '--font-family-code': appearance.family, '--font-size-code': appearance.size, '--surface-code': appearance.background, '--text-code': appearance.foreground };
    return { getPropertyValue: (name: string) => variables[name] ?? '' } as CSSStyleDeclaration;
  });
  const view = show(); await advance(); const instance = harness.instances[0];
  expect(instance.options).toMatchObject({ fontFamily: appearance.family, fontSize: 12, theme: { background: appearance.background, foreground: appearance.foreground, cursor: appearance.foreground } });
  instance.data('中文 mixed input\r'); await advance();
  const chunks = instance.chunks.slice(); const measures = instance.resizeCalls.length;
  appearance = { family: '"JetBrains Mono", ui-monospace, "PingFang SC", monospace', size: '20px', background: '#202020', foreground: '#fafafa' };
  harness.cols = 53; harness.rows = 17;
  await act(async () => { document.documentElement.setAttribute('data-preset', 'typography-test'); }); await advance();
  expect(instance.options).toMatchObject({ fontFamily: appearance.family, fontSize: 20, theme: { background: appearance.background, foreground: appearance.foreground, cursor: appearance.foreground } });
  expect(instance.resizeCalls.length).toBeGreaterThan(measures);
  expect(calls('resize_development_terminal').at(-1)?.[1]).toEqual({ input: { id: 'pty', rows: 17, cols: 53 } });
  const loadedMeasures = instance.resizeCalls.length;
  act(() => { fonts.dispatchEvent(new Event('loadingdone')); }); await advance();
  expect(instance.resizeCalls.length).toBeGreaterThan(loadedMeasures);
  expect(instance.resizeCalls.at(-1)).toEqual([53, 17]);
  expect(instance.chunks).toEqual(chunks); expect(instance.resets).toBe(0); expect(instance.focus).not.toHaveBeenCalled();
  expect(harness.instances).toHaveLength(1); expect(instance.disposed).toBe(false);
  expect(calls('write_development_terminal').map(([, args]) => args.input.data)).toEqual(['中文 mixed input\r']);
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
  await view.update(false); const hiddenMeasures = instance.resizeCalls.length;
  act(() => { fonts.dispatchEvent(new Event('loadingdone')); }); await advance();
  expect(instance.resizeCalls).toHaveLength(hiddenMeasures);
  await view.update(); expect(instance.resizeCalls.length).toBeGreaterThan(hiddenMeasures);
  await view.update(true, true, process(), false);
  const disposedMeasures = instance.resizeCalls.length;
  act(() => { fonts.dispatchEvent(new Event('loadingdone')); }); await advance();
  expect(instance.resizeCalls).toHaveLength(disposedMeasures);
  document.documentElement.removeAttribute('data-preset');
});

it('restores acknowledged size and clear-display position after remount without deleting history or replaying input', async () => {
  const view = show(); await advance(); const first = harness.instances[0];
  first.data('once\r'); await advance();
  fireEvent.click(screen.getByTestId('dev-terminal-clear')); await advance();
  expect(first.chunks.at(-1)).toBe('\x1b[2J\x1b[3J\x1b[H');
  expect(useDevelopmentRuntime.getState().processes[0].output).toBe('one\r\n');
  await view.update(false, false, process('one\r\ntwo\r\n'));
  await view.update(false, false, process('one\r\ntwo\r\n'), false);
  await view.update(true, true, process('one\r\ntwo\r\n'));
  const restored = harness.instances[1];
  expect(restored.chunks).toEqual(['two\r\n']);
  expect(restored.options).toMatchObject({ rows: 24, cols: 80 });
  expect(calls('resize_development_terminal')).toHaveLength(1);
  expect(calls('write_development_terminal').map(([, args]) => args.input.data)).toEqual(['once\r']);
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
  await view.update(true, true, { ...process('other task\r\n'), id: 'other', task_id: 'other-task' });
  expect(harness.instances[2].chunks).toEqual(['other task\r\n']);
});

it('suppresses terminal-query responses while replaying a restored snapshot and accepts only fresh input afterwards', async () => {
  harness.deferRestore = true;
  show(); await advance(); const instance = harness.instances[0];
  instance.data('\x1b[1;1R'); await advance();
  expect(calls('write_development_terminal')).toHaveLength(0);
  act(() => { harness.restored.splice(0).forEach(ready => ready()); });
  instance.data('fresh\r'); await advance();
  expect(calls('write_development_terminal').map(([, args]) => args.input.data)).toEqual(['fresh\r']);
});

it('keeps an unknown-write warning across remount without replaying it through clear display', async () => {
  harness.invoke.mockImplementation(async (command: string) => { if (command === 'write_development_terminal') throw 'unknown'; return null; });
  const view = show(); await advance(); harness.instances[0].data('unconfirmed\r'); await advance();
  await view.update(true, true, process(), false); await view.update();
  expect(screen.getByTestId('dev-terminal-input-feedback')).toBeTruthy();
  fireEvent.click(screen.getByTestId('dev-terminal-clear')); await advance();
  expect(screen.getByTestId('dev-terminal-input-feedback')).toBeTruthy();
  expect(calls('write_development_terminal')).toHaveLength(1);
});

it('treats parent panel collapse independently of surface activity and never makes native output interactive', async () => {
  const value = process();
  const view = render(<DevelopmentTerminal process={value} panelVisible={false} />); await advance();
  const instance = harness.instances[0]; instance.data('collapsed\r'); await advance();
  expect(calls('resize_development_terminal')).toHaveLength(0); expect(calls('write_development_terminal')).toHaveLength(0);
  view.rerender(<DevelopmentTerminal process={value} panelVisible />); await advance();
  expect(harness.instances).toHaveLength(1); expect(calls('resize_development_terminal')).toHaveLength(1);
  view.rerender(<DevelopmentTerminal process={{ ...value, id: 'native', source: 'native_agent' }} panelVisible />); await advance();
  harness.instances.at(-1)!.data('native\r'); await advance();
  expect(calls('resize_development_terminal')).toHaveLength(1); expect(calls('write_development_terminal')).toHaveLength(0);
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
});

it.each([[0, 80], [24, 0], [1001, 80], [24, 1001], [1.5, 80], [24, Number.POSITIVE_INFINITY]])('rejects out-of-bound layout %s × %s before xterm or PTY resizing', async (rows, cols) => {
  harness.rows = rows; harness.cols = cols;
  show(); await advance();
  expect(calls('resize_development_terminal')).toHaveLength(0);
  expect(harness.instances[0].resizeCalls.every(([width, height]) => width === 80 && height === 24)).toBe(true);
});

it('serializes a resize still in flight across remount and dispatches only the latest valid size afterwards', async () => {
  const pending = Promise.withResolvers<null>();
  harness.invoke.mockImplementation((command: string) => command === 'resize_development_terminal' ? pending.promise : Promise.resolve(null));
  const view = show(); await advance();
  await view.update(true, true, process(), false);
  harness.cols = 100;
  await view.update();
  expect(calls('resize_development_terminal')).toHaveLength(1);
  harness.invoke.mockResolvedValue(null);
  await act(async () => { pending.resolve(null); }); await advance();
  expect(calls('resize_development_terminal')).toHaveLength(2);
  expect(calls('resize_development_terminal').at(-1)?.[1].input).toEqual({ id: 'pty', rows: 24, cols: 100 });
  expect(calls('write_development_terminal')).toHaveLength(0);
  expect(calls('start_development_process')).toHaveLength(0); expect(calls('stop_development_process')).toHaveLength(0);
});
