import { useEffect, useState } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentOpenInput, type Engine } from '../../stores/agentRunnerStore';
import { useDevelopment, type DevelopmentSession } from '../../stores/developmentStore';
import { chooseContext, contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceCloseGuard, useSurfaceStatus } from '../../workspace/SurfaceContext';
import { confirmDiscard, prepareAgentClose } from '../../workspace/closeCoordinator';
import { confirmAction, errorText } from '../../components/development/shared';
import { ProjectPanel } from '../../components/development/ProjectPanel';
import { TaskPanel } from '../../components/development/TaskPanel';
import { SessionPanel } from '../../components/development/SessionPanel';
import { UnclassifiedPanel } from '../../components/development/UnclassifiedPanel';
import { ProjectReview } from '../../components/development/ProjectReview';
import { AgentConversation } from './AgentConversation';
import { useDevelopmentEditor, hasDevelopmentDraft, guardDevelopmentDraft, discardDevelopmentDraft } from '../../stores/developmentEditor';
import { useDevelopmentRuntime, processActive } from '../../stores/developmentRuntime';
import '../../components/development/development.css';
import { useProjectCapabilities, type CapabilityRevision } from '../../stores/developmentCapabilities';
import { hostCode } from '../../stores/hostError';
import { SessionCapabilitySnapshot } from '../../components/development/CapabilitySnapshotView';
import './workbench.css';

const RESTING = ['idle', 'closed', 'error'];

export default function AgentWorkbench() {
  const { t } = useLocale();
  const development = useDevelopment();
  const runner = useAgentRunner();
  const context = useContextSelection();
  const fileEditor = useDevelopmentEditor();
  const hostRuntime = useDevelopmentRuntime();
  const [engine, setEngine] = useState<Engine>('codex');
  const [model, setModel] = useState('');
  const [trusted, setTrusted] = useState(false);
  const [drafts, setDrafts] = useState<{ [sessionId: string]: string }>({});
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const project = development.projects.find(item => item.id === development.selection.project_id) ?? null;
  const task = development.tasks.find(item => item.id === development.selection.task_id) ?? null;
  const sessions = task ? development.sessions.filter(item => item.task_id === task.id) : [];
  const unclassified = development.sessions.filter(item => item.task_id === null);
  const selectedSaved = development.sessions.find(item => item.id === development.selection.session_id) ?? null;
  const projectPath = project?.path ?? null;
  const projectCapabilities = useProjectCapabilities(project?.id ?? null);
  const [capabilityChoice, setCapabilityChoice] = useState<{ projectId: string; epoch: number; revision: CapabilityRevision } | null>(null);
  const selectedCapabilities = capabilityChoice && capabilityChoice.projectId === project?.id && capabilityChoice.epoch === development.epoch ? capabilityChoice.revision : null;
  // Trust is per directory: changing the project or its path requires a new explicit confirmation.
  useEffect(() => { setTrusted(false); }, [projectPath]);
  useEffect(() => { setBusy(false); setFailure(null); }, [development.epoch, context.generation]);
  useEffect(() => { setDrafts({}); setTrusted(false); }, [development.epoch]);
  const liveActive = Object.values(runner.live).some(status => !RESTING.includes(status));
  const dirty = Object.values(drafts).some(value => value.trim()) || (fileEditor.file !== null && hasDevelopmentDraft());
  useSurfaceStatus({ dirty, running: liveActive || runner.busy || hostRuntime.processes.some(processActive) });
  useEffect(() => { void useDevelopment.getState().load(); }, []);
  useEffect(() => {
    if (!runner.session || context.pending) return;
    const timer = window.setTimeout(() => { void useDevelopment.getState().load(); }, 500);
    return () => window.clearTimeout(timer);
  }, [runner.session?.id, runner.session?.status, context.pending]);
  useSurfaceCloseGuard(async () => {
    if (!await guardDevelopmentDraft()) return false;
    if (liveActive) {
      const accepted = await confirmAction(t('agent.title'), t('shell.closeRunningConfirm'), t('shell.stopAndClose'));
      if (!accepted || !await prepareAgentClose()) return false;
    } else if (Object.values(drafts).some(value => value.trim()) && !await confirmDiscard()) return false;
    useDevelopmentEditor.getState().reset();
    return true;
  });

  // Starts a new runtime or resumes a saved one. Each call is one explicit user action and never retries. Both the
  // development and runtime account epochs are captured first; a response that crosses an account change is dropped.
  async function start(input: AgentOpenInput, projectId: string, taskId: string, sessionId: string | null) {
    if (!await discardDevelopmentDraft()) return;
    const devEpoch = useDevelopment.getState().epoch;
    const runEpoch = useAgentRunner.getState().epoch;
    const generation = contextGeneration();
    setBusy(true); setFailure(null);
    await runner.open(input);
    if (useDevelopment.getState().epoch !== devEpoch || useAgentRunner.getState().epoch !== runEpoch || contextGeneration() !== generation) return;
    const opened = useAgentRunner.getState();
    if (opened.error || opened.session?.id !== input.id) { setFailure(opened.error ?? 'unknown'); setBusy(false); return; }
    await useDevelopment.getState().load();
    if (useDevelopment.getState().epoch !== devEpoch || useAgentRunner.getState().epoch !== runEpoch || contextGeneration() !== generation) return;
    const selecting = chooseContext({ project_id: projectId, task_id: taskId, session_id: sessionId ?? opened.session.id });
    const selectionGeneration = contextGeneration();
    // chooseContext publishes its settlement once; any further generation belongs to another selection intent.
    const selected = await selecting;
    if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== selectionGeneration + 1) return;
    if (!selected.ok) setFailure(selected.code);
    setBusy(false);
  }
  function startNew() {
    if (!project || !task) return;
    const selectedEngine = selectedCapabilities?.configuration.engine ?? engine;
    void start({ id: crypto.randomUUID(), engine: selectedEngine, task_id: task.id, capability_revision: selectedCapabilities?.revision ?? 0, options: { workspace: project.path, resume_id: null, executable: null, model: selectedCapabilities ? null : model || null, trust_workspace: trusted } }, project.id, task.id, null);
  }
  async function resume(session: DevelopmentSession) {
    if (!await discardDevelopmentDraft()) return;
    if (!project || !session.task_id) return;
    if (runner.live[session.id]) {
      // The runtime is still running: read its snapshot only, no process is started.
      const devEpoch = useDevelopment.getState().epoch;
      const generation = contextGeneration();
      setBusy(true);
      void runner.attach(session.id).then(async () => {
        if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== generation) return;
        const attached = useAgentRunner.getState();
        if (attached.error || attached.session?.id !== session.id) { setFailure(attached.error ?? 'unknown'); setBusy(false); return; }
        const selecting = chooseContext({ project_id: project.id, task_id: session.task_id, session_id: session.id });
        const selectionGeneration = contextGeneration();
        const selected = await selecting;
        if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== selectionGeneration + 1) return;
        if (!selected.ok) setFailure(selected.code);
        setBusy(false);
      });
      return;
    }
    void start({ id: session.id, engine: session.engine, task_id: session.task_id, options: { workspace: session.workspace, resume_id: null, executable: null, model: session.model, trust_workspace: trusted } }, project.id, session.task_id, session.id);
  }
  async function classify(session: DevelopmentSession) {
    if (!task) return;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    const result = await development.classify(session.id, task.id);
    if (epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setFailure(result.ok ? null : result.code);
  }
  const current = context.pending ? null : runner.session;
  const failureCode = failure ?? development.error ?? runner.error;
  return <section className="agent-workbench">
    <header className="task-header"><div><h1 className="m-0 text-lg font-semibold">{t('dev.title')}</h1><p className="mt-1 text-sm text-muted-foreground">{t('agent.subtitle')}</p></div></header>
    {failureCode ? <p role="alert" className="text-sm text-destructive">{errorText(failureCode)}</p> : null}
    <div className="dev-columns">
      <aside className="dev-column"><ProjectPanel /></aside>
      <aside className="dev-column">
        {project ? <>
          <TaskPanel project={project} />
          <SessionPanel project={project} task={task} sessions={sessions} live={runner.live} selectedId={runner.selected}
            trusted={trusted} onTrusted={setTrusted} engine={engine} onEngine={setEngine} model={model} onModel={setModel} busy={context.pending || busy || runner.busy}
            projectCapabilities={projectCapabilities.value} selectedCapabilities={selectedCapabilities} capabilitiesLoading={projectCapabilities.loading} capabilitiesError={projectCapabilities.error ? errorText(hostCode(projectCapabilities.error)) : null}
            onCapabilities={revision => setCapabilityChoice(revision && project ? { projectId: project.id, epoch: development.epoch, revision } : null)}
            onStart={startNew} onResume={resume} />
          <UnclassifiedPanel sessions={unclassified} taskId={task?.id ?? null} busy={context.pending || busy || runner.busy} onClassify={session => void classify(session)} />
        </> : <><p className="text-sm text-muted-foreground">{t('dev.none')}</p><UnclassifiedPanel sessions={unclassified} taskId={null} busy={context.pending || busy || runner.busy} onClassify={() => {}} /></>}
      </aside>
      <div className="agent-conversation">
        {task ? <div className="dev-goal" data-testid="dev-task-summary"><strong>{t('dev.taskFor', { goal: task.goal })}</strong><p className="m-0 whitespace-pre-wrap text-sm">{task.requirements}</p></div> : <p className="text-sm text-muted-foreground">{t('dev.noTaskSelected')}</p>}
        {project && !context.pending ? <ProjectReview key={`${development.epoch}:${context.generation}:${project.id}:${task?.id ?? ''}`} projectId={project.id} taskId={task?.id ?? null} /> : null}
        {current ? <AgentConversation key={`${development.epoch}:${current.id}`} session={current} drafts={drafts} setDraft={(id, value) => { if (useDevelopment.getState().epoch === development.epoch) setDrafts(state => ({ ...state, [id]: value })); }} busy={busy || runner.busy} error={Boolean(runner.error)} />
          : !context.pending && selectedSaved && selectedSaved.task_id ? <div className="grid gap-2" data-testid="dev-resume-entry"><p className="m-0 text-sm text-muted-foreground">{t('dev.resumeNotStarted')}</p>
            <SessionCapabilitySnapshot key={selectedSaved.id} sessionId={selectedSaved.id} snapshot={undefined} />
            <Button size="xs" variant="secondary" disabled={busy || !trusted} onClick={() => resume(selectedSaved)}>{t('dev.trustResume')}</Button></div>
          : <p className="text-sm text-muted-foreground">{t('agent.choose')}</p>}
      </div>
    </div>
  </section>;
}
