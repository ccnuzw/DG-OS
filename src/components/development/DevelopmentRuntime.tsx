import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { formatDateTime, useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { readDevelopmentCommands, saveDevelopmentCommands, startDevelopmentProcess, readDevelopmentProcess, stopDevelopmentProcess, openDevelopmentExternal, processActive, useDevelopmentRuntime, type CommandCatalog, type DevelopmentCommand, type DevelopmentProcess, type StartProcessInput } from '../../stores/developmentRuntime';
import { errorText } from './shared';
const DevelopmentTerminal = lazy(() => import('./DevelopmentTerminal').then(module => ({ default: module.DevelopmentTerminal })));

export function DevelopmentRuntime({ projectId, taskId }: { projectId: string; taskId: string }) {
  const { t } = useLocale();
  const runtime = useDevelopmentRuntime();
  const [catalog, setCatalog] = useState<CommandCatalog>({ commands: [], discovered: [] });
  const [commandId, setCommandId] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [editorApplication, setEditorApplication] = useState('');
  const [program, setProgram] = useState('');
  const [args, setArgs] = useState<string[]>([]);
  const [cwd, setCwd] = useState('');
  const [env, setEnv] = useState<{ target: string; source: string }[]>([]);
  const [kind, setKind] = useState<StartProcessInput['kind']>('command');
  const [trusted, setTrusted] = useState(false);
  const [verification, setVerification] = useState(false);
  const [port, setPort] = useState('');
  const [probePath, setProbePath] = useState('/');
  const [timeout, setTimeoutValue] = useState('10000');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [detail, setDetail] = useState<DevelopmentProcess | null>(null);
  const detailTicket = useRef(0);
  const selected = runtime.taskId === taskId ? runtime.selected : null;
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    let active = true;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    void readDevelopmentCommands(projectId).then(result => {
      if (!active || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      if (result.ok) setCatalog(result.value); else setError(result.code);
    });
    let reading = false;
    const refreshHistory = async () => {
      if (reading || !active) return;
      reading = true;
      await useDevelopmentRuntime.getState().refresh(taskId);
      reading = false;
    };
    void refreshHistory();
    const timer = window.setInterval(() => { void refreshHistory(); }, 1000);
    return () => { active = false; window.clearInterval(timer); };
  }, [projectId, taskId]);
  useEffect(() => {
    const ticket = ++detailTicket.current;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    setDetail(null);
    if (!selected) return;
    let inFlight = false;
    const refresh = async () => {
      if (inFlight) return;
      inFlight = true;
      const result = await readDevelopmentProcess(selected);
      inFlight = false;
      if (ticket !== detailTicket.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      if (result.ok && result.value.task_id === taskId && result.value.project_id === projectId) setDetail(result.value); else if (!result.ok) setError(result.code);
    };
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 800);
    return () => { ++detailTicket.current; window.clearInterval(timer); };
  }, [selected]);
  function load(command: DevelopmentCommand) {
    setCommandId(catalog.commands.some(item => item.id === command.id) ? command.id : null);
    setName(command.name); setProgram(command.program); setArgs([...command.args]); setCwd(command.cwd);
    setEnv(Object.entries(command.env_refs).map(([target, source]) => ({ target, source }))); setSaved(false);
  }
  async function execute(action: 'start' | 'save' | 'stop' | 'preview' | 'editor') {
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    const valid = () => mounted.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
    setBusy(true); setError(null); setSaved(false);
    const env_refs = Object.fromEntries(env.filter(item => item.target && item.source).map(item => [item.target, item.source]));
    if (action === 'save') {
      const command: DevelopmentCommand = { id: commandId ?? crypto.randomUUID(), name, program, args, cwd, env_refs };
      const result = await saveDevelopmentCommands(projectId, [...catalog.commands.filter(item => item.id !== command.id), command]);
      if (!valid()) return;
      if (result.ok) { setCatalog(result.value); setCommandId(command.id); setSaved(true); } else setError(result.code);
    } else if (action === 'start') {
      const result = await startDevelopmentProcess({ id: crypto.randomUUID(), task_id: taskId, kind, program, args, cwd, env_refs, trust_workspace: trusted, verification: kind === 'command' && verification, readiness: kind === 'service' ? { port: Number(port), path: probePath, timeout_ms: Number(timeout) } : null });
      if (!valid()) return;
      if (result.ok) { await useDevelopmentRuntime.getState().refresh(taskId); if (valid()) useDevelopmentRuntime.getState().select(result.value.id); } else setError(result.code);
    } else if (action === 'stop') {
      if (!selected) { setBusy(false); return; }
      const result = await stopDevelopmentProcess(selected);
      if (!valid()) return;
      if (result.ok && useDevelopmentRuntime.getState().selected === result.value.id) { setDetail(result.value); await useDevelopmentRuntime.getState().refresh(taskId); } else if (!result.ok) setError(result.code);
    } else {
      const result = await openDevelopmentExternal(projectId, action, action === 'preview' ? selected : null, action === 'editor' ? editorApplication : null);
      if (!valid()) return;
      if (!result.ok) setError(result.code);
    }
    if (valid()) setBusy(false);
  }
  const readinessValid = Number.isInteger(Number(port)) && Number(port) > 0 && Number(port) <= 65535 && probePath.startsWith('/') && !probePath.startsWith('//') && !/[\s#]/.test(probePath) && Number.isInteger(Number(timeout)) && Number(timeout) >= 100 && Number(timeout) <= 120000;
  const configValid = program.trim().length > 0 && env.every(item => /^[A-Za-z_][A-Za-z0-9_]*$/.test(item.target) && /^[A-Za-z_][A-Za-z0-9_]*$/.test(item.source)) && new Set(env.map(item => item.target)).size === env.length;
  return <section className="dev-review-stack" aria-label={t('direct.title')} data-testid="dev-development-runtime">
    <p className="m-0 text-xs text-muted-foreground">{t('direct.lifecycle')}</p>
    <div className="dev-review-toolbar"><Input label={t('direct.editorApplication')} value={editorApplication} onChange={event => setEditorApplication(event.target.value)} /><Button size="xs" variant="secondary" disabled={busy || !editorApplication.trim()} onClick={() => void execute('editor')}>{t('direct.openEditor')}</Button></div>
    {error ?? runtime.error ? <p role="alert" className="text-xs text-destructive">{errorText(error ?? runtime.error ?? 'unknown')}</p> : null}
    <p className="dev-evidence-warning">{t('direct.closeAgent')}</p>
    <div className="dev-command-catalog"><label className="dev-field">{t('direct.savedCommands')}<select value={commandId ?? ''} onChange={event => { const command = catalog.commands.find(item => item.id === event.target.value); if (command) load(command); else { setCommandId(null); setName(''); setProgram(''); setArgs([]); setCwd(''); setEnv([]); } }}><option value="">{t('direct.newCommand')}</option>{catalog.commands.map(command => <option key={command.id} value={command.id}>{command.name}</option>)}</select></label>
      <label className="dev-field">{t('direct.discovered')}<select value="" onChange={event => { const command = catalog.discovered.find(item => item.id === event.target.value); if (command) load(command); }}><option value="">{t('direct.chooseDiscovered')}</option>{catalog.discovered.map(command => <option key={command.id} value={command.id}>{command.name}</option>)}</select></label></div>
    <p className="m-0 text-xs text-muted-foreground">{t('direct.discoveryHint')}</p>
    <form className="dev-form" onSubmit={event => { event.preventDefault(); if (trusted && configValid && (kind !== 'service' || readinessValid)) void execute('start'); }}>
      <div className="dev-command-catalog"><Input label={t('direct.commandName')} value={name} onChange={event => setName(event.target.value)} /><label className="dev-field">{t('direct.kind')}<select data-testid="dev-process-kind" value={kind} onChange={event => setKind(event.target.value as StartProcessInput['kind'])}>{['command', 'terminal', 'service'].map(item => <option key={item} value={item}>{t(`direct.${item}`)}</option>)}</select></label></div>
      <Input label={t('direct.program')} data-testid="dev-process-program" value={program} onChange={event => setProgram(event.target.value)} />
      <fieldset className="dev-runtime-fields"><legend>{t('direct.arguments')}</legend>{args.map((argument, index) => <div className="dev-structured-row" key={index}><Input label={t('direct.argument', { index: index + 1 })} value={argument} onChange={event => setArgs(values => values.map((value, position) => position === index ? event.target.value : value))} /><Button size="xs" variant="ghost" type="button" aria-label={t('direct.removeArgument', { index: index + 1 })} onClick={() => setArgs(values => values.filter((_, position) => position !== index))}>{t('direct.remove')}</Button></div>)}<Button size="xs" variant="secondary" type="button" onClick={() => setArgs(values => [...values, ''])}>{t('direct.addArgument')}</Button></fieldset>
      <Input label={t('direct.cwd')} data-testid="dev-process-cwd" value={cwd} onChange={event => setCwd(event.target.value)} />
      <fieldset className="dev-runtime-fields"><legend>{t('direct.environment')}</legend><p className="m-0 text-xs text-muted-foreground">{t('direct.environmentHint')}</p>{env.map((entry, index) => <div className="dev-structured-row" key={index}><Input label={t('direct.envTarget')} value={entry.target} onChange={event => setEnv(values => values.map((value, position) => position === index ? { ...value, target: event.target.value } : value))} /><Input label={t('direct.envSource')} value={entry.source} onChange={event => setEnv(values => values.map((value, position) => position === index ? { ...value, source: event.target.value } : value))} /><Button size="xs" variant="ghost" type="button" aria-label={t('direct.removeEnvironment', { index: index + 1 })} onClick={() => setEnv(values => values.filter((_, position) => position !== index))}>{t('direct.remove')}</Button></div>)}<Button size="xs" type="button" variant="secondary" onClick={() => setEnv(values => [...values, { target: '', source: '' }])}>{t('direct.addEnvironment')}</Button></fieldset>
      {kind === 'service' ? <fieldset className="dev-runtime-fields"><legend>{t('direct.readiness')}</legend><p className="m-0 text-xs text-muted-foreground">{t('direct.readinessHint')}</p><div className="dev-command-catalog"><Input type="number" min={1} max={65535} label={t('direct.port')} value={port} onChange={event => setPort(event.target.value)} /><Input label={t('direct.probePath')} value={probePath} onChange={event => setProbePath(event.target.value)} /><Input type="number" min={1} label={t('direct.timeout')} value={timeout} onChange={event => setTimeoutValue(event.target.value)} /></div></fieldset> : null}
      {kind === 'command' ? <label className="dev-checkbox"><input type="checkbox" checked={verification} onChange={event => setVerification(event.target.checked)} />{t('direct.verify')}</label> : null}
      <label className="dev-checkbox"><input type="checkbox" data-testid="dev-process-trust" checked={trusted} onChange={event => setTrusted(event.target.checked)} />{t('direct.trust')}</label>
      <div className="flex flex-wrap gap-2"><Button size="xs" type="submit" data-testid="dev-process-start" disabled={busy || !trusted || !configValid || kind === 'service' && !readinessValid}>{t(busy ? 'direct.working' : 'direct.start')}</Button><Button size="xs" type="button" variant="secondary" disabled={busy || !configValid || !name.trim()} onClick={() => void execute('save')}>{t('direct.saveCommand')}</Button>{saved ? <span role="status" className="text-xs">{t('direct.commandSaved')}</span> : null}</div>
    </form>
    <section className="dev-review-stack" aria-label={t('direct.history')}><div className="dev-review-toolbar"><h3 className="m-0 text-sm">{t('direct.history')}</h3><Button size="xs" variant="secondary" onClick={() => void runtime.refresh(taskId)}>{t('review.refresh')}</Button></div>
      <nav className="dev-process-history" aria-label={t('direct.history')}>{runtime.taskId === taskId ? runtime.processes.map(process => <Button key={process.id} data-testid={`dev-process-history-${process.id}`} size="xs" variant={selected === process.id ? 'secondary' : 'ghost'} className="dev-change-entry" aria-pressed={selected === process.id} onClick={() => runtime.select(process.id)}><span>{t(`direct.source.${process.source}`)} · {process.program}</span><span>{t(`direct.status.${process.status}`)} · {formatDateTime(process.started_at * 1000)}</span></Button>) : null}</nav>
      {!runtime.processes.length ? <p className="text-xs text-muted-foreground">{t('direct.noHistory')}</p> : null}
      {detail ? <article className="dev-run dev-review-stack" data-testid="dev-process-detail"><header className="dev-review-toolbar"><strong className="text-xs">{t(`direct.source.${detail.source}`)}</strong><span role="status" className="text-xs">{t(`direct.status.${detail.status}`)}</span></header><pre className="dev-event-text">{JSON.stringify([detail.program, ...detail.args])}</pre><p className="m-0 break-all text-xs">{t('review.cwd')}: {detail.cwd || '.'}</p><p className="m-0 break-all text-xs">{detail.id}</p><p className="m-0 text-xs">{t('direct.started')}: {formatDateTime(detail.started_at * 1000)} · {t('direct.finished')}: {detail.finished_at === null ? t('direct.pending') : formatDateTime(detail.finished_at * 1000)}</p><p className="m-0 text-xs">{t('review.exitCode')}: {detail.exit_code ?? t('review.run.unknown')}</p>
      {detail.reason ? <p role="status" className="dev-evidence-warning">{errorText(detail.reason)}</p> : null}
      <section className="dev-review-stack" aria-label={t('direct.verification')}><strong className="text-xs">{t('direct.verification')}: {t(`direct.verification.${detail.verification_status}`)}</strong><p className="m-0 break-all text-xs">{t('direct.startFingerprint')}: {detail.start_fingerprint || t('direct.pending')}</p><p className="m-0 break-all text-xs">{t('direct.currentFingerprint')}: {detail.fingerprint || t('direct.pending')}</p><p className="m-0 text-xs text-muted-foreground">{t('direct.observationHint')}</p>{!detail.verification_complete && detail.verification_status !== 'not_requested' ? <p className="dev-evidence-warning">{t('direct.verification.incomplete')}</p> : null}{detail.mutation_observed ? <p className="dev-evidence-warning">{t('direct.verification.changed')}</p> : null}</section>
      {detail.truncated ? <p role="status" className="dev-evidence-warning">{t('direct.outputTruncated')}</p> : null}
      {detail.kind === 'terminal' ? <Suspense fallback={<p role="status" className="text-xs">{t('review.loading')}</p>}><DevelopmentTerminal key={detail.id} process={detail} onError={setError} /></Suspense> : <pre className="dev-diff" tabIndex={0} aria-label={t('direct.output')}>{detail.output || t('direct.noOutput')}</pre>}
      <div className="flex flex-wrap gap-2"><Button size="xs" variant="danger" data-testid="dev-process-stop" disabled={busy || !processActive(detail)} onClick={() => void execute('stop')}>{t('direct.stop')}</Button>{detail.kind === 'service' ? <Button size="xs" variant="secondary" data-testid="dev-service-preview" disabled={busy || detail.status !== 'ready' || !detail.preview_url} onClick={() => void execute('preview')}>{t('direct.preview')}</Button> : null}</div>
      </article> : null}
    </section>
  </section>;
}
