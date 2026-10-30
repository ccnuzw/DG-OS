import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { Button, Stack } from '@dgos/ui';
import { ModuleSlot, useModuleInstance } from '../modules/ModuleSlots';
import {
  listModuleCrashes,
  listModules,
  runOperations,
  type ModuleApiError,
  type ModuleCrashRecord,
  type ModuleSummary,
} from '../modules/moduleApi';
import { usePageError } from '../pageError';
import { CrashLogList, ErrorNotice, MODULE_LINK_CLASS, NoticeBox, toApiError } from './ModuleManagement';

export default function ModulePanel() {
  const { moduleId = '' } = useParams();
  const navigate = useNavigate();
  const reportError = usePageError('ModulePanel');
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
      reportError(failure);
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
    <div className="mx-auto min-w-0 max-w-240 p-8 text-foreground">
      <h1 className="m-0 mb-2 text-2xl font-semibold text-foreground wrap-anywhere">{summary?.name ?? moduleId}</h1>
      <p className="m-0 mb-4 text-sm">
        <Link to="/settings/modules" className={MODULE_LINK_CLASS}>Back to module management</Link>
      </p>

      {showCrashed && (
        <NoticeBox tone="error" role="alert" className="my-4 items-center">
          <Stack direction="horizontal" gap={3} className="flex-wrap items-center">
            <strong>Module {moduleId} crashed</strong>
            <Button variant="secondary" size="sm" disabled={busy} onClick={viewDetails}>View details</Button>
            <Button variant="secondary" size="sm" disabled={busy} onClick={reload}>Reload module</Button>
            <Button variant="danger" size="sm" disabled={busy || !canUninstall} onClick={() => setConfirmUninstall(true)}>Uninstall</Button>
          </Stack>
        </NoticeBox>
      )}
      {faulty && (
        <NoticeBox tone="warning" role="alert">
          This module may be faulty; it was disabled. Consider uninstalling it.
        </NoticeBox>
      )}
      {error !== null && <ErrorNotice error={error} />}
      {confirmUninstall && (
        <section role="dialog" aria-label={`Uninstall ${moduleId}`} className="my-3 rounded-lg border border-border bg-muted p-4 text-sm text-foreground">
          <Stack gap={3}>
            <p className="m-0">Uninstall {summary?.name ?? moduleId}? The module is stopped and removed.</p>
            <Stack direction="horizontal" gap={2} className="flex-wrap">
              <Button variant="danger" disabled={busy} onClick={uninstall}>Confirm uninstall</Button>
              <Button variant="secondary" disabled={busy} onClick={() => setConfirmUninstall(false)}>Cancel</Button>
            </Stack>
          </Stack>
        </section>
      )}
      {crashes !== null && <CrashLogList crashes={crashes} />}

      {instance === null ? <p className="text-sm text-muted-foreground">This module is not running</p> : <ModuleSlot instance={instance} />}
    </div>
  );
}
