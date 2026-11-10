import { lazy, Suspense, useCallback, useEffect, useId, useRef, useState } from 'react';
import { Button, Input, ResourceDetail, ResourceList } from '@dgos/ui';
import { formatDateTime, useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { useWorkspaceTrust } from '../../stores/developmentTrust';
import { useDesktopSlot } from '../../stores/developmentDesktop';
import { commandIntent, matchingService, runtimeCall, runtimeOutputExcerpt, type RuntimeIntent, type ReadinessInput } from '../../stores/developmentRuntime';
import { readDevelopmentCommands, readDevelopmentProcess, submitRuntimeOperation, retryRuntimeOperation, runtimeOperationBlocked, openDevelopmentExternal, processActive, useRuntimeDraft, useRuntimeProcessView, useDevelopmentRuntime, type CommandCatalog, type DevelopmentCommand, type DevelopmentProcess, type StartProcessInput } from '../../stores/developmentRuntime';
import { confirmAction, errorText as developmentErrorText } from './shared';
import { beginResourceRead, failResourceRead, resourceView, type ResourceReadState } from '../../resourceState';
import { ManualTakeover } from './ManualTakeover';
const DevelopmentTerminal = lazy(() => import('./DevelopmentTerminal').then(module => ({ default: module.DevelopmentTerminal })));

const emptyRuntimePreferences: { choices: Partial<Record<RuntimeIntent, string>>; readiness: Record<string, ReadinessInput> } = { choices: {}, readiness: {} };

function RuntimeOutput({ process, disabled }: { process: DevelopmentProcess; disabled: boolean }) {
  const { t } = useLocale();
  const { view, setView } = useRuntimeProcessView(process);
  const [query, setQuery] = useState('');
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const output = process.output.slice(view.clearOffset ?? 0);
  const lines = query ? output.split('\n').filter(line => line.toLocaleLowerCase().includes(query.toLocaleLowerCase())).join('\n') : output;
  return <div className="dev-review-stack"><div className="dev-review-toolbar"><Button size="xs" variant="secondary" data-testid="dev-output-clear" disabled={disabled} onClick={() => setView({ clearOffset: process.output.length })}>{t('workbenchRuntime.clearDisplay')}</Button><Button size="xs" variant="secondary" disabled={disabled || !output} onClick={async () => { setCopyError(false); try { await navigator.clipboard.writeText(lines); setCopied(true); } catch { setCopyError(true); } }}>{t(copied ? 'runtimeWorkflow.copied' : 'runtimeWorkflow.copyOutput')}</Button><span className="text-xs text-ink-secondary">{t('workbenchRuntime.clearHint')}</span></div><Input label={t('runtimeWorkflow.searchOutput')} type="search" value={query} onChange={event => { setQuery(event.target.value); setCopied(false); }} />{copyError ? <p role="alert">{t('runtimeWorkflow.clipboardFailed')}</p> : null}<pre className="dev-diff" tabIndex={0} aria-label={t('direct.output')}>{lines || t(query ? 'runtimeWorkflow.noMatches' : view.clearOffset ? 'workbenchRuntime.displayCleared' : 'direct.noOutput')}</pre></div>;
}

export function DevelopmentRuntime({ projectId, taskId, visible = true, onUseOutput, requestedAction }: { projectId: string; taskId: string; visible?: boolean; onUseOutput?: (text: string) => void; requestedAction?: { kind: 'test' | 'preview' | 'build' | 'terminal'; nonce: number } }) {
  const { t } = useLocale();
  const errorText = (code: string) => code === 'storage_failed' ? t('direct.operation.storageUnavailable') : code === 'unknown' ? t('direct.operation.receiptUnavailable') : developmentErrorText(code);
  const runtime = useDevelopmentRuntime();
  const surfaceEligible = useSurfaceActive();
  const surfaceActive = surfaceEligible && visible;
  const epoch = useDevelopment(state => state.epoch);
  const context = useContextSelection();
  const owner = JSON.stringify([epoch, context.generation, projectId, taskId]);
  const project = useDevelopment(state => state.projects.find(item => item.id === projectId));
  const trust = useWorkspaceTrust(projectId, taskId, surfaceActive);
  const [preferences, setPreferences] = useDesktopSlot(`runtime:${projectId}:${taskId}`, emptyRuntimePreferences);
  const [outputDraft, setOutputDraft] = useState<{ processId: string; text: string } | null>(null);
  const launchFlight = useRef(false);
  const restartFlight = useRef(false);
  const selected = runtime.taskId === taskId && runtime.processes.some(process => process.id === runtime.selected && process.project_id === projectId && process.task_id === taskId && process.source !== 'native_agent') ? runtime.selected : null;
  const retainedDetail = selected ? runtime.details[JSON.stringify([epoch, projectId, taskId, selected])] ?? null : null;
  const [catalog, setCatalog] = useState<CommandCatalog>({ commands: [], discovered: [] });
  const { draft, setField } = useRuntimeDraft(projectId, taskId);
  const { commandId, name, editorApplication, program, args, cwd, env, kind, verification, port, probePath, timeout } = draft;
  const setCommandId = (value: typeof commandId) => setField('commandId', value);
  const setName = (value: typeof name) => setField('name', value);
  const setEditorApplication = (value: typeof editorApplication) => setField('editorApplication', value);
  const setProgram = (value: typeof program) => setField('program', value);
  const setArgs = (value: typeof args | ((previous: typeof args) => typeof args)) => setField('args', value);
  const setCwd = (value: typeof cwd) => setField('cwd', value);
  const setEnv = (value: typeof env | ((previous: typeof env) => typeof env)) => setField('env', value);
  const setKind = (value: typeof kind) => setField('kind', value);
  const setVerification = (value: typeof verification) => setField('verification', value);
  const setPort = (value: typeof port) => setField('port', value);
  const setProbePath = (value: typeof probePath) => setField('probePath', value);
  const setTimeoutValue = (value: typeof timeout) => setField('timeout', value);
  const [externalAction, setExternalAction] = useState<'preview' | 'editor' | null>(null);
  const externalFlight = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [validation, setValidation] = useState<'start' | 'save' | 'terminal' | null>(null);
  const [configurationOpen, setConfigurationOpen] = useState(false);
  const [takeoverReleased, setTakeoverReleased] = useState<string | null>(null);
  const draftRevision = useRef(0);
  const currentOwner = useRef('');
  const appliedReceipts = useRef(new Set(runtime.operations.filter(operation => operation.phase === 'succeeded' || operation.phase === 'failed').map(operation => operation.id)));
  const envErrorId = useId();
  const [detailSnapshot, setDetail] = useState<DevelopmentProcess | null>(retainedDetail);
  const [detailRead, setDetailRead] = useState<ResourceReadState>(retainedDetail ? { status: 'ready', refresh: { status: 'stale' } } : { status: 'idle' });
  const [catalogRead, setCatalogRead] = useState<ResourceReadState>({ status: 'idle' });
  const detailOwner = useRef(owner);
  const catalogOwner = useRef('');
  const detailReadOwner = useRef(JSON.stringify([owner, selected]));
  currentOwner.current = owner;
  const operations = runtime.operations.filter(operation => operation.epoch === epoch && operation.projectId === projectId && operation.taskId === taskId);
  const detail = detailOwner.current === owner && detailSnapshot?.id === runtime.selected && detailSnapshot.task_id === taskId && detailSnapshot.project_id === projectId ? detailSnapshot : null;
  const detailReading = useRef<Promise<void> | null>(null);
  const detailVersion = useRef(0);
  const catalogVersion = useRef(0);
  const [catalogWake, setCatalogWake] = useState(0);
  const catalogReading = useRef<{ refresh: boolean } | null>(null);
  const activity = useRef(surfaceActive);
  activity.current = surfaceActive;
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    ++catalogVersion.current;
    setCatalog({ commands: [], discovered: [] }); setDetail(null); setError(null); setExternalAction(null); setValidation(null);
    ++draftRevision.current;
    for (const operation of useDevelopmentRuntime.getState().operations) if (operation.phase === 'succeeded' || operation.phase === 'failed') appliedReceipts.current.add(operation.id);
    setConfigurationOpen(false); setTakeoverReleased(null);
    setOutputDraft(null);
    catalogOwner.current = owner;
    setCatalogRead({ status: 'idle' }); setDetailRead({ status: 'idle' });
  }, [projectId, taskId, epoch, context.generation]);
  useEffect(() => {
    if (surfaceActive) return;
    const stale = (state: ResourceReadState): ResourceReadState => state.status === 'ready' && state.refresh.status === 'current' ? { ...state, refresh: { status: 'stale' } } : state;
    setCatalogRead(stale); setDetailRead(stale);
  }, [surfaceActive]);
  useEffect(() => {
    if (!surfaceActive || context.pending) {
      if (catalogReading.current) catalogReading.current.refresh = true;
      return;
    }
    if (catalogReading.current) return;
    const flight = { refresh: false };
    catalogReading.current = flight;
    const generation = contextGeneration();
    const version = catalogVersion.current;
    setCatalogRead(beginResourceRead);
    catalogOwner.current = owner;
    void readDevelopmentCommands(taskId).then(result => {
      if (!mounted.current || version !== catalogVersion.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      if (result.ok) { setCatalog(result.value); setCatalogRead({ status: 'ready', refresh: { status: 'current' } }); }
      else setCatalogRead(state => failResourceRead(state, errorText(result.code)));
    }).finally(() => {
      catalogReading.current = null;
      if (mounted.current && activity.current && (flight.refresh || version !== catalogVersion.current)) setCatalogWake(value => value + 1);
    });
  }, [projectId, taskId, epoch, context.generation, context.pending, surfaceActive, catalogWake]);
  useEffect(() => {
    for (const operation of runtime.operations) {
      if (operation.epoch !== epoch || operation.projectId !== projectId || operation.taskId !== taskId || appliedReceipts.current.has(operation.id) || operation.phase !== 'succeeded' || !operation.catalog) continue;
      appliedReceipts.current.add(operation.id);
      ++catalogVersion.current;
      catalogOwner.current = owner;
      setCatalog(operation.catalog); setCatalogRead({ status: 'ready', refresh: { status: 'current' } });
    }
  }, [runtime.operations, epoch, projectId, taskId, owner]);
  // The outer Agent consumer owns live-process synchronization across internal tab unmounts.
  // This panel only needs its initial snapshot and idle history while it is active.
  useEffect(() => {
    if (!surfaceActive || context.pending) return;
    void useDevelopmentRuntime.getState().refresh(taskId);
    const timer = window.setInterval(() => {
      const current = useDevelopmentRuntime.getState();
      if (current.taskId !== taskId || !current.processes.some(processActive)) void current.refresh(taskId);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [projectId, taskId, epoch, context.generation, context.pending, surfaceActive]);
  const refreshDetail = useCallback(() => {
    if (!selected || context.pending) return Promise.resolve();
    if (detailReading.current) return detailReading.current;
    const generation = contextGeneration();
    const version = detailVersion.current;
    setDetailRead(beginResourceRead);
    detailReadOwner.current = JSON.stringify([owner, selected]);
    const reading = (async () => {
      const result = await readDevelopmentProcess(selected);
      if (!mounted.current || version !== detailVersion.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration() || useDevelopmentRuntime.getState().taskId !== taskId || useDevelopmentRuntime.getState().selected !== selected) return;
      if (result.ok && result.value.id === selected && result.value.task_id === taskId && result.value.project_id === projectId) { detailOwner.current = owner; setDetail(result.value); setDetailRead({ status: 'ready', refresh: { status: 'current' } }); }
      else setDetailRead(state => failResourceRead(state, result.ok ? t('direct.targetMismatch') : errorText(result.code)));
    })().finally(() => { if (detailReading.current === reading) detailReading.current = null; });
    detailReading.current = reading;
    return reading;
  }, [selected, projectId, taskId, epoch, context.generation, context.pending]);
  useEffect(() => {
    const retained = selected ? useDevelopmentRuntime.getState().details[JSON.stringify([epoch, projectId, taskId, selected])] ?? null : null;
    detailOwner.current = owner; detailReadOwner.current = JSON.stringify([owner, selected]);
    setDetail(retained); setDetailRead(retained ? { status: 'ready', refresh: { status: 'stale' } } : { status: 'idle' });
    detailReading.current = null; ++detailVersion.current;
  }, [selected, projectId, taskId, epoch, context.generation]);
  useEffect(() => {
    if (!surfaceActive || context.pending) return;
    void refreshDetail();
    const timer = window.setInterval(() => { void refreshDetail(); }, 800);
    return () => window.clearInterval(timer);
  }, [surfaceActive, refreshDetail, context.pending]);
  async function load(command: DevelopmentCommand | null) {
    if (context.pending) return false;
    const loadingOwner = owner;
    const key = JSON.stringify([epoch, projectId, taskId]);
    const before = useDevelopmentRuntime.getState().drafts[key];
    if (before?.dirty && !await confirmAction(t('dev.discardDraft'), t('dev.discardDraft.command'), t('dev.discardDraft'))) return false;
    if (currentOwner.current !== loadingOwner || epoch !== useDevelopment.getState().epoch || before !== useDevelopmentRuntime.getState().drafts[key]) return false;
    ++draftRevision.current;
    setCommandId(command && catalog.commands.some(item => item.id === command.id) ? command.id : null);
    setName(command?.name ?? ''); setProgram(command?.program ?? ''); setArgs([...(command?.args ?? [])]); setCwd(command?.cwd ?? '');
    setEnv(Object.entries(command?.env_refs ?? {}).map(([target, source]) => ({ target, source })));
    setField('dirty', false);
    setValidation(null);
    if (!command) setConfigurationOpen(true);
    return true;
  }
  async function refreshHistory() {
    const generation = contextGeneration();
    await useDevelopmentRuntime.getState().refresh(taskId);
    if (mounted.current && activity.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) await refreshDetail();
  }
  async function execute(action: 'start' | 'save' | 'stop' | 'preview' | 'editor' | 'terminal') {
    if (context.pending) return;
    const submittingOwner = owner;
    const revision = draftRevision.current;
    const valid = () => mounted.current && currentOwner.current === submittingOwner;
    if (action === 'preview' || action === 'editor') {
      if (externalFlight.current) return;
      externalFlight.current = true; setExternalAction(action); setError(null);
      const result = await openDevelopmentExternal(projectId, taskId, action, action === 'preview' ? selected : null, action === 'editor' ? editorApplication : null);
      externalFlight.current = false;
      if (valid()) { setExternalAction(null); if (!result.ok) setError(result.code); }
      return;
    }
    if (action === 'start' || action === 'save' || action === 'terminal') {
      setValidation(action);
      const invalidConfiguration = action === 'terminal' ? false : !configValid || action === 'save' && (!name.trim() || name.length > 200 || catalogRead.status !== 'ready') || action === 'start' && kind === 'service' && !readinessValid;
      if (invalidConfiguration) setConfigurationOpen(true);
      if (invalidConfiguration) return;
    }
    if (runtimeOperationBlocked(useDevelopmentRuntime.getState().operations, projectId, taskId, action === 'terminal' ? 'start' : action)) return;
    if (action === 'save') ++catalogVersion.current;
    if (action === 'stop') ++detailVersion.current;
    const env_refs = Object.fromEntries(env.map(item => [item.target, item.source]));
    if (action === 'save') {
      const command: DevelopmentCommand = { id: commandId ?? crypto.randomUUID(), name, program, args: [...args], cwd, env_refs };
      setCommandId(command.id);
      const result = await submitRuntimeOperation({ action, projectId, taskId, command, commands: [...catalog.commands.filter(item => item.id !== command.id), command] });
      if (valid() && result?.phase === 'succeeded' && revision === draftRevision.current) setCommandId(command.id);
    } else if (action === 'start' || action === 'terminal') {
      if (action === 'start' && kind === 'service') {
        const command = catalog.commands.concat(catalog.discovered).find(item => item.program === program && JSON.stringify(item.args) === JSON.stringify(args) && item.cwd === cwd);
        if (command) setPreferences({ ...preferences, readiness: { ...preferences.readiness, [command.id]: { port: Number(port), path: probePath, timeout_ms: Number(timeout) } } });
      }
      await launch({ id: crypto.randomUUID(), task_id: taskId, kind: action === 'terminal' ? 'terminal' : kind, program: action === 'terminal' ? '/bin/sh' : program, args: action === 'terminal' ? [] : [...args], cwd: action === 'terminal' ? '' : cwd, env_refs: action === 'terminal' ? {} : env_refs, trust_workspace: false, verification: action === 'start' && kind === 'command' && verification, readiness: action === 'start' && kind === 'service' ? { port: Number(port), path: probePath, timeout_ms: Number(timeout) } : null });
    } else if (selected) {
      const result = await submitRuntimeOperation({ action, projectId, taskId, processId: selected });
      if (valid() && result?.process && useDevelopmentRuntime.getState().selected === selected) {
        ++detailVersion.current; detailOwner.current = owner; detailReadOwner.current = JSON.stringify([owner, selected]);
        setDetail(result.process); setDetailRead({ status: 'ready', refresh: { status: 'current' } });
      }
    }
  }
  const portValid = Number.isInteger(Number(port)) && Number(port) > 0 && Number(port) <= 65535;
  const pathValid = probePath.startsWith('/') && !probePath.startsWith('//') && probePath.length <= 2048 && !/[\s#\u0000-\u001f\u007f]/.test(probePath);
  const timeoutValid = Number.isInteger(Number(timeout)) && Number(timeout) >= 100 && Number(timeout) <= 120000;
  const readinessValid = portValid && pathValid && timeoutValid;
  const programValid = program.trim().length > 0 && new TextEncoder().encode(program).length <= 4096 && !program.includes('\0');
  const argsValid = args.length <= 256 && args.reduce((size, value) => size + new TextEncoder().encode(value).length, 0) <= 32768 && args.every(value => !value.includes('\0'));
  const cwdValid = new TextEncoder().encode(cwd).length <= 4096 && !cwd.startsWith('/') && !cwd.includes('\\') && !cwd.includes('\0') && cwd.split('/').every(part => part !== '..' && part !== '.git') && cwd !== '.' && !cwd.startsWith('./');
  const envValid = env.length <= 64 && env.every(item => /^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(item.target) && /^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(item.source)) && new Set(env.map(item => item.target)).size === env.length;
  const configValid = programValid && argsValid && cwdValid && envValid;
  const startBlocked = runtimeOperationBlocked(runtime.operations, projectId, taskId, 'start');
  const saveBlocked = runtimeOperationBlocked(runtime.operations, projectId, taskId, 'save');
  const stopBlocked = runtimeOperationBlocked(runtime.operations, projectId, taskId, 'stop');
  const historyRead = runtime.taskId === taskId ? runtime.read : { status: 'idle' } as const;
  const historyView = resourceView(historyRead.status === 'error' ? { ...historyRead, error: errorText(historyRead.error) } : historyRead.status === 'ready' && (historyRead.refresh.status === 'error' || historyRead.refresh.status === 'refreshing' && historyRead.refresh.error) ? { ...historyRead, refresh: { ...historyRead.refresh, error: errorText(historyRead.refresh.error!) } } : historyRead, runtime.processes.length === 0);
  const latestOperation = operations[operations.length - 1];
  const busyOperation = latestOperation?.phase === 'failed' && latestOperation.error === 'workspace_busy' ? latestOperation : null;
  const services = runtime.taskId === taskId ? runtime.processes.filter(process => process.kind === 'service') : [];
  const commands = [...catalog.commands, ...catalog.discovered];
  async function launch(input: StartProcessInput) {
    if (launchFlight.current || context.pending) return;
    const submittingOwner = owner;
    launchFlight.current = true;
    try {
      const scope = input.kind === 'service' ? 'service' : 'terminal';
      if (!trust.value?.[scope] && !await trust.grant(scope)) return;
      if (currentOwner.current !== submittingOwner) return;
      if (input.kind === 'service') {
        const workspace = await runtimeCall<{ path: string }>('read_development_workspace_trust', { input: { project_id: projectId, task_id: taskId } });
        if (currentOwner.current !== submittingOwner) return;
        if (!workspace.ok) { setError(workspace.code); return; }
        await useDevelopmentRuntime.getState().refresh(taskId, true);
        if (currentOwner.current !== submittingOwner) return;
        const state = useDevelopmentRuntime.getState();
        if (state.read.status !== 'ready' || state.read.refresh.status !== 'current') { setError('unknown'); return; }
        const expectedCwd = `${workspace.value.path.replace(/\/$/, '')}${input.cwd ? `/${input.cwd}` : ''}`;
        const existing = matchingService(state.processes, taskId, input.program, input.args, expectedCwd);
        if (existing) { runtime.select(existing.id); return; }
      }
      await submitRuntimeOperation({ action: 'start', projectId, taskId, input });
    } finally { launchFlight.current = false; }
  }
  async function runCommand(command: DevelopmentCommand, intent: RuntimeIntent) {
    const existing = intent === 'preview' ? matchingService(useDevelopmentRuntime.getState().processes, taskId, command.program, command.args, `${(trust.value?.path ?? project?.path ?? '').replace(/\/$/, '')}${command.cwd ? `/${command.cwd}` : ''}`) : null;
    if (existing) { runtime.select(existing.id); return; }
    const readiness = preferences.readiness[command.id];
    if (intent === 'preview' && !readiness) {
      if (!await load(command)) return;
      setKind('service'); setConfigurationOpen(true); setValidation('start');
      return;
    }
    await launch({ id: crypto.randomUUID(), task_id: taskId, kind: intent === 'preview' ? 'service' : 'command', program: command.program, args: command.args, cwd: command.cwd, env_refs: command.env_refs, trust_workspace: false, verification: intent === 'test' || intent === 'build', readiness: intent === 'preview' ? readiness : null });
  }
  async function restartService(process: DevelopmentProcess) {
    if (launchFlight.current || restartFlight.current || startBlocked || ['unknown', 'cleanup_pending'].includes(process.status)) return;
    const command = commands.find(item => item.program === process.program && JSON.stringify(item.args) === JSON.stringify(process.args) && `${(trust.value?.path ?? project?.path ?? '').replace(/\/$/, '')}${item.cwd ? `/${item.cwd}` : ''}` === process.cwd);
    if (!command || !preferences.readiness[command.id]) { setConfigurationOpen(true); return; }
    const restartingOwner = owner;
    restartFlight.current = true;
    try {
      if (!await confirmAction(t('runtimeWorkflow.restart'), t('runtimeWorkflow.restartConfirm'), t('runtimeWorkflow.restart'))) return;
      if (currentOwner.current !== restartingOwner) return;
      const stopped = await submitRuntimeOperation({ action: 'stop', projectId, taskId, processId: process.id });
      if (currentOwner.current !== restartingOwner || stopped?.phase !== 'succeeded') return;
      // A fresh read must confirm cleanup before a new request can exist.
      const confirmed = await readDevelopmentProcess(process.id);
      if (currentOwner.current === restartingOwner && confirmed.ok && !processActive(confirmed.value)) await runCommand(command, 'preview');
    } finally { restartFlight.current = false; }
  }
  return <section className="dev-runtime dev-review-stack" aria-label={t('runtimeWorkflow.actions')} data-testid="dev-development-runtime">
    {requestedAction ? <p role="status" className="dev-evidence-warning" key={requestedAction.nonce}>{t('runtimeWorkflow.requested', { action: t(requestedAction.kind === 'terminal' ? 'runtimeWorkflow.openTerminal' : `runtimeWorkflow.${requestedAction.kind}`) })}</p> : null}
    <dl className="dev-review-stack text-xs" data-testid="dev-runtime-context"><div><dt>{t('workbenchRuntime.project')}</dt><dd className="m-0 break-all"><span>{project?.name ?? projectId}</span>{project?.path ? <span className="block text-ink-secondary">{project.path}</span> : null}</dd></div><div><dt>{t('workbenchRuntime.directory')}</dt><dd className="m-0 break-all">{detail ? detail.cwd || '.' : t('workbenchRuntime.taskDirectory')}</dd></div><div><dt>{t('workbenchRuntime.status')}</dt><dd className="m-0">{detail ? t(`direct.status.${detail.status}`) : t('workbenchRuntime.noProcess')}</dd></div></dl>
    <p className="m-0 text-xs text-ink-secondary">{t('workbenchRuntime.lifecycle')}</p>
    {operations.length ? <section className="dev-review-stack" aria-label={t('direct.operation.receipts')} data-testid="dev-runtime-receipts">{operations.map(operation => <div key={operation.id} className="dev-review-stack">
      <p role={surfaceActive ? operation.phase === 'failed' || operation.phase === 'unknown' ? 'alert' : 'status' : undefined} className={`dev-feedback dev-feedback-${operation.phase === 'failed' ? 'error' : operation.phase === 'succeeded' ? 'success' : 'info'}`}>{operation.command?.name ?? operation.process?.program ?? t('direct.title')}: {t(`direct.operation.${operation.action}.${operation.phase}`)}</p>
      <details className="dev-technical-details"><summary>{t('agent.details')}</summary><code>{operation.targetId}</code></details>
      {operation.error ? <p className="m-0 text-xs text-ink-secondary">{operation.error === 'target_mismatch' ? t('direct.targetMismatch') : errorText(operation.error)}</p> : null}
      {operation.phase === 'unknown' || operation.action === 'stop' && operation.phase === 'accepted' ? <Button size="xs" variant="secondary" onClick={() => void retryRuntimeOperation(operation.id)}>{t('direct.operation.confirm')}</Button> : null}
      {operation.refreshError ? <><p role="alert" className="m-0 text-xs text-status-error">{t('direct.operation.refreshFailed')} {errorText(operation.refreshError)}</p><Button size="xs" variant="secondary" disabled={operation.refreshing} onClick={() => void retryRuntimeOperation(operation.id)}>{t('direct.operation.retryRead')}</Button></> : null}
      {operation.phase === 'unknown' && operation.error === 'process_not_found' ? <p className="m-0 text-xs text-ink-secondary">{t('direct.operation.notFound')}</p> : null}
    </div>)}</section> : null}
    {busyOperation && takeoverReleased !== busyOperation.id ? <ManualTakeover key={busyOperation.id} projectId={projectId} taskId={taskId} onReleased={() => setTakeoverReleased(busyOperation.id)} /> : null}
    {busyOperation && takeoverReleased === busyOperation.id ? <p role="status" className="dev-evidence-warning">{t('runtimeWorkflow.resubmit')}</p> : null}
    {services.length ? <section className="dev-review-stack" aria-label={t('runtimeWorkflow.services')}><h3 className="m-0 text-sm">{t('runtimeWorkflow.services')}</h3><nav className="dev-process-history" aria-label={t('runtimeWorkflow.services')}>{services.map(process => <Button key={process.id} size="xs" variant={selected === process.id ? 'secondary' : 'ghost'} aria-pressed={selected === process.id} onClick={() => runtime.select(process.id)}><span>{process.program}</span><span>{t(`direct.status.${process.status}`)}</span></Button>)}</nav></section> : null}
    {error ? <p role="alert" className="text-xs text-status-error">{errorText(error)}</p> : null}
    <ResourceDetail state={resourceView(catalogOwner.current === owner ? catalogRead : { status: 'idle' }, false)} retry={<Button size="xs" variant="secondary" onClick={() => setCatalogWake(value => value + 1)}>{t('direct.retryCatalog')}</Button>}>
    <div className="dev-command-catalog"><label className="dev-field">{t('direct.savedCommands')}<select value={commandId ?? ''} onChange={event => { void load(catalog.commands.find(item => item.id === event.target.value) ?? null); }}><option value="">{t('direct.newCommand')}</option>{catalog.commands.map(command => <option key={command.id} value={command.id}>{command.name}</option>)}</select></label>
      <label className="dev-field">{t('direct.discovered')}<select value="" onChange={event => { const command = catalog.discovered.find(item => item.id === event.target.value); if (command) load(command); }}><option value="">{t('direct.chooseDiscovered')}</option>{catalog.discovered.map(command => <option key={command.id} value={command.id}>{command.name}</option>)}</select></label></div>
    </ResourceDetail>
    <section className="dev-runtime-actions" aria-label={t('runtimeWorkflow.actions')}>
      {(['preview', 'test', 'build', 'run'] as const).map(intent => {
        const available = commands.filter(command => (preferences.readiness[command.id] ? 'preview' : commandIntent(command)) === intent);
        if (!available.length) return null;
        const chosen = available.find(command => command.id === preferences.choices[intent]) ?? (available.length === 1 ? available[0] : null);
        return <div className="dev-runtime-action" data-requested={requestedAction?.kind === intent || undefined} key={intent}>
          {available.length > 1 ? <label className="dev-field">{t(`runtimeWorkflow.${intent}`)}<select value={chosen?.id ?? ''} onChange={event => setPreferences({ ...preferences, choices: { ...preferences.choices, [intent]: event.target.value } })}><option value="">{t('runtimeWorkflow.chooseScript')}</option>{available.map(command => <option key={command.id} value={command.id}>{command.name}{command.cwd ? ` (${command.cwd})` : ''}</option>)}</select></label> : <span className="text-xs">{available[0].name}</span>}
          <Button size="sm" variant="secondary" disabled={!chosen || context.pending || startBlocked || trust.loading} onClick={() => { if (chosen) void runCommand(chosen, intent); }}>{t(`runtimeWorkflow.${intent}`)}</Button>
          {chosen ? <details className="dev-technical-details"><summary>{t('runtimeWorkflow.showCommand')}</summary><code>{JSON.stringify([chosen.program, ...chosen.args])}</code><p className="m-0">{chosen.cwd || trust.value?.path || project?.path}</p></details> : null}
        </div>;
      })}
      {!commands.length && catalogRead.status === 'ready' ? <div className="dev-review-stack"><p className="text-xs">{t('runtimeWorkflow.noScripts')}</p><div className="dev-review-toolbar"><Button size="xs" variant="secondary" onClick={() => setConfigurationOpen(true)}>{t('runtimeWorkflow.configuration')}</Button>{onUseOutput ? <Button size="xs" variant="secondary" onClick={() => onUseOutput(t('runtimeWorkflow.helpPrompt'))}>{t('runtimeWorkflow.askAgent')}</Button> : null}</div></div> : null}
    </section>
    <p className="m-0 text-xs text-ink-secondary">{t('direct.discoveryHint')}</p>
    <p className="m-0 break-all text-xs" data-testid="dev-runtime-draft-summary">{program ? JSON.stringify([program, ...args]) : t('runtimeWorkflow.chooseCommand')} · {t('review.cwd')}: {cwd || '.'}</p>
    <form className="dev-form" noValidate onChangeCapture={() => { ++draftRevision.current; }} onClickCapture={event => { if ((event.target as HTMLElement).closest('button[type="button"]')) ++draftRevision.current; }} onSubmit={event => { event.preventDefault(); void execute('start'); }}>
      <details className="dev-runtime-configuration" data-testid="dev-runtime-configuration" open={configurationOpen} onToggle={event => setConfigurationOpen(event.currentTarget.open)}><summary>{t('runtimeWorkflow.configuration')}</summary><div className="dev-review-stack">
    <div className="dev-review-toolbar"><Input label={t('direct.editorApplication')} value={editorApplication} onChange={event => setEditorApplication(event.target.value)} /><Button size="xs" variant="secondary" disabled={externalAction !== null || !editorApplication.trim()} onClick={() => void execute('editor')}>{t(externalAction === 'editor' ? 'direct.openingEditor' : 'direct.openEditor')}</Button></div>
      <div className="dev-command-catalog"><Input label={t('direct.commandName')} error={validation === 'save' && (!name.trim() || name.length > 200) ? t('direct.validation.name') : undefined} value={name} onChange={event => setName(event.target.value)} /><label className="dev-field">{t('direct.kind')}<select data-testid="dev-process-kind" value={kind} onChange={event => setKind(event.target.value as StartProcessInput['kind'])}>{['command', 'terminal', 'service'].map(item => <option key={item} value={item}>{t(`direct.${item}`)}</option>)}</select></label></div>
      <Input label={t('direct.program')} error={validation && validation !== 'terminal' && !programValid ? t('direct.validation.program') : undefined} data-testid="dev-process-program" value={program} onChange={event => setProgram(event.target.value)} />
      <fieldset className="dev-runtime-fields"><legend>{t('direct.arguments')}</legend>{args.map((argument, index) => <div className="dev-structured-row" key={index}><Input label={t('direct.argument', { index: index + 1 })} value={argument} onChange={event => setArgs(values => values.map((value, position) => position === index ? event.target.value : value))} /><Button size="xs" variant="ghost" type="button" aria-label={t('direct.removeArgument', { index: index + 1 })} onClick={() => setArgs(values => values.filter((_, position) => position !== index))}>{t('direct.remove')}</Button></div>)}<Button size="xs" variant="secondary" type="button" onClick={() => setArgs(values => [...values, ''])}>{t('direct.addArgument')}</Button></fieldset>
      {validation && validation !== 'terminal' && !argsValid ? <p role="alert" className="m-0 text-xs text-status-error">{t('direct.validation.args')}</p> : null}
      <Input label={t('direct.cwd')} error={validation && !cwdValid ? t('direct.validation.cwd') : undefined} data-testid="dev-process-cwd" value={cwd} onChange={event => setCwd(event.target.value)} />
      <fieldset className="dev-runtime-fields" aria-invalid={validation && !envValid ? true : undefined} aria-describedby={validation && !envValid ? envErrorId : undefined}><legend>{t('direct.environment')}</legend><p className="m-0 text-xs text-ink-secondary">{t('direct.environmentHint')}</p>{env.map((entry, index) => <div className="dev-structured-row" key={index}><Input label={t('direct.envTarget')} aria-describedby={validation && !envValid ? envErrorId : undefined} value={entry.target} onChange={event => setEnv(values => values.map((value, position) => position === index ? { ...value, target: event.target.value } : value))} /><Input label={t('direct.envSource')} aria-describedby={validation && !envValid ? envErrorId : undefined} value={entry.source} onChange={event => setEnv(values => values.map((value, position) => position === index ? { ...value, source: event.target.value } : value))} /><Button size="xs" variant="ghost" type="button" aria-label={t('direct.removeEnvironment', { index: index + 1 })} onClick={() => setEnv(values => values.filter((_, position) => position !== index))}>{t('direct.remove')}</Button></div>)}<Button size="xs" type="button" variant="secondary" onClick={() => setEnv(values => [...values, { target: '', source: '' }])}>{t('direct.addEnvironment')}</Button>
        {validation && !envValid ? <p id={envErrorId} role="alert" className="m-0 text-xs text-status-error">{t('direct.validation.env')}</p> : null}
      </fieldset>
      {kind === 'service' ? <fieldset className="dev-runtime-fields"><legend>{t('direct.readiness')}</legend><p className="m-0 text-xs text-ink-secondary">{t('direct.readinessHint')}</p><div className="dev-command-catalog"><Input type="number" min={1} max={65535} error={validation === 'start' && !portValid ? t('direct.validation.port') : undefined} label={t('direct.port')} value={port} onChange={event => setPort(event.target.value)} /><Input error={validation === 'start' && !pathValid ? t('direct.validation.path') : undefined} label={t('direct.probePath')} value={probePath} onChange={event => setProbePath(event.target.value)} /><Input type="number" min={100} max={120000} error={validation === 'start' && !timeoutValid ? t('direct.validation.timeout') : undefined} label={t('direct.timeout')} value={timeout} onChange={event => setTimeoutValue(event.target.value)} /></div></fieldset> : null}
      {kind === 'command' ? <label className="dev-checkbox"><input type="checkbox" checked={verification} onChange={event => setVerification(event.target.checked)} />{t('direct.verify')}</label> : null}
      <Button size="xs" type="button" variant="secondary" disabled={context.pending || saveBlocked} onClick={() => void execute('save')}>{t(operations.some(operation => operation.action === 'save' && ['pending', 'confirming'].includes(operation.phase)) ? 'direct.savingCommand' : 'direct.saveCommand')}</Button>
      {validation === 'save' && catalogRead.status !== 'ready' ? <p role="alert" className="m-0 text-xs text-status-error">{t('direct.validation.catalog')}</p> : null}
      </div></details>
      <details className="dev-technical-details"><summary>{t('runtimeWorkflow.permissions')}</summary><p className="text-xs break-all">{trust.value?.path || project?.path}</p>{(['terminal', 'service'] as const).map(scope => <div className="dev-review-toolbar" key={scope}><span className="text-xs">{t(`runtimeWorkflow.trust.${scope}`)}</span><Button size="xs" variant="secondary" disabled={trust.loading || context.pending} onClick={() => { void (trust.value?.[scope] ? trust.revoke(scope) : trust.grant(scope)); }}>{t(trust.value?.[scope] ? 'runtimeWorkflow.revoke' : 'runtimeWorkflow.authorize')}</Button></div>)}</details>
      {trust.error ? <p role="alert" className="text-xs text-status-error">{developmentErrorText(trust.error)}</p> : null}
      <div className="flex flex-wrap gap-2"><Button size="xs" type="submit" data-testid="dev-process-start" disabled={context.pending || startBlocked}>{t(operations.some(operation => operation.action === 'start' && ['pending', 'confirming'].includes(operation.phase)) ? 'direct.startingProcess' : 'direct.start')}</Button><Button size="xs" type="button" variant="secondary" data-testid="dev-open-terminal" disabled={context.pending || startBlocked} onClick={() => void execute('terminal')}>{t('runtimeWorkflow.openTerminal')}</Button></div>
      <p className="m-0 text-xs text-ink-secondary">{t('runtimeWorkflow.terminalHint')}</p>
    </form>
    <p className="m-0 text-xs text-ink-secondary">{t('direct.lifecycle')}</p>
    <section className="dev-review-stack" aria-label={t('direct.history')}><div className="dev-review-toolbar"><h3 className="m-0 text-sm">{t('direct.history')}</h3><Button size="xs" variant="secondary" onClick={() => void refreshHistory()}>{t('review.refresh')}</Button></div>
      <ResourceList state={historyView} emptyContent={<p className="text-xs text-ink-secondary">{t('direct.noHistory')}</p>} retry={<Button size="xs" variant="secondary" onClick={() => void useDevelopmentRuntime.getState().refresh(taskId)}>{t('direct.retryHistory')}</Button>}>
      {historyRead.status === 'ready' && historyRead.refresh.status !== 'current' && runtime.processes.length > 0 ? <p className="dev-evidence-warning">{t('direct.lastKnownHistory')}</p> : null}
      <nav className="dev-process-history" aria-label={t('direct.history')}>{runtime.taskId === taskId ? runtime.processes.map(process => <Button key={process.id} data-testid={`dev-process-history-${process.id}`} size="xs" variant={selected === process.id ? 'secondary' : 'ghost'} className="dev-change-entry" aria-pressed={selected === process.id} onClick={() => runtime.select(process.id)}><span>{t(`direct.source.${process.source}`)} · {process.program}</span><span>{t(`direct.status.${process.status}`)} · {formatDateTime(process.started_at * 1000)}</span></Button>) : null}</nav>
      </ResourceList>
      {selected ? <ResourceDetail state={resourceView(detailReadOwner.current === JSON.stringify([owner, selected]) ? detailRead : { status: 'idle' }, false)} retry={<Button size="xs" variant="secondary" onClick={() => void refreshDetail()}>{t('direct.retryDetail')}</Button>}>
      {detail && detailRead.status === 'ready' && detailRead.refresh.status !== 'current' ? <p className="dev-evidence-warning">{t('direct.lastKnown')}</p> : null}
      {detail ? <article className="dev-run dev-review-stack" data-testid="dev-process-detail"><header className="dev-review-toolbar"><strong className="text-xs">{t(`direct.source.${detail.source}`)}</strong><span role={surfaceActive ? 'status' : undefined} className="text-xs">{t(`direct.status.${detail.status}`)}</span></header><pre className="dev-event-text">{JSON.stringify([detail.program, ...detail.args])}</pre><p className="m-0 break-all text-xs">{t('review.cwd')}: {detail.cwd || '.'}</p><details className="dev-technical-details"><summary>{t('agent.details')}</summary><code>{detail.id}</code><p>{t('direct.started')}: {formatDateTime(detail.started_at * 1000)} · {t('direct.finished')}: {detail.finished_at === null ? t('direct.pending') : formatDateTime(detail.finished_at * 1000)}</p></details><p className="m-0 text-xs">{t('review.exitCode')}: {detail.exit_code ?? t('review.run.unknown')}</p>
      {detail.reason ? <p role="status" className="dev-evidence-warning">{errorText(detail.reason)}</p> : null}
      {detail.truncated ? <p role="status" className="dev-evidence-warning">{t('direct.outputTruncated')}</p> : null}
      {detail.kind === 'terminal' && detail.source === 'host_terminal' ? <Suspense fallback={<p role="status" className="text-xs">{t('review.loading')}</p>}><DevelopmentTerminal key={detail.id} process={detail} panelVisible={visible} /></Suspense> : <RuntimeOutput process={detail} disabled={!surfaceActive || context.pending} />}
      {detail.verification_status !== 'not_requested' ? <section className="dev-review-stack" aria-label={t('direct.verification')}><strong className="text-xs">{t('direct.verification')}: {t(`direct.verification.${detail.verification_status}`)}</strong><details className="dev-technical-details"><summary>{t('agent.details')}</summary><p>{t('direct.startFingerprint')}: {detail.start_fingerprint || t('direct.pending')}</p><p>{t('direct.currentFingerprint')}: {detail.fingerprint || t('direct.pending')}</p><p className="m-0">{t('direct.observationHint')}</p></details>{!detail.verification_complete ? <p className="dev-evidence-warning">{t('direct.verification.incomplete')}</p> : null}{detail.mutation_observed ? <p className="dev-evidence-warning">{t('direct.verification.changed')}</p> : null}</section> : null}
      <div className="flex flex-wrap gap-2"><Button size="xs" variant="danger" data-testid="dev-process-stop" disabled={context.pending || stopBlocked || !processActive(detail)} onClick={() => void execute('stop')}>{t(operations.some(operation => operation.action === 'stop' && operation.targetId === detail.id && ['pending', 'confirming'].includes(operation.phase)) ? 'direct.stoppingProcess' : 'direct.stop')}</Button>{detail.kind === 'service' ? <Button size="xs" variant="secondary" data-testid="dev-service-preview" disabled={externalAction !== null || detail.status !== 'ready' || !detail.preview_url} onClick={() => void execute('preview')}>{t(externalAction === 'preview' ? 'direct.openingPreview' : 'direct.preview')}</Button> : null}</div>
      {detail.kind === 'service' ? <div className="dev-review-toolbar">{detail.preview_url && detail.status === 'ready' ? <span className="text-xs break-all">{detail.preview_url}</span> : null}<Button size="xs" variant="secondary" disabled={context.pending || startBlocked || ['unknown', 'cleanup_pending'].includes(detail.status) || !commands.some(command => command.program === detail.program && JSON.stringify(command.args) === JSON.stringify(detail.args) && preferences.readiness[command.id])} onClick={() => void restartService(detail)}>{t('runtimeWorkflow.restart')}</Button>{detail.status === 'unknown' || detail.status === 'cleanup_pending' ? <Button size="xs" variant="secondary" onClick={() => void refreshDetail()}>{t('direct.operation.confirm')}</Button> : null}</div> : null}
      {onUseOutput && (detail.status === 'failed' || detail.exit_code !== null && detail.exit_code !== 0) ? <Button size="xs" variant="secondary" disabled={!surfaceActive || context.pending} onClick={() => setOutputDraft({ processId: detail.id, text: runtimeOutputExcerpt(detail) })}>{t('runtimeWorkflow.reviewOutput')}</Button> : null}
      {outputDraft?.processId === detail.id ? <section className="dev-review-stack" aria-label={t('runtimeWorkflow.reviewOutput')}><p className="text-xs">{t('runtimeWorkflow.outputPrivacy')}</p><label className="dev-field">{t('runtimeWorkflow.outputDraft')}<textarea rows={8} maxLength={12000} value={outputDraft.text} onChange={event => setOutputDraft({ processId: detail.id, text: event.target.value })} /></label><div className="dev-review-toolbar"><Button size="xs" disabled={!surfaceActive || context.pending || !outputDraft.text.trim()} onClick={() => { onUseOutput?.(outputDraft.text.slice(0, 12000)); setOutputDraft(null); }}>{t('runtimeWorkflow.useOutput')}</Button><Button size="xs" variant="ghost" onClick={() => setOutputDraft(null)}>{t('runtimeWorkflow.cancel')}</Button></div></section> : null}
      </article> : null}
      </ResourceDetail> : null}
    </section>
  </section>;
}
