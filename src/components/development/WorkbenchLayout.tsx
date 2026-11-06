import { useEffect, useRef, useState, type PointerEvent } from 'react';

export type CodeView = 'files' | 'changes' | 'capabilities' | 'delivery' | 'review';
interface Layout { navigation: boolean; agent: boolean; runtime: boolean; agentWidth: number; runtimeHeight: number; codeView: CodeView; compactView: 'code' | 'agent'; runtimeView: 'processes' | 'runs' }
const KEY = 'dgos.development.layout.v2';
const defaults: Layout = { navigation: true, agent: true, runtime: false, agentWidth: 38, runtimeHeight: 40, codeView: 'files', compactView: 'code', runtimeView: 'processes' };
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
function restore(): Layout {
  try {
    const value = JSON.parse(localStorage.getItem(KEY) ?? 'null');
    if (!value || typeof value !== 'object') return defaults;
    return {
      navigation: typeof value.navigation === 'boolean' ? value.navigation : defaults.navigation,
      agent: typeof value.agent === 'boolean' ? value.agent : defaults.agent,
      runtime: typeof value.runtime === 'boolean' ? value.runtime : defaults.runtime,
      agentWidth: Number.isFinite(value.agentWidth) ? clamp(Math.round(value.agentWidth), 26, 55) : defaults.agentWidth,
      runtimeHeight: Number.isFinite(value.runtimeHeight) ? clamp(Math.round(value.runtimeHeight), 22, 65) : defaults.runtimeHeight,
      codeView: ['files', 'changes', 'capabilities', 'delivery', 'review'].includes(value.codeView) ? value.codeView : defaults.codeView,
      compactView: value.compactView === 'agent' ? 'agent' : 'code',
      runtimeView: value.runtimeView === 'runs' ? 'runs' : 'processes',
    };
  } catch { return defaults; }
}
// Presentation preferences only. No object IDs, inputs, permissions or execution state enter storage.
export function useWorkbenchLayout() {
  const [layout, setLayout] = useState(restore);
  const root = useRef<HTMLElement>(null);
  const [compact, setCompact] = useState(false);
  useEffect(() => {
    try { localStorage.setItem(KEY, JSON.stringify(layout)); } catch { /* In-memory layout remains usable. */ }
  }, [layout]);
  useEffect(() => {
    const element = root.current;
    if (!element) return;
    const measure = () => { if (element.clientWidth > 0) setCompact(element.clientWidth < 1000); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return { root, layout, setLayout, compact };
}

export function PanelResizer({ axis, value, onChange, label, testId, controls }: { axis: 'agent' | 'runtime'; value: number; onChange(value: number): void; label: string; testId: string; controls: string }) {
  const min = axis === 'agent' ? 26 : 22, max = axis === 'agent' ? 55 : 65;
  const drag = useRef<{ pointer: number; start: number; value: number; extent: number } | null>(null);
  function move(event: PointerEvent<HTMLDivElement>) {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    const position = axis === 'agent' ? event.clientX : event.clientY;
    onChange(clamp(Math.round(current.value - (position - current.start) / current.extent * 100), min, max));
  }
  return <div className={`dev-panel-resizer dev-panel-resizer-${axis}`} role="separator" tabIndex={0} aria-label={label} aria-controls={controls} aria-orientation={axis === 'agent' ? 'vertical' : 'horizontal'} aria-valuemin={min} aria-valuemax={max} aria-valuenow={value} aria-valuetext={`${value}%`} data-testid={testId}
    onPointerDown={event => {
      if (event.button !== 0) return;
      const container = event.currentTarget.parentElement;
      if (!container) return;
      event.preventDefault(); event.currentTarget.focus(); event.currentTarget.setPointerCapture(event.pointerId);
      drag.current = { pointer: event.pointerId, start: axis === 'agent' ? event.clientX : event.clientY, value, extent: Math.max(1, axis === 'agent' ? container.clientWidth : container.clientHeight) };
    }} onPointerMove={move} onPointerUp={event => { if (drag.current?.pointer === event.pointerId) { drag.current = null; event.currentTarget.releasePointerCapture(event.pointerId); } }} onPointerCancel={() => { drag.current = null; }} onLostPointerCapture={() => { drag.current = null; }}
    onKeyDown={event => {
      const decrease = axis === 'agent' ? 'ArrowRight' : 'ArrowDown', increase = axis === 'agent' ? 'ArrowLeft' : 'ArrowUp';
      if (![decrease, increase, 'Home', 'End'].includes(event.key)) return;
      event.preventDefault(); onChange(event.key === 'Home' ? min : event.key === 'End' ? max : clamp(value + (event.key === increase ? 2 : -2), min, max));
    }} />;
}
