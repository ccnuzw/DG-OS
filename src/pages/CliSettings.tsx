import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as openDirectoryDialog } from '@tauri-apps/plugin-dialog';
import { Button, Input, Modal, type ModalHandle } from '@dgos/ui';
import { ArrowUpCircle, FolderOpen, RefreshCw, Terminal } from 'lucide-react';
import { useLocale } from '../i18n';
import { useSurfaceCloseGuard } from '../workspace/SurfaceContext';
import { confirmDiscard } from '../workspace/closeCoordinator';
const tools = ['codex', 'claude', 'gemini', 'node'] as const;
type Tool = typeof tools[number];
type Paths = Record<Tool, string | null>;
interface Probe { status: string; path: string | null; version: string | null }
interface UpdateCheck {
  status: 'available' | 'up_to_date' | 'unsupported' | 'check_failed' | 'not_installed';
  current_version: string | null;
  latest_version: string | null;
  method: string | null;
  ticket?: string | null;
}
interface Detection extends Probe { update: UpdateCheck | null }
interface Snapshot { revision: number; paths: Paths; detections: Partial<Record<Tool, Detection>>; codex_protocol_reference: { version: string } }
type UpdateStatus = 'updated' | 'failed' | 'unknown' | 'stale' | 'busy';
type Action = 'load' | 'save' | 'folder' | 'update';
type Phase = 'probe' | 'check';
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const response = await invoke<{ success: boolean; data: T }>(command, args);
  if (!response.success) throw new Error('CLI command failed');
  return response.data;
}
function hasUpdate(offer: UpdateCheck | null | undefined) {
  return offer?.status === 'available' && !!offer.current_version && !!offer.latest_version && !!offer.method;
}
export default function CliSettings() {
  const { t } = useLocale();
  const id = useId();
  const [saved, setSaved] = useState<Snapshot | null>(null);
  const savedRef = useRef<Snapshot | null>(null);
  const [probes, setProbes] = useState<Partial<Record<Tool, Detection>>>({});
  const [nodeDraft, setNodeDraft] = useState<Probe | null>(null);
  const [action, setAction] = useState<Action | null>(null);
  const actionRef = useRef<Action | null>(null);
  const [pending, setPending] = useState<Partial<Record<Tool, Phase>>>({});
  const [detecting, setDetecting] = useState(false);
  const detectingRef = useRef(false);
  const [feedback, setFeedback] = useState<'load-error' | 'save-error' | null>(null);
  const [probeErrors, setProbeErrors] = useState<Partial<Record<Tool, boolean>>>({});
  const [checkErrors, setCheckErrors] = useState<Partial<Record<Tool, boolean>>>({});
  const [confirmedSave, setConfirmedSave] = useState(false);
  const [saveBlocked, setSaveBlocked] = useState(false);
  const [updateResults, setUpdateResults] = useState<Partial<Record<Tool, UpdateStatus>>>({});
  const [nodeError, setNodeError] = useState(false);
  const confirmation = useRef<ModalHandle | null>(null);
  const alive = useRef(true);
  const generation = useRef(0);
  const dirty = nodeDraft !== null && nodeDraft.path !== saved?.paths.node;
  // Read-only detection never holds navigation hostage; only mutations and a Node draft need a guard.
  useSurfaceCloseGuard(async () => actionRef.current !== 'save' && actionRef.current !== 'update' && (!dirty || await confirmDiscard()));
  const current = (value: number) => alive.current && value === generation.current;
  const begin = (value: Action) => { actionRef.current = value; setAction(value); };
  const finish = (value: number) => { if (current(value)) { actionRef.current = null; setAction(null); } };
  const applySnapshot = (snapshot: Snapshot) => {
    if (savedRef.current && snapshot.revision < savedRef.current.revision) return;
    savedRef.current = snapshot;
    setSaved(snapshot);
    setProbes(snapshot.detections ?? {});
  };
  const refresh = async (value: number) => {
    const snapshot = await call<Snapshot>('get_cli_preferences');
    if (current(value)) applySnapshot(snapshot);
  };
  const load = async (initial = false) => {
    if (!initial && actionRef.current) return;
    if (!initial && dirty && !(await confirmDiscard())) return;
    if (!alive.current || (!initial && actionRef.current)) return;
    const value = ++generation.current;
    detectingRef.current = false; setDetecting(false); setPending({});
    begin('load');
    try {
      // Hydration is storage-only. PATH, processes and registries are touched only by explicit actions.
      await refresh(value);
      if (current(value)) { setNodeDraft(null); setProbeErrors({}); setCheckErrors({}); setUpdateResults({}); setNodeError(false); setFeedback(null); setSaveBlocked(false); setConfirmedSave(false); }
    } catch { if (current(value)) { setFeedback('load-error'); setSaveBlocked(true); } }
    finally { finish(value); }
  };
  useEffect(() => {
    alive.current = true; void load(true);
    return () => { alive.current = false; generation.current += 1; confirmation.current?.close(); confirmation.current = null; };
  }, []);
  const probeAll = async () => {
    if (actionRef.current || detectingRef.current || !savedRef.current) return;
    const value = generation.current;
    const nodePath = savedRef.current.paths.node;
    detectingRef.current = true; setDetecting(true);
    setPending({ codex: 'probe', claude: 'probe', gemini: 'probe', node: 'probe' });
    setProbeErrors({}); setCheckErrors({}); setUpdateResults({}); setNodeError(false); setConfirmedSave(false);
    await Promise.all(tools.map(async tool => {
      try {
        // CLI paths come from discovery, not editable configuration. Keep an explicitly chosen Node location.
        const result = await call<Probe>('probe_cli', { tool, path: tool === 'node' ? nodePath : null });
        if (!current(value)) return;
        setProbes(previous => ({ ...previous, [tool]: { ...result, update: null } }));
        if (tool !== 'node' && result.status === 'available' && result.path && result.version) {
          setPending(previous => ({ ...previous, [tool]: 'check' }));
          let update: UpdateCheck;
          try { update = await call<UpdateCheck>('check_cli_update', { tool, path: result.path }); }
          catch { update = { status: 'check_failed', current_version: result.version, latest_version: null, method: null }; if (current(value)) setCheckErrors(previous => ({ ...previous, [tool]: true })); }
          if (!current(value)) return;
          setProbes(previous => ({ ...previous, [tool]: { ...result, update } }));
        }
      } catch { if (current(value)) setProbeErrors(previous => ({ ...previous, [tool]: true })); }
      finally {
        if (current(value)) {
          try { await refresh(value); }
          catch { if (current(value)) { setFeedback('load-error'); setSaveBlocked(true); } }
          if (current(value)) setPending(previous => ({ ...previous, [tool]: undefined }));
        }
      }
    }));
    if (current(value)) { detectingRef.current = false; setDetecting(false); }
  };
  const chooseNode = async () => {
    if (actionRef.current || !savedRef.current || pending.node) return;
    const value = generation.current;
    begin('folder'); setNodeError(false);
    try {
      const directory = await openDirectoryDialog({ directory: true, multiple: false, title: t('cli.node-folder') });
      if (!current(value) || typeof directory !== 'string') return;
      const result = await call<Probe>('probe_node_directory', { directory });
      if (!current(value)) return;
      if (result.status !== 'available' || !result.path || !result.version) { setNodeError(true); return; }
      setConfirmedSave(false); setNodeDraft(result); setProbeErrors(previous => ({ ...previous, node: false }));
    } catch { if (current(value)) setNodeError(true); }
    finally { finish(value); }
  };
  const requestUpdate = async (tool: Tool) => {
    if (actionRef.current || !savedRef.current || tool === 'node' || pending[tool] || !hasUpdate(probes[tool]?.update)) return;
    const value = generation.current;
    begin('update');
    let offer: UpdateCheck;
    try {
      // The persisted hint is not an installation capability. Revalidate on this explicit click for a fresh ticket.
      offer = await call<UpdateCheck>('check_cli_update', { tool, path: probes[tool]?.path ?? savedRef.current.paths[tool] });
      if (!current(value)) return;
      setCheckErrors(previous => ({ ...previous, [tool]: false }));
      setProbes(previous => { const probe = previous[tool]; return probe ? { ...previous, [tool]: { ...probe, update: offer } } : previous; });
      if (!hasUpdate(offer) || !offer.ticket) { finish(value); return; }
    } catch {
      if (current(value)) { setCheckErrors(previous => ({ ...previous, [tool]: true })); setProbes(previous => { const probe = previous[tool]; return probe ? { ...previous, [tool]: { ...probe, update: null } } : previous; }); }
      finish(value); return;
    }
    let consumed = false;
    confirmation.current = Modal.confirm({
      title: t('cli.update-title', { tool: t(`cli.tool.${tool}`) }),
      content: t('cli.update-confirm', { tool: t(`cli.tool.${tool}`), current: offer.current_version!, latest: offer.latest_version!, method: offer.method! }),
      confirmText: t('cli.update'), cancelText: t('common.cancel'),
      onCancel: () => { consumed = true; if (current(value)) confirmation.current = null; finish(value); },
      onConfirm: async () => {
        if (!current(value) || consumed) return;
        consumed = true;
        setProbes(previous => { const probe = previous[tool]; return probe ? { ...previous, [tool]: { ...probe, update: null } } : previous; });
        try {
          const result = await call<{ status: UpdateStatus; probe: Probe | null }>('update_cli', { ticket: offer.ticket });
          if (!current(value)) return;
          if (result.status === 'updated' && result.probe?.status === 'available' && result.probe.path && result.probe.version) {
            const updated = result.probe;
            setProbes(previous => ({ ...previous, [tool]: { ...updated, update: null } }));
            setProbeErrors(previous => ({ ...previous, [tool]: false }));
            setUpdateResults(previous => ({ ...previous, [tool]: 'updated' }));
            await refresh(value);
          } else setUpdateResults(previous => ({ ...previous, [tool]: result.status === 'updated' ? 'unknown' : result.status }));
        } catch { if (current(value)) setUpdateResults(previous => ({ ...previous, [tool]: 'unknown' })); }
        finally { if (current(value)) confirmation.current = null; finish(value); }
      },
    });
  };
  const save = async () => {
    if (!savedRef.current || !nodeDraft || !dirty || saveBlocked || actionRef.current || pending.node) return;
    const value = generation.current;
    begin('save'); setConfirmedSave(false);
    try {
      const result = await call<{ saved: boolean; snapshot: Snapshot }>('set_cli_preferences', { paths: { ...savedRef.current.paths, node: nodeDraft.path }, expectedRevision: savedRef.current.revision });
      if (!current(value)) return;
      if (!result.saved) { setFeedback('save-error'); setSaveBlocked(true); return; }
      await refresh(value);
      if (current(value)) { setNodeDraft(null); setFeedback(null); setSaveBlocked(false); setConfirmedSave(true); }
    } catch { if (current(value)) { setFeedback('save-error'); setSaveBlocked(true); } }
    finally { finish(value); }
  };
  return <div className="cli-settings">
    <div className="cli-settings-body">
      <div className="cli-settings-content">
        <section className="cli-settings-card" aria-labelledby={`${id}-title`}>
          <header className="cli-settings-header">
            <span className="cli-settings-mark" aria-hidden="true"><Terminal size={22} /></span>
            <div className="cli-settings-intro"><h1 id={`${id}-title`}>{t('cli.title')}</h1><p>{t('cli.help')}</p></div>
            <Button variant="secondary" disabled={!saved || detecting || action !== null} onClick={() => void probeAll()}><RefreshCw size={16} aria-hidden="true" />{t(detecting ? 'cli.busy-probe' : 'cli.detect-all')}</Button>
          </header>
          <div className="cli-tool-list">
            {tools.map(tool => {
              const probe = tool === 'node' && nodeDraft ? { ...nodeDraft, update: null } : probes[tool];
              const phase = pending[tool];
              const canUpdate = tool !== 'node' && !checkErrors[tool] && probe?.status === 'available' && hasUpdate(probe.update);
              const update = probe?.update;
              const message = updateResults[tool] ? t(`cli.update-${updateResults[tool]}`) : checkErrors[tool] ? t('cli.check-check_failed') : update?.status === 'check_failed' || update?.status === 'unsupported' ? t(`cli.check-${update.status}`) : null;
              return <section key={tool} className={`cli-tool-row cli-tool-${tool}`} aria-labelledby={`${id}-${tool}`} aria-busy={!!phase}>
                <div className="cli-tool-heading">
                  <span className={`cli-tool-status${probe?.status === 'available' ? ' is-available' : ''}`} aria-hidden="true" />
                  <h2 id={`${id}-${tool}`}>{t(`cli.tool.${tool}`)}{probe?.status === 'available' && probe.version ? <span className="cli-tool-version">{probe.version}</span> : null}</h2>
                  <p>{t(`cli.description.${tool}`)}</p>
                  {phase || probe?.status === 'available' ? <span className="cli-tool-source" aria-live="polite">{t(phase === 'probe' ? 'cli.probing-path' : phase === 'check' ? 'cli.checking-update' : 'cli.detected')}</span> : null}
                </div>
                <div className={`cli-tool-control${tool === 'node' || canUpdate ? ' has-action' : ''}`}>
                  <span className="cli-path-icon" aria-hidden="true"><Terminal size={16} /></span>
                  <Input aria-label={t(`cli.tool.${tool}`)} error={tool === 'node' && nodeError ? t('cli.node-folder-error') : probeErrors[tool] ? t('cli.probe-error') : probe && probe.status !== 'available' ? t(`cli.${probe.status}`) : undefined} disabled={!saved} readOnly placeholder={t('cli.path')} className="type-code" value={probe?.path ?? saved?.paths[tool] ?? ''} />
                  {tool === 'node' ? <Button variant="secondary" iconOnly title={t('cli.node-folder')} aria-label={t('cli.node-folder')} disabled={!saved || action !== null || !!phase} onClick={() => void chooseNode()}><FolderOpen size={16} aria-hidden="true" /></Button> : canUpdate ? <Button variant="secondary" iconOnly title={t('cli.update-tool', { tool: t(`cli.tool.${tool}`) })} aria-label={t('cli.update-tool', { tool: t(`cli.tool.${tool}`) })} disabled={!saved || action !== null || !!phase} onClick={() => void requestUpdate(tool)}><ArrowUpCircle size={16} aria-hidden="true" /></Button> : null}
                </div>
                {tool === 'codex' && saved?.codex_protocol_reference ? <p className="cli-tool-feedback">{t('cli.protocol-reference', { version: saved.codex_protocol_reference.version })}</p> : null}
                {message ? <p className="cli-tool-feedback" aria-live="polite">{message}</p> : null}
              </section>;
            })}
          </div>
          <p className="cli-settings-note">{t('cli.path-note')}</p>
        </section>
      </div>
    </div>
    <footer className="cli-settings-footer">
      {feedback && <p role="alert" className="cli-feedback-error">{t(feedback === 'load-error' ? 'cli.load-error' : 'cli.save-error')}</p>}
      <div className="settings-actions cli-settings-actions">
        <Button disabled={!dirty || action !== null || !!pending.node || saveBlocked} onClick={() => void save()}>{t('common.save')}</Button>
        <Button variant="secondary" disabled={action !== null} onClick={() => void load()}>{t('cli.reload')}</Button>
        <span role="status" className="cli-draft-state">{t(action ? `cli.busy-${action}` : detecting ? 'cli.busy-probe' : dirty ? 'cli.unsaved' : feedback ? 'cli.action-error' : confirmedSave ? 'cli.save-confirmed' : 'cli.saved-state')}</span>
      </div>
    </footer>
  </div>;
}
