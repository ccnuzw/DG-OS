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
import { usePageError } from '../pageError';

// V1-FR-007 §2.3 item 6 / AC15 and V1-FR-010 (D039, D099, D107): the host side of LAN collaboration. The page
// reads the service through the LAN host commands, polls status and devices while it is open, and issues an access
// code when it opens and on "重新生成". The plaintext code and pair URL live in React state only: nothing here
// writes to localStorage, sessionStorage or any store, and the state is gone when the page unmounts.

const POLL_MS = 5000;
const AUDIT_PAGE = 20;

const STATE_TEXT: Record<LanServiceState, string> = {
  stopped: '已停止',
  starting: '正在启动',
  running: '运行中',
  failed: '失败',
};

const AUDIT_EVENT_TEXT: Record<LanAuditEventKind, string> = {
  pair_succeeded: '配对成功',
  pair_rejected: '配对被拒绝',
  pair_rate_limited: '配对被限速',
  code_voided: '访问凭据作废',
  session_revoked: '会话被撤销',
  request_rejected: '请求被拒绝',
};

const AUDIT_REASON_TEXT: Record<string, string> = {
  invalid_code: '访问凭据无效或已使用',
  expired_code: '访问凭据已过期',
  expired: '已过期',
  session_limit: '达到会话上限',
  host: 'Host 不匹配',
  origin: 'Origin 不匹配',
  host_disconnect: '主机断开该设备',
  host_disconnect_all: '主机断开全部设备',
  switch_off: '开关已关闭',
  app_exit: '应用退出',
  service_failed: '服务故障',
  network_changed: '网络变化',
  port_changed: '端口变更',
  issuer_ended: '签发账户已结束',
  ip_changed: '设备 IP 变化',
  idle_expired: '闲置过期',
  absolute_expired: '达到最长使用时限',
  re_paired: '重新配对',
  logout: '设备登出',
};

const PORT_RANGE_TEXT = `端口须为 ${LAN_PORT_MIN}～${LAN_PORT_MAX} 的整数`;
const PORT_UNAVAILABLE_TEXT = '端口不可用：须为 1024～65535，且不是浏览器禁止的端口';
const NO_CODE_TEXT = '服务未运行或尚未登录，无法生成访问凭据';

const nowSeconds = () => Math.floor(Date.now() / 1000);
const formatTime = (seconds: number) => new Date(seconds * 1000).toLocaleString('zh-CN', { hour12: false });

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
  if (!/^\d+$/.test(text.trim())) return PORT_RANGE_TEXT;
  const port = Number(text);
  return port >= LAN_PORT_MIN && port <= LAN_PORT_MAX ? undefined : PORT_RANGE_TEXT;
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast.success('已复制');
  } catch {
    toast.error('无法复制，请手动选择文本');
  }
}

const cell = 'border-b border-input px-3 py-2 text-left align-top wrap-anywhere';
const head = `${cell} font-semibold text-muted-foreground`;

function AuditLog({ reportError }: { reportError: (error: unknown) => void }) {
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
          <h2 className="m-0 text-base font-semibold">审计记录</h2>
          <Button variant="secondary" size="sm" disabled={busy} onClick={() => void load()}>刷新</Button>
        </div>
      </Card.Header>
      <Card.Body>
        <Stack gap={3}>
          {loaded && items.length === 0 ? (
            <p className="m-0 text-sm text-muted-foreground">暂无审计记录</p>
          ) : null}
          {items.length > 0 ? (
            <div className="overflow-x-auto">
              <table className="w-full border-collapse text-sm" aria-label="审计记录">
                <thead>
                  <tr>
                    <th scope="col" className={head}>时间</th>
                    <th scope="col" className={head}>事件</th>
                    <th scope="col" className={head}>原因</th>
                    <th scope="col" className={head}>来源 IP</th>
                    <th scope="col" className={head}>会话</th>
                    <th scope="col" className={head}>错误码</th>
                    <th scope="col" className={head}>次数</th>
                  </tr>
                </thead>
                <tbody>
                  {items.map((item) => (
                    <tr key={item.id}>
                      <td className={cell}>{formatTime(item.last_at)}</td>
                      <td className={cell}>{AUDIT_EVENT_TEXT[item.event] ?? item.event}</td>
                      <td className={cell}>{item.reason ? (AUDIT_REASON_TEXT[item.reason] ?? item.reason) : '—'}</td>
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
              <Button variant="secondary" loading={busy} onClick={() => void load(cursor)}>加载更多</Button>
            </div>
          ) : null}
        </Stack>
      </Card.Body>
    </Card>
  );
}

export default function LanSettings() {
  const reportError = usePageError('LanSettings');
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
      title: '关闭局域网协同？',
      icon: 'warning',
      content: '关闭后会断开全部设备，并停止接受局域网连接。',
      confirmText: '确认关闭',
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
      if (error instanceof LanCommandError && error.code === LAN_PORT_UNAVAILABLE) setPortError(PORT_UNAVAILABLE_TEXT);
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
      title: '修改端口？',
      icon: 'warning',
      content: `服务正在运行，改用端口 ${port} 会断开全部设备并作废当前访问凭据。`,
      confirmText: '修改并断开',
      confirmVariant: 'danger',
      onConfirm: () => applyPort(port, true),
    });
  };

  const disconnectOne = (device: LanDevice) => {
    confirm({
      title: '断开设备？',
      icon: 'warning',
      content: `将断开“${device.label}”（${device.peer_ip}），该设备需要重新配对才能再次使用。`,
      confirmText: '断开',
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
      title: '断开全部设备？',
      icon: 'warning',
      content: `将断开当前 ${devices.length} 台设备，它们都需要重新配对。`,
      confirmText: '断开全部',
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
    <div className="mx-auto flex max-w-3xl flex-col gap-6 p-6 text-foreground">
      <h1 className="m-0 text-2xl font-semibold text-foreground">局域网协同</h1>
      <div role="note" className="rounded-lg border border-warning bg-warning-bg p-3 text-sm text-warning-text wrap-anywhere">
        仅在可信网络中使用：局域网连接未加密
      </div>

      {status === null ? (
        <Card>
          <Card.Body>
            {loadFailed ? (
              <Stack gap={3}>
                <p role="alert" className="m-0 text-sm text-error-text">无法读取局域网协同状态。</p>
                <div>
                  <Button variant="secondary" onClick={() => void refresh()}>重试</Button>
                </div>
              </Stack>
            ) : (
              <p role="status" className="m-0 text-sm text-muted-foreground">正在读取状态…</p>
            )}
          </Card.Body>
        </Card>
      ) : (
        <>
          <Card>
            <Card.Header>
              <h2 className="m-0 text-base font-semibold">服务</h2>
            </Card.Header>
            <Card.Body>
              <Stack gap={4}>
                <Switch label="启用局域网协同" checked={status.enabled} disabled={switching} onChange={toggle} />
                <dl className="m-0 flex flex-col gap-2 text-sm">
                  <div className="flex gap-4">
                    <dt className="w-28 shrink-0 text-muted-foreground">服务状态</dt>
                    <dd className="m-0">{STATE_TEXT[status.state]}</dd>
                  </div>
                  {status.failure_code !== null ? (
                    <div className="flex gap-4">
                      <dt className="w-28 shrink-0 text-muted-foreground">失败原因</dt>
                      <dd className="m-0 text-error-text">{LAN_FAILURE_TEXT[status.failure_code] ?? `服务启动失败（错误码 ${status.failure_code}）`}</dd>
                    </div>
                  ) : null}
                  {running && status.address ? (
                    <>
                      <div className="flex gap-4">
                        <dt className="w-28 shrink-0 text-muted-foreground">访问地址</dt>
                        <dd className="m-0 select-all break-all">{`http://${status.address}`}</dd>
                      </div>
                      <div className="flex gap-4">
                        <dt className="w-28 shrink-0 text-muted-foreground">当前端口</dt>
                        <dd className="m-0">{actualPort ?? '—'}</dd>
                      </div>
                    </>
                  ) : null}
                  <div className="flex gap-4">
                    <dt className="w-28 shrink-0 text-muted-foreground">首选端口</dt>
                    <dd className="m-0">{status.preferred_port}</dd>
                  </div>
                  <div className="flex gap-4">
                    <dt className="w-28 shrink-0 text-muted-foreground">已连接设备</dt>
                    <dd className="m-0">{status.session_count}</dd>
                  </div>
                </dl>
                {running && status.port_fallback && actualPort !== null ? (
                  <p role="status" className="m-0 text-sm text-warning-text">
                    {`首选端口 ${status.preferred_port} 被其他程序占用，当前使用 ${actualPort}`}
                  </p>
                ) : null}
              </Stack>
            </Card.Body>
          </Card>

          <Card>
            <Card.Header>
              <h2 className="m-0 text-base font-semibold">首选端口</h2>
            </Card.Header>
            <Card.Body>
              <form onSubmit={submitPort} noValidate>
                <Stack gap={3}>
                  <Input
                    label="首选端口"
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
                  <p className="m-0 text-xs text-muted-foreground">
                    {`范围 ${LAN_PORT_MIN}～${LAN_PORT_MAX}。首选端口被占用时，会依次尝试其后 19 个端口。`}
                  </p>
                  <div>
                    <Button type="submit" variant="primary" loading={portBusy}>保存</Button>
                  </div>
                </Stack>
              </form>
            </Card.Body>
          </Card>

          <Card>
            <Card.Header>
              <div className="flex items-center justify-between gap-3">
                <h2 className="m-0 text-base font-semibold">访问凭据</h2>
                <Button variant="secondary" size="sm" loading={issuing} disabled={!running} onClick={regenerate}>
                  重新生成
                </Button>
              </div>
            </Card.Header>
            <Card.Body>
              <Stack gap={3}>
                {!running ? (
                  <p className="m-0 text-sm text-muted-foreground">{NO_CODE_TEXT}</p>
                ) : codeUnavailable ? (
                  <p role="status" className="m-0 text-sm text-muted-foreground">{NO_CODE_TEXT}</p>
                ) : code === null ? (
                  <p role="status" className="m-0 text-sm text-muted-foreground">
                    {issuing ? '正在生成访问凭据…' : '尚未生成访问凭据，请点击“重新生成”。'}
                  </p>
                ) : expired ? (
                  <p role="status" className="m-0 text-sm text-warning-text">访问凭据已过期，请点击“重新生成”。</p>
                ) : (
                  <>
                    <div className="flex flex-wrap items-center gap-3">
                      <code data-testid="lan-code" className="select-all rounded-md bg-secondary px-3 py-2 font-mono text-lg tracking-wider">
                        {groupCode(code.code)}
                      </code>
                      <Button variant="secondary" size="sm" onClick={() => void copyText(groupCode(code.code))}>复制凭据</Button>
                    </div>
                    <p className="m-0 text-sm text-muted-foreground">{`有效期至 ${formatTime(code.expires_at)}，仅可使用一次。`}</p>
                    <div className="flex flex-wrap items-center gap-3">
                      <code data-testid="lan-pair-url" className="select-all break-all rounded-md bg-secondary px-3 py-2 font-mono text-sm">
                        {code.pair_url}
                      </code>
                      <Button variant="secondary" size="sm" onClick={() => void copyText(code.pair_url)}>复制链接</Button>
                    </div>
                  </>
                )}
              </Stack>
            </Card.Body>
          </Card>

          <Card>
            <Card.Header>
              <div className="flex items-center justify-between gap-3">
                <h2 className="m-0 text-base font-semibold">已连接设备</h2>
                <Button variant="danger" size="sm" disabled={devices.length === 0} onClick={disconnectAll}>
                  断开全部
                </Button>
              </div>
            </Card.Header>
            <Card.Body>
              {devices.length === 0 ? (
                <p className="m-0 text-sm text-muted-foreground">当前没有已连接的设备</p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full border-collapse text-sm" aria-label="已连接设备">
                    <thead>
                      <tr>
                        <th scope="col" className={head}>设备</th>
                        <th scope="col" className={head}>IP 地址</th>
                        <th scope="col" className={head}>短标识</th>
                        <th scope="col" className={head}>连接时间</th>
                        <th scope="col" className={head}>最近活动</th>
                        <th scope="col" className={head}>操作</th>
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
                              aria-label={`断开 ${device.label}`}
                              onClick={() => disconnectOne(device)}
                            >
                              断开
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
