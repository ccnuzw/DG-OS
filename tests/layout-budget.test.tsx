import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useLayoutBudget } from '../src/ui/layout/useLayoutBudget';
import { shellMinimumWidth, workbenchMinimumWidth } from '../src/ui/layout/layoutBudgets';
import { PanelResizer, useWorkbenchLayout } from '../src/components/development/WorkbenchLayout';

let width = 1400;
let resize: () => void;
let frames: Map<number, FrameRequestCallback>;
let sequence: number;
let fonts: EventTarget;
function flush() { act(() => { const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0)); }); }
async function appearance() { await act(async () => {}); flush(); }
function Probe() {
  const { root, narrow } = useLayoutBudget<HTMLDivElement>(workbenchMinimumWidth);
  return <div ref={root}><output>{narrow ? 'compact' : 'full'}</output><input aria-label="draft" /></div>;
}
function WorkbenchProbe() {
  const { root, compact, layout, setLayout } = useWorkbenchLayout();
  return <section ref={root}><output>{compact ? 'compact' : 'full'}</output><input aria-label="draft" /><PanelResizer axis="agent" value={layout.agentWidth} onChange={agentWidth => setLayout(value => ({ ...value, agentWidth }))} label="Agent width" testId="split" controls="agent" /></section>;
}
beforeEach(() => {
  width = 1400; frames = new Map(); sequence = 0; fonts = new EventTarget();
  document.documentElement.style.fontSize = '14px';
  document.documentElement.style.setProperty('--font-size-ui', '12px');
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => width);
  vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { resize = callback; } observe() {} disconnect() {} });
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++sequence, callback); return sequence; });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
  Object.defineProperty(document, 'fonts', { configurable: true, value: fonts });
});
afterEach(() => { cleanup(); localStorage.clear(); document.documentElement.removeAttribute('style'); document.documentElement.removeAttribute('data-density'); Reflect.deleteProperty(document, 'fonts'); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('layout budgets', () => {
  it('requires the complete workbench plus the hypothetically expanded rail', () => {
    expect(workbenchMinimumWidth(document.body)).toBe(961);
    expect(shellMinimumWidth(document.body)).toBe(1149);
  });
  it.each([[12.6, 877.1, 1053.1], [14, 961, 1149], [16.8, 1152.8, 1368.8]])('calibrates geometry at root %spx without scaling the UI font', (geometry, workbench, shell) => {
    document.documentElement.style.fontSize = `${geometry}px`;
    expect(workbenchMinimumWidth(document.body)).toBeCloseTo(workbench);
    expect(shellMinimumWidth(document.body)).toBeCloseTo(shell);
  });
  it('changes at the container boundary and ignores zero-sized hidden containers', () => {
    render(<Probe />); flush();
    width = 960; resize(); flush(); expect(screen.getByText('compact')).toBeTruthy();
    width = 0; resize(); flush(); expect(screen.getByText('compact')).toBeTruthy();
    width = 961; resize(); flush(); expect(screen.getByText('full')).toBeTruthy();
    resize(); resize(); expect(frames.size).toBe(1); flush();
  });
  it('remeasures independent font and density changes without a resize', async () => {
    width = 1000; render(<Probe />); flush(); expect(screen.getByText('full')).toBeTruthy();
    document.documentElement.style.setProperty('--font-size-ui', '16px'); await appearance();
    expect(screen.getByText('compact')).toBeTruthy();
    document.documentElement.style.setProperty('--font-size-ui', '12px'); await appearance();
    expect(screen.getByText('full')).toBeTruthy();
    document.documentElement.style.fontSize = '16.8px'; document.documentElement.dataset.density = '120'; await appearance();
    expect(screen.getByText('compact')).toBeTruthy();
    act(() => fonts.dispatchEvent(new Event('loadingdone'))); expect(frames.size).toBe(1); flush();
  });
});

it('keeps explicit split preferences, draft nodes and focus through automatic compact transitions', () => {
  localStorage.setItem('dgos.development.layout.v2', JSON.stringify({ navigation: false, agent: true, runtime: true, agentWidth: 55, runtimeHeight: 61, codeView: 'changes', compactView: 'agent', runtimeView: 'runs' }));
  const view = render(<WorkbenchProbe />); flush();
  const draft = screen.getByRole('textbox', { name: 'draft' });
  fireEvent.change(draft, { target: { value: 'unsent work' } }); draft.focus();
  const saved = localStorage.getItem('dgos.development.layout.v2');
  width = 700; resize(); flush(); expect(screen.getByText('compact')).toBeTruthy();
  width = 1400; resize(); flush(); expect(screen.getByText('full')).toBeTruthy();
  expect(localStorage.getItem('dgos.development.layout.v2')).toBe(saved);
  expect(screen.getByRole('textbox', { name: 'draft' })).toBe(draft);
  expect(draft).toHaveProperty('value', 'unsent work'); expect(document.activeElement).toBe(draft);
  fireEvent.keyDown(screen.getByRole('separator'), { key: 'ArrowRight' });
  expect(screen.getByRole('separator').getAttribute('aria-valuenow')).toBe('53');
  expect(JSON.parse(localStorage.getItem('dgos.development.layout.v2') ?? '{}')).toMatchObject({ navigation: false, agentWidth: 53, runtimeHeight: 61, codeView: 'changes', compactView: 'agent' });
  view.unmount(); render(<WorkbenchProbe />); flush();
  expect(screen.getByRole('separator').getAttribute('aria-valuenow')).toBe('53');
});
