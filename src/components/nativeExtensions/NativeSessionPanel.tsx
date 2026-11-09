import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Select } from '@dgos/ui';
import { t } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { nativeError } from '../../stores/nativeExtensions';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import type { NativeObservedSkill } from '../../types/nativeExtensions';
import { hostCode } from '../../stores/hostError';

type Domain = 'skills' | 'mcp';
interface Capabilities { observe_skills: boolean; observe_mcp: boolean; reload_skills: boolean; reload_plugins: boolean; reconnect_mcp: boolean; toggle_mcp: boolean; native_login: boolean; native_logout: boolean; native_auth_guidance: null | { verified_version: string; reason: string; login_argv: string[]; logout_argv: string[] }; mcp_control_guidance?: null | { verified_version: string; reason: string; inspect_argv: string[] } }
interface Handle { session_id: string; generation: string; project_id: string; engine: 'codex' | 'claude'; capabilities: Capabilities }
interface Observation { session_id: string; generation: string; engine: Handle['engine']; domain: Domain; observed_at: number; capabilities: Capabilities; skills: NativeObservedSkill[]; mcp_servers: { name: string; status: string; auth_status: string | null; source: string | null; scope: string | null; tools: string[]; schema_observed: false }[]; incomplete: boolean }
type Action = { kind: 'reload_skills' } | { kind: 'reload_plugins'; hold_on_cache_impact: boolean } | { kind: 'reconnect_mcp'; server_name: string } | { kind: 'toggle_mcp'; server_name: string; enabled: boolean };
interface Receipt { session_id: string; generation: string; request_id: string; state: 'applied' | 'held' | 'unknown' }

// Retain the original request across views; verification never repeats the action.
const uncertain = new Map<string, { request_id: string; domain: Domain }>();
const listeners = new Set<() => void>();
let revision = 0;
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const notify = () => { revision++; listeners.forEach(listener => listener()); };
useDevelopment.subscribe((next, previous) => { if (next.epoch !== previous.epoch) { uncertain.clear(); notify(); } });

export default function NativeSessionPanel({ target, domain, initialProjectId, initialSessionId }: { target: 'Codex' | 'Claude'; domain: Domain; initialProjectId?: string; initialSessionId?: string }) {
  const epoch = useDevelopment(state => state.epoch);
  const projects = useDevelopment(state => state.projects);
  useSyncExternalStore(subscribe, () => revision);
  const runnerSession = useAgentRunner(state => state.session);
  const [project, setProject] = useState(initialProjectId ?? '');
  const [handles, setHandles] = useState<Handle[]>([]);
  const [handlesContext, setHandlesContext] = useState('');
  const [selected, setSelected] = useState('');
  const [observation, setObservation] = useState<Observation | null>(null);
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const generation = useRef(0);
  const inFlight = useRef(false);
  const entryKey = JSON.stringify([initialProjectId, initialSessionId]);
  const requested = useRef({ project: initialProjectId, session: initialSessionId, epoch });
  useEffect(() => {
    requested.current = { project: initialProjectId, session: initialSessionId, epoch };
    setProject(initialProjectId ?? '');
  }, [entryKey]);
  const available = projects.filter(item => !item.archived);
  const projectId = available.some(item => item.id === project) ? project : project ? '' : available[0]?.id ?? '';
  const contextKey = JSON.stringify([epoch, projectId, available.find(item => item.id === projectId)?.directory_identity, target, domain, entryKey]);
  const [observationContext, setObservationContext] = useState('');
  const handle = handlesContext === contextKey ? handles.find(item => item.session_id === selected) : undefined;
  const handleKey = handle ? JSON.stringify([epoch, handle.session_id, handle.generation]) : '';
  const blocked = uncertain.has(handleKey);
  const unavailable = runnerSession?.id === selected && (runnerSession.generation !== handle?.generation || runnerSession.status === 'closed' || runnerSession.status === 'unknown' || runnerSession.status === 'starting');
  const visibleObservation = !unavailable && observationContext === contextKey && observation?.session_id === handle?.session_id && observation?.generation === handle?.generation ? observation : null;
  useEffect(() => {
    if (!unavailable) return;
    generation.current++; setObservation(null); setBusy(false); inFlight.current = false;
  }, [unavailable]);

  useEffect(() => {
    const seq = ++generation.current;
    setHandles([]); setSelected(''); setObservation(null); setMessage(''); setBusy(false); inFlight.current = false;
    if (!projectId) { if (project) setMessage(t('nativeExtensions.entrySessionUnavailable')); return; }
    void invoke<Handle[]>('list_native_sessions', { input: { project_id: projectId, domain } }).then(values => {
      if (generation.current !== seq) return;
      const owned = values.filter(item => item.engine === target.toLowerCase() && item.project_id === projectId);
      setHandlesContext(contextKey);
      setHandles(owned);
      const entry = requested.current;
      if (entry.epoch === epoch && entry.project === projectId && entry.session) {
        const exact = owned.find(item => item.session_id === entry.session);
        setSelected(exact?.session_id ?? '');
        if (!exact) setMessage(t('nativeExtensions.entrySessionUnavailable'));
      }
    }, error => { if (generation.current === seq) setMessage(t(`nativeExtensions.error.${nativeError(error)}`)); });
    return () => { generation.current++; };
  }, [contextKey, refreshKey]);

  const observe = async () => {
    if (!handle || unavailable || inFlight.current) return;
    const seq = generation.current;
    inFlight.current = true; setBusy(true); setObservation(null); setMessage('');
    try {
      const value = await invoke<Observation>('observe_native_session', { input: { session_id: handle.session_id, generation: handle.generation, domain } });
      if (value.session_id !== handle.session_id || value.generation !== handle.generation || value.domain !== domain || value.engine !== handle.engine) throw new Error('mismatched observation');
      if (seq === generation.current) { setObservationContext(contextKey); setObservation(value); }
    } catch (error) { if (seq === generation.current) { setObservation(null); setMessage(t(`nativeExtensions.error.${nativeError(error)}`)); } }
    finally { if (seq === generation.current) { inFlight.current = false; setBusy(false); } }
  };
  const act = async (action: Action) => {
    if (!handle || unavailable || inFlight.current || uncertain.has(handleKey)) return;
    const seq = generation.current;
    const request_id = crypto.randomUUID();
    inFlight.current = true; setBusy(true); setObservation(null); setMessage('');
    // An interrupted renderer must not turn a lost response into an implicit retry.
    const pending = { request_id, domain };
    uncertain.set(handleKey, pending); notify();
    try {
      const receipt = await invoke<Receipt>('native_session_action', { input: { session_id: handle.session_id, generation: handle.generation, request_id, action } });
      if (receipt.session_id !== handle.session_id || receipt.generation !== handle.generation || receipt.request_id !== request_id || !['applied', 'held', 'unknown'].includes(receipt.state)) throw new Error('mismatched receipt');
      if (receipt.state !== 'unknown' && useDevelopment.getState().epoch === epoch && uncertain.get(handleKey) === pending) { uncertain.delete(handleKey); notify(); }
      if (seq === generation.current) setMessage(t(`nativeExtensions.action.${receipt.state}`));
    } catch (error) {
      // Busy admission and a missing Codex method both prove no mutation ran.
      // Other failures retain the original request until its outcome is known.
      const unsupportedSkills = handle.engine === 'codex' && action.kind === 'reload_skills' && error === 'native_extension_Unsupported';
      const refused = error === 'session_busy' || unsupportedSkills;
      if (refused && useDevelopment.getState().epoch === epoch && uncertain.get(handleKey) === pending) { uncertain.delete(handleKey); notify(); }
      if (seq === generation.current) {
        if (unsupportedSkills) setHandles(values => values.map(value => value.session_id === handle.session_id && value.generation === handle.generation ? { ...value, capabilities: { ...value.capabilities, observe_skills: false, reload_skills: false } } : value));
        const code = hostCode(error);
        setMessage(refused ? t(`nativeExtensions.error.${nativeError(error)}`) : code === 'unknown' ? t('nativeExtensions.action.unknown') : `${t(`nativeExtensions.error.${nativeError(error)}`)} ${t('nativeExtensions.action.unknown')}`);
      }
    }
    finally { if (seq === generation.current) { inFlight.current = false; setBusy(false); } }
  };
  const inspect = async () => {
    const pending = uncertain.get(handleKey);
    if (!handle || unavailable || !pending || inFlight.current) return;
    const seq = generation.current;
    inFlight.current = true; setBusy(true); setObservation(null);
    try {
      const receipt = await invoke<Receipt | null>('inspect_native_session_request', { input: { session_id: handle.session_id, generation: handle.generation, ...pending } });
      if (!receipt || receipt.session_id !== handle.session_id || receipt.generation !== handle.generation || receipt.request_id !== pending.request_id || !['applied', 'held', 'unknown'].includes(receipt.state)) throw new Error('unavailable receipt');
      if (receipt.state !== 'unknown' && useDevelopment.getState().epoch === epoch && uncertain.get(handleKey) === pending) { uncertain.delete(handleKey); notify(); }
      if (seq === generation.current) setMessage(t(`nativeExtensions.action.${receipt.state}`));
    } catch { if (seq === generation.current) setMessage(t('nativeExtensions.action.unknown')); }
    finally { if (seq === generation.current) { inFlight.current = false; setBusy(false); } }
  };
  const select = (id: string) => { requested.current.session = undefined; generation.current++; setSelected(id); setObservation(null); setMessage(''); setBusy(false); inFlight.current = false; };
  const caps = unavailable ? undefined : handle?.capabilities;
  const mcpControlAllowed = (item: Observation['mcp_servers'][number]) => item.status !== 'unknown' && !visibleObservation?.incomplete && ['user', 'project', 'local'].includes(item.source ?? '') && visibleObservation?.mcp_servers.filter(server => server.name === item.name).length === 1;
  return <section className="flex min-w-0 flex-col gap-3 border-t border-border-subtle pt-3" aria-label={t('nativeExtensions.sessions')}>
    <h3 className="m-0 text-base">{t('nativeExtensions.sessions')}</h3>
    <div className="flex flex-wrap gap-3">
      <Select label={t('nativeExtensions.project')} value={projectId} onChange={id => { requested.current.session = undefined; setProject(id); }} options={available.map(item => ({ value: item.id, label: item.name }))} />
      <Select label={t('nativeExtensions.chooseSession')} value={selected} onChange={select} options={handles.map(item => ({ value: item.session_id, label: `${target} · ${item.session_id}` }))} />
      <Button variant="secondary" disabled={busy || !handle || !(domain === 'skills' ? caps?.observe_skills : caps?.observe_mcp)} onClick={() => void observe()}>{t('nativeExtensions.observeSession')}</Button>
      <Button variant="secondary" disabled={busy || !projectId} onClick={() => setRefreshKey(value => value + 1)}>{t('nativeExtensions.refresh')}</Button>
    </div>
    {!handles.length && <p className="m-0 text-sm text-ink-secondary">{t('nativeExtensions.noSessions')}</p>}
    {(message || blocked || unavailable) && <p role="status" className="m-0 text-sm">{unavailable ? t('nativeExtensions.sessionUnobservable') : message || t('nativeExtensions.action.unknown')}</p>}
    {blocked && <div className="flex flex-wrap gap-2 text-sm"><span className="break-all">{t('nativeExtensions.operationId')}: {uncertain.get(handleKey)?.request_id}</span><Button variant="secondary" disabled={busy || unavailable} onClick={() => void inspect()}>{t('nativeExtensions.verify')}</Button></div>}
    {handle && !unavailable && <div className="flex min-w-0 flex-col gap-1 text-sm text-ink-secondary">
      <p className="m-0">{t('nativeExtensions.discoveryStage')}</p>
      {runnerSession?.id === handle.session_id && runnerSession.generation === handle.generation && <dl className="m-0 grid min-w-0 gap-1">
        <div className="break-all"><dt className="inline">{t('capabilities.cli')}: </dt><dd className="m-0 inline">{runnerSession.cli_version || t('nativeExtensions.unknown')}</dd></div>
        <div className="break-all"><dt className="inline">{t('capabilities.workspace')}: </dt><dd className="m-0 inline">{runnerSession.workspace}</dd></div>
      </dl>}
      {domain === 'mcp' && <>
        <p className="m-0">{t('nativeExtensions.authUnsupported')}</p>
        {caps?.native_auth_guidance && <>
          <p className="m-0">{t('nativeExtensions.verifiedProtocol', { version: caps.native_auth_guidance.verified_version })}</p>
          <p className="m-0">{t(`nativeExtensions.guidance.${caps.native_auth_guidance.reason}`)}</p>
          <code className="block break-all">{caps.native_auth_guidance.login_argv.join(' ')}</code>
          <code className="block break-all">{caps.native_auth_guidance.logout_argv.join(' ')}</code>
        </>}
        {caps?.mcp_control_guidance && <>
          <p className="m-0">{t(`nativeExtensions.guidance.${caps.mcp_control_guidance.reason}`)}</p>
          <code className="block break-all">{caps.mcp_control_guidance.inspect_argv.join(' ')}</code>
          <p className="m-0">{t('nativeExtensions.nativeMcpNextStep')}</p>
        </>}
      </>}
    </div>}
    {domain === 'skills' && <div className="flex flex-wrap gap-2">
      {caps?.reload_skills && <Button disabled={busy || blocked} onClick={() => void act({ kind: 'reload_skills' })}>{t('nativeExtensions.reloadSkills')}</Button>}
      {caps?.reload_plugins && <Button variant="secondary" disabled={busy || blocked} onClick={() => void act({ kind: 'reload_plugins', hold_on_cache_impact: true })}>{t('nativeExtensions.reloadPlugins')}</Button>}
    </div>}
    {visibleObservation && <div className="flex min-w-0 flex-col gap-2 text-sm">
      <p className="m-0">{t('nativeExtensions.observationSnapshot')}</p>
      <time dateTime={new Date(visibleObservation.observed_at).toISOString()}>{new Date(visibleObservation.observed_at).toLocaleString()}</time>
      {visibleObservation.incomplete && <p>{t('nativeExtensions.incomplete')}</p>}
      {domain === 'skills' && <><p className="m-0">{t(visibleObservation.engine === 'codex' ? 'nativeExtensions.observedViaCodex' : 'nativeExtensions.observedViaClaude')}</p><p className="m-0 text-ink-secondary">{t('nativeExtensions.observedReadOnly')}</p></>}
      {domain === 'mcp' && <p className="m-0 text-ink-secondary">{t(visibleObservation.engine === 'codex' ? 'nativeExtensions.mcpObservedViaCodex' : 'nativeExtensions.mcpObservedViaClaude')}</p>}
      {domain === 'skills' ? visibleObservation.skills.map((item, index) => <div className="flex min-w-0 flex-col gap-1 border-t border-border-subtle pt-2" key={`${item.name}:${index}`}>
        <p className="m-0 break-all">{item.name}</p>
        <dl className="m-0 grid min-w-0 gap-1">
          <div><dt className="inline">{t('nativeExtensions.observedClassification')}: </dt><dd className="m-0 inline">{t(`nativeExtensions.classification.${item.classification ?? 'unknown'}`)}</dd></div>
          <div><dt className="inline">{t('nativeExtensions.observedEnabled')}: </dt><dd className="m-0 inline">{item.enabled == null ? t('nativeExtensions.unknown') : t(item.enabled ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</dd></div>
          {visibleObservation.engine === 'claude' && <div><dt className="inline">{t('nativeExtensions.observedSource')}: </dt><dd className="m-0 inline">{t('nativeExtensions.commandSourceUnknown')}</dd></div>}
          {item.scope && <div><dt className="inline">{t('nativeExtensions.observedScope')}: </dt><dd className="m-0 inline">{t(`nativeExtensions.observedScope.${item.scope}`)}</dd></div>}
          {item.plugin_id && <div className="break-all"><dt className="inline">{t('nativeExtensions.observedPlugin')}: </dt><dd className="m-0 inline">{item.plugin_id}</dd></div>}
          {item.path && <div className="break-all"><dt className="inline">{t('nativeExtensions.observedPath')}: </dt><dd className="m-0 inline"><code>{item.path}</code></dd></div>}
        </dl>
      </div>) : visibleObservation.mcp_servers.map((item, index) => <div key={`${item.name}:${index}`} className="flex min-w-0 flex-wrap items-center gap-2 border-t border-border-subtle pt-2">
        <span className="break-all">{item.name} · {item.status} · {item.auth_status ?? t('nativeExtensions.unknown')}</span>
        <span className="break-all">{t('nativeExtensions.observedSource')}: {item.source ?? t('nativeExtensions.unknown')}</span>
        {item.scope && <span>{item.scope}</span>}
        {caps?.reconnect_mcp && mcpControlAllowed(item) && <Button size="xs" disabled={busy || blocked} onClick={() => void act({ kind: 'reconnect_mcp', server_name: item.name })}>{t('nativeExtensions.reconnect')}</Button>}
        {caps?.toggle_mcp && mcpControlAllowed(item) && <><Button size="xs" disabled={busy || blocked} onClick={() => void act({ kind: 'toggle_mcp', server_name: item.name, enabled: true })}>{t('nativeExtensions.enable')}</Button><Button size="xs" variant="secondary" disabled={busy || blocked} onClick={() => void act({ kind: 'toggle_mcp', server_name: item.name, enabled: false })}>{t('nativeExtensions.disable')}</Button></>}
        <span className="w-full break-all">{t('nativeExtensions.toolsObserved')}: {item.tools.join(', ') || t('nativeExtensions.unknown')}</span>
        <span className="w-full">{t('nativeExtensions.schemaUnobserved')}</span>
      </div>)}
    </div>}
  </section>;
}
