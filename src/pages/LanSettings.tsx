import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { Button, Card, Input, Modal, Stack, Switch, toast, type ConfirmOptions, type ModalHandle } from '@dgos/ui';
import {
  LAN_FAILURE_TEXT,
  LAN_PORT_MAX,
  LAN_PORT_MIN,
  LAN_PORT_UNAVAILABLE,
  LanCommandError,
  disconnectAllLanDevices,
  disconnectLanDevice,
  getLanStatus,
  issueLanAccessCode,
  listLanAuditEvents,
  listLanDevices,
  setLanEnabled,
  setLanPort,
  type LanAuditEvent,
  type LanAuditEventKind,
  type LanDevice,
  type LanIssuedCode,
  type LanServiceState,
  type LanStatus,
} from '../lan/lanApi';
import { formatDateTime, t, useLocale as usePageLocale } from '../i18n';
import { usePageError } from '../pageError';

// V1-FR-007 §2.3 item 6 / AC15 and V1-FR-010 (D039, D099, D107): the host side of LAN collaboration. The page
// reads the service through the LAN host commands, polls status and devices while it is open, and issues an access
// code when it opens and on "重新生成". The plaintext code and pair URL live in React state only: nothing here
// writes to localStorage, sessionStorage or any store, and the state is gone when the page unmounts.

const POLL_MS = 5000;
const AUDIT_PAGE = 20;

const STATE_KEYS: Record<LanServiceState, string> = { stopped: 'lan.stopped', starting: 'lan.starting', running: 'lan.running', failed: 'lan.failed' };

const AUDIT_EVENT_KEYS: Record<LanAuditEventKind, string> = {
  pair_succeeded: 'lan.pairSucceeded', pair_rejected: 'lan.pairRejected', pair_rate_limited: 'lan.pairRateLimited', code_voided: 'lan.codeVoided', session_revoked: 'lan.sessionRevoked', request_rejected: 'lan.requestRejected',
};

const AUDIT_REASON_KEYS: Record<string, string> = {
  invalid_code: 'lan.invalidCode', expired_code: 'lan.expiredCode', expired: 'lan.expired', session_limit: 'lan.sessionLimit', host: 'lan.hostMismatch', origin: 'lan.originMismatch', host_disconnect: 'lan.hostDisconnect', host_disconnect_all: 'lan.hostDisconnectAll', switch_off: 'lan.switchOff', app_exit: 'lan.appExit', service_failed: 'lan.serviceFailed', network_changed: 'lan.networkChanged', port_changed: 'lan.portChanged', issuer_ended: 'lan.issuerEnded', ip_changed: 'lan.ipChanged', idle_expired: 'lan.idleExpired', absolute_expired: 'lan.absoluteExpired', re_paired: 'lan.rePaired', logout: 'lan.logout',
};


const nowSeconds = () => Math.floor(Date.now() / 1000);
const formatTime = (seconds: number) => formatDateTime(seconds * 1000);

/** The host already returns `XXXX-XXXX-...`; a code without separators is grouped by four. */
function groupCode(code: string): string {
  return code.includes('-') ? code : (code.match(/.{1,4}/g) ?? [code]).join('-');
}

function portOf(address: string | null): number | null {
  if (!address) return null;
  const port = Number(address.slice(address.lastIndexOf(':') + 1));
  return Number.isInteger(port) ? port : null;
}

function portProblem(text: string): string | undefined {
  if (!/^\d+$/.test(text.trim())) return t('lan.portRange', { min: LAN_PORT_MIN, max: LAN_PORT_MAX });
  const port = Number(text);
  return port >= LAN_PORT_MIN && port <= LAN_PORT_MAX ? undefined : t('lan.portRange', { min: LAN_PORT_MIN, max: LAN_PORT_MAX });
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(t('lan.copied'));
  } catch {
    toast.error(t('lan.copyFailed'));
  }
}

const cell = 'border-b border-edge-control px-3 py-2 text-left align-top wrap-anywhere';
const head = `${cell} font-semibold text-ink-secondary`;

function AuditLog({ reportError }: { reportError: (error: unknown) => void }) {
  usePageLocale();
  const [items, setItems] = useState<LanAuditEvent[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const alive = useRef(true);
  const request = useRef(0);

  const load = useCallback(async (from?: string) => {
    const mine = ++request.current;
    setBusy(true);
    try {
      const page = await listLanAuditEvents(from, AUDIT_PAGE);
      if (!alive.current || mine !== request.current) return;
      setItems((previous) => {
        if (from === undefined) return page.items;
        const seen = new Set(previous.map((item) => item.id));
        return [...previous, ...page.items.filter((item) => !seen.has(item.id))];
      });
      setCursor(page.next_cursor);
      setLoaded(true);
    } catch (error) {
      if (alive.current && mine === request.current) reportError(error);
    } finally {
      if (alive.current && mine === request.current) setBusy(false);
    }
  }, [reportError]);

  useEffect(() => {
    alive.current = true;
    void load();
    return () => { alive.current = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <Card>
      <Card.Header>
        <div className="flex items-center justify-between gap-3">
          <h2 className="m-0 text-base font-semibold">{t('lan.audit')}</h2>
          <Button variant="secondary" size="sm" disabled={busy} onClick={() => void load()}>{t('lan.refresh')}</Button>
        </div>
      </Card.Header>
      <Card.Body>
        <Stack gap={3}>
          {loaded && items.length === 0 ? (
            <p className="m-0 text-sm text-ink-secondary">{t('lan.noAudit')}</p>
          ) : null}
          {items.length > 0 ? (
            <div className="overflow-x-auto">
              <table className="w-full border-collapse text-sm" aria-label={t('lan.audit')}>
                <thead>
                  <tr>
                    <th scope="col" className={head}>{t('lan.time')}</th>
                    <th scope="col" className={head}>{t('lan.event')}</th>
                    <th scope="col" className={head}>{t('lan.reason')}</th>
                    <th scope="col" className={head}>{t('lan.sourceIp')}</th>
                    <th scope="col" className={head}>{t('lan.session')}</th>
                    <th scope="col" className={head}>{t('lan.errorCode')}</th>
                    <th scope="col" className={head}>{t('lan.count')}</th>
                  </tr>
                </thead>
                <tbody>
                  {items.map((item) => (
                    <tr key={item.id}>
                      <td className={cell}>{formatTime(item.last_at)}</td>
                      <td className={cell}>{AUDIT_EVENT_KEYS[item.event] ? t(AUDIT_EVENT_KEYS[item.event]) : item.event}</td>
                      <td className={cell}>{item.reason ? (AUDIT_REASON_KEYS[item.reason] ? t(AUDIT_REASON_KEYS[item.reason]) : item.reason) : '—'}</td>
                      <td className={cell}>{item.peer_ip ?? '—'}</td>
                      <td className={cell}>{item.session_short_id ?? '—'}</td>
                      <td className={cell}>{item.error_code ?? '—'}</td>
                      <td className={cell}>{item.count}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
          {cursor !== null ? (
            <div>
              <Button variant="secondary" loading={busy} onClick={() => void load(cursor)}>{t('lan.loadMore')}</Button>
            </div>
          ) : null}
        </Stack>
      </Card.Body>
    </Card>
  );
}

export default function LanSettings() {
  const reportError = usePageError('LanSettings');
  usePageLocale();
  const reportRef = useRef(reportError);
  reportRef.current = reportError;
  const report = useCallback((error: unknown) => reportRef.current(error), []);

  const [status, setStatus] = useState<LanStatus | null>(null);
  const [devices, setDevices] = useState<LanDevice[]>([]);
  const [loadFailed, setLoadFailed] = useState(false);
  const [code, setCode] = useState<LanIssuedCode | null>(null);
  const [codeUnavailable, setCodeUnavailable] = useState(false);
  const [issuing, setIssuing] = useState(false);
  const [now, setNow] = useState(nowSeconds);
  const [switching, setSwitching] = useState(false);
  const [portText, setPortText] = useState('');
  const [portError, setPortError] = useState<string>();
  const [portBusy, setPortBusy] = useState(false);

  const alive = useRef(true);
  const pollSeq = useRef(0);
  const codeSeq = useRef(0);
  const codeRequested = useRef(false);
  const pollFailed = useRef(false);
  const portDirty = useRef(false);
  const handles = useRef<ModalHandle[]>([]);

  const confirm = (options: ConfirmOptions) => {
    handles.current.push(Modal.confirm(options));
  };

  const issueCode = useCallback(async () => {
    const mine = ++codeSeq.current;
    setIssuing(true);
    try {
      const issued = await issueLanAccessCode();
      if (!alive.current || mine !== codeSeq.current) return;
      setCode(issued);
      setCodeUnavailable(issued === null);
      setNow(nowSeconds());
    } catch (error) {
      if (alive.current && mine === codeSeq.current) report(error);
    } finally {
      if (alive.current && mine === codeSeq.current) setIssuing(false);
    }
  }, [report]);

  // A code is issued once per running period: when the page sees the service running and none was requested yet.
  // Any other state drops the code, because the host voids it when the service stops.
  const applyStatus = useCallback((next: LanStatus) => {
    setStatus(next);
    setLoadFailed(false);
    setNow(nowSeconds());
    if (!portDirty.current) setPortText(String(next.preferred_port));
    if (next.state === 'running') {
      if (!codeRequested.current) {
        codeRequested.current = true;
        void issueCode();
      }
    } else {
      codeRequested.current = false;
      codeSeq.current += 1;
      setCode(null);
      setCodeUnavailable(false);
      setIssuing(false);
    }
  }, [issueCode]);

  const reloadDevices = useCallback(async () => {
    try {
      const list = await listLanDevices();
      if (alive.current) setDevices(list);
    } catch (error) {
      if (alive.current) report(error);
    }
  }, [report]);

  const refresh = useCallback(async () => {
    const mine = ++pollSeq.current;
    setNow(nowSeconds());
    try {
      const [next, list] = await Promise.all([getLanStatus(), listLanDevices()]);
      if (!alive.current || mine !== pollSeq.current) return;
      pollFailed.current = false;
      setDevices(list);
      applyStatus(next);
    } catch (error) {
      if (!alive.current || mine !== pollSeq.current) return;
      setLoadFailed(true);
      // A failing host would raise the same Toast every poll; report the first failure of a run only.
      if (!pollFailed.current) report(error);
      pollFailed.current = true;
    }
  }, [applyStatus, report]);

  useEffect(() => {
    alive.current = true;
    void refresh();
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    const open = handles.current;
    return () => {
      alive.current = false;
      window.clearInterval(timer);
      for (const handle of open) handle.close();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const changeEnabled = async (enabled: boolean) => {
    setSwitching(true);
    try {
      pollSeq.current += 1;
      const next = await setLanEnabled(enabled);
      if (!alive.current) return;
      applyStatus(next);
      await reloadDevices();
    } catch (error) {
      if (alive.current) report(error);
    } finally {
      if (alive.current) setSwitching(false);
    }
  };

  const toggle = (enabled: boolean) => {
    if (enabled) {
      void changeEnabled(true);
      return;
    }
    confirm({
      title: t('lan.disableTitle'),
      icon: 'warning',
      content: t('lan.disableContent'),
      confirmText: t('lan.disableConfirm'),
      confirmVariant: 'danger',
      onConfirm: () => changeEnabled(false),
    });
  };

  const regenerate = () => {
    codeRequested.current = true;
    void issueCode();
  };

  const currentPort = portOf(status?.address ?? null);

  const applyPort = async (port: number, invalidatesCode: boolean) => {
    setPortBusy(true);
    try {
      pollSeq.current += 1;
      const next = await setLanPort(port);
      if (!alive.current) return;
      portDirty.current = false;
      // Changing the port of a running service revoked every session and voided the code: issue a new one.
      if (invalidatesCode) codeRequested.current = false;
      applyStatus(next);
      setPortText(String(next.preferred_port));
      await reloadDevices();
    } catch (error) {
      if (!alive.current) return;
      if (error instanceof LanCommandError && error.code === LAN_PORT_UNAVAILABLE) setPortError(t('lan.portUnavailable'));
      else report(error);
    } finally {
      if (alive.current) setPortBusy(false);
    }
  };

  const submitPort = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (portBusy) return;
    const problem = portProblem(portText);
    setPortError(problem);
    if (problem) return;
    const port = Number(portText);
    const running = status?.state === 'running';
    const differs = running && port !== currentPort;
    if (!differs) {
      void applyPort(port, false);
      return;
    }
    confirm({
      title: t('lan.changePortTitle'),
      icon: 'warning',
      content: t('lan.changePortContent', { port }),
      confirmText: t('lan.changePortConfirm'),
      confirmVariant: 'danger',
      onConfirm: () => applyPort(port, true),
    });
  };

  const disconnectOne = (device: LanDevice) => {
    confirm({
      title: t('lan.disconnectTitle'),
      icon: 'warning',
      content: t('lan.disconnectContent', { label: device.label, ip: device.peer_ip }),
      confirmText: t('lan.disconnect'),
      confirmVariant: 'danger',
      onConfirm: async () => {
        try {
          await disconnectLanDevice(device.session_id);
        } catch (error) {
          if (alive.current) report(error);
        }
        if (alive.current) await refresh();
      },
    });
  };

  const disconnectAll = () => {
    confirm({
      title: t('lan.disconnectAllTitle'),
      icon: 'warning',
      content: t('lan.disconnectAllContent', { count: devices.length }),
      confirmText: t('lan.disconnectAll'),
      confirmVariant: 'danger',
      onConfirm: async () => {
        try {
          await disconnectAllLanDevices();
        } catch (error) {
          if (alive.current) report(error);
        }
        if (alive.current) await refresh();
      },
    });
  };

  const running = status?.state === 'running';
  const expired = code !== null && code.expires_at <= now;
  const actualPort = currentPort;

  return (
    <div className="settings-content settings-sections text-ink-primary">
      <h1 className="m-0 text-lg font-semibold text-ink-primary">{t('lan.title')}</h1>
        <div role="note" className="rounded-lg border border-status-warning bg-status-warning-surface p-3 text-sm text-status-warning-text wrap-anywhere">
         {t('lan.trustedNetwork')}
      </div>

      {status === null ? (
        <Card>
          <Card.Body>
            {loadFailed ? (
              <Stack gap={3}>
                <p role="alert" className="m-0 text-sm text-status-error-text">{t('lan.readFailed')}</p>
                <div>
                  <Button variant="secondary" onClick={() => void refresh()}>{t('system.retry')}</Button>
                </div>
              </Stack>
            ) : (
              <p role="status" className="m-0 text-sm text-ink-secondary">{t('lan.reading')}</p>
            )}
          </Card.Body>
        </Card>
      ) : (
        <>
          <Card>
            <Card.Header>
              <h2 className="m-0 text-base font-semibold">{t('lan.service')}</h2>
            </Card.Header>
            <Card.Body>
              <Stack gap={4}>
                <Switch label={t('lan.enabled')} checked={status.enabled} disabled={switching} onChange={toggle} />
                <dl className="m-0 flex flex-col gap-2 text-sm">
                  <div className="flex gap-4">
                    <dt className="w-28 shrink-0 text-ink-secondary">{t('lan.serviceStatus')}</dt>
                    <dd className="m-0">{t(STATE_KEYS[status.state])}</dd>
                  </div>
                  {status.failure_code !== null ? (
                    <div className="flex gap-4">
                      <dt className="w-28 shrink-0 text-ink-secondary">{t('lan.failureReason')}</dt>
                 <dd className="m-0 text-status-error-text">{LAN_FAILURE_TEXT[status.failure_code] ?? t('lan.failureReason')}</dd>
                    </div>
                  ) : null}
                  {running && status.address ? (
                    <>
                      <div className="flex gap-4">
                        <dt className="w-28 shrink-0 text-ink-secondary">{t('lan.address')}</dt>
                        <dd className="m-0 select-all break-all">{`http://${status.address}`}</dd>
                      </div>
                      <div className="flex gap-4">
                        <dt className="w-28 shrink-0 text-ink-secondary">{t('lan.currentPort')}</dt>
                        <dd className="m-0">{actualPort ?? '—'}</dd>
                      </div>
                    </>
                  ) : null}
                  <div className="flex gap-4">
                    <dt className="w-28 shrink-0 text-ink-secondary">{t('lan.preferredPort')}</dt>
                    <dd className="m-0">{status.preferred_port}</dd>
                  </div>
                  <div className="flex gap-4">
                    <dt className="w-28 shrink-0 text-ink-secondary">{t('lan.devices')}</dt>
                    <dd className="m-0">{status.session_count}</dd>
                  </div>
                </dl>
                {running && status.port_fallback && actualPort !== null ? (
                  <p role="status" className="m-0 text-sm text-status-warning-text">
                    {t('lan.fallback', { preferred: status.preferred_port, actual: actualPort })}
                  </p>
                ) : null}
              </Stack>
            </Card.Body>
          </Card>

          <Card>
            <Card.Header>
              <h2 className="m-0 text-base font-semibold">{t('lan.preferredPort')}</h2>
            </Card.Header>
            <Card.Body>
              <form onSubmit={submitPort} noValidate>
                <Stack gap={3}>
                  <Input
                    label={t('lan.preferredPort')}
                    type="number"
                    inputMode="numeric"
                    min={LAN_PORT_MIN}
                    max={LAN_PORT_MAX}
                    value={portText}
                    error={portError}
                    onChange={(event) => {
                      portDirty.current = true;
                      setPortText(event.target.value);
                      setPortError(undefined);
                    }}
                  />
                  <p className="m-0 text-xs text-ink-secondary">
                    {t('lan.portHelp', { min: LAN_PORT_MIN, max: LAN_PORT_MAX })}
                  </p>
                  <div>
                    <Button type="submit" variant="primary" loading={portBusy}>{t('common.save')}</Button>
                  </div>
                </Stack>
              </form>
            </Card.Body>
          </Card>

          <Card>
            <Card.Header>
              <div className="flex items-center justify-between gap-3">
                <h2 className="m-0 text-base font-semibold">{t('lan.credentials')}</h2>
                <Button variant="secondary" size="sm" loading={issuing} disabled={!running} onClick={regenerate}>
                   {t('lan.regenerate')}
                </Button>
              </div>
            </Card.Header>
            <Card.Body>
              <Stack gap={3}>
                {!running ? (
                  <p className="m-0 text-sm text-ink-secondary">{t('lan.noCode')}</p>
                ) : codeUnavailable ? (
                  <p role="status" className="m-0 text-sm text-ink-secondary">{t('lan.noCode')}</p>
                ) : code === null ? (
                  <p role="status" className="m-0 text-sm text-ink-secondary">
                    {issuing ? t('lan.generatingCode') : t('lan.noCodeYet')}
                  </p>
                ) : expired ? (
                  <p role="status" className="m-0 text-sm text-status-warning-text">{t('lan.codeExpired')}</p>
                ) : (
                  <>
                    <div className="flex flex-wrap items-center gap-3">
                      <code data-testid="lan-code" className="select-all rounded-md bg-surface-inset px-3 py-2 font-mono text-lg tracking-wider">
                        {groupCode(code.code)}
                      </code>
                      <Button variant="secondary" size="sm" onClick={() => void copyText(groupCode(code.code))}>{t('lan.copyCode')}</Button>
                    </div>
                    <p className="m-0 text-sm text-ink-secondary">{t('lan.validUntil', { time: formatTime(code.expires_at) })}</p>
                    <div className="flex flex-wrap items-center gap-3">
                      <code data-testid="lan-pair-url" className="select-all break-all rounded-md bg-surface-inset px-3 py-2 font-mono text-sm">
                        {code.pair_url}
                      </code>
                      <Button variant="secondary" size="sm" onClick={() => void copyText(code.pair_url)}>{t('lan.copyLink')}</Button>
                    </div>
                  </>
                )}
              </Stack>
            </Card.Body>
          </Card>

          <Card>
            <Card.Header>
              <div className="flex items-center justify-between gap-3">
                <h2 className="m-0 text-base font-semibold">{t('lan.devices')}</h2>
                <Button variant="danger" size="sm" disabled={devices.length === 0} onClick={disconnectAll}>
                   {t('lan.disconnectAll')}
                </Button>
              </div>
            </Card.Header>
            <Card.Body>
              {devices.length === 0 ? (
                <p className="m-0 text-sm text-ink-secondary">{t('lan.noDevices')}</p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full border-collapse text-sm" aria-label={t('lan.devices')}>
                    <thead>
                      <tr>
                        <th scope="col" className={head}>{t('lan.device')}</th>
                        <th scope="col" className={head}>{t('lan.ip')}</th>
                        <th scope="col" className={head}>{t('lan.shortId')}</th>
                        <th scope="col" className={head}>{t('lan.connectedAt')}</th>
                        <th scope="col" className={head}>{t('lan.lastActivity')}</th>
                        <th scope="col" className={head}>{t('lan.actions')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {devices.map((device) => (
                        <tr key={device.session_id}>
                          <td className={cell}>{device.label}</td>
                          <td className={cell}>{device.peer_ip}</td>
                          <td className={cell}>{device.short_id}</td>
                          <td className={cell}>{formatTime(device.created_at)}</td>
                          <td className={cell}>{formatTime(device.last_active_at)}</td>
                          <td className={cell}>
                            <Button
                              variant="secondary"
                              size="sm"
                               aria-label={`${t('lan.disconnect')} ${device.label}`}
                              onClick={() => disconnectOne(device)}
                            >
                              {t('lan.disconnect')}
                            </Button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </Card.Body>
          </Card>
        </>
      )}

      <AuditLog reportError={report} />
    </div>
  );
}
