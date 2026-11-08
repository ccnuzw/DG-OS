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
import './lan-settings.css';

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
    <section className="lan-section" aria-labelledby="lan-audit-heading">
      <header className="lan-heading">
        <h2 id="lan-audit-heading">{t('lan.audit')}</h2>
        <Button variant="secondary" size="sm" disabled={busy} onClick={() => void load()}>{t('lan.refresh')}</Button>
      </header>
        <Stack gap={3}>
          {loaded && items.length === 0 ? (
            <p className="m-0 text-sm text-ink-secondary">{t('lan.noAudit')}</p>
          ) : null}
          {items.length > 0 ? (
            <ul className="lan-list" aria-label={t('lan.audit')}>
              {items.map(item => <li key={item.id} className="lan-record">
                <div className="lan-heading">
                  <strong>{AUDIT_EVENT_KEYS[item.event] ? t(AUDIT_EVENT_KEYS[item.event]) : item.event}</strong>
                  <time className="lan-muted" dateTime={new Date(item.last_at * 1000).toISOString()}>{formatTime(item.last_at)}</time>
                </div>
                <div className="lan-record-context">
                  <span>{item.reason ? (AUDIT_REASON_KEYS[item.reason] ? t(AUDIT_REASON_KEYS[item.reason]) : item.reason) : '—'}</span>
                  <span className="lan-muted">{t('lan.sourceIp')}：{item.peer_ip ?? '—'}</span>
                </div>
                <details className="lan-record-details"><summary>{t('lan.details')}</summary>
                  <dl className="lan-metadata">
                    <div><dt>{t('lan.session')}</dt><dd>{item.session_short_id ?? '—'}</dd></div>
                    <div><dt>{t('lan.errorCode')}</dt><dd>{item.error_code ?? '—'}</dd></div>
                    <div><dt>{t('lan.count')}</dt><dd>{item.count}</dd></div>
                  </dl>
                </details>
              </li>)}
            </ul>
          ) : null}
          {cursor !== null ? (
            <div>
              <Button variant="secondary" loading={busy} onClick={() => void load(cursor)}>{t('lan.loadMore')}</Button>
            </div>
          ) : null}
        </Stack>
    </section>
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
    <div className="lan-settings settings-content text-ink-primary">
      <h1 className="m-0 text-lg font-semibold text-ink-primary">{t('lan.title')}</h1>

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
          <section className="lan-section" aria-labelledby="lan-service-heading">
            <header className="lan-heading">
              <h2 id="lan-service-heading">{t('lan.service')}</h2>
              <div className="lan-service-controls">
                <span className={`lan-service-state is-${status.state}`} role="status"><span aria-hidden="true" />{t(STATE_KEYS[status.state])}</span>
                <Switch label={t('lan.enabled')} checked={status.enabled} disabled={switching} onChange={toggle} />
              </div>
            </header>
            {status.failure_code !== null ? <p role="alert" className="lan-error">{LAN_FAILURE_TEXT[status.failure_code] ?? t('lan.failureReason')}</p> : null}
            {running && status.address ? <dl className="lan-address"><dt>{t('lan.address')}</dt><dd>{`http://${status.address}`}</dd></dl> : null}
            <form className="lan-port-form" onSubmit={submitPort} noValidate>
              <div className="lan-port-controls">
                <Input label={t('lan.preferredPort')} type="number" inputMode="numeric" min={LAN_PORT_MIN} max={LAN_PORT_MAX} value={portText} error={portError} onChange={event => {
                  portDirty.current = true;
                  setPortText(event.target.value);
                  setPortError(undefined);
                }} />
                <Button type="submit" loading={portBusy} disabled={switching || portText.trim() === String(status.preferred_port)}>{t('lan.savePort')}</Button>
              </div>
              <p className="lan-muted">{t('lan.portHelp', { min: LAN_PORT_MIN, max: LAN_PORT_MAX })}</p>
            </form>
            {running && status.port_fallback && actualPort !== null ? <p role="status" className="lan-warning">{t('lan.fallback', { preferred: status.preferred_port, actual: actualPort })}</p> : null}
            <p role="note" className="lan-warning">{t('lan.trustedNetwork')}</p>
          </section>

          <section className="lan-section" aria-labelledby="lan-connect-heading">
            <header className="lan-heading">
              <h2 id="lan-connect-heading">{t('lan.connectDevice')}</h2>
              <Button variant="secondary" size="sm" loading={issuing} disabled={!running} onClick={regenerate}>{t('lan.regenerate')}</Button>
            </header>
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
                    <span className="lan-muted">{t('lan.credentials')}</span>
                    <div className="lan-copy-row">
                      <code data-testid="lan-code" className="lan-code">
                        {groupCode(code.code)}
                      </code>
                      <Button variant="secondary" size="sm" onClick={() => void copyText(groupCode(code.code))}>{t('lan.copyCode')}</Button>
                    </div>
                    <p className="m-0 text-sm text-ink-secondary">{t('lan.validUntil', { time: formatTime(code.expires_at) })}</p>
                    <span className="lan-muted">{t('lan.pairLink')}</span>
                    <div className="lan-copy-row">
                      <code data-testid="lan-pair-url" className="lan-pair-url">
                        {code.pair_url}
                      </code>
                      <Button variant="secondary" size="sm" onClick={() => void copyText(code.pair_url)}>{t('lan.copyLink')}</Button>
                    </div>
                    <p className="lan-muted">{t('lan.regenerateHelp')}</p>
                  </>
                )}
              </Stack>
          </section>

          <section className="lan-section" aria-labelledby="lan-devices-heading">
            <header className="lan-heading">
              <h2 id="lan-devices-heading">{t('lan.devices')} <span className="lan-count">{status.session_count}</span></h2>
              <Button variant="danger" size="sm" disabled={devices.length === 0} onClick={disconnectAll}>{t('lan.disconnectAll')}</Button>
            </header>
              {devices.length === 0 ? (
                <p className="lan-muted">{t(running ? 'lan.noDevices' : 'lan.serviceNotRunning')}</p>
              ) : (
                <ul className="lan-list" aria-label={t('lan.devices')}>
                  {devices.map(device => <li key={device.session_id} className="lan-device">
                    <div className="lan-heading">
                      <div className="lan-device-identity"><strong>{device.label}</strong><span className="lan-muted">{device.peer_ip}</span></div>
                      <Button variant="secondary" size="sm" aria-label={`${t('lan.disconnect')} ${device.label}`} onClick={() => disconnectOne(device)}>{t('lan.disconnect')}</Button>
                    </div>
                    <dl className="lan-metadata">
                      <div><dt>{t('lan.connectedAt')}</dt><dd>{formatTime(device.created_at)}</dd></div>
                      <div><dt>{t('lan.lastActivity')}</dt><dd>{formatTime(device.last_active_at)}</dd></div>
                      <div><dt>{t('lan.shortId')}</dt><dd>{device.short_id}</dd></div>
                    </dl>
                  </li>)}
                </ul>
              )}
          </section>
        </>
      )}

      <AuditLog reportError={report} />
    </div>
  );
}
