import { useEffect, useRef, useState } from 'react';
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
  const [saved, setSaved] = useState<Snapshot | null>(null);
  const [paths, setPaths] = useState<Paths>({ codex: null, claude: null, gemini: null, node: null });
  const [probes, setProbes] = useState<Partial<Record<Tool, Probe>>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const alive = useRef(true);
  const ticket = useRef(0);
  const dirty = saved !== null && JSON.stringify(paths) !== JSON.stringify(saved.paths);
  useSurfaceCloseGuard(async () => !busy && (!dirty || await confirmDiscard()));
  // Each async action takes a ticket. Only the newest live ticket may write state or clear busy.
  const begin = () => { ticket.current += 1; setBusy(true); setError(false); return ticket.current; };
  const current = (id: number) => alive.current && id === ticket.current;
  const finish = (id: number) => { if (current(id)) setBusy(false); };
  const load = async () => {
    const id = begin();
    try { const value = await call<Snapshot>('get_cli_preferences'); if (current(id)) { setSaved(value); setPaths(value.paths); setProbes({}); } }
    catch { if (current(id)) setError(true); }
    finally { finish(id); }
  };
  useEffect(() => { alive.current = true; void load(); return () => { alive.current = false; ticket.current += 1; }; }, []);
  const probe = async (tool: Tool) => {
    const requested = paths[tool] ?? null;
    const id = begin();
    try {
      const result = await call<Omit<Probe, 'requested'>>('probe_cli', { tool, path: requested });
      if (current(id)) setProbes(previous => ({ ...previous, [tool]: { ...result, requested } }));
    } catch { if (current(id)) setError(true); }
    finally { finish(id); }
  };
  const save = async () => {
    if (!saved) return;
    const id = begin();
    try {
      const result = await call<{ saved: boolean; snapshot: Snapshot }>('set_cli_preferences', { paths, expectedRevision: saved.revision });
      if (!current(id)) return;
      if (!result.saved) { setError(true); return; }
      const confirmed = await call<Snapshot>('get_cli_preferences');
      if (current(id)) { setSaved(confirmed); setPaths(confirmed.paths); }
    } catch { if (current(id)) setError(true); }
    finally { finish(id); }
  };
  return <div className="settings-content settings-sections"><h1 className="m-0 text-lg font-semibold">{t('cli.title')}</h1><p className="text-sm text-muted-foreground">{t('cli.help')}</p>
    {tools.map(tool => <section key={tool} className="cli-tool-row"><Input label={tool === 'node' ? 'Node.js' : tool[0].toUpperCase() + tool.slice(1)} disabled={!saved || busy} placeholder={t('cli.path')} value={paths[tool] ?? ''} onChange={event => { setPaths(current => ({ ...current, [tool]: event.target.value || null })); setProbes(current => ({ ...current, [tool]: undefined })); }} /><Button variant="secondary" disabled={!saved || busy} onClick={() => void probe(tool)}>{t('cli.detect')}</Button>{probes[tool] && probes[tool]?.requested === (paths[tool] ?? null) && <div role="status" className="min-w-0 break-all text-sm"><p>{t(`cli.${probes[tool]?.status}`)}</p><p>{probes[tool]?.path}</p><p>{probes[tool]?.version}</p></div>}</section>)}
    {error && <p role="alert" className="text-sm text-error-text">{t('cli.error')}</p>}
    <div className="settings-actions"><Button disabled={!dirty || busy || error} loading={busy} onClick={() => void save()}>{t('common.save')}</Button><Button variant="secondary" disabled={busy} onClick={() => void load()}>{t('cli.reload')}</Button></div>
  </div>;
}
