import { useCallback, useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { processActive, resizeDevelopmentTerminal, writeDevelopmentTerminal, useRuntimeProcessView, type DevelopmentProcess } from '../../stores/developmentRuntime';
import { useSurfaceActive, useSurfaceVisible } from '../../workspace/SurfaceContext';

type TerminalFeedback = { owner: string; resize: string | null; resizing: boolean };

function terminalAppearance(element: HTMLElement) {
  const styles = getComputedStyle(element);
  const foreground = styles.getPropertyValue('--text-code').trim();
  return {
    fontFamily: styles.getPropertyValue('--font-family-code').trim(),
    fontSize: Number.parseFloat(styles.getPropertyValue('--font-size-code')) || 12,
    theme: { background: styles.getPropertyValue('--surface-code').trim(), foreground, cursor: foreground },
  };
}

export function DevelopmentTerminal({ process, panelVisible = true }: { process: DevelopmentProcess; panelVisible?: boolean }) {
  const { t } = useLocale();
  const surfaceActive = useSurfaceActive();
  const surfaceVisible = useSurfaceVisible();
  const active = surfaceActive && panelVisible;
  const visible = surfaceVisible && panelVisible;
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const { view, setView } = useRuntimeProcessView(process);
  const processView = useRef(view);
  processView.current = view;
  const hostTerminal = process.kind === 'terminal' && process.source === 'host_terminal';
  const owner = JSON.stringify([epoch, generation, process.project_id, process.task_id, process.id]);
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const [feedback, setFeedback] = useState<TerminalFeedback | null>(null);
  const viewFeedback = feedback?.owner === owner ? feedback : null;
  const updateFeedback = useCallback((change: Partial<Omit<TerminalFeedback, 'owner'>>) => {
    setFeedback(state => ({ ...(state?.owner === owner ? state : { owner, resize: null, resizing: false }), ...change }));
  }, [owner]);
  const retryResize = useRef<(() => void) | null>(null);
  const interactive = useRef(active);
  interactive.current = visible && active && !pending && hostTerminal;
  const fitter = useRef<FitAddon | null>(null);
  const sized = useRef<{ rows: number; cols: number } | null>(null);
  const resizeFlight = useRef<Promise<void> | null>(null);
  const container = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal | null>(null);
  const output = useRef('');
  const clearOffset = useRef(view.clearOffset ?? 0);
  const restoreOutput = useRef<((value: string) => void) | null>(null);
  const current = useRef(process);
  current.current = process;
  useEffect(() => {
    if (!container.current) return;
    setFeedback(null);
    let disposed = false;
    const valid = () => !disposed && currentOwner.current === owner && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
    let writes = Promise.resolve();
    let appearance = terminalAppearance(container.current);
    const restored = processView.current;
    let restoring = 0;
    const instance = new Terminal({ convertEol: false, cursorBlink: true, scrollback: 2000, screenReaderMode: true, ...(restored.rows && restored.cols ? { rows: restored.rows, cols: restored.cols } : {}), ...appearance });
    const fit = new FitAddon();
    instance.loadAddon(fit); instance.open(container.current);
    terminal.current = instance; fitter.current = fit; sized.current = restored.rows && restored.cols ? { rows: restored.rows, cols: restored.cols } : null; resizeFlight.current = null;
    clearOffset.current = restored.clearOffset ?? 0;
    output.current = current.current.output.slice(clearOffset.current);
    // Restoring output may contain terminal queries. Their generated replies are
    // not new user input and must never be sent back to the existing PTY.
    const replay = (value: string) => { ++restoring; instance.write(value, () => { --restoring; }); };
    restoreOutput.current = replay;
    replay(output.current);
    instance.attachCustomKeyEventHandler(() => interactive.current && !document.hidden);
    const input = instance.onData(data => {
      if (restoring || !valid() || document.hidden || !interactive.current || !processActive(current.current)) return;
      writes = writes.then(async () => {
        if (!valid() || document.hidden || !interactive.current || !processActive(current.current)) return;
        const result = await writeDevelopmentTerminal(process.id, data);
        // An unacknowledged write may already have reached the PTY. Never replay it.
        if (valid() && !result.ok) setView({ inputError: result.code });
      });
    });
    const updateAppearance = () => {
      if (!valid() || !container.current) return;
      const next = terminalAppearance(container.current);
      const metricsChanged = next.fontFamily !== appearance.fontFamily || next.fontSize !== appearance.fontSize;
      const colorsChanged = next.theme.background !== appearance.theme.background || next.theme.foreground !== appearance.theme.foreground;
      if (next.fontFamily !== appearance.fontFamily) instance.options.fontFamily = next.fontFamily;
      if (next.fontSize !== appearance.fontSize) instance.options.fontSize = next.fontSize;
      if (colorsChanged) instance.options.theme = next.theme;
      appearance = next;
      if (metricsChanged || colorsChanged) retryResize.current?.();
    };
    const fontsLoaded = () => {
      updateAppearance();
      // A loaded face changes cell metrics even when its CSS family name is unchanged.
      if (valid()) retryResize.current?.();
    };
    const themeObserver = new MutationObserver(updateAppearance);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'data-theme', 'data-preset', 'data-density'] });
    document.fonts?.addEventListener('loadingdone', fontsLoaded);
    document.documentElement.addEventListener('appearance-font-ready', fontsLoaded);
    return () => { disposed = true; themeObserver.disconnect(); document.fonts?.removeEventListener('loadingdone', fontsLoaded); document.documentElement.removeEventListener('appearance-font-ready', fontsLoaded); input.dispose(); instance.dispose(); terminal.current = null; fitter.current = null; };
  }, [owner, epoch, generation, process.id, setView, updateFeedback]);
  const running = hostTerminal && processActive(process);
  useEffect(() => {
    if (!visible || !active || pending) return;
    let frame = 0;
    let revealed = false;
    const reveal = () => {
      if (revealed || frame || document.hidden) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const element = container.current;
        if (document.hidden || !element || !element.clientWidth || !element.clientHeight) return;
        // Selecting/showing the terminal reveals it beneath expanded runtime configuration.
        // Scroll only once; later app activation must respect the user's scroll position.
        element.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'instant' });
        revealed = true;
      });
    };
    document.addEventListener('visibilitychange', reveal);
    window.addEventListener('focus', reveal);
    reveal();
    return () => { cancelAnimationFrame(frame); document.removeEventListener('visibilitychange', reveal); window.removeEventListener('focus', reveal); };
  }, [owner, visible, active, pending]);
  useEffect(() => {
    const instance = terminal.current;
    const fit = fitter.current;
    const element = container.current;
    if (!visible || pending || !instance || !fit || !element) return;
    let disposed = false;
    let frame = 0;
    let retry = 0;
    let recovery = 0;
    let recoveryAttempts = 0;
    let waiting = false;
    const valid = () => !disposed && currentOwner.current === owner && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
    const schedule = () => {
      if (!valid() || frame) return;
      frame = requestAnimationFrame(() => { frame = 0; resize(); });
    };
    const recover = () => {
      window.clearTimeout(recovery); recovery = 0; recoveryAttempts = 0;
      schedule();
    };
    const retryMetrics = () => {
      // A hidden WKWebView can open with zero character metrics. Retry only this
      // activation's layout, never poll an idle terminal or replay its contents.
      if (recovery || recoveryAttempts >= 3) return;
      recoveryAttempts++;
      recovery = window.setTimeout(() => { recovery = 0; schedule(); }, 100);
    };
    retryResize.current = recover;
    const resize = () => {
      if (!valid() || document.hidden || element.clientWidth <= 0 || element.clientHeight <= 0) return;
      // The same-size public resize remeasures invalid character metrics (#785).
      // Fit cannot bootstrap them: proposeDimensions returns undefined at zero.
      instance.resize(instance.cols, instance.rows);
      const dimensions = fit.proposeDimensions();
      if (!dimensions) { retryMetrics(); return; }
      if (!Number.isInteger(dimensions.rows) || !Number.isInteger(dimensions.cols) || dimensions.rows < 1 || dimensions.cols < 1 || dimensions.rows > 1000 || dimensions.cols > 1000) return;
      window.clearTimeout(recovery); recovery = 0;
      // Fit's public proposal lets us reject invalid layouts before changing xterm or the PTY.
      if (instance.rows !== dimensions.rows || instance.cols !== dimensions.cols) instance.resize(dimensions.cols, dimensions.rows);
      // Same-size activation has no resize event. Refresh both renderer and its
      // accessibility consumer from the existing buffer; never reset/write/focus.
      instance.refresh(0, instance.rows - 1);
      if (!running || !processActive(current.current)) return;
      if (resizeFlight.current) {
        if (!waiting) {
          waiting = true;
          void resizeFlight.current.then(() => { waiting = false; schedule(); });
        }
        return;
      }
      if (sized.current?.rows === dimensions.rows && sized.current.cols === dimensions.cols) return;
      window.clearTimeout(retry); retry = 0;
      updateFeedback({ resizing: true });
      const flight = resizeDevelopmentTerminal(process.id, dimensions.rows, dimensions.cols).then(result => {
        // Acknowledged size belongs to the host process, not this mount/session.
        // Keep it even after disposal so a remount does not repeat a same-size IPC.
        if (epoch === useDevelopment.getState().epoch && result.ok) setView({ rows: dimensions.rows, cols: dimensions.cols });
        const sameTerminal = terminal.current === instance && currentOwner.current === owner && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
        if (!sameTerminal) return;
        if (result.ok) sized.current = dimensions;
        else sized.current = null;
        updateFeedback({ resizing: false, resize: result.ok ? null : result.code });
        if (!valid() || result.ok) return;
        if (result.code === 'terminal_starting') {
          // Retry only an acknowledged startup race; success, hiding, exit or unmount ends it.
          retry = window.setTimeout(() => { retry = 0; schedule(); }, 1500);
        }
      }).finally(() => { if (resizeFlight.current === flight) resizeFlight.current = null; });
      resizeFlight.current = flight;
    };
    const observer = new ResizeObserver(recover);
    observer.observe(element);
    const visibility = () => {
      instance.options.disableStdin = document.hidden || !interactive.current;
      if (document.hidden) {
        cancelAnimationFrame(frame); frame = 0;
        window.clearTimeout(recovery); recovery = 0;
        window.clearTimeout(retry); retry = 0;
      } else recover();
    };
    document.addEventListener('visibilitychange', visibility);
    window.addEventListener('focus', recover);
    window.addEventListener('pageshow', recover);
    recover();
    // Font metrics may become available after the initial layout without a container resize.
    void document.fonts?.ready.then(() => { if (valid()) recover(); });
    return () => {
      disposed = true; observer.disconnect(); cancelAnimationFrame(frame); window.clearTimeout(retry); window.clearTimeout(recovery);
      document.removeEventListener('visibilitychange', visibility);
      window.removeEventListener('focus', recover);
      window.removeEventListener('pageshow', recover);
      if (retryResize.current === recover) retryResize.current = null;
    };
  }, [owner, epoch, generation, process.id, visible, pending, running, updateFeedback]);
  useEffect(() => {
    if (terminal.current) terminal.current.options.disableStdin = document.hidden || !visible || !active || pending;
  }, [visible, active, pending, owner]);
  useEffect(() => {
    const instance = terminal.current;
    if (!instance) return;
    const boundary = view.clearOffset ?? 0;
    const displayed = process.output.slice(boundary);
    if (clearOffset.current !== boundary) {
      // Ordered local erase: no PTY input, stop, host-history deletion or replay.
      instance.write('\x1b[2J\x1b[3J\x1b[H');
      if (displayed) instance.write(displayed);
      clearOffset.current = boundary;
    } else if (output.current === displayed) return;
    else if (displayed.startsWith(output.current)) instance.write(displayed.slice(output.current.length));
    else { instance.reset(); restoreOutput.current?.(displayed); }
    output.current = displayed;
  }, [process.output, view.clearOffset, owner]);
  return <div className="dev-review-stack">
    <p className="m-0 text-xs text-ink-secondary">{t('direct.terminalHint')}</p>
    <div className="dev-review-toolbar"><Button size="xs" variant="secondary" data-testid="dev-terminal-clear" disabled={!visible || !active || pending} onClick={() => setView({ clearOffset: process.output.length })}>{t('workbenchRuntime.clearDisplay')}</Button><span className="text-xs text-ink-secondary">{t('workbenchRuntime.clearHint')}</span></div>
    {viewFeedback?.resize ? <div className="dev-review-stack" data-testid="dev-terminal-size-feedback" aria-busy={viewFeedback.resizing}>
      <p role={viewFeedback.resize === 'terminal_starting' ? 'status' : 'alert'} className="m-0 text-xs dev-evidence-warning">{t(viewFeedback.resize === 'terminal_starting' ? 'direct.terminalViewStarting' : 'direct.terminalResizeFailed')}</p>
      <Button size="xs" variant="secondary" disabled={!visible || !active || pending || !running || viewFeedback.resizing} onClick={() => retryResize.current?.()}>{t('direct.terminalRetrySize')}</Button>
    </div> : null}
    {view.inputError ? <div className="dev-review-stack" data-testid="dev-terminal-input-feedback">
      <p role="alert" className="m-0 text-xs dev-evidence-warning">{t(view.inputError === 'terminal_starting' ? 'direct.terminalInputStarting' : 'direct.terminalInputUnconfirmed')}</p>
      <Button size="xs" variant="secondary" disabled={!visible || !active || pending} onClick={() => setView({ inputError: null })}>{t('direct.terminalDismissInputError')}</Button>
    </div> : null}
    <div ref={container} className="dev-terminal" role="region" aria-label={t('direct.terminal')} data-testid="dev-terminal" />
  </div>;
}
