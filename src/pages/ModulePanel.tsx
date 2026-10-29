import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { ModuleSlot, useModuleInstance } from '../modules/ModuleSlots';
import {
  listModuleCrashes,
  listModules,
  runOperations,
  type ModuleApiError,
  type ModuleCrashRecord,
  type ModuleSummary,
} from '../modules/moduleApi';
import { CrashLogList, ErrorNotice, toApiError } from './ModuleManagement';
import './ModuleManagement.css';

export default function ModulePanel() {
  const { moduleId = '' } = useParams();
  const navigate = useNavigate();
  const instance = useModuleInstance(moduleId);
  const crashed = instance?.crashed === true;
  const runtimeId = instance?.runtimeId ?? null;
  const [summary, setSummary] = useState<ModuleSummary | null>(null);
  const [error, setError] = useState<ModuleApiError | null>(null);
  const [crashes, setCrashes] = useState<ModuleCrashRecord[] | null>(null);
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // The crash counter and the enabled flag change when an instance crashes or the host disables it.
  const refresh = useCallback(async () => {
    try {
      const modules = await listModules();
      if (mounted.current) setSummary(modules.find((entry) => entry.module_id === moduleId) ?? null);
    } catch (failure) {
      if (mounted.current) setError(toApiError(failure));
    }
  }, [moduleId]);

  // The host records a crash and marks the runtime failed a moment after the page reports it, and
  // the instance can be released before that, so one refresh at the moment the instance goes away
  // can see the old state. A few short follow-up reads settle it.
  useEffect(() => {
    void refresh();
    if (runtimeId !== null) return undefined;
    const timers = [300, 900, 2000].map((delay) => setTimeout(() => void refresh(), delay));
    return () => timers.forEach(clearTimeout);
  }, [refresh, runtimeId, crashed]);

  async function act(task: () => Promise<void>) {
    setBusy(true);
    setError(null);
    setConfirmUninstall(false);
    try {
      await task();
    } catch (failure) {
      if (mounted.current) setError(toApiError(failure));
    }
    if (!mounted.current) return;
    await refresh();
    if (mounted.current) setBusy(false);
  }

  const reload = () =>
    act(async () => {
      await runOperations(['stop_module', 'start_module'], moduleId);
      setCrashes(null);
    });

  const uninstall = () =>
    act(async () => {
      await runOperations(['uninstall_module'], moduleId);
      navigate('/settings/modules');
    });

  const viewDetails = () =>
    act(async () => {
      setCrashes(await listModuleCrashes(moduleId));
    });

  const faulty = summary !== null && (summary.recent_crashes >= 3 || (!summary.enabled && summary.recent_crashes > 0));
  const canUninstall = summary !== null && !summary.official;
  // Once the host marks the runtime failed it releases the page instance, so the instance (and its
  // own `crashed` flag) is gone by the time the user looks. The host's state keeps the banner up.
  const showCrashed = crashed || (summary !== null && summary.runtime.state === 'failed' && summary.recent_crashes > 0);

  return (
    <div className="module-panel">
      <h1>{summary?.name ?? moduleId}</h1>
      <p>
        <Link to="/settings/modules">Back to module management</Link>
      </p>

      {showCrashed && (
        <div role="alert" className="module-banner">
          <strong>Module {moduleId} crashed</strong>
          <button type="button" disabled={busy} onClick={viewDetails}>View details</button>
          <button type="button" disabled={busy} onClick={reload}>Reload module</button>
          <button type="button" disabled={busy || !canUninstall} onClick={() => setConfirmUninstall(true)}>Uninstall</button>
        </div>
      )}
      {faulty && (
        <div role="alert" className="module-notice module-notice-warning">
          This module may be faulty; it was disabled. Consider uninstalling it.
        </div>
      )}
      {error !== null && <ErrorNotice error={error} />}
      {confirmUninstall && (
        <section role="dialog" aria-label={`Uninstall ${moduleId}`} className="module-dialog">
          <p>Uninstall {summary?.name ?? moduleId}? The module is stopped and removed.</p>
          <div className="module-actions">
            <button type="button" disabled={busy} onClick={uninstall}>Confirm uninstall</button>
            <button type="button" disabled={busy} onClick={() => setConfirmUninstall(false)}>Cancel</button>
          </div>
        </section>
      )}
      {crashes !== null && <CrashLogList crashes={crashes} />}

      {instance === null ? <p>This module is not running</p> : <ModuleSlot instance={instance} />}
    </div>
  );
}
