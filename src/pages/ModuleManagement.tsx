import { useCallback, useEffect, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import {
  ModuleApiError,
  grantModulePermission,
  listModuleCrashes,
  listModules,
  revokeModulePermission,
  runOperations,
  type ModuleCrashRecord,
  type ModuleSummary,
} from '../modules/moduleApi';
import './ModuleManagement.css';

// Anything that is not a ModuleApiError is a programming error: show a neutral message, never the raw error.
export function toApiError(error: unknown): ModuleApiError {
  return error instanceof ModuleApiError ? error : new ModuleApiError(0, 'Unexpected error while talking to the host');
}

export function ErrorNotice({ error }: { error: ModuleApiError }) {
  return (
    <div role="alert" className={error.incompatible ? 'module-notice module-notice-warning' : 'module-notice module-notice-error'}>
      {error.incompatible && <strong>This module needs a newer or older host version. </strong>}
      <span>{error.message}</span>
      {error.code !== 0 && <span className="module-notice-code"> (code {error.code})</span>}
    </div>
  );
}

export function CrashLogList({ crashes }: { crashes: ModuleCrashRecord[] }) {
  if (crashes.length === 0) return <p>No crashes recorded.</p>;
  return (
    <ul className="module-crash-list">
      {crashes.map((crash) => {
        const at = new Date(crash.created_at * 1000);
        return (
          <li key={crash.crash_id}>
            <div>
              <time dateTime={at.toISOString()}>{at.toLocaleString()}</time> · {crash.source} · <strong>{crash.error_name}</strong>{' '}
              (version {crash.module_version})
            </div>
            <pre>{crash.stack}</pre>
          </li>
        );
      })}
    </ul>
  );
}

function EnableConfirm({ module, busy, onConfirm, onCancel }: {
  module: ModuleSummary;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const [confirmed, setConfirmed] = useState(false);
  return (
    <section role="dialog" aria-label={`Enable ${module.name}`} className="module-dialog">
      {module.permissions.length === 0 ? (
        <p>{module.name} declares no permissions.</p>
      ) : (
        <>
          <p>{module.name} requests these permissions:</p>
          <ul>
            {module.permissions.map((entry) => (
              <li key={entry.permission}>
                <code>{entry.permission}</code>
                {entry.granted ? ' (already granted)' : ''}
              </li>
            ))}
          </ul>
        </>
      )}
      <label>
        <input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.currentTarget.checked)} />
        I have reviewed these permissions and want to enable {module.name}
      </label>
      <div className="module-actions">
        <button type="button" disabled={!confirmed || busy} onClick={onConfirm}>Confirm enable</button>
        <button type="button" disabled={busy} onClick={onCancel}>Cancel</button>
      </div>
    </section>
  );
}

interface Notice {
  error?: ModuleApiError;
  info?: string;
}

type Pending = { kind: 'enable' | 'uninstall'; moduleId: string } | null;

export default function ModuleManagement() {
  const [modules, setModules] = useState<ModuleSummary[] | null>(null);
  const [loadError, setLoadError] = useState<ModuleApiError | null>(null);
  const [busy, setBusy] = useState(false);
  const [notices, setNotices] = useState<Record<string, Notice>>({});
  const [pending, setPending] = useState<Pending>(null);
  const [crashLog, setCrashLog] = useState<{ moduleId: string; crashes: ModuleCrashRecord[] } | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      const next = await listModules();
      if (!mounted.current) return;
      setModules(next);
      setLoadError(null);
    } catch (error) {
      if (mounted.current) setLoadError(toApiError(error));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  function setNotice(moduleId: string, notice: Notice) {
    setNotices((current) => ({ ...current, [moduleId]: notice }));
  }

  // Runs one user action: the task resolves to a success message or throws. The list is refreshed afterwards either way.
  async function act(moduleId: string, task: () => Promise<string>) {
    setBusy(true);
    setPending(null);
    setNotice(moduleId, {});
    let notice: Notice;
    try {
      notice = { info: await task() };
    } catch (error) {
      notice = { error: toApiError(error) };
    }
    if (mounted.current) setNotice(moduleId, notice);
    await refresh();
    if (mounted.current) setBusy(false);
  }

  // A disabled module cannot be granted anything (the host refuses it), so the order is: enable,
  // grant what the user just confirmed, then start. Nothing runs before the grants are in place.
  const enable = (module: ModuleSummary) =>
    act(module.module_id, async () => {
      await runOperations(['enable_module'], module.module_id);
      for (const entry of module.permissions) {
        if (!entry.granted) await grantModulePermission(module.module_id, entry.permission);
      }
      await runOperations(['start_module'], module.module_id);
      return 'Enabled and started.';
    });

  const update = (module: ModuleSummary) =>
    act(module.module_id, async () => {
      const view = await runOperations(['update_module'], module.module_id);
      if (view.outcome === 'unchanged') return 'Already up to date.';
      if (view.outcome === 'updated_pending_release') return 'Updated. The new version takes over once the old one is released.';
      return 'Updated.';
    });

  const runSteps = (module: ModuleSummary, steps: Parameters<typeof runOperations>[0], done: string) =>
    act(module.module_id, async () => {
      await runOperations(steps, module.module_id);
      return done;
    });

  async function toggleCrashLog(moduleId: string) {
    if (crashLog?.moduleId === moduleId) {
      setCrashLog(null);
      return;
    }
    try {
      const crashes = await listModuleCrashes(moduleId);
      if (mounted.current) setCrashLog({ moduleId, crashes });
    } catch (error) {
      if (mounted.current) setNotice(moduleId, { error: toApiError(error) });
    }
  }

  return (
    <div className="module-management">
      <header className="module-header">
        <h1>Module management</h1>
        <p>Enable, update and remove modules, and decide which permissions they get.</p>
        <Link to="/settings">Back to Settings</Link>
      </header>

      {loadError !== null && <ErrorNotice error={loadError} />}
      {modules === null && loadError === null && <p>Loading modules…</p>}
      {modules?.length === 0 && <p>No modules are installed.</p>}

      {modules?.map((module) => {
        const notice = notices[module.module_id] ?? {};
        const confirming = pending?.moduleId === module.module_id ? pending.kind : null;
        return (
          <article key={module.module_id} className="module-card" aria-label={module.name} data-testid={`module-${module.module_id}`}>
            <div className="module-card-head">
              <h2>{module.name}</h2>
              <span className="module-badge">Version {module.version}</span>
              {module.official && <span className="module-badge module-badge-official">Official</span>}
              <span className="module-badge">{module.enabled ? 'Enabled' : 'Disabled'}</span>
              <span className="module-badge">Runtime: {module.runtime.state}</span>
            </div>
            <p className="module-meta">
              {module.module_id} · Recent crashes: {module.recent_crashes}
            </p>

            {module.permissions.length === 0 ? (
              <p>No permissions declared.</p>
            ) : (
              <ul className="module-permissions">
                {module.permissions.map((entry) => (
                  <li key={entry.permission}>
                    <code>{entry.permission}</code> <span>{entry.granted ? 'Granted' : 'Not granted'}</span>{' '}
                    {entry.granted ? (
                      <button
                        type="button"
                        aria-label={`Revoke ${entry.permission} for ${module.name}`}
                        disabled={busy}
                        onClick={() =>
                          act(module.module_id, async () => {
                            await revokeModulePermission(module.module_id, entry.permission);
                            return `Revoked ${entry.permission}.`;
                          })
                        }
                      >
                        Revoke
                      </button>
                    ) : (
                      <button
                        type="button"
                        aria-label={`Grant ${entry.permission} for ${module.name}`}
                        disabled={busy}
                        onClick={() =>
                          act(module.module_id, async () => {
                            await grantModulePermission(module.module_id, entry.permission);
                            return `Granted ${entry.permission}.`;
                          })
                        }
                      >
                        Grant
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            )}

            {notice.error !== undefined && <ErrorNotice error={notice.error} />}
            {notice.info !== undefined && <p role="status" className="module-notice module-notice-info">{notice.info}</p>}

            <div className="module-actions">
              {module.enabled ? (
                <>
                  <button type="button" disabled={busy} onClick={() => runSteps(module, ['disable_module'], 'Disabled.')}>Disable</button>
                  <button type="button" disabled={busy} onClick={() => runSteps(module, ['stop_module', 'start_module'], 'Reloaded.')}>Reload</button>
                </>
              ) : (
                <button type="button" disabled={busy} onClick={() => setPending({ kind: 'enable', moduleId: module.module_id })}>Enable</button>
              )}
              <button type="button" disabled={busy} onClick={() => update(module)}>Update</button>
              {!module.official && (
                <button type="button" disabled={busy} onClick={() => setPending({ kind: 'uninstall', moduleId: module.module_id })}>Uninstall</button>
              )}
              <button type="button" onClick={() => toggleCrashLog(module.module_id)}>Crash log</button>
            </div>

            {confirming === 'enable' && (
              <EnableConfirm module={module} busy={busy} onConfirm={() => enable(module)} onCancel={() => setPending(null)} />
            )}
            {confirming === 'uninstall' && (
              <section role="dialog" aria-label={`Uninstall ${module.name}`} className="module-dialog">
                <p>Uninstall {module.name}? The module is stopped and removed.</p>
                <div className="module-actions">
                  <button type="button" disabled={busy} onClick={() => runSteps(module, ['uninstall_module'], 'Uninstalled.')}>Confirm uninstall</button>
                  <button type="button" disabled={busy} onClick={() => setPending(null)}>Cancel</button>
                </div>
              </section>
            )}

            {crashLog?.moduleId === module.module_id && <CrashLogList crashes={crashLog.crashes} />}
          </article>
        );
      })}
    </div>
  );
}
