import { FormEvent, ReactNode, useEffect, useId, useRef, useState } from 'react';
import { Button, Card, Checkbox, Input, Loading, Modal, Select, Stack, Switch, cn } from '@dgos/ui';
import type { ConfirmOptions, ModalHandle } from '@dgos/ui';
import { usePageError } from '../pageError';
import {
  useMcpStore, mcpErrorMessage, McpServer, McpStatus, McpTool, McpTransport,
  CreateMcpServerRequest, CallMcpToolRequest, CallMcpToolResponse,
} from '../stores/mcpStore';

const emptyConfig = (): CreateMcpServerRequest => ({
  name: '', transport: 'stdio', stdio: { executable: '', args: [] },
  enabled: false, auto_connect: false,
});

const TRANSPORT_OPTIONS = [
  { value: 'stdio', label: 'stdio process' },
  { value: 'http', label: 'HTTP endpoint' },
];
const STATUS_DOT: Record<McpStatus, string> = {
  ready: 'bg-success', degraded: 'bg-warning', error: 'bg-error',
  connecting: 'bg-info', disconnecting: 'bg-info', configured: 'bg-muted-foreground',
};
const KICKER = 'block text-xs font-bold text-primary';
const HINT = 'm-0 text-xs text-muted-foreground wrap-anywhere';
const MUTED = 'm-0 text-sm text-muted-foreground wrap-anywhere';
const INLINE_ERROR = 'm-0 text-sm text-error-text wrap-anywhere';
const LOG_ROW = 'flex flex-wrap items-center gap-x-4 gap-y-1 rounded-md border border-border bg-background px-3 py-2 text-xs';
const PRE = 'm-0 max-h-60 overflow-auto whitespace-pre-wrap wrap-anywhere rounded-md border border-border bg-background p-3 font-mono text-xs text-foreground';
const SUMMARY = 'cursor-pointer py-1 text-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus-ring';

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

function CardTitle({ kicker, title, aside }: { kicker: string; title: ReactNode; aside?: ReactNode }) {
  return <Card.Header className="flex flex-wrap items-start justify-between gap-3">
    <div className="min-w-0"><span className={KICKER}>{kicker}</span><h2 className="m-0 text-lg font-semibold wrap-anywhere">{title}</h2></div>
    {aside}
  </Card.Header>;
}

function ServerForm({ editing, onSaved, onCancel }: {
  editing: McpServer | null; onSaved: (server: McpServer) => void; onCancel: () => void;
}) {
  const { create, update, getConfig } = useMcpStore();
  const reportError = usePageError('McpManagement');
  // The load effect must not restart when the router hands out a new `navigate`, so it reads the latest reporter here.
  const report = useRef(reportError);
  useEffect(() => { report.current = reportError; });
  const argvHint = useId();
  const urlHint = useId();
  const [config, setConfig] = useState(emptyConfig);
  const [argv, setArgv] = useState('[]');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [configReady, setConfigReady] = useState(!editing);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  useEffect(() => {
    let current = true;
    setError(''); setArgv('[]'); setConfig(emptyConfig()); setLoading(false); setConfigReady(!editing);
    if (!editing) return;
    setLoading(true);
    getConfig(editing.id).then(value => {
      if (current) { setConfig(value); setArgv(JSON.stringify(value.stdio?.args ?? [])); setConfigReady(true); }
    }).catch(error => { if (current) { setError(mcpErrorMessage(error)); report.current(error); } })
      .finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [editing?.id, getConfig]);

  async function save(event: FormEvent) {
    event.preventDefault();
    if (savingRef.current || loading || !configReady) return;
    setError('');
    let args: string[] = [];
    if (config.transport === 'stdio') {
      try {
        const parsed: unknown = JSON.parse(argv);
        if (!Array.isArray(parsed) || !parsed.every(arg => typeof arg === 'string')) throw new Error();
        args = parsed;
      } catch { setError('Arguments must be a JSON array of strings.'); return; }
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
    try {
      const server = editing ? await update(editing.id, request) : await create(request);
      setConfig(emptyConfig()); setArgv('[]'); onSaved(server);
    } catch (error) { setError(mcpErrorMessage(error)); reportError(error); }
    finally { savingRef.current = false; setSaving(false); }
  }
  const frozen = saving || loading || !configReady;
  return <Card className="min-w-0 flex-1 basis-72">
    <form onSubmit={save} aria-label="Server configuration">
      <CardTitle
        kicker={editing ? 'EDIT CONFIGURATION' : 'REGISTER'}
        title={editing ? `Edit ${editing.name}` : 'Add MCP server'}
        aside={<span className="text-xs text-muted-foreground">No secrets in server configs</span>}
      />
      <Card.Body>
        <Stack gap={4}>
          {error && <p role="alert" className={INLINE_ERROR}>{error}</p>}
          {loading && <p role="status" className={MUTED}>Loading configuration...</p>}
          <fieldset disabled={frozen} className="m-0 min-w-0 border-0 p-0">
            <Stack gap={4}>
              <Input label="Name" required maxLength={100} value={config.name} disabled={frozen} onChange={e => setConfig({ ...config, name: e.target.value })} placeholder="Local filesystem" />
              <Select label="Transport" options={TRANSPORT_OPTIONS} value={config.transport} disabled={frozen} onChange={value => setConfig({ ...config, transport: value as McpTransport })} />
              {config.transport === 'stdio' ? <>
                <Input label="Executable" required value={config.stdio?.executable ?? ''} disabled={frozen} onChange={e => setConfig({ ...config, stdio: { args: [], ...config.stdio, executable: e.target.value } })} placeholder="/usr/local/bin/mcp-server" />
                <Stack gap={1}>
                  <Input label="Arguments" aria-describedby={argvHint} value={argv} disabled={frozen} onChange={e => setArgv(e.target.value)} placeholder='["--root", "/Users/me/My Documents"]' />
                  <p id={argvHint} className={HINT}>JSON array; paths with spaces remain one argument. Shell syntax is not evaluated.</p>
                </Stack>
                <Input label="Working directory" value={config.stdio?.cwd ?? ''} disabled={frozen} onChange={e => setConfig({ ...config, stdio: { executable: '', args: [], ...config.stdio, cwd: e.target.value } })} placeholder="Optional absolute directory" />
              </> : <>
                <Stack gap={1}>
                  {/* InputType has no `url`: the host validates the URL when the server is saved. */}
                  <Input label="Server URL" aria-describedby={urlHint} required inputMode="url" value={config.http?.url ?? ''} disabled={frozen} onChange={e => setConfig({ ...config, http: { url: e.target.value } })} placeholder="https://mcp.example.com" />
                  <p id={urlHint} className={HINT}>Prefer HTTPS or exact localhost. Changing the URL removes its previous credential binding and plaintext consent.</p>
                </Stack>
                {config.http?.url.trim().toLowerCase().startsWith('http:') && <Checkbox className="items-start" checked={!!config.http?.allow_insecure_http} disabled={frozen} onChange={checked => setConfig({ ...config, http: { ...config.http!, allow_insecure_http: checked } })} label="I accept remote plaintext HTTP risks: tool arguments, results and authentication tokens can be read or changed in transit. Not needed for exact localhost." />}
              </>}
              <Checkbox checked={!!config.enabled} disabled={frozen} onChange={checked => setConfig({ ...config, enabled: checked })} label="Enable server" />
              <Checkbox checked={!!config.auto_connect} disabled={frozen} onChange={checked => setConfig({ ...config, auto_connect: checked })} label="Connect automatically on app startup" />
              <p className={HINT}>Saving never launches a process or connects immediately. Auto-connect is off by default.</p>
            </Stack>
          </fieldset>
        </Stack>
      </Card.Body>
      <Card.Footer className="flex-wrap justify-start">
        <Button type="submit" disabled={frozen} loading={saving}>{saving ? 'Saving...' : editing ? 'Save changes' : 'Save server'}</Button>
        {editing && <Button type="button" variant="secondary" disabled={saving} onClick={onCancel}>Cancel editing</Button>}
      </Card.Footer>
    </form>
  </Card>;
}

function Credentials({ server }: { server: McpServer }) {
  const { setToken, clearToken, pending } = useMcpStore();
  const reportError = usePageError('McpManagement');
  const confirm = useSingleConfirm();
  const [token, setValue] = useState('');
  const [notice, setNotice] = useState('');
  const busy = pending.includes(server.id);
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!token || busy) return;
    const value = token; setValue(''); setNotice('');
    try { await setToken(server.id, value); setNotice('Token stored in the OS keychain. Reconnect to use it.'); }
    catch (error) { setNotice(mcpErrorMessage(error)); reportError(error); }
  }
  return <Card>
    <CardTitle kicker="HOST KEYCHAIN" title="HTTP authentication" aside={<span className="text-xs text-muted-foreground">{server.credential_ref ? 'Token configured' : 'No token'}</span>} />
    <Card.Body>
      <Stack gap={4}>
        <p className={MUTED}>Only an opaque reference is stored in DGOS. Tokens are never returned to this page. Rotation or removal disconnects the server.</p>
        <form onSubmit={save} className="flex flex-wrap items-end gap-3">
          <div className="min-w-0 flex-1 basis-48">
            <Input label="Bearer token" type="password" maxLength={8192} value={token} onChange={e => setValue(e.target.value)} autoComplete="new-password" placeholder={server.credential_ref ? 'Enter replacement token' : 'Enter token once'} />
          </div>
          <Button type="submit" disabled={busy || !token}>Store token</Button>
          {server.credential_ref && <Button type="button" variant="secondary" disabled={busy} onClick={() => confirm({
            title: 'Remove token?',
            content: 'Remove this server token and disconnect?',
            confirmText: 'Remove', cancelText: 'Cancel', confirmVariant: 'danger',
            onConfirm: async () => { await clearToken(server.id).catch(reportError); },
          })}>Remove token</Button>}
        </form>
        {notice && <p role="status" className={MUTED}>{notice}</p>}
      </Stack>
    </Card.Body>
  </Card>;
}

function ToolDialog({ tool, server, onClose }: { tool: McpTool; server: McpServer; onClose: () => void }) {
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
    } catch { setError('Arguments must be a JSON object. No request was sent.'); }
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
      setCancelNotice(accepted ? 'Cancellation requested. External effects may already have occurred; cancellation is not rollback.' : 'The call is not active yet or has already finished. Wait for the result; retry cancellation if it is still running.');
    } catch (error) { setCancelNotice(`Could not request cancellation: ${mcpErrorMessage(error)}`); reportError(error); }
    finally { cancelBusy.current = false; setCancelling(false); }
  }
  const unavailable = !server.enabled || server.status !== 'ready';
  const resultTone = result?.is_error ? 'border-error bg-error-bg text-error-text' : 'border-success bg-success-bg text-success-text';
  return <Modal
    open
    onOpenChange={open => { if (!open && !running) onClose(); }}
    title={tool.title || tool.tool_name}
    description={tool.description || 'No description provided.'}
    footer={review
      ? <div className="flex flex-wrap justify-end gap-2">
        <Button variant="secondary" disabled={running} onClick={() => setReview(null)}>Back to arguments</Button>
        {running && <Button variant="secondary" disabled={cancelling} onClick={() => void cancel()}>{cancelling ? 'Requesting cancellation...' : 'Cancel call'}</Button>}
        <Button disabled={running || unavailable} onClick={() => void execute()}>{running ? 'Calling...' : 'Confirm and call tool'}</Button>
      </div>
      : <Button type="submit" form={formId} disabled={unavailable}>Review tool call</Button>}
  >
    {/* First in DOM order, so it receives the initial focus when the inspector opens. */}
    <div className="flex flex-wrap items-center justify-between gap-2">
      <span className={cn(KICKER, 'min-w-0 wrap-anywhere')}>{server.name} / TOOL INSPECTOR</span>
      <Button variant="secondary" size="sm" disabled={running} onClick={onClose}>Close</Button>
    </div>
    <p className={HINT}>Server descriptions and schemas are untrusted data, not host permissions.</p>
    <details><summary className={SUMMARY}>Input schema</summary><pre className={PRE}>{JSON.stringify(tool.input_schema, null, 2)}</pre></details>
    {tool.output_schema !== undefined && <details><summary className={SUMMARY}>Output schema</summary><pre className={PRE}>{JSON.stringify(tool.output_schema, null, 2)}</pre></details>}
    <p className={HINT}>Schema {tool.schema_hash?.slice(0, 12)} · Discovered {tool.discovered_at ? new Date(tool.discovered_at * 1000).toLocaleString() : 'recently'}</p>
    {error && <p role="alert" className={INLINE_ERROR}>{error}</p>}
    {cancelNotice && <p role="status" className={HINT}>{cancelNotice}</p>}
    {review ? <section aria-label="Review tool call" aria-busy={running} className="flex flex-col gap-3 rounded-lg border border-warning bg-warning-bg p-4 text-warning-text">
      <h3 className="m-0 text-base font-semibold">Confirm external operation</h3>
      <p className="m-0 text-sm wrap-anywhere">Server: <strong>{server.name}</strong> · Tool: <code className="font-mono">{tool.tool_name}</code></p>
      <pre className={PRE}>{JSON.stringify(review.arguments, null, 2)}</pre>
      <p className="m-0 text-sm">This tool can read, change or send data outside DGOS. Only continue if you trust this server and understand these exact arguments. The result is shown in memory, not saved in the usage log.</p>
      {running && <p role="status" className="m-0 text-sm">Waiting for the host or tool. You can request cancellation; it does not undo external effects.</p>}
    </section> : <form id={formId} onSubmit={reviewCall}>
      <Stack gap={4}>
        <Input multiline label="Arguments (JSON)" className="font-mono text-xs" maxLength={65536} value={argumentsText} onChange={e => setArgumentsText(e.target.value)} rows={7} spellCheck={false} />
        <Input label="Timeout (ms)" type="number" min={100} max={120000} step={100} required value={timeoutMs} onChange={e => setTimeoutMs(Number(e.target.value))} />
      </Stack>
    </form>}
    {result && <section aria-label="Tool result" className={cn('flex flex-col gap-2 rounded-lg border p-4', resultTone)}>
      <h3 className="m-0 text-base font-semibold">{result.outcome === 'cancelled' ? 'Stopped waiting for tool' : result.outcome === 'timeout' ? 'Tool timed out' : result.is_error ? 'Tool reported an error' : 'Tool completed'}</h3>
      {(result.outcome === 'cancelled' || result.outcome === 'timeout') && <p role="alert" className="m-0 text-sm">The external operation may already have run. Cancellation or timeout does not roll back its effects. Check the external result before trying again.</p>}
      <p className="m-0 text-sm">{result.duration_ms} ms · Usage record #{result.usage_log_id}</p>
      {result.audit_warning && <p role="alert" className="m-0 text-sm">The tool ran, but the final audit update failed. Do not retry automatically; verify the external result first.</p>}
      <pre className={PRE}>{JSON.stringify(result.content, null, 2)}</pre>
      {result.structured_content != null && <details><summary className={SUMMARY}>Structured result</summary><pre className={PRE}>{JSON.stringify(result.structured_content, null, 2)}</pre></details>}
    </section>}
  </Modal>;
}

function LogRows({ rows }: { rows: { id: number; outcome: string; tone?: string; label: ReactNode; notes: ReactNode[]; duration?: number; at: number }[] }) {
  return <div className="flex flex-col gap-2">{rows.map(row => <div className={LOG_ROW} key={row.id}>
    <span className={cn('min-w-16 text-xs font-bold uppercase', row.tone ?? 'text-muted-foreground')}>{row.outcome}</span>
    <span className="min-w-0 flex-1 basis-32 font-semibold wrap-anywhere">{row.label}{row.notes}</span>
    <span>{row.duration ?? '-'} ms</span>
    <time className="text-muted-foreground">{new Date(row.at * 1000).toLocaleString()}</time>
  </div>)}</div>;
}

export default function McpManagement() {
  const { servers, tools, logsByServer, connectionsByServer, loading, pending, error, fetch, toggle, remove, connect, disconnect, loadTools, discoverTools, fetchLogs, fetchConnections } = useMcpStore();
  const reportError = usePageError('McpManagement');
  const confirm = useSingleConfirm();
  const [selected, setSelected] = useState<string | null>(null);
  const [editing, setEditing] = useState<McpServer | null>(null);
  const [tool, setTool] = useState<McpTool | null>(null);
  const [notice, setNotice] = useState('');
  const selectedServer = servers.find(server => server.id === selected);
  const toolServer = tool ? servers.find(server => server.id === tool.server_id) : null;
  useEffect(() => { void fetch(); }, [fetch]);
  useEffect(() => { if (selected) { void loadTools(selected); void fetchLogs(selected); void fetchConnections(selected); } }, [selected, loadTools, fetchLogs, fetchConnections]);
  // Poll only status/config metadata. Never poll or replay a tool call.
  useEffect(() => { const id = window.setInterval(() => { if (!useMcpStore.getState().pending.length) { void fetch(); if (selected) void fetchConnections(selected); } }, 10000); return () => window.clearInterval(id); }, [fetch, selected, fetchConnections]);
  return <main className="h-full overflow-auto bg-background p-6 text-foreground">
    <div className="mx-auto flex max-w-300 flex-col gap-5">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1 basis-72">
          <p className={cn(KICKER, 'mb-1')}>TOOLS BRIDGE</p>
          <h1 className="m-0 mb-2 text-3xl font-bold wrap-anywhere">MCP Management</h1>
          <p className={MUTED}>Only connect servers you trust. Local processes run with your operating-system permissions.</p>
        </div>
        <span className="rounded-full bg-secondary px-3 py-1 text-xs font-semibold text-secondary-foreground">V1 · D021</span>
      </header>
      {error && <div role="alert" className="rounded-lg border border-error bg-error-bg px-4 py-3 text-sm text-error-text wrap-anywhere">{error}</div>}
      {notice && <div role="status" className="rounded-lg border border-success bg-success-bg px-4 py-3 text-sm text-success-text wrap-anywhere">{notice}</div>}
      <section className="flex flex-wrap items-start gap-5">
        <ServerForm editing={editing} onCancel={() => setEditing(null)} onSaved={server => {
          setEditing(null); setSelected(server.id); setNotice('Server configuration saved. No connection was started by this action.');
        }} />
        <Card className="min-w-0 flex-1 grow-2 basis-72">
          <CardTitle kicker="CONTROL ROOM" title="Your servers" aside={<Button type="button" variant="secondary" size="sm" disabled={loading} onClick={() => void fetch()}>Refresh status</Button>} />
          <Card.Body>
            <Stack gap={3}>
              {loading && !servers.length && <Loading label="Loading servers..." />}
              {!loading && !servers.length && <div className="rounded-lg border border-dashed border-border px-4 py-8 text-center"><strong className="text-sm">No MCP servers yet</strong><p className={MUTED}>Add a stdio process or HTTPS endpoint to begin.</p></div>}
              {servers.map(server => {
                const busy = pending.includes(server.id);
                const connected = server.status === 'ready' || server.status === 'degraded';
                return <article className={cn('flex flex-wrap items-center gap-3 rounded-lg border p-3', selected === server.id ? 'border-primary bg-primary-light' : 'border-border bg-background')} key={server.id}>
                  <Button variant="ghost" className="h-auto min-w-0 flex-1 basis-44 items-start justify-start gap-3 whitespace-normal px-2 py-2 text-left" aria-label={`Select ${server.name}`} onClick={() => setSelected(server.id)} aria-pressed={selected === server.id}>
                    <span aria-hidden="true" className={cn('mt-1.5 size-2.5 shrink-0 rounded-full', STATUS_DOT[server.status])} />
                    <span className="flex min-w-0 flex-col gap-1">
                      <span className="text-sm font-semibold wrap-anywhere">{server.name}</span>
                      <span className="text-xs text-muted-foreground wrap-anywhere">{server.transport} · {server.status}{server.auto_connect ? ' · auto-connect' : ''}</span>
                      {server.last_error_class && <span className="text-xs text-error-text wrap-anywhere">{server.last_error_class}</span>}
                    </span>
                  </Button>
                  <div className="flex flex-wrap items-center gap-2">
                    <Button variant="secondary" size="sm" disabled={!server.enabled || busy} onClick={() => (connected ? disconnect(server.id) : connect(server.id)).catch(reportError)}>{busy ? 'Working...' : connected ? 'Disconnect' : 'Connect'}</Button>
                    <Button variant="secondary" size="sm" aria-label={`Edit ${server.name}`} disabled={busy} onClick={() => { setSelected(server.id); setEditing(server); }}>Edit</Button>
                    <Switch aria-label={`Enable ${server.name}`} disabled={busy} checked={server.enabled} onChange={checked => toggle(server.id, checked).catch(reportError)} />
                    <Button variant="danger" size="sm" disabled={busy} onClick={() => confirm({
                      title: 'Delete MCP server?',
                      content: `Delete ${server.name}? This disconnects its server.`,
                      confirmText: 'Delete', cancelText: 'Cancel', confirmVariant: 'danger',
                      onConfirm: async () => {
                        await remove(server.id).then(() => setEditing(current => (current?.id === server.id ? null : current))).catch(reportError);
                      },
                    })}>Delete</Button>
                  </div>
                </article>;
              })}
            </Stack>
          </Card.Body>
        </Card>
      </section>
      {selectedServer && <>
        {selectedServer.transport === 'http' && <Credentials key={selectedServer.id} server={selectedServer} />}
        <Card>
          <CardTitle kicker="DISCOVERED CAPABILITIES" title={`${selectedServer.name} tools`} aside={<div className="flex flex-wrap items-center gap-3"><span className="text-xs text-muted-foreground">Descriptions do not grant permissions</span><Button variant="secondary" size="sm" disabled={pending.includes(selectedServer.id) || selectedServer.status !== 'ready'} onClick={() => discoverTools(selectedServer.id).catch(reportError)}>Refresh tools</Button></div>} />
          <Card.Body>
            {!tools.some(item => item.server_id === selected) ? <p className={MUTED}>Connect the server to discover tools.</p> : <div className="flex flex-col gap-2">{tools.filter(item => item.server_id === selected).map(item => <article className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border border-border bg-background p-3" key={item.tool_name}>
              <div className="flex min-w-0 flex-1 basis-48 flex-col gap-1"><strong className="text-sm wrap-anywhere">{item.title || item.tool_name}</strong><span className="text-sm text-muted-foreground wrap-anywhere">{item.description || 'No description provided'}</span></div>
              <code className="min-w-0 font-mono text-xs text-primary wrap-anywhere">{item.tool_name}</code>
              <Button variant="secondary" size="sm" onClick={() => setTool(item)}>Inspect / call</Button>
            </article>)}</div>}
          </Card.Body>
        </Card>
        <Card>
          <CardTitle kicker="AUDIT TRAIL" title="Recent use" aside={<Button variant="secondary" size="sm" onClick={() => void fetchLogs(selectedServer.id)}>Refresh usage</Button>} />
          <Card.Body>
            <Stack gap={3}>
              <p className={HINT}>Parameters and output are never stored. Pending, interrupted, cancelled or timeout can mean the external result is unknown. Check it before retrying; no automatic replay occurs.</p>
              {!logsByServer[selectedServer.id]?.length ? <p className={MUTED}>No calls recorded for this server.</p> : <LogRows rows={logsByServer[selectedServer.id].map(log => ({
                id: log.id, outcome: log.outcome, tone: log.outcome === 'success' ? 'text-success-text' : log.outcome === 'error' ? 'text-error-text' : undefined, label: log.tool_name,
                notes: log.error_class ? [<small key="error" className="block text-xs font-normal text-error-text">{log.error_class}</small>] : [],
                duration: log.duration_ms, at: log.created_at,
              }))} />}
            </Stack>
          </Card.Body>
        </Card>
        <Card role="region" aria-label="Connection history">
          <CardTitle kicker="CONNECTION HISTORY" title="Connection events" aside={<Button variant="secondary" size="sm" onClick={() => void fetchConnections(selectedServer.id)}>Refresh connections</Button>} />
          <Card.Body>
            <Stack gap={3}>
              <p className={HINT}>Status changes and retry counts only. No command arguments, URLs, headers or tokens are recorded.</p>
              {!connectionsByServer[selectedServer.id]?.length ? <p className={MUTED}>No connection events for this server.</p> : <LogRows rows={connectionsByServer[selectedServer.id].map(log => ({
                id: log.id, outcome: log.status, label: log.event,
                notes: [
                  ...(log.error_class ? [<small key="error" className="block text-xs font-normal text-error-text">{log.error_class}</small>] : []),
                  ...(log.reconnect_attempt != null ? [<small key="retry" className="block text-xs font-normal text-error-text">Retry {log.reconnect_attempt} / 3</small>] : []),
                ],
                duration: log.duration_ms, at: log.created_at,
              }))} />}
            </Stack>
          </Card.Body>
        </Card>
      </>}
      {tool && toolServer && <ToolDialog tool={tool} server={toolServer} onClose={() => setTool(null)} />}
    </div>
  </main>;
}
