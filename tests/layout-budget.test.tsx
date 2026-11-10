import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useLayoutBudget } from '../src/ui/layout/useLayoutBudget';
import { shellMinimumWidth, workbenchMinimumWidth } from '../src/ui/layout/layoutBudgets';
import { PanelResizer, useWorkbenchLayout } from '../src/components/development/WorkbenchLayout';

const layoutKey = 'dgos.development.layout.v3';
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
  const open = layout.drawer !== null;
  return <section ref={root} className="agent-workbench dev-workspace" data-compact={compact} data-drawer-width={layout.drawerWidth}>
    <output>{compact ? 'compact' : 'full'}</output>
    <div className="dev-workspace-body" data-drawer-open={open}>
      <nav className="dev-tool-rail">{(['projects', 'files', 'git', 'tasks'] as const).map(drawer => <button key={drawer} data-active={layout.drawer === drawer} onClick={() => setLayout(value => ({ ...value, drawer: value.drawer === drawer ? null : drawer }))}>{drawer}</button>)}</nav>
      <aside id="dev-navigation" className="dev-navigation" hidden={!open}>
        <header className="dev-drawer-header">{layout.drawer}</header>
        <div className="dev-drawer-content">Tools</div>
        <PanelResizer value={layout.drawerWidth} onChange={drawerWidth => setLayout(value => ({ ...value, drawerWidth }))} label="Drawer width" testId="drawer-resizer" controls="dev-navigation" />
      </aside>
      <main className="dev-workspace-main"><nav className="dev-session-tabs"><button aria-current="page">Codex</button></nav><div className="dev-conversation-pane"><input aria-label="draft" /></div></main>
    </div>
  </section>;
}
beforeEach(() => {
  width = 1400; frames = new Map(); sequence = 0; fonts = new EventTarget();
  document.documentElement.style.fontSize = '14px';
  document.documentElement.style.setProperty('--font-size-ui', '12px');
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => width);
  vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { resize = callback; } observe() {} disconnect() {} });
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++sequence, callback); return sequence; });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
  vi.stubGlobal('PointerEvent', class extends MouseEvent {
    pointerId: number;
    constructor(type: string, init: PointerEventInit = {}) { super(type, init); this.pointerId = init.pointerId ?? 0; }
  });
  Object.defineProperty(document, 'fonts', { configurable: true, value: fonts });
});
afterEach(() => { cleanup(); localStorage.clear(); document.documentElement.removeAttribute('style'); document.documentElement.removeAttribute('data-density'); Reflect.deleteProperty(document, 'fonts'); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('layout budgets', () => {
  it('includes the expanded shell rail independently of the workspace budget', () => {
    const workbench = workbenchMinimumWidth(document.body);
    expect(workbench).toBeGreaterThan(0);
    expect(shellMinimumWidth(document.body)).toBeGreaterThan(workbench);
  });
  it('increases geometry budgets with density without changing the UI font', () => {
    const budgets = [12.6, 14, 16.8].map(geometry => {
      document.documentElement.style.fontSize = `${geometry}px`;
      return { workbench: workbenchMinimumWidth(document.body), shell: shellMinimumWidth(document.body) };
    });
    expect(budgets[1].workbench).toBeGreaterThan(budgets[0].workbench);
    expect(budgets[2].workbench).toBeGreaterThan(budgets[1].workbench);
    expect(budgets[1].shell).toBeGreaterThan(budgets[0].shell);
    expect(budgets[2].shell).toBeGreaterThan(budgets[1].shell);
    expect(document.documentElement.style.getPropertyValue('--font-size-ui')).toBe('12px');
  });
  it('changes at the container boundary and ignores zero-sized hidden containers', () => {
    const threshold = workbenchMinimumWidth(document.body);
    width = threshold; render(<Probe />); flush(); expect(screen.getByText('full')).toBeTruthy();
    width = threshold - 1; resize(); flush(); expect(screen.getByText('compact')).toBeTruthy();
    width = 0; resize(); flush(); expect(screen.getByText('compact')).toBeTruthy();
    width = threshold; resize(); flush(); expect(screen.getByText('full')).toBeTruthy();
    resize(); resize(); expect(frames.size).toBe(1); flush();
  });
  it('remeasures independent font and density changes without a resize', async () => {
    width = workbenchMinimumWidth(document.body) + 1;
    render(<Probe />); flush(); expect(screen.getByText('full')).toBeTruthy();
    document.documentElement.style.setProperty('--font-size-ui', '28px'); await appearance();
    expect(screen.getByText('compact')).toBeTruthy();
    document.documentElement.style.setProperty('--font-size-ui', '12px'); await appearance();
    expect(screen.getByText('full')).toBeTruthy();
    document.documentElement.style.fontSize = '16.8px'; document.documentElement.dataset.density = '120'; await appearance();
    expect(screen.getByText('compact')).toBeTruthy();
    act(() => fonts.dispatchEvent(new Event('loadingdone'))); expect(frames.size).toBe(1); flush();
  });
});

it('keeps drawer preferences, the resident conversation and focus through compact transitions', () => {
  localStorage.setItem(layoutKey, JSON.stringify({ drawer: 'git', drawerWidth: 31.5, codeView: 'changes', runtimeView: 'runs' }));
  width = workbenchMinimumWidth(document.body) + 1;
  const view = render(<WorkbenchProbe />); flush();
  const draft = screen.getByRole('textbox', { name: 'draft' });
  const pane = draft.parentElement;
  fireEvent.change(draft, { target: { value: 'unsent work' } }); draft.focus();
  const saved = localStorage.getItem(layoutKey);
  const threshold = workbenchMinimumWidth(view.container.querySelector('section')!);
  width = threshold - 1; resize(); flush(); expect(screen.getByText('compact')).toBeTruthy();
  width = threshold + 1; resize(); flush(); expect(screen.getByText('full')).toBeTruthy();
  expect(localStorage.getItem(layoutKey)).toBe(saved);
  expect(screen.getByRole('textbox', { name: 'draft' })).toBe(draft);
  expect(draft.parentElement).toBe(pane);
  expect(pane?.closest('[hidden]')).toBeNull();
  expect(draft).toHaveProperty('value', 'unsent work'); expect(document.activeElement).toBe(draft);
  fireEvent.keyDown(screen.getByRole('separator'), { key: 'ArrowRight' });
  expect(screen.getByRole('separator').getAttribute('aria-valuenow')).toBe('32.5');
  expect(screen.getByRole('separator').getAttribute('aria-valuetext')).toBe('32.5rem');
  view.unmount(); render(<WorkbenchProbe />); flush();
  expect(screen.getByRole('separator').getAttribute('aria-valuenow')).toBe('32.5');
});

it('stores a closed drawer without hiding or replacing the conversation', () => {
  localStorage.setItem(layoutKey, JSON.stringify({ drawer: 'files', drawerWidth: 27, codeView: 'files', runtimeView: 'processes' }));
  const view = render(<WorkbenchProbe />); flush();
  const draft = screen.getByRole('textbox', { name: 'draft' });
  fireEvent.click(screen.getByRole('button', { name: 'files' }));
  expect(view.container.querySelector('#dev-navigation')).toHaveProperty('hidden', true);
  expect(view.container.querySelector('.dev-workspace-body')?.getAttribute('data-drawer-open')).toBe('false');
  expect(screen.getByRole('textbox', { name: 'draft' })).toBe(draft);
  expect(JSON.parse(localStorage.getItem(layoutKey) ?? '{}').drawer).toBeNull();
  view.unmount(); render(<WorkbenchProbe />); flush();
  expect(screen.queryByRole('separator')).toBeNull();
  expect(screen.getByRole('textbox', { name: 'draft' })).toBeTruthy();
});


it.each([[5, 18], [70, 42], [25.5, 25.5]])('restores drawer width %s rem within bounds as %s rem', (saved, restored) => {
  localStorage.setItem(layoutKey, JSON.stringify({ drawer: 'tasks', drawerWidth: saved, codeView: 'review', runtimeView: 'runs' }));
  render(<WorkbenchProbe />); flush();
  expect(screen.getByRole('separator').getAttribute('aria-valuenow')).toBe(String(restored));
});

it('resizes the drawer with horizontal keys and clamps Home, End and arrow movement', () => {
  localStorage.setItem(layoutKey, JSON.stringify({ drawer: 'projects', drawerWidth: 30 }));
  render(<WorkbenchProbe />); flush();
  const separator = screen.getByRole('separator', { name: 'Drawer width' });
  expect(separator.getAttribute('aria-controls')).toBe('dev-navigation');
  expect(separator.getAttribute('aria-orientation')).toBe('vertical');
  expect(separator.getAttribute('aria-valuemin')).toBe('18');
  expect(separator.getAttribute('aria-valuemax')).toBe('42');
  fireEvent.keyDown(separator, { key: 'ArrowLeft' }); expect(separator.getAttribute('aria-valuenow')).toBe('29');
  fireEvent.keyDown(separator, { key: 'ArrowDown' }); expect(separator.getAttribute('aria-valuenow')).toBe('29');
  fireEvent.keyDown(separator, { key: 'Home' }); expect(separator.getAttribute('aria-valuenow')).toBe('18');
  fireEvent.keyDown(separator, { key: 'ArrowLeft' }); expect(separator.getAttribute('aria-valuenow')).toBe('18');
  fireEvent.keyDown(separator, { key: 'End' }); expect(separator.getAttribute('aria-valuenow')).toBe('42');
  fireEvent.keyDown(separator, { key: 'ArrowRight' }); expect(separator.getAttribute('aria-valuenow')).toBe('42');
});

it.each([14, 20])('converts pointer movement using the %spx root font rather than container percentages', rootFont => {
  document.documentElement.style.fontSize = `${rootFont}px`;
  localStorage.setItem(layoutKey, JSON.stringify({ drawer: 'projects', drawerWidth: 24 }));
  render(<WorkbenchProbe />); flush();
  const separator = screen.getByRole('separator');
  separator.setPointerCapture = vi.fn();
  separator.hasPointerCapture = vi.fn(() => true);
  separator.releasePointerCapture = vi.fn();
  fireEvent.pointerDown(separator, { pointerId: 1, clientX: 100, button: 0 });
  expect(document.activeElement).toBe(separator);
  fireEvent.pointerMove(separator, { pointerId: 2, clientX: 500 });
  expect(separator.getAttribute('aria-valuenow')).toBe('24');
  fireEvent.pointerMove(separator, { pointerId: 1, clientX: 100 + rootFont * 3 });
  expect(separator.getAttribute('aria-valuenow')).toBe('27');
  fireEvent.pointerMove(separator, { pointerId: 1, clientX: 100 + rootFont * 100 });
  expect(separator.getAttribute('aria-valuenow')).toBe('42');
  fireEvent.pointerMove(separator, { pointerId: 1, clientX: 100 - rootFont * 100 });
  expect(separator.getAttribute('aria-valuenow')).toBe('18');
  fireEvent.pointerUp(separator, { pointerId: 1 });
  expect(separator.releasePointerCapture).toHaveBeenCalledWith(1);
  fireEvent.pointerMove(separator, { pointerId: 1, clientX: 100 });
  expect(separator.getAttribute('aria-valuenow')).toBe('18');
});
