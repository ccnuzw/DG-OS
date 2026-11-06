import { t, useLocale as usePageLocale } from '../i18n';
import { FormEvent, ReactNode, useEffect, useId, useRef, useState } from 'react';
import { Button, Card, Checkbox, Input, Modal, ResourceDetail, ResourceList, Select, Stack, Switch, cn } from '@dgos/ui';
import type { ConfirmOptions, ModalHandle } from '@dgos/ui';
import { usePageError } from '../pageError';
import { formatDateTime } from '../i18n';
import { useSurfaceCloseGuard, useSurfaceActive, useSurfaceStatus } from '../workspace/SurfaceContext';
import { useDevelopment } from '../stores/developmentStore';
import { beginResourceRead, failResourceRead, resourceView, type ResourceReadState } from '../resourceState';
import {
  useMcpStore, mcpErrorMessage, McpServer, McpStatus, McpTool, McpTransport,
  CreateMcpServerRequest, CallMcpToolRequest, CallMcpToolResponse,
} from '../stores/mcpStore';
import './McpManagement.css';

const emptyConfig = (): CreateMcpServerRequest => ({
  name: '', transport: 'stdio', stdio: { executable: '', args: [] },
  enabled: false, auto_connect: false,
});

const TRANSPORT_OPTIONS = [
  { value: 'stdio', label: 'mcp.stdio' },
  { value: 'http', label: 'mcp.http' },
];
const STATUS_DOT: Record<McpStatus, string> = {
  ready: 'bg-status-success', degraded: 'bg-status-warning', error: 'bg-status-error',
  connecting: 'bg-status-info', disconnecting: 'bg-status-info', configured: 'bg-ink-secondary',
};
const KICKER = 'block text-xs font-bold text-accent-default';
const HINT = 'm-0 text-xs text-ink-secondary wrap-anywhere';
const MUTED = 'm-0 text-sm text-ink-secondary wrap-anywhere';
const INLINE_ERROR = 'm-0 text-sm text-status-error-text wrap-anywhere';
const LOG_ROW = 'flex flex-wrap items-center gap-x-4 gap-y-1 rounded-md border border-border-subtle bg-surface-canvas px-3 py-2 text-xs';
const PRE = 'm-0 max-h-60 overflow-auto whitespace-pre-wrap wrap-anywhere rounded-md border border-border-subtle bg-surface-canvas p-3 font-mono text-xs text-ink-primary';
const SUMMARY = 'cursor-pointer py-1 text-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-border-focus';

// Opens at most one confirm dialog per call site and ignores clicks while it is open (the dialog is released on
// cancel or after a successful confirm; a rejected confirm leaves it open for a retry). Closed on unmount.
function useSingleConfirm() {
  const handle = useRef<ModalHandle | null>(null);
  useEffect(() => () => { handle.current?.close(); handle.current = null; }, []);
  return (options: ConfirmOptions) => {
    if (handle.current) return;
    handle.current = Modal.confirm({
      ...options,
      onConfirm: async () => { await options.onConfirm?.(); handle.current = null; },
      onCancel: () => { handle.current = null; options.onCancel?.(); },
    });
  };
}

function CardTitle({ title, aside }: { kicker?: string; title: ReactNode; aside?: ReactNode }) {
  usePageLocale();
  return <Card.Header className="flex flex-wrap items-start justify-between gap-3">
    <h2 className="m-0 min-w-0 text-sm font-semibold wrap-anywhere">{title}</h2>
    {aside}
  </Card.Header>;
}

function ServerForm({ editing, onSaved, onCancel }: {
  editing: McpServer | null; onSaved: (server: McpServer) => void; onCancel: () => void;
}) {
  usePageLocale();
  const { create, update, getConfig } = useMcpStore();
  const surfaceActive = useSurfaceActive();
  const accountEpoch = useDevelopment(state => state.epoch);
  const ownerEpoch = useRef(accountEpoch);
  const reportError = usePageError('McpManagement');
  const argvHint = useId();
  const urlHint = useId();
  const [config, setConfig] = useState(emptyConfig);
  const [argv, setArgv] = useState('[]');
  const [error, setError] = useState('');
  const [argvError, setArgvError] = useState('');
  const mounted = useRef(false);
  const [configRead, setConfigRead] = useState<ResourceReadState>(() => editing ? { status: 'idle' } : { status: 'ready', refresh: { status: 'current' } });
  const [configAttempt, setConfigAttempt] = useState(0);
  const configReady = configRead.status === 'ready';
  const loading = configRead.status === 'loading';
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const baseline = useRef(JSON.stringify({ config: emptyConfig(), argv: '[]' }));
  const discardFlight = useRef<Promise<boolean> | null>(null);
  const discardHandle = useRef<ModalHandle | null>(null);
  const discardResolve = useRef<((discard: boolean) => void) | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      discardHandle.current?.close();
      discardResolve.current?.(false);
    };
  }, []);
  const canClose = async () => {
    if (savingRef.current) return false;
    if (baseline.current === JSON.stringify({ config, argv })) return true;
    if (!discardFlight.current) {
      let resolve!: (discard: boolean) => void;
      const promise = new Promise<boolean>(settle => { resolve = settle; });
      discardResolve.current = resolve;
      discardFlight.current = promise.finally(() => { discardFlight.current = null; discardHandle.current = null; discardResolve.current = null; });
      discardHandle.current = Modal.confirm({
        title: t('shell.unsavedTitle'), content: t('shell.unsavedConfirm'),
        confirmText: t('shell.discard'), cancelText: t('common.cancel'), confirmVariant: 'danger',
        onConfirm: () => { resolve(true); }, onCancel: () => { resolve(false); },
      });
    }
    const discard = await discardFlight.current;
    return discard && !savingRef.current && mounted.current && ownerEpoch.current === useDevelopment.getState().epoch;
  };
  useSurfaceStatus({ dirty: baseline.current !== JSON.stringify({ config, argv }) });
  useSurfaceCloseGuard(canClose);
  const configFlight = useRef<object | null>(null);
  useEffect(() => {
    setError(''); setArgvError(''); setArgv('[]'); setConfig(emptyConfig());
    setConfigRead(editing ? { status: 'idle' } : { status: 'ready', refresh: { status: 'current' } });
    setConfigAttempt(0);
    configFlight.current = null;
    return () => { configFlight.current = null; };
  }, [editing?.id, accountEpoch]);
  useEffect(() => {
    if (!surfaceActive || ownerEpoch.current !== accountEpoch || !editing || configReady || configFlight.current) return;
    const flight = {};
    configFlight.current = flight;
    const current = () => configFlight.current === flight && useDevelopment.getState().epoch === accountEpoch;
    setConfigRead(state => beginResourceRead(state));
    getConfig(editing.id).then(value => {
      if (current()) { setConfig(value); setArgv(JSON.stringify(value.stdio?.args ?? [])); baseline.current = JSON.stringify({ config: value, argv: JSON.stringify(value.stdio?.args ?? []) }); setConfigRead({ status: 'ready', refresh: { status: 'current' } }); }
    }).catch(error => { if (current()) setConfigRead(state => failResourceRead(state, mcpErrorMessage(error))); })
      .finally(() => { if (current()) configFlight.current = null; });
  }, [surfaceActive, editing?.id, accountEpoch, configAttempt, getConfig]);

  async function save(event: FormEvent) {
    event.preventDefault();
    if (savingRef.current || loading || !configReady) return;
    setError(''); setArgvError('');
    let args: string[] = [];
    if (config.transport === 'stdio') {
      try {
        const parsed: unknown = JSON.parse(argv);
        if (!Array.isArray(parsed) || !parsed.every(arg => typeof arg === 'string')) throw new Error();
        args = parsed;
      } catch { setArgvError(t('mcp.jsonArrayError')); return; }
    }
    const request: CreateMcpServerRequest = { name: config.name, transport: config.transport };
    if (config.transport === 'stdio') request.stdio = {
      executable: config.stdio?.executable ?? '', args,
      ...(config.stdio?.cwd ? { cwd: config.stdio.cwd } : {}),
    };
    if (config.transport === 'http') request.http = config.http ?? { url: '' };
    if (config.enabled) request.enabled = true;
    if (config.auto_connect) request.auto_connect = true;
    // Explicit false is needed to turn off existing preferences when editing.
    if (editing) { request.enabled = !!config.enabled; request.auto_connect = !!config.auto_connect; }
    savingRef.current = true; setSaving(true);
    const current = () => mounted.current && ownerEpoch.current === useDevelopment.getState().epoch;
    try {
      const server = editing ? await update(editing.id, request) : await create(request);
      if (current()) { setConfig(emptyConfig()); setArgv('[]'); baseline.current = JSON.stringify({ config: emptyConfig(), argv: '[]' }); onSaved(server); }
    } catch (error) { if (current()) { setError(mcpErrorMessage(error)); reportError(error); } }
    finally { savingRef.current = false; if (current()) setSaving(false); }
  }
  const frozen = saving || loading || !configReady;
  return <Card className="min-w-0 flex-1 basis-72">
    <form onSubmit={save} aria-label={t('mcp.serverConfig')}>
      <CardTitle
        kicker={editing ? t('mcp.editConfig') : t('mcp.register')}
        title={editing ? t('mcp.editName', { name: editing.name }) : t('mcp.addServer')}
        aside={<span className="text-xs text-ink-secondary">{t('mcp.noSecrets')}</span>}
      />
      <Card.Body>
        <Stack gap={4}>
          {error && <p role="alert" className={INLINE_ERROR}>{error}</p>}
          <ResourceDetail state={resourceView(configRead, false)} loadingLabel={t('mcp.loadingConfig')} retry={<Button type="button" variant="secondary" size="sm" onClick={() => setConfigAttempt(attempt => attempt + 1)}>{t('common.retry')}</Button>}>
          <fieldset disabled={frozen} className="m-0 min-w-0 border-0 p-0">
            <Stack gap={4}>
              <Input label={t('detail.name')} required maxLength={100} value={config.name} disabled={frozen} onChange={e => setConfig({ ...config, name: e.target.value })} placeholder={t('detail.filesystem')} />
              <Select label={t('mcp.transport')} options={TRANSPORT_OPTIONS.map(option => ({ ...option, label: t(option.label) }))} value={config.transport} disabled={frozen} onChange={value => setConfig({ ...config, transport: value as McpTransport })} />
              {config.transport === 'stdio' ? <>
                <Input label={t('mcp.executable')} required value={config.stdio?.executable ?? ''} disabled={frozen} onChange={e => setConfig({ ...config, stdio: { args: [], ...config.stdio, executable: e.target.value } })} placeholder={t('mcp.executablePlaceholder')} />
                <Stack gap={1}>
                  <Input label={t('mcp.arguments')} aria-describedby={argvHint} error={argvError || undefined} value={argv} disabled={frozen} onChange={e => { setArgv(e.target.value); setArgvError(''); }} placeholder={"[\"--root\", \"/Users/me/My Documents\"]"} />
                  <p id={argvHint} className={HINT}>{t('mcp.argumentsHint')}</p>
                </Stack>
                <Input label={t('mcp.workingDirectory')} value={config.stdio?.cwd ?? ''} disabled={frozen} onChange={e => setConfig({ ...config, stdio: { executable: '', args: [], ...config.stdio, cwd: e.target.value } })} placeholder={t('detail.optionalDirectory')} />
              </> : <>
                <Stack gap={1}>
                  {/* InputType has no `url`: the host validates the URL when the server is saved. */}
                  <Input label={t('mcp.serverUrl')} aria-describedby={urlHint} required inputMode="url" value={config.http?.url ?? ''} disabled={frozen} onChange={e => setConfig({ ...config, http: { url: e.target.value } })} placeholder={t('mcp.urlPlaceholder')} />
                  <p id={urlHint} className={HINT}>{t('mcp.urlHint')}</p>
                </Stack>
                {config.http?.url.trim().toLowerCase().startsWith('http:') && <Checkbox className="items-start" checked={!!config.http?.allow_insecure_http} disabled={frozen} onChange={checked => setConfig({ ...config, http: { ...config.http!, allow_insecure_http: checked } })} label={t('mcp.insecureHttpConsent')} />}
              </>}
              <Checkbox checked={!!config.enabled} disabled={frozen} onChange={checked => setConfig({ ...config, enabled: checked })} label={t('mcp.enableServer')} />
              <Checkbox checked={!!config.auto_connect} disabled={frozen} onChange={checked => setConfig({ ...config, auto_connect: checked })} label={t('mcp.autoConnect')} />
              <p className={HINT}>{t('mcp.saveNoConnect')}</p>
            </Stack>
          </fieldset>
          </ResourceDetail>
        </Stack>
      </Card.Body>
      <Card.Footer className="flex-wrap justify-start">
        <Button type="submit" disabled={frozen} loading={saving}>{saving ? t('providerForm.saving') : editing ? t('mcp.saveChanges') : t('mcp.saveServer')}</Button>
        <Button type="button" variant="secondary" disabled={saving} onClick={() => {
          if (savingRef.current) return;
          if (baseline.current === JSON.stringify({ config, argv })) { onCancel(); return; }
          void canClose().then(discard => { if (discard && !savingRef.current && mounted.current && ownerEpoch.current === useDevelopment.getState().epoch) onCancel(); });
        }}>{t('common.cancel')}</Button>
      </Card.Footer>
    </form>
  </Card>;
}

function Credentials({ server, visible }: { server: McpServer; visible: boolean }) {
  usePageLocale();
  const { setToken, clearToken, pending } = useMcpStore();
  const reportError = usePageError('McpManagement');
  const confirm = useSingleConfirm();
  const [token, setValue] = useState('');
  const surfaceActive = useSurfaceActive();
  useEffect(() => { if (!surfaceActive || !visible) setValue(''); }, [surfaceActive, visible]);
  const [notice, setNotice] = useState('');
  const busy = pending.includes(server.id);
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!token || busy) return;
    const value = token; setValue(''); setNotice('');
    try { await setToken(server.id, value); setNotice(t('mcp.tokenStored')); }
    catch (error) { setNotice(mcpErrorMessage(error)); reportError(error); }
  }
  return <Card>
    <CardTitle kicker={t('mcp.keychain')} title={t('mcp.httpAuth')} aside={<span className="text-xs text-ink-secondary">{server.credential_ref ? t('mcp.tokenConfigured') : t('mcp.noToken')}</span>} />
    <Card.Body>
      <Stack gap={4}>
        <p className={MUTED}>{t('mcp.credentialPrivacy')}</p>
        <form onSubmit={save} className="flex flex-wrap items-end gap-3">
          <div className="min-w-0 flex-1 basis-48">
            <Input label={t('mcp.bearerToken')} type="password" maxLength={8192} value={token} onChange={e => setValue(e.target.value)} autoComplete="new-password" placeholder={server.credential_ref ? t('mcp.replacementToken') : t('mcp.enterTokenOnce')} />
          </div>
          <Button type="submit" disabled={busy || !token}>{t('mcp.storeToken')}</Button>
          {server.credential_ref && <Button type="button" variant="secondary" disabled={busy} onClick={() => confirm({
            title: t('mcp.removeTokenTitle'),
            content: t('mcp.removeTokenConfirm'),
            confirmText: t('providerForm.remove'), cancelText: t('module.cancel'), confirmVariant: 'danger',
            onConfirm: async () => { await clearToken(server.id).catch(reportError); },
          })}>{t('mcp.removeToken')}</Button>}
        </form>
        {notice && <p role="status" className={MUTED}>{notice}</p>}
      </Stack>
    </Card.Body>
  </Card>;
}

function ToolDialog({ tool, server, onClose }: { tool: McpTool; server: McpServer; onClose: () => void }) {
  usePageLocale();
  const { confirmTool, call, cancelCall, fetchLogs } = useMcpStore();
  const reportError = usePageError('McpManagement');
  const formId = useId();
  const active = useRef(false);
  const cancelRequested = useRef(false);
  const inFlight = useRef<CallMcpToolRequest | null>(null);
  const [argumentsText, setArgumentsText] = useState('{}');
  const [timeoutMs, setTimeoutMs] = useState(30000);
  const [review, setReview] = useState<CallMcpToolRequest | null>(null);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<CallMcpToolResponse | null>(null);
  const [error, setError] = useState('');
  const [cancelNotice, setCancelNotice] = useState('');
  const [cancelling, setCancelling] = useState(false);
  const cancelBusy = useRef(false);
  function reviewCall(event: FormEvent) {
    event.preventDefault(); setError(''); setResult(null);
    try {
      const args: unknown = JSON.parse(argumentsText);
      if (!args || typeof args !== 'object' || Array.isArray(args)) throw new Error();
      setReview({ server_id: server.id, tool_name: tool.tool_name, arguments: args, timeout_ms: timeoutMs, call_id: crypto.randomUUID() });
    } catch { setError(t('mcp.jsonObjectError')); }
  }
  async function execute() {
    if (!review || active.current) return;
    active.current = true; cancelRequested.current = false; inFlight.current = null;
    setRunning(true); setError(''); setResult(null); setCancelNotice('');
    try {
      const token = await confirmTool(review);
      if (cancelRequested.current) { setReview(null); setCancelNotice('Cancelled before tool dispatch. No tool request was sent.'); return; }
      // The immutable reviewed request is reused; fields cannot change during consent.
      inFlight.current = review;
      const response = await call({ ...review, confirmation_token: token });
      setResult(response); setReview(null); void fetchLogs(server.id);
    } catch (error) { setError(mcpErrorMessage(error)); setReview(null); reportError(error); }
    finally { inFlight.current = null; active.current = false; setRunning(false); }
  }
  async function cancel() {
    if (!active.current || cancelBusy.current) return;
    cancelRequested.current = true;
    if (!inFlight.current?.call_id) { setCancelNotice('Cancellation requested. Waiting for confirmation to finish; no tool will be sent.'); return; }
    cancelBusy.current = true; setCancelling(true);
    try {
      const accepted = await cancelCall(server.id, inFlight.current.call_id);
      setCancelNotice(accepted ? t('mcp.cancelMayHaveRun') : t('mcp.cancelNotActive'));
    } catch (error) { setCancelNotice(t('mcp.cancelFailure', { error: mcpErrorMessage(error) })); reportError(error); }
    finally { cancelBusy.current = false; setCancelling(false); }
  }
  const unavailable = !server.enabled || server.status !== 'ready';
  const resultTone = result?.is_error ? 'border-status-error bg-status-error-surface text-status-error-text' : 'border-status-success bg-status-success-surface text-status-success-text';
  return <Modal
    open
    onOpenChange={open => { if (!open && !running) onClose(); }}
    title={tool.title || tool.tool_name}
    description={tool.description || t('mcp.noDescription')}
    footer={review
      ? <div className="flex flex-wrap justify-end gap-2">
        <Button variant="secondary" disabled={running} onClick={() => setReview(null)}>{t('mcp.backToArguments')}</Button>
        {running && <Button variant="secondary" disabled={cancelling} onClick={() => void cancel()}>{cancelling ? t('mcp.requestingCancel') : t('mcp.cancelCall')}</Button>}
        <Button disabled={running || unavailable} onClick={() => void execute()}>{running ? t('mcp.calling') : t('mcp.confirmAndCall')}</Button>
      </div>
      : <Button type="submit" form={formId} disabled={unavailable}>{t('mcp.reviewLabel')}</Button>}
  >
    {/* First in DOM order, so it receives the initial focus when the inspector opens. */}
    <div className="flex flex-wrap items-center justify-between gap-2">
      <span className={cn(KICKER, 'min-w-0 wrap-anywhere')}>{server.name} {t('detail.inspector')}</span>
      <Button variant="secondary" size="sm" disabled={running} onClick={onClose}>{t('mcp.close')}</Button>
    </div>
    <p className={HINT}>{t('mcp.untrustedDescription')}</p>
    <details><summary className={SUMMARY}>{t('mcp.inputSchema')}</summary><pre className={PRE}>{JSON.stringify(tool.input_schema, null, 2)}</pre></details>
    {tool.output_schema !== undefined && <details><summary className={SUMMARY}>{t('mcp.outputSchema')}</summary><pre className={PRE}>{JSON.stringify(tool.output_schema, null, 2)}</pre></details>}
    <p className={HINT}>{t('workbench.schema')}{tool.schema_hash?.slice(0, 12)} {t('detail.discovered')}{tool.discovered_at ? formatDateTime(tool.discovered_at * 1000) : t('mcp.recently')}</p>
    {error && <p role="alert" className={INLINE_ERROR}>{error}</p>}
    {cancelNotice && <p role="status" className={HINT}>{cancelNotice}</p>}
    {review ? <section aria-label={t('mcp.reviewLabel')} aria-busy={running} className="flex flex-col gap-3 rounded-lg border border-status-warning bg-status-warning-surface p-4 text-status-warning-text">
      <h3 className="m-0 text-base font-semibold">{t('mcp.confirmExternal')}</h3>
      <p className="m-0 text-sm wrap-anywhere">{t('detail.server')}<strong>{server.name}</strong> {t('detail.tool')}<code className="font-mono">{tool.tool_name}</code></p>
      <pre className={PRE}>{JSON.stringify(review.arguments, null, 2)}</pre>
      <p className="m-0 text-sm">{t('mcp.externalWarning')}</p>
      {running && <p role="status" className="m-0 text-sm">{t('mcp.waitingTool')}</p>}
    </section> : <form id={formId} onSubmit={reviewCall}>
      <Stack gap={4}>
        <Input multiline label={t('mcp.argumentsJson')} className="type-code" maxLength={65536} value={argumentsText} onChange={e => setArgumentsText(e.target.value)} rows={7} spellCheck={false} />
        <Input label={t('mcp.timeoutMs')} type="number" min={100} max={120000} step={100} required value={timeoutMs} onChange={e => setTimeoutMs(Number(e.target.value))} />
      </Stack>
    </form>}
    {result && <section aria-label={t('detail.toolResult')} className={cn('flex flex-col gap-2 rounded-lg border p-4', resultTone)}>
      <h3 className="m-0 text-base font-semibold">{result.outcome === 'cancelled' ? t('mcp.stoppedWaiting') : result.outcome === 'timeout' ? t('mcp.toolTimedOut') : result.is_error ? t('mcp.toolError') : t('mcp.toolCompleted')}</h3>
      {(result.outcome === 'cancelled' || result.outcome === 'timeout') && <p role="alert" className="m-0 text-sm">{t('mcp.externalMayHaveRun')}</p>}
      <p className="m-0 text-sm">{result.duration_ms} {t('detail.usageRecord')}{result.usage_log_id}</p>
      {result.audit_warning && <p role="alert" className="m-0 text-sm">{t('mcp.auditWarning')}</p>}
      <pre className={PRE}>{JSON.stringify(result.content, null, 2)}</pre>
      {result.structured_content != null && <details><summary className={SUMMARY}>{t('mcp.structuredResult')}</summary><pre className={PRE}>{JSON.stringify(result.structured_content, null, 2)}</pre></details>}
    </section>}
  </Modal>;
}

function LogRows({ rows }: { rows: { id: number; outcome: string; tone?: string; label: ReactNode; notes: ReactNode[]; duration?: number; at: number }[] }) {
  usePageLocale();
  return <div className="mcp-log-list flex flex-col gap-2" tabIndex={0}>{rows.map(row => <div className={LOG_ROW} key={row.id}>
    <span className={cn('min-w-16 text-xs font-bold uppercase', row.tone ?? 'text-ink-secondary')}>{row.outcome}</span>
    <span className="min-w-0 flex-1 basis-32 font-semibold wrap-anywhere">{row.label}{row.notes}</span>
    <span>{row.duration ?? '-'} {t('mcp.durationUnit')}</span>
        <time className="text-ink-secondary">{formatDateTime(row.at * 1000)}</time>
  </div>)}</div>;
}

export default function McpManagement() {
  usePageLocale();
  const surfaceActive = useSurfaceActive();
  const accountEpoch = useDevelopment(state => state.epoch);
  const { servers, tools, logsByServer, connectionsByServer, reads, discoveryByServer, markReadsStale, pending, error, fetch, toggle, remove, connect, disconnect, loadTools, discoverTools, fetchLogs, fetchConnections } = useMcpStore();
  const serversRead: ResourceReadState = reads.servers ?? { status: 'idle' };
  const loading = serversRead.status === 'loading' || serversRead.status === 'ready' && serversRead.refresh.status === 'refreshing';
  const reportError = usePageError('McpManagement');
  const confirm = useSingleConfirm();
  const [selected, setSelected] = useState<string | null>(null);
  const [editing, setEditing] = useState<McpServer | null>(null);
  const [tool, setTool] = useState<McpTool | null>(null);
  const [notice, setNotice] = useState('');
  const [formOpen, setFormOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [statusFilter, setStatusFilter] = useState('all');
  const [compactView, setCompactView] = useState<'list' | 'detail'>('list');
  const [containerNarrow, setContainerNarrow] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = contentRef.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(([entry]) => setContainerNarrow(entry.contentRect.width < Math.max(720, parseFloat(getComputedStyle(document.documentElement).fontSize) * 48)));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const previousLayout = useRef({ containerNarrow, compactView });
  useEffect(() => {
    const previous = previousLayout.current;
    previousLayout.current = { containerNarrow, compactView };
    if (!surfaceActive || !containerNarrow) return;
    if (!previous.containerNarrow && (formOpen || contentRef.current?.querySelector('.mcp-server-detail')?.contains(document.activeElement))) {
      setCompactView('detail'); return;
    }
    if (previous.compactView !== compactView) contentRef.current?.querySelector<HTMLElement>(compactView === 'list' ? '.mcp-server-list input' : '.mcp-server-detail')?.focus();
  }, [containerNarrow, compactView, surfaceActive, formOpen]);
  const filteredServers = servers.filter(server => server.name.toLocaleLowerCase().includes(query.toLocaleLowerCase()) && (statusFilter === 'all' || (statusFilter === 'ready' ? server.status === 'ready' || server.status === 'degraded' : server.status !== 'ready' && server.status !== 'degraded')));
  const selectedServer = servers.find(server => server.id === selected);
  const discovery = selectedServer ? discoveryByServer[selectedServer.id] : undefined;
  const toolServer = tool ? servers.find(server => server.id === tool.server_id) : null;
  const live = useRef({ surfaceActive, selected, accountEpoch });
  live.current = { surfaceActive, selected, accountEpoch };
  useEffect(() => {
    live.current.surfaceActive = surfaceActive;
    return () => { live.current.surfaceActive = false; };
  }, [surfaceActive]);
  const wasActive = useRef(false);
  useEffect(() => {
    live.current.selected = null;
    setSelected(null); setEditing(null); setTool(null); setFormOpen(false); setNotice('');
  }, [accountEpoch]);
  useEffect(() => {
    const resumed = surfaceActive && wasActive.current;
    if (!surfaceActive) { wasActive.current = true; markReadsStale(); return; }
    const current = () => live.current.surfaceActive && live.current.accountEpoch === accountEpoch;
    void fetch(resumed ? current : undefined);
  }, [surfaceActive, accountEpoch, fetch, markReadsStale]);
  useEffect(() => {
    if (!surfaceActive || !selected || live.current.selected !== selected) return;
    const current = () => live.current.surfaceActive && live.current.selected === selected && live.current.accountEpoch === accountEpoch;
    void loadTools(selected, current); void fetchLogs(selected, current); void fetchConnections(selected, current);
  }, [surfaceActive, accountEpoch, selected, loadTools, fetchLogs, fetchConnections]);
  useEffect(() => {
    if (editing && !servers.some(server => server.id === editing.id)) { setEditing(null); setFormOpen(false); }
  }, [servers, editing]);
  // Only presentation reads pause. Tool execution and the shell recovery notice keep their lifetimes.
  useEffect(() => {
    if (!surfaceActive) return;
    const id = window.setInterval(() => {
      if (!useMcpStore.getState().pending.length) { void fetch(); if (selected) void fetchConnections(selected); }
    }, 10000);
    return () => window.clearInterval(id);
  }, [surfaceActive, fetch, selected, fetchConnections]);
  const showList = !containerNarrow || compactView === 'list';
  const showDetail = !containerNarrow || compactView === 'detail';
  const deletingServer = async (server: McpServer) => {
    await remove(server.id).then(() => {
      if (selected === server.id) setSelected(null);
      if (editing?.id === server.id) { setEditing(null); setFormOpen(false); }
    }).catch(reportError);
  };
  return <main className="mcp-page">
    <div className="mcp-content" ref={contentRef}>
      <header className="mcp-header">
        <div className="min-w-0"><h1 className="m-0 text-lg font-semibold">{t('mcp.title')}</h1><p className={MUTED}>{t('mcp.subtitle')}</p></div>
        <div className="mcp-header-actions">
          {formOpen && !showDetail && <Button size="xs" onClick={() => setCompactView('detail')}>{t('mcp.resumeEditing')}</Button>}
          <Button size="xs" disabled={formOpen} onClick={() => { setEditing(null); setFormOpen(true); if (containerNarrow) setCompactView('detail'); }}>{t('mcp.addServer')}</Button>
          <Button variant="secondary" size="xs" disabled={loading} onClick={() => void fetch()}>{t('mcp.refreshStatus')}</Button>
        </div>
      </header>
      {error && <div role="alert" className="rounded-lg border border-status-error bg-status-error-surface px-4 py-3 text-sm text-status-error-text wrap-anywhere">{error}</div>}
      {notice && <div role="status" className="rounded-lg border border-status-success bg-status-success-surface px-4 py-3 text-sm text-status-success-text wrap-anywhere">{notice}</div>}
      {!showList && (serversRead.status !== 'ready' || serversRead.refresh.status !== 'current') && <ResourceDetail state={resourceView(serversRead, false)} loadingLabel={t('mcp.loadingServers')} retry={<Button variant="secondary" size="sm" onClick={() => void fetch()}>{t('common.retry')}</Button>} />}
      <div className={cn('mcp-columns', containerNarrow && 'is-compact')}>
        <section className="mcp-server-list" hidden={!showList} aria-label={t('mcp.yourServers')}>
          <header className="mcp-list-heading"><h2 className="m-0 text-sm font-semibold">{t('mcp.yourServers')}</h2><span className="mcp-server-count">{filteredServers.length}</span></header>
          <Input label={t('mcp.searchServers')} value={query} onChange={event => setQuery(event.target.value)} />
          <Select label={t('mcp.filterServers')} value={statusFilter} onChange={setStatusFilter} options={['all', 'ready', 'offline'].map(value => ({ value, label: t(`mcp.filter.${value}`) }))} />
          <ResourceList state={resourceView(serversRead, !filteredServers.length)} loadingLabel={t('mcp.loadingServers')}
            emptyContent={!servers.length ? <div className="rounded-lg border border-dashed border-border-subtle px-4 py-8 text-center"><strong className="text-sm">{t('mcp.noServers')}</strong><p className={MUTED}>{t('mcp.addServerHelp')}</p></div> : <p className={MUTED}>{t('mcp.noMatches')}</p>}
            retry={<Button variant="secondary" size="sm" onClick={() => void fetch()}>{t('common.retry')}</Button>}>
            <Stack gap={3}>
              {filteredServers.map(server => {
                const busy = pending.includes(server.id);
                const connected = server.status === 'ready' || server.status === 'degraded';
                return <article className={cn('mcp-server-row', selected === server.id && 'is-selected')} key={server.id}>
                  <Button variant="ghost" className="mcp-server-select h-auto min-w-0 items-start justify-start gap-3 whitespace-normal px-2 py-2 text-left" aria-label={t('mcp.selectServer', { name: server.name })} onClick={() => { setSelected(server.id); setCompactView('detail'); }} aria-pressed={selected === server.id}>
                    <span aria-hidden="true" className={cn('mt-1.5 size-2.5 shrink-0 rounded-full', STATUS_DOT[server.status])} />
                    <span className="flex min-w-0 flex-col gap-1">
                      <span className="text-sm font-semibold wrap-anywhere">{server.name}</span>
                      <span className="text-xs text-ink-secondary wrap-anywhere">{server.transport} · {t(`mcp.status.${server.status}`)}{server.auto_connect ? ` · ${t('mcp.autoConnect')}` : ''}</span>
                      {server.last_error_class && <span className="text-xs text-status-error-text wrap-anywhere">{server.last_error_class}</span>}
                    </span>
                  </Button>
                  <div className="mcp-row-actions">
                    <Button variant="secondary" size="sm" disabled={!server.enabled || busy} onClick={() => (connected ? disconnect(server.id) : connect(server.id)).catch(reportError)}>{busy ? t('mcp.working') : connected ? t('mcp.disconnect') : t('remote.connect')}</Button>
                    <Switch aria-label={`${t('mcp.enable')} ${server.name}`} disabled={busy} checked={server.enabled} onChange={checked => toggle(server.id, checked).catch(reportError)} />
                    {selected !== server.id && <Button variant="secondary" size="sm" aria-label={t('mcp.editName', { name: server.name })} disabled={busy || formOpen} onClick={() => { setSelected(server.id); setEditing(server); setFormOpen(true); setCompactView('detail'); }}>{t('mcp.edit')}</Button>}
                  </div>
                </article>;
              })}
            </Stack>
          </ResourceList>
        </section>
        <section className="mcp-server-detail" hidden={!showDetail} tabIndex={-1} aria-label={t('mcp.title')}>
          {containerNarrow && <Button variant="secondary" size="sm" className="mcp-back-list" onClick={() => setCompactView('list')}>{t('mcp.yourServers')}{selectedServer ? ` · ${selectedServer.name}` : ''}</Button>}
          {formOpen && <ServerForm key={editing?.id ?? 'new'} editing={editing} onCancel={() => { setFormOpen(false); setEditing(null); }} onSaved={server => { setFormOpen(false); setEditing(null); if (live.current.selected === selected) setSelected(server.id); setNotice(t('mcp.saved')); }} />}
          {selectedServer ? <>
            <header className="mcp-detail-heading">
              <div className="mcp-detail-title"><span aria-hidden="true" className={cn('size-2.5 shrink-0 rounded-full', STATUS_DOT[selectedServer.status])} /><div className="min-w-0"><h2 className="m-0 text-base font-semibold wrap-anywhere">{selectedServer.name}</h2><p className={HINT}>{selectedServer.transport} · {t(`mcp.status.${selectedServer.status}`)}{selectedServer.auto_connect ? ` · ${t('mcp.autoConnect')}` : ''}</p></div></div>
              <div className="mcp-detail-actions">
                <Button variant="secondary" size="sm" aria-label={t('mcp.editName', { name: selectedServer.name })} disabled={pending.includes(selectedServer.id) || formOpen} onClick={() => { setEditing(selectedServer); setFormOpen(true); }}>{t('mcp.edit')}</Button>
                <Button variant="danger" size="sm" disabled={pending.includes(selectedServer.id)} onClick={() => confirm({ title: t('mcp.deleteServer'), content: t('mcp.deleteServerConfirm', { name: selectedServer.name }), confirmText: t('providerForm.delete'), cancelText: t('module.cancel'), confirmVariant: 'danger', onConfirm: () => deletingServer(selectedServer) })}>{t('providerForm.delete')}</Button>
              </div>
            </header>
          {selectedServer.transport === 'http' && <Credentials key={selectedServer.id} server={selectedServer} visible={showDetail} />}
        <Card>
          <CardTitle kicker={t('mcp.discovered')} title={t('mcp.serverTools', { name: selectedServer.name })} aside={<div className="flex flex-wrap items-center gap-3"><span className="text-xs text-ink-secondary">{t('mcp.noPermissionFromDescriptions')}</span><Button variant="secondary" size="sm" disabled={pending.includes(selectedServer.id) || discovery?.status === 'unconfirmed' || selectedServer.status !== 'ready'} onClick={() => { void discoverTools(selectedServer.id).catch(() => {}); }}>{discovery?.status === 'pending' ? t('mcp.refreshingTools') : t('mcp.refreshTools')}</Button></div>} />
          <Card.Body>
            {discovery && <div role={discovery.status === 'failed' ? 'alert' : 'status'} aria-label={t('mcp.discoveryFeedback')} className={cn('mb-3 flex flex-col gap-2 rounded-md border p-3 text-sm wrap-anywhere', discovery.status === 'failed' ? 'border-status-error bg-status-error-surface text-status-error-text' : discovery.status === 'succeeded' ? 'border-status-success bg-status-success-surface text-status-success-text' : discovery.status === 'unconfirmed' ? 'border-status-warning bg-status-warning-surface text-status-warning-text' : 'border-border-subtle text-ink-secondary')}>
              <p className="m-0">{t(`mcp.discovery${discovery.status === 'pending' ? 'Pending' : discovery.status === 'succeeded' ? 'Succeeded' : discovery.status === 'failed' ? 'Failed' : 'Unconfirmed'}`, { name: selectedServer.name })}</p>
              {discovery.status === 'failed' && <p className="m-0">{discovery.error}</p>}
              {discovery.status === 'unconfirmed' && <Button variant="secondary" size="sm" onClick={() => void loadTools(selectedServer.id)}>{t('mcp.readToolSnapshot')}</Button>}
            </div>}
            <ResourceList state={resourceView(reads[`tools:${selectedServer.id}`] ?? { status: 'idle' }, !tools.some(item => item.server_id === selected))} loadingLabel={t('mcp.loadingTools')}
              emptyContent={<p className={MUTED}>{t('mcp.connectToDiscover')}</p>} retry={<Button variant="secondary" size="sm" onClick={() => void loadTools(selectedServer.id)}>{t('common.retry')}</Button>}>
              <div className="flex flex-col gap-2">{tools.filter(item => item.server_id === selected).map(item => <article className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border border-border-subtle bg-surface-canvas p-3" key={item.tool_name}>
                <div className="flex min-w-0 flex-1 basis-48 flex-col gap-1"><strong className="text-sm wrap-anywhere">{item.title || item.tool_name}</strong><span className="text-sm text-ink-secondary wrap-anywhere">{item.description || t('mcp.noDescription')}</span></div>
                <code className="min-w-0 font-mono text-xs text-accent-default wrap-anywhere">{item.tool_name}</code>
                <Button variant="secondary" size="sm" onClick={() => setTool(item)}>{t('mcp.inspectCall')}</Button>
              </article>)}</div>
            </ResourceList>
          </Card.Body>
        </Card>
        <Card>
          <CardTitle kicker={t('mcp.auditTrail')} title={t('mcp.recentUse')} aside={<Button variant="secondary" size="sm" onClick={() => void fetchLogs(selectedServer.id)}>{t('mcp.refreshUsage')}</Button>} />
          <Card.Body>
            <Stack gap={3}>
              <p className={HINT}>{t('mcp.auditPrivacy')}</p>
              <ResourceList state={resourceView(reads[`usage:${selectedServer.id}`] ?? { status: 'idle' }, !logsByServer[selectedServer.id]?.length)} loadingLabel={t('mcp.loadingUsage')}
                emptyContent={<p className={MUTED}>{t('mcp.noCalls')}</p>} retry={<Button variant="secondary" size="sm" onClick={() => void fetchLogs(selectedServer.id)}>{t('common.retry')}</Button>}>
                <LogRows rows={(logsByServer[selectedServer.id] ?? []).map(log => ({
                  id: log.id, outcome: log.outcome, tone: log.outcome === 'success' ? 'text-status-success-text' : log.outcome === 'error' ? 'text-status-error-text' : undefined, label: log.tool_name,
                  notes: log.error_class ? [<small key="error" className="block text-xs font-normal text-status-error-text">{log.error_class}</small>] : [],
                  duration: log.duration_ms, at: log.created_at,
                }))} />
              </ResourceList>
            </Stack>
          </Card.Body>
        </Card>
        <details className="mcp-diagnostics" open={(() => { const read = reads[`connections:${selectedServer.id}`]; if (!read) return false; if (read.status === 'error') return true; return read.status === 'ready' && read.refresh.status === 'error'; })()}>
          <summary className={SUMMARY}>{t('mcp.connectionHistory')}</summary>
          <Card role="region" aria-label={t('mcp.connectionHistory')}>
          <CardTitle title={t('mcp.connectionEvents')} aside={<Button variant="secondary" size="sm" onClick={() => void fetchConnections(selectedServer.id)}>{t('mcp.refreshConnections')}</Button>} />
          <Card.Body>
            <Stack gap={3}>
              <p className={HINT}>{t('mcp.connectionPrivacy')}</p>
              <ResourceList state={resourceView(reads[`connections:${selectedServer.id}`] ?? { status: 'idle' }, !connectionsByServer[selectedServer.id]?.length)} loadingLabel={t('mcp.loadingConnections')}
                emptyContent={<p className={MUTED}>{t('mcp.noConnectionEvents')}</p>} retry={<Button variant="secondary" size="sm" onClick={() => void fetchConnections(selectedServer.id)}>{t('common.retry')}</Button>}>
                <LogRows rows={(connectionsByServer[selectedServer.id] ?? []).map(log => ({
                  id: log.id, outcome: log.status, label: log.event,
                  notes: [
                    ...(log.error_class ? [<small key="error" className="block text-xs font-normal text-status-error-text">{log.error_class}</small>] : []),
                    ...(log.reconnect_attempt != null ? [<small key="retry" className="block text-xs font-normal text-status-error-text">{t('remote.retry')}{log.reconnect_attempt} {t('mcp.retryCount', { count: 3 })}</small>] : []),
                  ],
                  duration: log.duration_ms, at: log.created_at,
                }))} />
              </ResourceList>
            </Stack>
          </Card.Body>
          </Card>
        </details>
          </> : !formOpen && <div className="mcp-no-selection"><p className={MUTED}>{t('mcp.selectToView')}</p></div>}
        </section>
      </div>
      {tool && toolServer && <ToolDialog tool={tool} server={toolServer} onClose={() => setTool(null)} />}
    </div>
  </main>;
}
