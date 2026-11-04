import { useEffect, useRef } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { processActive, resizeDevelopmentTerminal, writeDevelopmentTerminal, type DevelopmentProcess } from '../../stores/developmentRuntime';

export function DevelopmentTerminal({ process, onError }: { process: DevelopmentProcess; onError(code: string): void }) {
  const { t } = useLocale();
  const container = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal | null>(null);
  const output = useRef('');
  const current = useRef(process);
  const error = useRef(onError);
  current.current = process; error.current = onError;
  useEffect(() => {
    if (!container.current) return;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    let disposed = false;
    const valid = () => !disposed && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
    let writes = Promise.resolve();
    const styles = getComputedStyle(container.current);
    const instance = new Terminal({ convertEol: false, cursorBlink: true, scrollback: 2000, fontSize: 12, screenReaderMode: true, theme: { background: styles.getPropertyValue('--surface-code').trim(), foreground: styles.getPropertyValue('--text-primary').trim(), cursor: styles.getPropertyValue('--text-primary').trim() } });
    const fit = new FitAddon();
    instance.loadAddon(fit); instance.open(container.current);
    terminal.current = instance; output.current = '';
    instance.write(current.current.output); output.current = current.current.output;
    const input = instance.onData(data => {
      if (!valid() || !processActive(current.current)) return;
      writes = writes.then(async () => {
        if (!valid()) return;
        const result = await writeDevelopmentTerminal(process.id, data);
        if (valid() && !result.ok) error.current(result.code);
      });
    });
    const resize = () => {
      if (!valid() || !container.current?.clientWidth) return;
      fit.fit();
      if (processActive(current.current)) void resizeDevelopmentTerminal(process.id, instance.rows, instance.cols).then(result => { if (valid() && !result.ok && result.code !== 'terminal_starting') error.current(result.code); });
    };
    const observer = new ResizeObserver(resize); observer.observe(container.current); resize();
    const retrySize = window.setInterval(resize, 1500);
    return () => { disposed = true; window.clearInterval(retrySize); observer.disconnect(); input.dispose(); instance.dispose(); terminal.current = null; };
  }, [process.id]);
  useEffect(() => {
    const instance = terminal.current;
    if (!instance || output.current === process.output) return;
    if (process.output.startsWith(output.current)) instance.write(process.output.slice(output.current.length));
    else { instance.reset(); instance.write(process.output); }
    output.current = process.output;
  }, [process.output]);
  return <div className="dev-review-stack"><p className="m-0 text-xs text-muted-foreground">{t('direct.terminalHint')}</p><div ref={container} className="dev-terminal" role="region" aria-label={t('direct.terminal')} data-testid="dev-terminal" /></div>;
}
