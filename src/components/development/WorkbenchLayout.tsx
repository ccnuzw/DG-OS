import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { useLayoutBudget } from '../../ui/layout/useLayoutBudget';
import { workbenchMinimumWidth } from '../../ui/layout/layoutBudgets';

export type CodeView = 'files' | 'changes' | 'capabilities' | 'delivery' | 'review';
interface Layout {
  drawer: 'projects' | 'files' | 'tasks' | 'plugins' | null;
  drawerWidth: number;
  codeView: CodeView;
  runtimeView: 'processes' | 'runs';
  documentWidth: number;
}
const KEY = 'dgos.development.layout.v3';
const MIN_DRAWER_WIDTH = 18;
const MAX_DRAWER_WIDTH = 42;
const defaults: Layout = { drawer: 'projects', drawerWidth: 24, documentWidth: 36, codeView: 'files', runtimeView: 'processes' };
const clamp = (value: number) => Math.max(MIN_DRAWER_WIDTH, Math.min(MAX_DRAWER_WIDTH, Math.round(value * 2) / 2));
function restore(): Layout {
  try {
    const value = JSON.parse(localStorage.getItem(KEY) ?? 'null');
    if (!value || typeof value !== 'object') return defaults;
    return {
      drawer: value.drawer === 'git' ? 'files' : value.drawer === null || ['projects', 'files', 'tasks', 'plugins'].includes(value.drawer) ? value.drawer : defaults.drawer,
      drawerWidth: Number.isFinite(value.drawerWidth) ? clamp(value.drawerWidth) : defaults.drawerWidth,
      documentWidth: Number.isFinite(value.documentWidth) ? clamp(value.documentWidth) : defaults.documentWidth,
      codeView: ['files', 'changes', 'capabilities', 'delivery', 'review'].includes(value.codeView) ? value.codeView : defaults.codeView,
      runtimeView: value.runtimeView === 'runs' ? 'runs' : 'processes',
    };
  } catch { return defaults; }
}
// Presentation preferences only. No object IDs, inputs, permissions or execution state enter storage.
export function useWorkbenchLayout() {
  const [layout, setLayout] = useState(restore);
  const { root, narrow: compact } = useLayoutBudget<HTMLElement>(workbenchMinimumWidth);
  useEffect(() => {
    try { localStorage.setItem(KEY, JSON.stringify(layout)); } catch { /* In-memory layout remains usable. */ }
  }, [layout]);
  return { root, compact, layout, setLayout };
}

export function PanelResizer({ value, onChange, label, testId, controls, reverse = false }: { value: number; onChange(value: number): void; label: string; testId: string; controls: string; reverse?: boolean }) {
  const drag = useRef<{ pointer: number; start: number; value: number; rem: number } | null>(null);
  function move(event: PointerEvent<HTMLDivElement>) {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    onChange(clamp(current.value + (reverse ? -1 : 1) * (event.clientX - current.start) / current.rem));
  }
  return <div className="dev-panel-resizer" role="separator" tabIndex={0} aria-label={label} aria-controls={controls} aria-orientation="vertical" aria-valuemin={MIN_DRAWER_WIDTH} aria-valuemax={MAX_DRAWER_WIDTH} aria-valuenow={value} aria-valuetext={`${value}rem`} data-testid={testId}
    onPointerDown={event => {
      if (event.button !== 0) return;
      const rem = Number.parseFloat(getComputedStyle(event.currentTarget.ownerDocument.documentElement).fontSize) || 16;
      event.preventDefault(); event.currentTarget.focus(); event.currentTarget.setPointerCapture(event.pointerId);
      drag.current = { pointer: event.pointerId, start: event.clientX, value, rem };
    }} onPointerMove={move} onPointerUp={event => {
      if (drag.current?.pointer !== event.pointerId) return;
      drag.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    }} onPointerCancel={() => { drag.current = null; }} onLostPointerCapture={() => { drag.current = null; }}
    onKeyDown={event => {
      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      onChange(event.key === 'Home' ? MIN_DRAWER_WIDTH : event.key === 'End' ? MAX_DRAWER_WIDTH : clamp(value + (event.key === 'ArrowRight' ? 1 : -1) * (reverse ? -1 : 1)));
    }} />;
}
