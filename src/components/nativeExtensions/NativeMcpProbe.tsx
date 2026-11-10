import { useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { t } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { nativeCatalogKey, nativeProbeKey, useNativeMcpCatalog, type ProbeRead } from '../../stores/nativeMcpCatalog';
import { useNativeManagerGuard } from './NativeManagerLifecycle';
import type { NativeDraft, NativeSelection } from '../../types/nativeExtensions';
import type { NativeMcpConfigProjection, NativeProbeInput } from '../../types/nativeMcpCatalog';
import './native-mcp-catalog.css';

export function nativeDraftProbeInput(selection: NativeSelection, draft: NativeDraft): NativeProbeInput | null {
  const stdio = draft.transport === 'Stdio';
  const header_env: [string, string][] = [];
  if (!stdio) for (const line of draft.header_env.split('\n').filter(line => line.trim())) {
    const at = line.indexOf('=');
    if (at <= 0 || !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(line.slice(0, at).trim()) || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(line.slice(at + 1).trim())) return null;
    header_env.push([line.slice(0, at).trim(), line.slice(at + 1).trim()]);
  }
  if (!draft.name.trim() || (stdio ? !draft.program : !draft.url)) return null;
  return { selection, source: { kind: 'draft', draft: { name: draft.name.trim(), transport: draft.transport, program: stdio ? draft.program : '', args: stdio && draft.args ? draft.args.split('\n') : [], url: stdio ? '' : draft.url, cwd: stdio && selection.target === 'Codex' ? draft.cwd : '', bearer_env: !stdio && selection.target === 'Codex' ? draft.bearer_env : '', forwarded_env: stdio ? draft.forwarded_env.split('\n').map(name => name.trim()).filter(Boolean) : [], header_env } } };
}
export function NativeMcpProbeRecovery({ record, probeKey }: { record: ProbeRead; probeKey: string }) {
  const busy = useNativeMcpCatalog(state => Boolean(state.busy[probeKey])), snapshot = record.snapshot;
  return <section className="native-mcp-probe" aria-label={t('nativeMcpCatalog.inspectProbe')}>
    <h3>{t('nativeMcpCatalog.inspectProbe')}</h3>
    <p>{snapshot.selection.target} · {t(`nativeExtensions.scope.${snapshot.selection.scope}`)} · {snapshot.config.name} · {snapshot.probe_id}</p>
    <dl><dt>{t('nativeMcpCatalog.resolvedProgram')}</dt><dd>{snapshot.executable ?? snapshot.config.url}</dd><dt>{t('nativeMcpCatalog.workspaceRoot')}</dt><dd>{snapshot.root}</dd><dt>{t('nativeExtensions.cwd')}</dt><dd>{snapshot.cwd}</dd><dt>{t('nativeMcpCatalog.configDigest')}</dt><dd>{snapshot.digest}</dd></dl>
    <pre>{JSON.stringify(snapshot.probe_args, null, 2)}</pre>
    <p role="status">{t(`nativeMcpCatalog.state.${snapshot.state}`)} · {t(`nativeMcpCatalog.cleanup.${snapshot.cleanup}`)}</p>
    <div className="native-mcp-catalog-actions">{(['cancel', 'inspect'] as const).map(action => <Button key={action} variant="secondary" disabled={busy} onClick={() => void useNativeMcpCatalog.getState().operation(snapshot.selection, action, probeKey)}>{t(action === 'cancel' ? 'nativeMcpCatalog.cancelProbe' : 'nativeMcpCatalog.inspectProbe')}</Button>)}</div>
  </section>;
}

export default function NativeMcpProbe({ selection, serverName, draft, disabled = false }: { selection: NativeSelection; serverName?: string; draft?: NativeDraft; disabled?: boolean }) {
  const epoch = useDevelopment(state => state.epoch), context = useDevelopment(() => nativeCatalogKey(selection));
  const input: NativeProbeInput | null = serverName ? { selection, source: { kind: 'existing', server_name: serverName } } : draft ? nativeDraftProbeInput(selection, draft) : null;
  const key = input ? nativeProbeKey(input) : JSON.stringify([context, serverName ?? null]);
  const probes = useNativeMcpCatalog(state => state.probes);
  const probeKey = probes[key] ? key : Object.entries(probes).find(([, value]) => ['running', 'unknown'].includes(value.snapshot.state) && JSON.stringify(value.snapshot.selection) === JSON.stringify(selection) && value.input.source.kind === input?.source.kind && (value.input.source.kind === 'existing' ? value.input.source.server_name === serverName : value.input.source.draft.name === draft?.name.trim()))?.[0] ?? key;
  const probe = probes[probeKey], busy = useNativeMcpCatalog(state => Boolean(state.busy[probeKey]));
  const [projection, setProjection] = useState<NativeMcpConfigProjection | null>(null), [message, setMessage] = useState(''), [reading, setReading] = useState(false), [approved, setApproved] = useState(false);
  const revision = useRef(0), inFlight = useRef(false);
  const inputKey = JSON.stringify(input), contextKey = JSON.stringify([context, serverName ?? null]);
  useEffect(() => { revision.current++; inFlight.current = false; setProjection(null); setMessage(''); setReading(false); setApproved(false); }, [contextKey, epoch]);
  useEffect(() => { setApproved(false); }, [inputKey, probe?.snapshot.probe_id]);
  useNativeManagerGuard(() => {
    const current = useNativeMcpCatalog.getState();
    if (!inFlight.current && !Object.values(current.busy).some(Boolean) && !Object.values(current.probes).some(value => value.snapshot.state === 'running' || value.snapshot.state === 'unknown')) return true;
    setMessage(t('nativeMcpCatalog.closeBlocked')); return false;
  });
  const snapshot = probe?.snapshot;
  const stale = Boolean(probe && (probeKey !== key || JSON.stringify(probe.input) !== inputKey || projection && projection.config_digest !== probe.snapshot.config.config_digest));
  const unresolved = snapshot?.state === 'running' || snapshot?.state === 'unknown';
  const showError = (code: string) => t(code.startsWith('registry_') || code.startsWith('probe_') ? `nativeMcpCatalog.error.${code}` : `nativeExtensions.error.${code}`);
  const project = async () => {
    if (!serverName || inFlight.current) return;
    const captured = revision.current; inFlight.current = true; setReading(true); setMessage('');
    const result = await useNativeMcpCatalog.getState().project(selection, serverName);
    if (captured !== revision.current || context !== nativeCatalogKey(selection)) return;
    inFlight.current = false; setReading(false);
    if (result.ok) setProjection(result.value); else setMessage(showError(result.code));
  };
  const preview = async () => {
    if (!input || disabled || inFlight.current || busy || unresolved) return;
    const captured = revision.current; inFlight.current = true; setMessage(''); setApproved(false);
    const result = await useNativeMcpCatalog.getState().preview(input);
    if (captured !== revision.current || context !== nativeCatalogKey(selection)) return;
    inFlight.current = false; if (!result.ok) setMessage(showError(result.code));
  };
  const operate = async (action: 'run' | 'cancel' | 'inspect') => {
    if (inFlight.current || busy || action === 'run' && (!approved || stale || disabled)) return;
    const captured = revision.current; inFlight.current = true; setMessage('');
    const result = await useNativeMcpCatalog.getState().operation(snapshot?.selection ?? selection, action, probeKey);
    if (captured !== revision.current || context !== nativeCatalogKey(selection)) return;
    inFlight.current = false; if (!result.ok) setMessage(showError(result.code));
  };
  const config = snapshot?.config ?? projection;
  return <section className="native-mcp-probe" aria-label={t('nativeMcpCatalog.probe')}>
    <header><h3>{t('nativeMcpCatalog.probe')}</h3><p>{t('nativeMcpCatalog.probeNotice')}</p></header>
    <div className="native-mcp-catalog-actions">
      {serverName && <Button variant="secondary" disabled={disabled || reading || busy} onClick={() => void project()}>{t(reading ? 'nativeMcpCatalog.reading' : 'nativeMcpCatalog.readConfig')}</Button>}
      <Button variant="secondary" disabled={disabled || !input || busy || reading || unresolved} onClick={() => void preview()}>{t('nativeMcpCatalog.previewProbe')}</Button>
    </div>
    {!input && <p>{t('nativeMcpCatalog.completeDraft')}</p>}
    {config && <details open><summary>{t('nativeMcpCatalog.safeConfiguration')}</summary><dl>
      <dt>{t('nativeExtensions.name')}</dt><dd>{config.name}</dd><dt>{t('nativeExtensions.transport')}</dt><dd>{config.transport}</dd>
      <dt>{t('nativeExtensions.program')}</dt><dd>{config.program ?? '—'}</dd><dt>{t('nativeExtensions.args')}</dt><dd><pre>{JSON.stringify(config.args, null, 2)}</pre></dd>
      <dt>{t('nativeExtensions.url')}</dt><dd>{config.url ?? '—'}</dd><dt>{t('nativeExtensions.cwd')}</dt><dd>{config.cwd ?? '—'}</dd>
      <dt>{t('nativeMcpCatalog.credentialReferences')}</dt><dd>{config.credential_references.join('\n') || t('nativeMcpCatalog.noCredentials')}</dd>
    </dl>{config.redacted && <p>{t('nativeMcpCatalog.redacted')}</p>}</details>}
    {snapshot && <>
      <h4>{t('nativeMcpCatalog.exactPlan')}</h4>
      <dl><dt>{t('nativeMcpCatalog.requestId')}</dt><dd>{snapshot.probe_id}</dd><dt>{t('nativeMcpCatalog.resolvedProgram')}</dt><dd>{snapshot.executable ?? snapshot.config.url}</dd><dt>{t('nativeExtensions.cwd')}</dt><dd>{snapshot.cwd}</dd><dt>{t('nativeMcpCatalog.workspaceRoot')}</dt><dd>{snapshot.root}</dd><dt>{t('nativeMcpCatalog.platformIdentity')}</dt><dd>{snapshot.platform_identity}</dd><dt>{t('nativeMcpCatalog.binaryIdentity')}</dt><dd>{snapshot.executable_identity ?? '—'}</dd><dt>{t('nativeMcpCatalog.configDigest')}</dt><dd>{snapshot.digest}</dd><dt>{t('nativeExtensions.expires')}</dt><dd>{new Date(snapshot.expires_at * 1000).toLocaleString()}</dd></dl>
      {snapshot.config.transport === 'Stdio' && <><h4>{t('nativeMcpCatalog.effectiveArgv')}</h4><pre>{JSON.stringify(snapshot.probe_args, null, 2)}</pre></>}
      {snapshot.cleanup_plan && <><h4>{t('nativeMcpCatalog.containerCleanup')}</h4><p>{t('nativeMcpCatalog.containerCleanupNotice')}</p><pre>{JSON.stringify(snapshot.cleanup_plan, null, 2)}</pre></>}
      <p>{t('nativeMcpCatalog.bounds')}</p>{snapshot.may_download_execute && <p className="native-mcp-catalog-warning">{t('nativeMcpCatalog.executionWarning')}</p>}
      {stale && <p role="status">{t('nativeMcpCatalog.expiredResult')}</p>}
      <p role="status">{t(`nativeMcpCatalog.state.${snapshot.state}`)} · {t('nativeMcpCatalog.phase')}: {t(`nativeMcpCatalog.phase.${snapshot.phase}`)} · {t('nativeMcpCatalog.cleanup')}: {t(`nativeMcpCatalog.cleanup.${snapshot.cleanup}`)}</p>
      {snapshot.error && <p role="alert">{showError(snapshot.error)}</p>}
      {probe?.error && <p role="alert">{showError(probe.error)}</p>}
      <div className="native-mcp-catalog-actions">
        {snapshot.state === 'ready' && <><label className="native-mcp-catalog-consent"><input type="checkbox" checked={approved} onChange={event => setApproved(event.target.checked)} disabled={disabled || stale || busy}/>{t('nativeMcpCatalog.approval')}</label><Button disabled={disabled || busy || stale || !approved} onClick={() => void operate('run')}>{t('nativeMcpCatalog.runProbe')}</Button></>}
        {unresolved && <Button variant="secondary" disabled={busy} onClick={() => void operate('cancel')}>{t('nativeMcpCatalog.cancelProbe')}</Button>}
        {snapshot.state !== 'ready' && <Button variant="secondary" disabled={busy} onClick={() => void operate('inspect')}>{t('nativeMcpCatalog.inspectProbe')}</Button>}
      </div>
      {snapshot.observed_at && <p>{t('nativeMcpCatalog.observedAt', { time: new Date(snapshot.observed_at * 1000).toLocaleString() })} · {snapshot.protocol_version ?? '—'}</p>}
      {snapshot.state === 'succeeded' && <><h4>{t('nativeMcpCatalog.schemas', { count: snapshot.tools.length })}</h4><p>{t('nativeMcpCatalog.schemaOrigin')}</p>{snapshot.tools.map((tool, index) => <details key={`${String(tool.name)}:${index}`}><summary>{String(tool.name)}</summary><pre>{JSON.stringify(tool, null, 2)}</pre></details>)}</>}
    </>}
    {message && <p role="alert">{message}</p>}
  </section>;
}
