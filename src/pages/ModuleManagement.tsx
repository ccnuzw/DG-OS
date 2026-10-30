import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { Button, Card, Checkbox, Loading, Stack, StatusIcon, cn } from '@dgos/ui';
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
import { usePageError } from '../pageError';

// Failures are shown twice on purpose: inline (`ErrorNotice`, with the host's own message and code, which stays next
// to the module it belongs to) and through the unified Toast. Module errors are 1004xxx codes with no entry in
// ERROR_MESSAGES yet (FR-005 §13.2), so the Toast carries the generic text plus the code. The raw error is reported,
// not the one `toApiError` builds, so a non-host failure shows the generic text without a made-up code.

// Anything that is not a ModuleApiError is a programming error: show a neutral message, never the raw error.
export function toApiError(error: unknown): ModuleApiError {
  return error instanceof ModuleApiError ? error : new ModuleApiError(0, 'Unexpected error while talking to the host');
}

/** Text link used by both module pages. */
export const MODULE_LINK_CLASS =
  'text-primary underline underline-offset-4 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus-ring';

type NoticeTone = 'error' | 'warning' | 'success';

const NOTICE_TONE: Record<NoticeTone, { box: string; icon: 'error' | 'warning' | 'success' }> = {
  error: { box: 'border-error bg-error-bg text-error-text', icon: 'error' },
  warning: { box: 'border-warning bg-warning-bg text-warning-text', icon: 'warning' },
  success: { box: 'border-success bg-success-bg text-success-text', icon: 'success' },
};

/** Tinted message box: a status icon plus content that wraps instead of overflowing a narrow window. */
export function NoticeBox({
  tone,
  role,
  className,
  children,
}: {
  tone: NoticeTone;
  role: 'alert' | 'status';
  className?: string;
  children: ReactNode;
}) {
  const { box, icon } = NOTICE_TONE[tone];
  return (
    <div role={role} className={cn('my-2 flex items-start gap-2 rounded-md border p-3 text-sm', box, className)}>
      <StatusIcon name={icon} className="mt-0.5" />
      <div className="min-w-0 wrap-anywhere">{children}</div>
    </div>
  );
}

export function ErrorNotice({ error }: { error: ModuleApiError }) {
  return (
    <NoticeBox tone={error.incompatible ? 'warning' : 'error'} role="alert">
      {error.incompatible && <strong>This module needs a newer or older host version. </strong>}
      <span>{error.message}</span>
      {error.code !== 0 && <span> (code {error.code})</span>}
    </NoticeBox>
  );
}

export function CrashLogList({ crashes }: { crashes: ModuleCrashRecord[] }) {
  if (crashes.length === 0) return <p className="my-2 text-sm text-muted-foreground">No crashes recorded.</p>;
  return (
    <ul className="m-0 list-none p-0">
      {crashes.map((crash) => {
        const at = new Date(crash.created_at * 1000);
        return (
          <li key={crash.crash_id} className="border-t border-border py-2 text-sm first:border-t-0">
            <div className="wrap-anywhere">
              <time dateTime={at.toISOString()}>{at.toLocaleString()}</time> · {crash.source} · <strong>{crash.error_name}</strong>{' '}
              (version {crash.module_version})
            </div>
            <pre className="m-0 mt-2 overflow-x-auto rounded-md bg-secondary p-2 font-mono text-xs text-secondary-foreground">
              {crash.stack}
            </pre>
          </li>
        );
      })}
    </ul>
  );
}

function ConfirmPanel({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section role="dialog" aria-label={label} className="my-3 rounded-lg border border-border bg-muted p-4 text-sm text-foreground">
      <Stack gap={3}>{children}</Stack>
    </section>
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
    <ConfirmPanel label={`Enable ${module.name}`}>
      {module.permissions.length === 0 ? (
        <p className="m-0">{module.name} declares no permissions.</p>
      ) : (
        <>
          <p className="m-0">{module.name} requests these permissions:</p>
          <ul className="m-0 list-disc pl-6">
            {module.permissions.map((entry) => (
              <li key={entry.permission}>
                <code className="font-mono wrap-anywhere">{entry.permission}</code>
                {entry.granted ? ' (already granted)' : ''}
              </li>
            ))}
          </ul>
        </>
      )}
      <Checkbox
        checked={confirmed}
        onChange={setConfirmed}
        label={`I have reviewed these permissions and want to enable ${module.name}`}
      />
      <Stack direction="horizontal" gap={2} className="flex-wrap">
        <Button variant="primary" disabled={!confirmed || busy} onClick={onConfirm}>Confirm enable</Button>
        <Button variant="secondary" disabled={busy} onClick={onCancel}>Cancel</Button>
      </Stack>
    </ConfirmPanel>
  );
}

interface ActionNotice {
  error?: ModuleApiError;
  info?: string;
}

type Pending = { kind: 'enable' | 'uninstall'; moduleId: string } | null;

const BADGE_CLASS = 'rounded-full bg-secondary px-2 py-0.5 text-xs text-secondary-foreground';

export default function ModuleManagement() {
  const [modules, setModules] = useState<ModuleSummary[] | null>(null);
  const [loadError, setLoadError] = useState<ModuleApiError | null>(null);
  const [busy, setBusy] = useState(false);
  const [notices, setNotices] = useState<Record<string, ActionNotice>>({});
  const [pending, setPending] = useState<Pending>(null);
  const [crashLog, setCrashLog] = useState<{ moduleId: string; crashes: ModuleCrashRecord[] } | null>(null);
  const mounted = useRef(true);
  const reportError = usePageError('ModuleManagement');
  // `refresh` runs on mount and after every action; keeping the reporter in a ref keeps it out of its dependencies.
  const report = useRef(reportError);
  report.current = reportError;

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
      if (mounted.current) {
        setLoadError(toApiError(error));
        report.current(error);
      }
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  function setNotice(moduleId: string, notice: ActionNotice) {
    setNotices((current) => ({ ...current, [moduleId]: notice }));
  }

  // Runs one user action: the task resolves to a success message or throws. The list is refreshed afterwards either way.
  async function act(moduleId: string, task: () => Promise<string>) {
    setBusy(true);
    setPending(null);
    setNotice(moduleId, {});
    let notice: ActionNotice;
    try {
      notice = { info: await task() };
    } catch (error) {
      notice = { error: toApiError(error) };
      reportError(error);
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
      reportError(error);
    }
  }

  return (
    <div className="mx-auto min-w-0 max-w-240 p-8 text-foreground">
      <header className="mb-4">
        <h1 className="m-0 mb-2 text-2xl font-semibold text-foreground">Module management</h1>
        <p className="m-0 mb-2 text-sm text-muted-foreground">
          Enable, update and remove modules, and decide which permissions they get.
        </p>
        <Link to="/settings" className={cn('text-sm', MODULE_LINK_CLASS)}>Back to Settings</Link>
      </header>

      {loadError !== null && <ErrorNotice error={loadError} />}
      {modules === null && loadError === null && <Loading label="Loading modules…" />}
      {modules?.length === 0 && <p className="text-sm text-muted-foreground">No modules are installed.</p>}

      <Stack gap={4}>
        {modules?.map((module) => {
          const notice = notices[module.module_id] ?? {};
          const confirming = pending?.moduleId === module.module_id ? pending.kind : null;
          return (
            <article key={module.module_id} aria-label={module.name} data-testid={`module-${module.module_id}`}>
              <Card>
                <Card.Header>
                  <Stack direction="horizontal" gap={2} className="flex-wrap items-center">
                    <h2 className="m-0 mr-2 min-w-0 text-xl font-semibold wrap-anywhere">{module.name}</h2>
                    <span className={BADGE_CLASS}>Version {module.version}</span>
                    {module.official && <span className="rounded-full bg-info-bg px-2 py-0.5 text-xs text-info-text">Official</span>}
                    <span className={BADGE_CLASS}>{module.enabled ? 'Enabled' : 'Disabled'}</span>
                    <span className={BADGE_CLASS}>Runtime: {module.runtime.state}</span>
                  </Stack>
                </Card.Header>
                <Card.Body>
                  <p className="m-0 mb-2 text-muted-foreground wrap-anywhere">
                    {module.module_id} · Recent crashes: {module.recent_crashes}
                  </p>

                  {module.permissions.length === 0 ? (
                    <p className="m-0 my-2">No permissions declared.</p>
                  ) : (
                    <ul className="m-0 my-2 list-none p-0">
                      {module.permissions.map((entry) => (
                        <li key={entry.permission} className="flex flex-wrap items-center gap-2 py-1">
                          <code className="min-w-0 font-mono wrap-anywhere">{entry.permission}</code>{' '}
                          <span>{entry.granted ? 'Granted' : 'Not granted'}</span>{' '}
                          {entry.granted ? (
                            <Button
                              variant="secondary"
                              size="sm"
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
                            </Button>
                          ) : (
                            <Button
                              variant="secondary"
                              size="sm"
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
                            </Button>
                          )}
                        </li>
                      ))}
                    </ul>
                  )}

                  {notice.error !== undefined && <ErrorNotice error={notice.error} />}
                  {notice.info !== undefined && (
                    <NoticeBox tone="success" role="status">{notice.info}</NoticeBox>
                  )}

                  <Stack direction="horizontal" gap={2} className="my-3 flex-wrap">
                    {module.enabled ? (
                      <>
                        <Button variant="secondary" disabled={busy} onClick={() => runSteps(module, ['disable_module'], 'Disabled.')}>Disable</Button>
                        <Button variant="secondary" disabled={busy} onClick={() => runSteps(module, ['stop_module', 'start_module'], 'Reloaded.')}>Reload</Button>
                      </>
                    ) : (
                      <Button variant="primary" disabled={busy} onClick={() => setPending({ kind: 'enable', moduleId: module.module_id })}>Enable</Button>
                    )}
                    <Button variant="secondary" disabled={busy} onClick={() => update(module)}>Update</Button>
                    {!module.official && (
                      <Button variant="danger" disabled={busy} onClick={() => setPending({ kind: 'uninstall', moduleId: module.module_id })}>Uninstall</Button>
                    )}
                    <Button variant="ghost" onClick={() => toggleCrashLog(module.module_id)}>Crash log</Button>
                  </Stack>

                  {confirming === 'enable' && (
                    <EnableConfirm module={module} busy={busy} onConfirm={() => enable(module)} onCancel={() => setPending(null)} />
                  )}
                  {confirming === 'uninstall' && (
                    <ConfirmPanel label={`Uninstall ${module.name}`}>
                      <p className="m-0">Uninstall {module.name}? The module is stopped and removed.</p>
                      <Stack direction="horizontal" gap={2} className="flex-wrap">
                        <Button variant="danger" disabled={busy} onClick={() => runSteps(module, ['uninstall_module'], 'Uninstalled.')}>Confirm uninstall</Button>
                        <Button variant="secondary" disabled={busy} onClick={() => setPending(null)}>Cancel</Button>
                      </Stack>
                    </ConfirmPanel>
                  )}

                  {crashLog?.moduleId === module.module_id && <CrashLogList crashes={crashLog.crashes} />}
                </Card.Body>
              </Card>
            </article>
          );
        })}
      </Stack>
    </div>
  );
}
