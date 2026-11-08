import { t, useLocale as usePageLocale } from '../i18n';
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Button, Checkbox, Loading, Stack, StatusIcon, cn } from '@dgos/ui';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import {
  ModuleApiError,
  grantModulePermission,
  listModuleCrashes,
  listModulePermissionAudits,
  listModules,
  reloadModule,
  revokeModulePermission,
  runOperations,
  type ModuleCrashRecord,
  type ModulePermissionAuditEntry,
  type ModuleSummary,
} from '../modules/moduleApi';
import { usePageError } from '../pageError';
import { formatDateTime } from '../i18n';
import { Package } from 'lucide-react';
import './module-management.css';

// Failures are shown twice on purpose: inline (`ErrorNotice`, with the host's own message and code, which stays next
// to the module it belongs to) and through the unified Toast. Module errors are 1004xxx codes with no entry in
// ERROR_MESSAGES yet (FR-005 §13.2), so the Toast carries the generic text plus the code. The raw error is reported,
// not the one `toApiError` builds, so a non-host failure shows the generic text without a made-up code.

// Anything that is not a ModuleApiError is a programming error: show a neutral message, never the raw error.
export function toApiError(error: unknown): ModuleApiError {
  return error instanceof ModuleApiError ? error : new ModuleApiError(0, 'Unexpected error while talking to the host');
}

/** Text link used by both module pages. */
export const MODULE_LINK_CLASS = cn('text-accent-default underline underline-offset-4', FOCUS_RING);

type NoticeTone = 'error' | 'warning' | 'success';

const NOTICE_TONE: Record<NoticeTone, { box: string; icon: 'error' | 'warning' | 'success' }> = {
  error: { box: 'border-status-error bg-status-error-surface text-status-error-text', icon: 'error' },
  warning: { box: 'border-status-warning bg-status-warning-surface text-status-warning-text', icon: 'warning' },
  success: { box: 'border-status-success bg-status-success-surface text-status-success-text', icon: 'success' },
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
  usePageLocale();
  const { box, icon } = NOTICE_TONE[tone];
  return (
    <div role={role} className={cn('my-2 flex items-start gap-2 rounded-md border p-3 text-sm', box, className)}>
      <StatusIcon name={icon} className="mt-0.5" />
      <div className="min-w-0 wrap-anywhere">{children}</div>
    </div>
  );
}

export function ErrorNotice({ error }: { error: ModuleApiError }) {
  usePageLocale();
  return (
    <NoticeBox tone={error.incompatible ? 'warning' : 'error'} role="alert">
      {error.incompatible && <strong>{t('module.errorIncompatible')}</strong>}
      <span>{error.message}</span>
      {error.code !== 0 && <span> {t('detail.code')}{error.code})</span>}
    </NoticeBox>
  );
}

export function CrashLogList({ crashes }: { crashes: ModuleCrashRecord[] }) {
  usePageLocale();
  if (crashes.length === 0) return <p className="my-2 text-sm text-ink-secondary">{t('module.noCrashes')}</p>;
  return (
    <ul className="m-0 list-none p-0">
      {crashes.map((crash) => {
        const at = new Date(crash.created_at * 1000);
        return (
          <li key={crash.crash_id} className="border-t border-border-subtle py-2 text-sm first:border-t-0">
            <div className="wrap-anywhere">
              <time dateTime={at.toISOString()}>{formatDateTime(at)}</time> · {crash.source} · <strong>{crash.error_name}</strong>{' '}
              {t('detail.versionParen')}{crash.module_version})
            </div>
            <pre className="m-0 mt-2 overflow-x-auto rounded-md bg-surface-inset p-2 font-mono text-xs text-ink-primary">
              {crash.stack}
            </pre>
          </li>
        );
      })}
    </ul>
  );
}

function ConfirmPanel({ label, children }: { label: string; children: ReactNode }) {
  usePageLocale();
  return (
    <section role="dialog" aria-label={label} className="my-3 rounded-lg border border-border-subtle bg-surface-panel p-4 text-sm text-ink-primary">
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
  usePageLocale();
  const [confirmed, setConfirmed] = useState(false);
  return (
    <ConfirmPanel label={t('module.enableAria', { name: module.name })}>
      {module.permissions.length === 0 ? (
        <p className="m-0">{module.name} {t('detail.noPermissions')}</p>
      ) : (
        <>
          <p className="m-0">{module.name} {t('detail.requestsPermissions')}</p>
          <ul className="m-0 list-disc pl-6">
            {module.permissions.map((entry) => (
              <li key={entry.permission}>
                <code className="font-mono wrap-anywhere">{entry.permission}</code>
                {entry.granted ? t('module.alreadyGranted') : ''}
              </li>
            ))}
          </ul>
        </>
      )}
      <Checkbox
        checked={confirmed}
        onChange={setConfirmed}
        label={t('module.reviewPermissions', { name: module.name })}
      />
      <Stack direction="horizontal" gap={2} className="flex-wrap">
        <Button variant="primary" disabled={!confirmed || busy} onClick={onConfirm}>{t('module.confirmEnable')}</Button>
        <Button variant="secondary" disabled={busy} onClick={onCancel}>{t('module.cancel')}</Button>
      </Stack>
    </ConfirmPanel>
  );
}

interface GrantFailure {
  permission: string;
  error: ModuleApiError;
}

interface ActionNotice {
  error?: ModuleApiError;
  info?: string;
  /** Confirmed permissions the host refused while enabling; the module was not started. */
  failures?: GrantFailure[];
}

/** What an update changed in the declared permissions (D093, D086). */
interface PermissionDiff {
  added: string[];
  /** Permissions that were granted and are no longer declared; the host withdrew those grants. */
  withdrawn: string[];
}

interface AuditLogState {
  entries: ModulePermissionAuditEntry[];
  nextBefore: string | null;
}

function UpdateDiff({ module, diff, busy, onGrant, onDismiss }: {
  module: ModuleSummary;
  diff: PermissionDiff;
  busy: boolean;
  onGrant: (permission: string) => void;
  onDismiss: () => void;
}) {
  usePageLocale();
  // A new permission stops being listed as soon as it is granted. The page's module list may still be the
  // pre-update one for a moment, so only a granted entry hides it; a missing entry does not.
  const added = diff.added.filter(
    (permission) => !module.permissions.some((entry) => entry.permission === permission && entry.granted),
  );
  if (added.length === 0 && diff.withdrawn.length === 0) return null;
  return (
    <NoticeBox tone="warning" role="status">
      <strong>{t('module.changedPermissions')}</strong>
      {added.length > 0 && (
        <>
          <p className="m-0 mt-2">{t('module.newPermissions')}</p>
          <ul className="m-0 list-none p-0">
            {added.map((permission) => (
              <li key={permission} className="flex flex-wrap items-center gap-2 py-1">
                <code className="font-mono wrap-anywhere">{permission}</code>
                <Button
                  variant="secondary"
                  size="sm"
                  aria-label={t('module.grantNewAria', { permission, name: module.name })}
                  disabled={busy}
                  onClick={() => onGrant(permission)}
                >
                  {t('module.grant')}</Button>
              </li>
            ))}
          </ul>
        </>
      )}
      {diff.withdrawn.length > 0 && (
        <>
          <p className="m-0 mt-2">{t('module.noLongerDeclared')}</p>
          <ul className="m-0 list-disc pl-6">
            {diff.withdrawn.map((permission) => (
              <li key={permission}>
                <code className="font-mono wrap-anywhere">{permission}</code>
              </li>
            ))}
          </ul>
        </>
      )}
      <div className="mt-2">
        <Button variant="ghost" size="sm" onClick={onDismiss}>{t('module.dismiss')}</Button>
      </div>
    </NoticeBox>
  );
}

function PermissionAuditList({ log, busy, onLoadOlder }: {
  log: AuditLogState;
  busy: boolean;
  onLoadOlder: () => void;
}) {
  usePageLocale();
  return (
    <div aria-label={t('module.permissionLogButton')} role="region">
      {log.entries.length === 0 ? (
        <p className="my-2 text-sm text-ink-secondary">{t('module.noPermissionActivity')}</p>
      ) : (
        <ul className="m-0 list-none p-0">
          {log.entries.map((entry, index) => {
            const at = new Date(entry.created_at * 1000);
            return (
              <li key={index} className="border-t border-border-subtle py-2 text-sm first:border-t-0 wrap-anywhere">
                <code className="font-mono">{entry.permission}</code> · <strong>{entry.action}</strong> ·{' '}
                <time dateTime={at.toISOString()}>{formatDateTime(at)}</time>
              </li>
            );
          })}
        </ul>
      )}
      {log.nextBefore !== null && (
        <Button variant="secondary" size="sm" disabled={busy} onClick={onLoadOlder}>{t('module.loadOlder')}</Button>
      )}
    </div>
  );
}

type Pending = { kind: 'enable' | 'uninstall'; moduleId: string } | null;


export default function ModuleManagement() {
  usePageLocale();
  const [modules, setModules] = useState<ModuleSummary[] | null>(null);
  const [loadError, setLoadError] = useState<ModuleApiError | null>(null);
  const [busy, setBusy] = useState(false);
  const [notices, setNotices] = useState<Record<string, ActionNotice>>({});
  const [pending, setPending] = useState<Pending>(null);
  const [crashLog, setCrashLog] = useState<{ moduleId: string; crashes: ModuleCrashRecord[] } | null>(null);
  const [diffs, setDiffs] = useState<Record<string, PermissionDiff>>({});
  const [auditLogs, setAuditLogs] = useState<Record<string, AuditLogState>>({});
  const [auditBusy, setAuditBusy] = useState(false);
  const [filter, setFilter] = useState('all');
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

  function setDiff(moduleId: string, diff: PermissionDiff | null) {
    setDiffs(({ [moduleId]: _old, ...rest }) => (diff === null ? rest : { ...rest, [moduleId]: diff }));
  }

  // Runs one user action: the task resolves to a success message (or a ready-made notice) or throws. The list is
  // refreshed afterwards either way.
  async function act(moduleId: string, task: () => Promise<string | ActionNotice>) {
    setBusy(true);
    setPending(null);
    setNotice(moduleId, {});
    let notice: ActionNotice;
    try {
      const result = await task();
      notice = typeof result === 'string' ? { info: result } : result;
    } catch (error) {
      notice = { error: toApiError(error) };
      reportError(error);
    }
    if (mounted.current) setNotice(moduleId, notice);
    await refresh();
    if (mounted.current) setBusy(false);
  }

  // A disabled module cannot be granted anything (the host refuses it), so the order is: enable,
  // grant what the user just confirmed, then start. The steps are separate host commands and not one
  // transaction (D093): every confirmed grant is attempted, a failed one never stops the others, and
  // the module is started only when all of them succeeded. Grants that did succeed stay in place.
  const enable = (module: ModuleSummary) =>
    act(module.module_id, async () => {
      await runOperations(['enable_module'], module.module_id);
      const failures: GrantFailure[] = [];
      for (const entry of module.permissions) {
        if (entry.granted) continue;
        try {
          await grantModulePermission(module.module_id, entry.permission);
        } catch (error) {
          failures.push({ permission: entry.permission, error: toApiError(error) });
          reportError(error);
        }
      }
      if (failures.length > 0) return { failures };
      await runOperations(['start_module'], module.module_id);
      return t('module.enabledStarted');
    });

  // D093: the page remembers what the module declared and had granted before the update, then compares it
  // with the refreshed list. There is no preview before the update. `updated` and `updated_pending_release`
  // both mean the new manifest is in effect, so both get the diff; pending release only changes the message.
  const update = (module: ModuleSummary) =>
    act(module.module_id, async () => {
      setDiff(module.module_id, null);
      const before = new Map(module.permissions.map((entry) => [entry.permission, entry.granted]));
      const view = await runOperations(['update_module'], module.module_id);
      if (view.outcome === 'unchanged') return t('module.upToDate');
      try {
        const current = (await listModules()).find((entry) => entry.module_id === module.module_id);
        if (current !== undefined) {
          const declared = new Set(current.permissions.map((entry) => entry.permission));
          const diff: PermissionDiff = {
            added: current.permissions.filter((entry) => !before.has(entry.permission)).map((entry) => entry.permission),
            withdrawn: [...before].filter(([permission, granted]) => granted && !declared.has(permission)).map(([permission]) => permission),
          };
          if (diff.added.length > 0 || diff.withdrawn.length > 0) setDiff(module.module_id, diff);
        }
      } catch {
        // The update itself succeeded; the refresh that follows reports a failed list.
      }
      if (view.outcome === 'updated_pending_release') return t('module.updatedPending');
      return t('module.updated');
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

  async function loadAudit(moduleId: string, before: string | null) {
    setAuditBusy(true);
    try {
      const page = await listModulePermissionAudits(moduleId, before);
      if (!mounted.current) return;
      setAuditLogs((current) => ({
        ...current,
        [moduleId]: {
          entries: before === null ? page.entries : [...(current[moduleId]?.entries ?? []), ...page.entries],
          nextBefore: page.next_before,
        },
      }));
    } catch (error) {
      if (mounted.current) setNotice(moduleId, { error: toApiError(error) });
      reportError(error);
    } finally {
      if (mounted.current) setAuditBusy(false);
    }
  }

  async function toggleAuditLog(moduleId: string) {
    if (auditLogs[moduleId] !== undefined) {
      setAuditLogs(({ [moduleId]: _closed, ...rest }) => rest);
      return;
    }
    await loadAudit(moduleId, null);
  }
  const visibleModules = modules?.filter(module => filter === 'all' || (filter === 'enabled' ? module.enabled : module.runtime.state === 'failed' || module.permissions.some(permission => !permission.granted)));

  return (
    <div className="settings-content module-management">
      <header className="module-page-heading">
        <h1>{t('module.title')}</h1>
        <p>{t('module.subtitle')}</p>
      </header>

      <div role="group" aria-label={t('module.filters')} className="module-filters">{['all', 'enabled', 'pending'].map(value => <Button key={value} variant="ghost" aria-pressed={filter === value} onClick={() => setFilter(value)}>{t(`module.filter.${value}`)}</Button>)}</div>
      {loadError !== null && <ErrorNotice error={loadError} />}
      {modules === null && loadError === null && <Loading label={t('module.loading')} />}
      {modules?.length === 0 && <p className="text-sm text-ink-secondary">{t('module.none')}</p>}
      {Boolean(modules?.length) && visibleModules?.length === 0 && <p role="status" className="text-sm text-ink-secondary">{t('module.noMatches')}</p>}

      <div className="module-list">
        {visibleModules?.map((module) => {
          const notice = notices[module.module_id] ?? {};
          const confirming = pending?.moduleId === module.module_id ? pending.kind : null;
          return (
            <article className="module-card" key={module.module_id} aria-label={module.name} data-testid={`module-${module.module_id}`}>
              <header className="module-card-heading">
                <span className="module-mark" aria-hidden="true"><Package size={22} /></span>
                <div className="module-identity">
                  <div className="module-name-row"><h2>{module.name}</h2>{module.official && <span className="module-official">{t('module.official')}</span>}</div>
                  <span className="module-id">{module.module_id}</span>
                </div>
                <span className="module-version">{t('module.version')} {module.version}</span>
              </header>
              <div className="module-status-row">
                <span className={cn('module-state', module.enabled && 'is-enabled')}><i aria-hidden="true" />{module.enabled ? t('module.enabled') : t('module.disabled')}</span>
                <span className={cn('module-runtime', `is-${module.runtime.state}`)}>{t('detail.runtime')}{module.runtime.state}</span>
                {module.recent_crashes > 0 && <span className="module-crash-count">{t('module.recentCrashes')}: {module.recent_crashes}</span>}
              </div>
              <div className="module-primary-actions">
                    {module.enabled ? (
                      <>
                        <Button variant="secondary" disabled={busy} onClick={() => runSteps(module, ['disable_module'], t('module.disabledNotice'))}>{t('module.disable')}</Button>
                        <Button
                          variant="secondary"
                          disabled={busy}
                          onClick={() =>
                            act(module.module_id, async () => {
                              await reloadModule(module.module_id);
                              return t('module.reloaded');
                            })
                          }
                        >
                          {t('module.reload')}</Button>
                      </>
                    ) : (
                      <Button variant="primary" disabled={busy} onClick={() => setPending({ kind: 'enable', moduleId: module.module_id })}>{t('module.enableButton')}</Button>
                    )}
                    <Button variant="secondary" disabled={busy} onClick={() => update(module)}>{t('module.update')}</Button>
                    {!module.official && (
                      <Button variant="danger" disabled={busy} onClick={() => setPending({ kind: 'uninstall', moduleId: module.module_id })}>{t('module.uninstall')}</Button>
                    )}
              </div>
                  {module.runtime.state === 'failed' && module.recent_crashes > 0 && <NoticeBox tone="error" role="alert">{t('module.moduleCrashed', { id: module.module_id })}</NoticeBox>}
                  {module.recent_crashes >= 3 && <NoticeBox tone="warning" role="alert">{t('module.faulty')}</NoticeBox>}
              <section className="module-permissions" aria-label={t('module.permissions')}>
                <h3>{t('module.permissions')} <span>{module.permissions.length}</span></h3>

                  {module.permissions.length === 0 ? (
                    <p className="m-0 my-2">{t('module.noPermissions')}</p>
                  ) : (
                    <ul className="module-permission-list">
                      {module.permissions.map((entry) => (
                        <li key={entry.permission} className="module-permission-row">
                          <code>{entry.permission}</code>
                          <span className={entry.granted ? 'module-permission-granted' : 'module-permission-pending'}>{entry.granted ? t('module.granted') : t('module.notGranted')}</span>
                          {entry.granted ? (
                            <Button
                              variant="secondary"
                              size="sm"
                              aria-label={t('module.revokeAria', { permission: entry.permission, name: module.name })}
                              disabled={busy}
                              onClick={() =>
                                act(module.module_id, async () => {
                                  await revokeModulePermission(module.module_id, entry.permission);
                                  return t('module.revokedNotice', { permission: entry.permission });
                                })
                              }
                            >
                              {t('module.revoke')}</Button>
                          ) : (
                            <Button
                              variant="secondary"
                              size="sm"
                              aria-label={t('module.grantAria', { permission: entry.permission, name: module.name })}
                              disabled={busy}
                              onClick={() =>
                                act(module.module_id, async () => {
                                  await grantModulePermission(module.module_id, entry.permission);
                                  return t('module.grantedNotice', { permission: entry.permission });
                                })
                              }
                            >
                              {t('module.grant')}</Button>
                          )}
                        </li>
                      ))}
                    </ul>
                  )}
              </section>

                  {notice.error !== undefined && <ErrorNotice error={notice.error} />}
                  {notice.info !== undefined && (
                    <NoticeBox tone="success" role="status">{notice.info}</NoticeBox>
                  )}
                  {notice.failures !== undefined && notice.failures.length > 0 && (
                    <NoticeBox tone="warning" role="alert">
                      <strong>
                        {t('module.enableFailed', { count: notice.failures.length })}</strong>
                      <ul className="m-0 mt-2 list-disc pl-6">
                        {notice.failures.map((failure) => (
                          <li key={failure.permission}>
                            <code className="font-mono wrap-anywhere">{failure.permission}</code>: {failure.error.message}
                            {failure.error.code !== 0 && <span> {t('detail.code')}{failure.error.code})</span>}
                          </li>
                        ))}
                      </ul>
                      <p className="m-0 mt-2">
                        {t('module.grantRemaining')}</p>
                    </NoticeBox>
                  )}
                  {diffs[module.module_id] !== undefined && (
                    <UpdateDiff
                      module={module}
                      diff={diffs[module.module_id]}
                      busy={busy}
                      onGrant={(permission) =>
                        act(module.module_id, async () => {
                          await grantModulePermission(module.module_id, permission);
                          return t('module.grantedNotice', { permission });
                        })
                      }
                      onDismiss={() => setDiff(module.module_id, null)}
                    />
                  )}

              <div className="module-log-actions">
                <Button variant="ghost" aria-expanded={crashLog?.moduleId === module.module_id} onClick={() => toggleCrashLog(module.module_id)}>{t('module.crashLog')}</Button>
                <Button variant="ghost" aria-expanded={auditLogs[module.module_id] !== undefined} onClick={() => toggleAuditLog(module.module_id)}>{t('module.permissionLogButton')}</Button>
              </div>

                  {confirming === 'enable' && (
                    <EnableConfirm module={module} busy={busy} onConfirm={() => enable(module)} onCancel={() => setPending(null)} />
                  )}
                  {confirming === 'uninstall' && (
                    <ConfirmPanel label={t('module.uninstallLabel', { id: module.name })}>
                      <p className="m-0">{t('module.uninstall')}{module.name}{t('detail.uninstallSuffix')}</p>
                      <Stack direction="horizontal" gap={2} className="flex-wrap">
                        <Button variant="danger" disabled={busy} onClick={() => runSteps(module, ['uninstall_module'], t('module.uninstalled'))}>{t('module.confirmUninstall')}</Button>
                        <Button variant="secondary" disabled={busy} onClick={() => setPending(null)}>{t('module.cancel')}</Button>
                      </Stack>
                    </ConfirmPanel>
                  )}

                  {crashLog?.moduleId === module.module_id && <CrashLogList crashes={crashLog.crashes} />}
                  {auditLogs[module.module_id] !== undefined && (
                    <PermissionAuditList
                      log={auditLogs[module.module_id]}
                      busy={auditBusy}
                      onLoadOlder={() => loadAudit(module.module_id, auditLogs[module.module_id].nextBefore)}
                    />
                  )}
            </article>
          );
        })}
      </div>
    </div>
  );
}
