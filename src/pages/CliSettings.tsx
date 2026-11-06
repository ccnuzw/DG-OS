import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../i18n';
import { useSurfaceCloseGuard } from '../workspace/SurfaceContext';
import { confirmDiscard } from '../workspace/closeCoordinator';
const tools = ['codex', 'claude', 'gemini', 'node'] as const;
type Tool = typeof tools[number];
type Paths = Record<Tool, string | null>;
interface Snapshot { revision: number; paths: Paths }
interface Probe { status: string; path: string | null; version: string | null; requested: string | null }
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const response = await invoke<{ success: boolean; data: T }>(command, args);
  if (!response.success) throw new Error('CLI command failed');
  return response.data;
}
export default function CliSettings() {
  const { t } = useLocale();
  const id = useId();
  const [saved, setSaved] = useState<Snapshot | null>(null);
  const [paths, setPaths] = useState<Paths>({ codex: null, claude: null, gemini: null, node: null });
  const [probes, setProbes] = useState<Partial<Record<Tool, Probe>>>({});
  const [busy, setBusy] = useState(false);
  const [busyAction, setBusyAction] = useState<'load' | 'save' | 'probe' | null>(null);
  const [feedback, setFeedback] = useState<'load-error' | 'save-error' | null>(null);
  const [probeErrors, setProbeErrors] = useState<Partial<Record<Tool, boolean>>>({});
  const [confirmedSave, setConfirmedSave] = useState(false);
  const [saveBlocked, setSaveBlocked] = useState(false);
  const alive = useRef(true);
  const ticket = useRef(0);
  const dirty = saved !== null && JSON.stringify(paths) !== JSON.stringify(saved.paths);
  useSurfaceCloseGuard(async () => !busy && (!dirty || await confirmDiscard()));
  // Each async action takes a ticket. Only the newest live ticket may write state or clear busy.
  const begin = (action: 'load' | 'save' | 'probe') => { ticket.current += 1; setBusy(true); setBusyAction(action); setConfirmedSave(false); return ticket.current; };
  const current = (id: number) => alive.current && id === ticket.current;
  const finish = (id: number) => { if (current(id)) { setBusy(false); setBusyAction(null); } };
  const load = async (discardDraft = false) => {
    if (dirty && !discardDraft && !(await confirmDiscard())) return;
    const id = begin('load');
    try {
      const value = await call<Snapshot>('get_cli_preferences');
      if (current(id)) { setSaved(value); setPaths(value.paths); setProbes({}); setProbeErrors({}); setFeedback(null); setSaveBlocked(false); }
    } catch { if (current(id)) { setFeedback('load-error'); setSaveBlocked(true); } }
    finally { finish(id); }
  };
  useEffect(() => { alive.current = true; void load(true); return () => { alive.current = false; ticket.current += 1; }; }, []);
  const probe = async (tool: Tool) => {
    const requested = paths[tool] ?? null;
    const id = begin('probe');
    setProbes(previous => ({ ...previous, [tool]: undefined }));
    setProbeErrors(previous => ({ ...previous, [tool]: false }));
    try {
      const result = await call<Omit<Probe, 'requested'>>('probe_cli', { tool, path: requested });
      if (current(id)) { setProbes(previous => ({ ...previous, [tool]: { ...result, requested } })); setProbeErrors(previous => ({ ...previous, [tool]: false })); }
    } catch { if (current(id)) setProbeErrors(previous => ({ ...previous, [tool]: true })); }
    finally { finish(id); }
  };
  const save = async () => {
    if (!saved || saveBlocked) return;
    const id = begin('save');
    try {
      const result = await call<{ saved: boolean; snapshot: Snapshot }>('set_cli_preferences', { paths, expectedRevision: saved.revision });
      if (!current(id)) return;
      if (!result.saved) { setFeedback('save-error'); setSaveBlocked(true); return; }
      const confirmed = await call<Snapshot>('get_cli_preferences');
      if (current(id)) { setSaved(confirmed); setPaths(confirmed.paths); setFeedback(null); setSaveBlocked(false); setConfirmedSave(true); }
    } catch { if (current(id)) { setFeedback('save-error'); setSaveBlocked(true); } }
    finally { finish(id); }
  };
  return <div className="cli-settings">
    <div className="cli-settings-body">
      <div className="settings-sections cli-settings-content">
        <h1 className="m-0 text-lg font-semibold">{t('cli.title')}</h1>
        <p className="m-0 text-sm text-ink-secondary">{t('cli.help')}</p>
        {tools.map(tool => {
          const probeResult = probes[tool]?.requested === (paths[tool] ?? null) ? probes[tool] : undefined;
          return <section key={tool} className="cli-tool-row" aria-labelledby={`${id}-${tool}`}>
            <div className="cli-tool-heading"><h2 id={`${id}-${tool}`}>{t(`cli.tool.${tool}`)}</h2><p>{t(tool === 'codex' || tool === 'claude' ? 'cli.can-run' : 'cli.detect-only')}</p></div>
            <div className="cli-tool-control">
              <Input label={<><span className="sr-only">{t(`cli.tool.${tool}`)} </span>{t('cli.path-label')}</>} error={probeErrors[tool] ? t('cli.probe-error') : probeResult && probeResult.status !== 'available' ? t(`cli.${probeResult.status}`) : undefined} disabled={!saved || busy} placeholder={t('cli.path')} className="type-code" value={paths[tool] ?? ''} onChange={event => { setPaths(current => ({ ...current, [tool]: event.target.value || null })); setProbes(current => ({ ...current, [tool]: undefined })); setProbeErrors(current => ({ ...current, [tool]: false })); setConfirmedSave(false); }} />
              <Button variant="secondary" disabled={!saved || busy} onClick={() => void probe(tool)}>{t('cli.detect')}</Button>
            </div>
            <p className="cli-saved-path"><span>{t('cli.saved-path')}</span> <code>{saved?.paths[tool] ?? t('cli.automatic')}</code></p>
            {probeResult && <div role="status" className="cli-probe-result">
              {probeResult.status === 'available' && <p>{t('cli.available')}</p>}
              {(probeResult.path || probeResult.version) && <details><summary>{t('cli.technical-details')}</summary><dl>
                {probeResult.path && <><dt>{t('cli.result-path')}</dt><dd><code>{probeResult.path}</code></dd></>}
                {probeResult.version && <><dt>{t('cli.version')}</dt><dd><code>{probeResult.version}</code></dd></>}
              </dl></details>}
            </div>}
          </section>;
        })}
      </div>
    </div>
    <footer className="cli-settings-footer">
      {feedback && <p role="alert" className="cli-feedback-error">{t(feedback === 'load-error' ? 'cli.load-error' : 'cli.save-error')}</p>}
      <div className="settings-actions cli-settings-actions">
        <Button disabled={!dirty || busy || saveBlocked} onClick={() => void save()}>{t('common.save')}</Button>
        <Button variant="secondary" disabled={busy} onClick={() => void load()}>{t('cli.reload')}</Button>
        <span role="status" className="cli-draft-state">{t(busyAction ? `cli.busy-${busyAction}` : dirty ? 'cli.unsaved' : feedback ? 'cli.action-error' : confirmedSave ? 'cli.save-confirmed' : 'cli.saved-state')}</span>
      </div>
    </footer>
  </div>;
}
