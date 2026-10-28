import { FormEvent, useEffect, useRef, useState } from 'react';
import {
  useMcpStore, mcpErrorMessage, McpServer, McpTool, McpTransport,
  CreateMcpServerRequest, CallMcpToolRequest, CallMcpToolResponse,
} from '../stores/mcpStore';
import './McpManagement.css';

const emptyConfig = (): CreateMcpServerRequest => ({
  name: '', transport: 'stdio', stdio: { executable: '', args: [] },
  enabled: false, auto_connect: false,
});
const ignoreReportedError = () => {};

function ServerForm({ editing, onSaved, onCancel }: {
  editing: McpServer | null; onSaved: (server: McpServer) => void; onCancel: () => void;
}) {
  const { create, update, getConfig } = useMcpStore();
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
    }).catch(error => { if (current) setError(mcpErrorMessage(error)); })
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
    } catch (error) { setError(mcpErrorMessage(error)); }
    finally { savingRef.current = false; setSaving(false); }
  }
  return <form className="mcp-card mcp-form" onSubmit={save} aria-label="Server configuration">
    <div className="card-title"><div><span className="section-kicker">{editing ? 'EDIT CONFIGURATION' : 'REGISTER'}</span><h2>{editing ? `Edit ${editing.name}` : 'Add MCP server'}</h2></div><span className="lock-note">No secrets in server configs</span></div>
    {error && <p role="alert" className="mcp-inline-error">{error}</p>}
    {loading && <p role="status">Loading configuration...</p>}
    <fieldset disabled={saving || loading || !configReady}>
      <label>Name<input required maxLength={100} value={config.name} onChange={e => setConfig({ ...config, name: e.target.value })} placeholder="Local filesystem" /></label>
      <label>Transport<select value={config.transport} onChange={e => setConfig({ ...config, transport: e.target.value as McpTransport })}><option value="stdio">stdio process</option><option value="http">HTTP endpoint</option></select></label>
      {config.transport === 'stdio' ? <>
        <label>Executable<input required value={config.stdio?.executable ?? ''} onChange={e => setConfig({ ...config, stdio: { args: [], ...config.stdio, executable: e.target.value } })} placeholder="/usr/local/bin/mcp-server" /></label>
        <label>Arguments<input value={argv} onChange={e => setArgv(e.target.value)} placeholder='["--root", "/Users/me/My Documents"]' /><small>JSON array; paths with spaces remain one argument. Shell syntax is not evaluated.</small></label>
        <label>Working directory<input value={config.stdio?.cwd ?? ''} onChange={e => setConfig({ ...config, stdio: { executable: '', args: [], ...config.stdio, cwd: e.target.value } })} placeholder="Optional absolute directory" /></label>
      </> : <>
        <label>Server URL<input required type="url" value={config.http?.url ?? ''} onChange={e => setConfig({ ...config, http: { url: e.target.value } })} placeholder="https://mcp.example.com" /><small>Prefer HTTPS or exact localhost. Changing the URL removes its previous credential binding and plaintext consent.</small></label>
        {config.http?.url.trim().toLowerCase().startsWith('http:') && <label className="mcp-check"><input type="checkbox" checked={!!config.http?.allow_insecure_http} onChange={e => setConfig({ ...config, http: { ...config.http!, allow_insecure_http: e.target.checked } })} />I accept remote plaintext HTTP risks: tool arguments, results and authentication tokens can be read or changed in transit. Not needed for exact localhost.</label>}
      </>}
      <label className="mcp-check"><input type="checkbox" checked={!!config.enabled} onChange={e => setConfig({ ...config, enabled: e.target.checked })} />Enable server</label>
      <label className="mcp-check"><input type="checkbox" checked={!!config.auto_connect} onChange={e => setConfig({ ...config, auto_connect: e.target.checked })} />Connect automatically on app startup</label>
      <small>Saving never launches a process or connects immediately. Auto-connect is off by default.</small>
    </fieldset>
    <div className="form-actions"><button className="mcp-primary" type="submit" disabled={saving || loading || !configReady}>{saving ? 'Saving...' : editing ? 'Save changes' : 'Save server'}</button>{editing && <button type="button" className="quiet" disabled={saving} onClick={onCancel}>Cancel editing</button>}</div>
  </form>;
}

function Credentials({ server }: { server: McpServer }) {
  const { setToken, clearToken, pending } = useMcpStore();
  const [token, setValue] = useState('');
  const [notice, setNotice] = useState('');
  const busy = pending.includes(server.id);
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!token || busy) return;
    const value = token; setValue(''); setNotice('');
    try { await setToken(server.id, value); setNotice('Token stored in the OS keychain. Reconnect to use it.'); }
    catch (error) { setNotice(mcpErrorMessage(error)); }
  }
  return <section className="mcp-card credentials-card">
    <div className="card-title"><div><span className="section-kicker">HOST KEYCHAIN</span><h2>HTTP authentication</h2></div><span className="server-count">{server.credential_ref ? 'Token configured' : 'No token'}</span></div>
    <p className="mcp-muted">Only an opaque reference is stored in DGOS. Tokens are never returned to this page. Rotation or removal disconnects the server.</p>
    <form onSubmit={save} className="credential-form">
      <label>Bearer token<input type="password" maxLength={8192} value={token} onChange={e => setValue(e.target.value)} autoComplete="new-password" placeholder={server.credential_ref ? 'Enter replacement token' : 'Enter token once'} /></label>
      <button className="mcp-primary" disabled={busy || !token}>Store token</button>
      {server.credential_ref && <button type="button" className="quiet" disabled={busy} onClick={() => {
        if (window.confirm('Remove this server token and disconnect?')) void clearToken(server.id).catch(ignoreReportedError);
      }}>Remove token</button>}
    </form>
    {notice && <p role="status">{notice}</p>}
  </section>;
}

function ToolDialog({ tool, server, onClose }: { tool: McpTool; server: McpServer; onClose: () => void }) {
  const { confirmTool, call, cancelCall, fetchLogs } = useMcpStore();
  const dialog = useRef<HTMLDialogElement>(null);
  const close = useRef<HTMLButtonElement>(null);
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
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const node = dialog.current;
    if (node && !node.open) {
      if (typeof node.showModal === 'function') node.showModal();
      else node.setAttribute('open', '');
    }
    close.current?.focus();
    return () => { node?.close?.(); previous?.focus(); };
  }, []);
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
    } catch (error) { setError(mcpErrorMessage(error)); setReview(null); }
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
    } catch (error) { setCancelNotice(`Could not request cancellation: ${mcpErrorMessage(error)}`); }
    finally { cancelBusy.current = false; setCancelling(false); }
  }
  return <dialog ref={dialog} className="mcp-modal" aria-labelledby="mcp-tool-title" onCancel={event => { event.preventDefault(); if (!running) onClose(); }}>
    <div className="card-title"><div><span className="section-kicker">{server.name} / TOOL INSPECTOR</span><h2 id="mcp-tool-title">{tool.title || tool.tool_name}</h2></div><button ref={close} type="button" className="quiet" disabled={running} onClick={onClose}>Close</button></div>
    <p>{tool.description || 'No description provided.'}</p>
    <p className="schema-note">Server descriptions and schemas are untrusted data, not host permissions.</p>
    <details><summary>Input schema</summary><pre>{JSON.stringify(tool.input_schema, null, 2)}</pre></details>
    {tool.output_schema !== undefined && <details><summary>Output schema</summary><pre>{JSON.stringify(tool.output_schema, null, 2)}</pre></details>}
    <p className="mcp-muted">Schema {tool.schema_hash?.slice(0, 12)} · Discovered {tool.discovered_at ? new Date(tool.discovered_at * 1000).toLocaleString() : 'recently'}</p>
    {error && <p role="alert" className="mcp-inline-error">{error}</p>}
    {cancelNotice && <p role="status" className="schema-note">{cancelNotice}</p>}
    {review ? <section aria-label="Review tool call" aria-busy={running} className="mcp-call-review">
      <h3>Confirm external operation</h3><p>Server: <strong>{server.name}</strong> · Tool: <code>{tool.tool_name}</code></p>
      <pre>{JSON.stringify(review.arguments, null, 2)}</pre>
      <p className="confirm-copy">This tool can read, change or send data outside DGOS. Only continue if you trust this server and understand these exact arguments. The result is shown in memory, not saved in the usage log.</p>
      {running && <p role="status">Waiting for the host or tool. You can request cancellation; it does not undo external effects.</p>}
      <div className="form-actions"><button className="quiet" disabled={running} onClick={() => setReview(null)}>Back to arguments</button>{running && <button className="quiet" disabled={cancelling} onClick={() => void cancel()}>{cancelling ? 'Requesting cancellation...' : 'Cancel call'}</button>}<button className="mcp-primary" disabled={running || !server.enabled || server.status !== 'ready'} onClick={() => void execute()}>{running ? 'Calling...' : 'Confirm and call tool'}</button></div>
    </section> : <form onSubmit={reviewCall} className="mcp-tool-form">
      <label>Arguments (JSON)<textarea maxLength={65536} value={argumentsText} onChange={e => setArgumentsText(e.target.value)} rows={7} spellCheck={false} /></label>
      <label>Timeout (ms)<input type="number" min={100} max={120000} step={100} required value={timeoutMs} onChange={e => setTimeoutMs(Number(e.target.value))} /></label>
      <button className="mcp-primary" disabled={!server.enabled || server.status !== 'ready'}>Review tool call</button>
    </form>}
    {result && <section aria-label="Tool result" className={result.is_error ? 'tool-result error' : 'tool-result'}>
      <h3>{result.outcome === 'cancelled' ? 'Stopped waiting for tool' : result.outcome === 'timeout' ? 'Tool timed out' : result.is_error ? 'Tool reported an error' : 'Tool completed'}</h3>
      {(result.outcome === 'cancelled' || result.outcome === 'timeout') && <p role="alert">The external operation may already have run. Cancellation or timeout does not roll back its effects. Check the external result before trying again.</p>}
      <p>{result.duration_ms} ms · Usage record #{result.usage_log_id}</p>
      {result.audit_warning && <p role="alert">The tool ran, but the final audit update failed. Do not retry automatically; verify the external result first.</p>}
      <pre>{JSON.stringify(result.content, null, 2)}</pre>
      {result.structured_content != null && <details><summary>Structured result</summary><pre>{JSON.stringify(result.structured_content, null, 2)}</pre></details>}
    </section>}
  </dialog>;
}

export default function McpManagement() {
  const { servers, tools, logsByServer, connectionsByServer, loading, pending, error, fetch, toggle, remove, connect, disconnect, loadTools, discoverTools, fetchLogs, fetchConnections } = useMcpStore();
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
  return <main className="mcp-page">
    <header className="mcp-header"><div><p className="eyebrow">TOOLS BRIDGE</p><h1>MCP Management</h1><p>Only connect servers you trust. Local processes run with your operating-system permissions.</p></div><span className="mcp-badge">V1 · D021</span></header>
    {error && <div role="alert" className="mcp-alert error">{error}</div>}
    {notice && <div role="status" className="mcp-alert">{notice}</div>}
    <section className="mcp-grid">
      <ServerForm editing={editing} onCancel={() => setEditing(null)} onSaved={server => {
        setEditing(null); setSelected(server.id); setNotice('Server configuration saved. No connection was started by this action.');
      }} />
      <section className="mcp-card"><div className="card-title"><div><span className="section-kicker">CONTROL ROOM</span><h2>Your servers</h2></div><button type="button" className="quiet" disabled={loading} onClick={() => void fetch()}>Refresh status</button></div>
        {loading && !servers.length && <p className="mcp-muted">Loading servers...</p>}
        {!loading && !servers.length && <div className="mcp-empty"><strong>No MCP servers yet</strong><p>Add a stdio process or HTTPS endpoint to begin.</p></div>}
        <div className="server-list">{servers.map(server => {
          const busy = pending.includes(server.id);
          const connected = server.status === 'ready' || server.status === 'degraded';
          return <article className={`server-row ${selected === server.id ? 'selected' : ''}`} key={server.id}>
            <button type="button" className="server-main" aria-label={`Select ${server.name}`} onClick={() => setSelected(server.id)} aria-pressed={selected === server.id}><span className={`status-dot ${server.status}`} /><span><strong>{server.name}</strong><span className="server-meta">{server.transport} · {server.status}{server.auto_connect ? ' · auto-connect' : ''}</span>{server.last_error_class && <span className="mcp-inline-error">{server.last_error_class}</span>}</span></button>
            <div className="server-actions">
              <button className="quiet" disabled={!server.enabled || busy} onClick={() => void (connected ? disconnect(server.id) : connect(server.id)).catch(ignoreReportedError)}>{busy ? 'Working...' : connected ? 'Disconnect' : 'Connect'}</button>
              <button className="quiet" aria-label={`Edit ${server.name}`} disabled={busy} onClick={() => { setSelected(server.id); setEditing(server); }}>Edit</button>
              <label className="switch"><input type="checkbox" aria-label={`Enable ${server.name}`} disabled={busy} checked={server.enabled} onChange={e => void toggle(server.id, e.target.checked).catch(ignoreReportedError)} /><span /></label>
              <button className="danger" disabled={busy} onClick={() => { if (window.confirm(`Delete ${server.name}? This disconnects its server.`)) void remove(server.id).then(() => { if (editing?.id === server.id) setEditing(null); }).catch(ignoreReportedError); }}>Delete</button>
            </div>
          </article>;
        })}</div>
      </section>
    </section>
    {selectedServer && <>
      {selectedServer.transport === 'http' && <Credentials key={selectedServer.id} server={selectedServer} />}
      <section className="mcp-card tools-card"><div className="card-title"><div><span className="section-kicker">DISCOVERED CAPABILITIES</span><h2>{selectedServer.name} tools</h2></div><div className="card-actions"><span className="tool-hint">Descriptions do not grant permissions</span><button className="quiet" disabled={pending.includes(selectedServer.id) || selectedServer.status !== 'ready'} onClick={() => void discoverTools(selectedServer.id).catch(ignoreReportedError)}>Refresh tools</button></div></div>
        {!tools.some(item => item.server_id === selected) ? <p className="mcp-muted">Connect the server to discover tools.</p> : <div className="tool-list">{tools.filter(item => item.server_id === selected).map(item => <article className="tool-row" key={item.tool_name}><div><strong>{item.title || item.tool_name}</strong><span>{item.description || 'No description provided'}</span></div><code>{item.tool_name}</code><button className="quiet" onClick={() => setTool(item)}>Inspect / call</button></article>)}</div>}
      </section>
      <section className="mcp-card logs-card"><div className="card-title"><div><span className="section-kicker">AUDIT TRAIL</span><h2>Recent use</h2></div><button className="quiet" onClick={() => void fetchLogs(selectedServer.id)}>Refresh usage</button></div><p className="tool-hint">Parameters and output are never stored. Pending, interrupted, cancelled or timeout can mean the external result is unknown. Check it before retrying; no automatic replay occurs.</p>
        {!logsByServer[selectedServer.id]?.length ? <p className="mcp-muted">No calls recorded for this server.</p> : <div className="log-list">{logsByServer[selectedServer.id].map(log => <div className="log-row" key={log.id}><span className={`log-outcome ${log.outcome}`}>{log.outcome}</span><strong>{log.tool_name}{log.error_class && <small>{log.error_class}</small>}</strong><span>{log.duration_ms ?? '-'} ms</span><time>{new Date(log.created_at * 1000).toLocaleString()}</time></div>)}</div>}
      </section>
      <section className="mcp-card logs-card" aria-label="Connection history"><div className="card-title"><div><span className="section-kicker">CONNECTION HISTORY</span><h2>Connection events</h2></div><button className="quiet" onClick={() => void fetchConnections(selectedServer.id)}>Refresh connections</button></div><p className="tool-hint">Status changes and retry counts only. No command arguments, URLs, headers or tokens are recorded.</p>
        {!connectionsByServer[selectedServer.id]?.length ? <p className="mcp-muted">No connection events for this server.</p> : <div className="log-list">{connectionsByServer[selectedServer.id].map(log => <div className="log-row" key={log.id}><span className="log-outcome">{log.status}</span><strong>{log.event}{log.error_class && <small>{log.error_class}</small>}{log.reconnect_attempt != null && <small>Retry {log.reconnect_attempt} / 3</small>}</strong><span>{log.duration_ms ?? '-'} ms</span><time>{new Date(log.created_at * 1000).toLocaleString()}</time></div>)}</div>}
      </section>
    </>}
    {tool && toolServer && <ToolDialog tool={tool} server={toolServer} onClose={() => setTool(null)} />}
  </main>;
}
