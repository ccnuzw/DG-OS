import { useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { Dialog } from 'radix-ui';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentOpenInput, type Engine } from '../../stores/agentRunnerStore';
import { useDevelopment, type DevelopmentSession, type Outcome } from '../../stores/developmentStore';
import { chooseContext, contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceActive, useSurfaceCloseGuard, useSurfaceStatus } from '../../workspace/SurfaceContext';
import { confirmDiscard, prepareAgentClose } from '../../workspace/closeCoordinator';
import { confirmAction, errorText } from '../../components/development/shared';
import { ProjectPanel } from '../../components/development/ProjectPanel';
import { TaskPanel } from '../../components/development/TaskPanel';
import { SessionPanel } from '../../components/development/SessionPanel';
import { UnclassifiedPanel } from '../../components/development/UnclassifiedPanel';
import { ProjectReview, Runs, type FileTarget } from '../../components/development/ProjectReview';
import { AgentConversation } from './AgentConversation';
import { useDevelopmentEditor, hasDevelopmentDraft, guardDevelopmentDraft, discardDevelopmentDraft } from '../../stores/developmentEditor';
import { useDevelopmentRuntime, processActive } from '../../stores/developmentRuntime';
import '../../components/development/development.css';
import { useProjectCapabilities, type CapabilityRevision } from '../../stores/developmentCapabilities';
import { hostCode } from '../../stores/hostError';
import { SessionCapabilitySnapshot } from '../../components/development/CapabilitySnapshotView';
import './workbench.css';
import { readDevelopmentWorkspaces, readDevelopmentGit } from '../../stores/developmentDelivery';
import { useDevelopmentDrafts, guardWorkbenchDrafts } from '../../stores/developmentDrafts';
import { useHostCapabilityConfirmations } from '../../components/development/HostCapabilityConfirmations';
import { invoke } from '@tauri-apps/api/core';
import type { AgentSession } from '../../stores/agentRunnerStore';
import { listCapabilityConfirmations } from '../../stores/developmentCapabilities';
import { PanelResizer, useWorkbenchLayout } from '../../components/development/WorkbenchLayout';
import { DevelopmentRuntime } from '../../components/development/DevelopmentRuntime';
import '../../components/development/workbench-layout.css';
import { TaskLaunch } from '../../components/development/TaskLaunch';
import { resolveDevelopmentFileReference, type AgentOutputReference } from '../../components/development/eventOutputReferences';

const RESTING = ['idle', 'closed', 'error'];

export default function AgentWorkbench() {
  const { t } = useLocale();
  const development = useDevelopment();
  const runner = useAgentRunner();
  const context = useContextSelection();
  const surfaceActive = useSurfaceActive();
  const fileEditor = useDevelopmentEditor();
  const hostRuntime = useDevelopmentRuntime();
  const editorDrafts = useDevelopmentDrafts(state => state.entries);
  const confirmations = useHostCapabilityConfirmations(context.pending ? null : runner.session?.status === 'closed' ? null : runner.session?.id ?? null);
  const [otherApprovals, setOtherApprovals] = useState<{ sessionId: string; count: number }[]>([]);
  const [engine, setEngine] = useState<Engine>('codex');
  const [model, setModel] = useState('');
  const [trusted, setTrusted] = useState(false);
  const [drafts, setDrafts] = useState<{ [sessionId: string]: string }>({});
  const [taskWorkspace, setTaskWorkspace] = useState<{taskId:string;path:string;kind:string;branch:string|null;head:string|null}|null>(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [workspaceRefresh, setWorkspaceRefresh] = useState(0);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const { root, layout, setLayout, compact } = useWorkbenchLayout();
  const [launch, setLaunch] = useState<{ taskId: string | null; revision: number | null; nonce: number } | null>(null);
  const [fileTarget, setFileTarget] = useState<FileTarget | null>(null);
  const [runTarget, setRunTarget] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const launchTrigger = useRef<HTMLButtonElement>(null);
  const navigationTrigger = useRef<HTMLButtonElement>(null);
  const agentTrigger = useRef<HTMLButtonElement>(null);
  const runtimeTrigger = useRef<HTMLButtonElement>(null);
  const navigation = useRef<HTMLElement>(null);
  const agentPanel = useRef<HTMLDivElement>(null);
  const navigationOpen = compact ? drawerOpen : layout.navigation;
  const agentVisible = compact ? layout.compactView === 'agent' : layout.agent;
  const codeVisible = !compact || layout.compactView === 'code';
  function revealAgent(approval = false) {
    setLayout(value => ({ ...value, agent: true, compactView: 'agent' }));
    window.requestAnimationFrame(() => {
      const target = approval ? agentPanel.current?.querySelector<HTMLElement>('.agent-dialogue-requests section') : agentPanel.current?.querySelector<HTMLElement>('textarea');
      if (target) { target.tabIndex = target.tabIndex < 0 ? -1 : target.tabIndex; target.focus(); target.scrollIntoView?.({ block: 'nearest' }); }
    });
  }
  const wasDrawerOpen = useRef(false);
  function closeNavigation() { setDrawerOpen(false); }
  useEffect(() => {
    if (compact && drawerOpen) navigation.current?.querySelector<HTMLElement>('button, input, summary')?.focus();
    else if (wasDrawerOpen.current && (navigation.current?.contains(document.activeElement) || document.activeElement === document.body)) navigationTrigger.current?.focus();
    wasDrawerOpen.current = compact && drawerOpen;
  }, [compact, drawerOpen]);
  const project = development.projects.find(item => item.id === development.selection.project_id) ?? null;
  const task = development.tasks.find(item => item.id === development.selection.task_id) ?? null;
  const sessions = task ? development.sessions.filter(item => item.task_id === task.id) : [];
  const unclassified = development.sessions.filter(item => item.task_id === null);
  const selectedSaved = development.sessions.find(item => item.id === development.selection.session_id) ?? null;
  const projectPath = project?.path ?? null;
  const projectCapabilities = useProjectCapabilities(project?.id ?? null);
  const [capabilityChoice, setCapabilityChoice] = useState<{ projectId: string; epoch: number; revision: CapabilityRevision } | null>(null);
  const selectedCapabilities = capabilityChoice && capabilityChoice.projectId === project?.id && capabilityChoice.epoch === development.epoch ? capabilityChoice.revision : null;
  function showCode(view: 'files' | 'changes' | 'review' | 'capabilities' | 'delivery') { setLayout(value => ({ ...value, compactView: 'code', codeView: view })); }
  function showRuntime() { setLayout(value => ({ ...value, runtime: true, runtimeView: 'processes' })); }
  function manageProjects() { setLaunch(null); setDrawerOpen(true); setLayout(value => ({ ...value, navigation: true })); window.requestAnimationFrame(() => navigation.current?.querySelector<HTMLElement>('[data-testid="dev-project-create-open"]')?.focus()); }
  function useConfiguration(revision: CapabilityRevision) { setLaunch({ taskId: task?.id ?? null, revision: revision.revision, nonce: Date.now() }); }
  async function openReference(reference: AgentOutputReference, mode: 'file' | 'diff') {
    if (!project || !task || !runner.session || context.pending) throw Error('invalid_selection');
    const epoch = development.epoch, generation = contextGeneration(), sessionId = runner.session.id;
    const result = await resolveDevelopmentFileReference({ project_id: project.id, task_id: task.id, session_id: sessionId, path: reference.path, mode, line: reference.line });
    const valid = () => epoch === useDevelopment.getState().epoch && generation === contextGeneration() && useAgentRunner.getState().session?.id === sessionId;
    if (!valid()) return;
    if (!result.ok) { setFailure(result.code); throw Error(result.code); }
    if (!await discardDevelopmentDraft() || !valid()) return;
    setFileTarget({ path: result.value.path, line: result.value.line ?? undefined, mode, nonce: Date.now() });
    showCode(mode === 'file' ? 'files' : 'changes');
  }
  // Trust is per directory: changing the project or its path requires a new explicit confirmation.
  useEffect(() => { setTrusted(false); }, [projectPath, task?.id, development.epoch]);
  useEffect(() => {
    setTaskWorkspace(null); setWorkspaceError(null);
    if (!project || !task || context.pending) return;
    let active = true; const epoch = development.epoch, generation = contextGeneration();
    void Promise.all([readDevelopmentWorkspaces({ project_id: project.id, task_id: task.id }), readDevelopmentGit(task.id)]).then(([result, git]) => {
      if (!active || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      if (!result.ok) { setWorkspaceError(result.code); return; }
      const selected = result.value.workspaces.find(item => result.value.selected_workspace_id ? item.id === result.value.selected_workspace_id : item.kind === 'original');
      if (!selected) { setWorkspaceError('workspace_not_found'); return; }
      setTaskWorkspace({ taskId: task.id, path: selected.path, kind: selected.kind, branch: git.ok ? git.value.branch : null, head: git.ok ? git.value.head : null });
      if (!git.ok) setWorkspaceError(git.code);
    });
    return () => { active = false; };
  }, [projectPath, task?.id, development.epoch, context.generation, context.pending, workspaceRefresh]);
  useEffect(() => { setBusy(false); setFailure(null); }, [development.epoch, context.generation]);
  useEffect(() => { setDrafts({}); setTrusted(false); }, [development.epoch]);
  useEffect(() => { setFileTarget(null); setRunTarget(null); }, [development.epoch, context.generation]);
  useEffect(() => { if (task && !context.pending) void useDevelopmentRuntime.getState().refresh(task.id); }, [task?.id, development.epoch, context.generation, context.pending]);
  const liveActive = Object.values(runner.live).some(status => !RESTING.includes(status));
  const hostActive = hostRuntime.processes.some(processActive);
  // The running marker outlives the internal development tab and the surface's activity.
  useEffect(() => {
    if (!hostActive || !hostRuntime.taskId || context.pending) return;
    const taskId = hostRuntime.taskId;
    const timer = window.setInterval(() => { void useDevelopmentRuntime.getState().refresh(taskId); }, 1000);
    return () => window.clearInterval(timer);
  }, [hostActive, hostRuntime.taskId, development.epoch, context.generation, context.pending]);
  const dirty = Object.values(drafts).some(value => value.trim()) || Object.keys(editorDrafts).length > 0 || Object.values(hostRuntime.drafts).some(draft => draft.dirty) || (fileEditor.file !== null && hasDevelopmentDraft());
  const hostMarker = hostRuntime.read.status !== 'ready' || hostRuntime.read.refresh.status !== 'current' ? 'direct.marker.lastKnown' : hostRuntime.processes.some(process => process.status === 'unknown') ? 'direct.status.unknown' : hostRuntime.processes.some(process => process.status === 'cleanup_pending') ? 'direct.status.cleanup_pending' : hostRuntime.processes.some(process => process.status === 'starting') ? 'direct.status.starting' : 'shell.running';
  useSurfaceStatus({ dirty, running: liveActive || runner.busy || hostActive, runningLabel: hostActive && !liveActive && !runner.busy ? t(hostMarker) : undefined });
  useEffect(() => { void useDevelopment.getState().load(); }, []);
  useEffect(() => {
    if (!runner.session || context.pending) return;
    const timer = window.setTimeout(() => { void useDevelopment.getState().load(); }, 500);
    return () => window.clearTimeout(timer);
  }, [runner.session?.id, runner.session?.status, context.pending]);
  useSurfaceCloseGuard(async () => {
    if (!await guardDevelopmentDraft()) return false;
    if (!await guardWorkbenchDrafts()) return false;
    if (liveActive || runner.busy) {
      const accepted = await confirmAction(t('agent.title'), t('shell.closeRunningConfirm'), t('shell.stopAndClose'));
      if (!accepted || !await prepareAgentClose()) return false;
    } else if (Object.values(drafts).some(value => value.trim()) && !await confirmDiscard()) return false;
    useDevelopmentEditor.getState().reset();
    useDevelopmentDrafts.getState().clear();
    useDevelopmentRuntime.setState({ drafts: {} });
    return true;
  });

  // Starts a new runtime or resumes a saved one. Each call is one explicit user action and never retries. Both the
  // development and runtime account epochs are captured first; a response that crosses an account change is dropped.
  async function start(input: AgentOpenInput, projectId: string, taskId: string, sessionId: string | null): Promise<Outcome<null>> {
    if (!await discardDevelopmentDraft()) return { ok: false, code: 'cancelled' };
    const devEpoch = useDevelopment.getState().epoch;
    const runEpoch = useAgentRunner.getState().epoch;
    const generation = contextGeneration();
    setBusy(true); setFailure(null);
    await runner.open(input);
    if (useDevelopment.getState().epoch !== devEpoch || useAgentRunner.getState().epoch !== runEpoch || contextGeneration() !== generation) return { ok: false, code: 'cancelled' };
    const opened = useAgentRunner.getState();
    if (opened.error || opened.session?.id !== input.id) { const code = opened.error ?? 'unknown'; setFailure(code); setBusy(false); return { ok: false, code }; }
    await useDevelopment.getState().load();
    if (useDevelopment.getState().epoch !== devEpoch || useAgentRunner.getState().epoch !== runEpoch || contextGeneration() !== generation) return { ok: false, code: 'cancelled' };
    const selecting = chooseContext({ project_id: projectId, task_id: taskId, session_id: sessionId ?? opened.session.id });
    const selectionGeneration = contextGeneration();
    // chooseContext publishes its settlement once; any further generation belongs to another selection intent.
    const selected = await selecting;
    if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== selectionGeneration + 1) return { ok: false, code: 'cancelled' };
    if (!selected.ok) setFailure(selected.code);
    setBusy(false);
    if (selected.ok) revealAgent();
    return selected;
  }
  async function inspectSession(sessionId: string) {
    const epoch = development.epoch, generation = contextGeneration();
    setBusy(true);
    await runner.attach(sessionId);
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    const attached = useAgentRunner.getState();
    if (attached.error || attached.session?.id !== sessionId) { setFailure(attached.error ?? 'unknown'); setBusy(false); return; }
    await useDevelopment.getState().load();
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    const saved = useDevelopment.getState().sessions.find(item => item.id === sessionId);
    if (saved?.task_id) {
      const selected = await chooseContext({ project_id: saved.project_id, task_id: saved.task_id, session_id: sessionId });
      if (selected.ok && epoch === useDevelopment.getState().epoch && useDevelopment.getState().selection.session_id === sessionId) { setLaunch(null); revealAgent(); }
    }
    setBusy(false);
  }
  function startNew() {
    if (!project || !task || taskWorkspace?.taskId !== task.id || workspaceError) return;
    const selectedEngine = selectedCapabilities?.configuration.engine ?? engine;
    void start({ id: crypto.randomUUID(), engine: selectedEngine, task_id: task.id, capability_revision: selectedCapabilities?.revision ?? 0, options: { workspace: taskWorkspace.path, resume_id: null, executable: null, model: selectedCapabilities ? null : model || null, trust_workspace: trusted } }, project.id, task.id, null);
  }
  async function resume(session: DevelopmentSession, trust = trusted) {
    if (!await discardDevelopmentDraft()) return;
    if (!project || !session.task_id) return;
    if (runner.live[session.id] && runner.live[session.id] !== 'closed') {
      const devEpoch = useDevelopment.getState().epoch;
      const generation = contextGeneration();
      setBusy(true);
      await runner.attach(session.id);
      if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== generation) return;
      const attached = useAgentRunner.getState();
      if (attached.error || attached.session?.id !== session.id) { setFailure(attached.error ?? 'unknown'); setBusy(false); return; }
      const selecting = chooseContext({ project_id: project.id, task_id: session.task_id, session_id: session.id });
      const selectionGeneration = contextGeneration();
      const selected = await selecting;
      if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== selectionGeneration + 1) return;
      if (!selected.ok) setFailure(selected.code);
      setBusy(false);
      if (selected.ok) { setLaunch(null); revealAgent(); }
      return;
    }
    const result = await start({ id: session.id, engine: session.engine, task_id: session.task_id, options: { workspace: session.workspace, resume_id: null, executable: null, model: session.model, trust_workspace: trust } }, project.id, session.task_id, session.id);
    if (result.ok) setLaunch(null);
  }
  async function classify(session: DevelopmentSession) {
    if (!task) return;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    const result = await development.classify(session.id, task.id);
    if (epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setFailure(result.ok ? null : result.code);
  }
  async function acceptHandoff(sessionId: string, summary: string) {
    if (!project || !task) return;
    const epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    setDrafts(state => ({ ...state, [sessionId]: summary }));
    setBusy(true);
    await useDevelopment.getState().load();
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    await useAgentRunner.getState().attach(sessionId);
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    const attached = useAgentRunner.getState();
    if (attached.error || attached.session?.id !== sessionId) { setFailure(attached.error ?? 'unknown'); setBusy(false); return; }
    const result = await chooseContext({ project_id: project.id, task_id: task.id, session_id: sessionId });
    if (epoch === useDevelopment.getState().epoch) { if (!result.ok) setFailure(result.code); setBusy(false); }
  }
  const current = context.pending ? null : runner.session;
  const pendingCount = current && current.status !== 'closed' ? current.pending.length + confirmations.pending.length : 0;
  const otherWaiting = Object.entries(runner.live).filter(([id, status]) => !context.pending && id !== current?.id && status !== 'closed' && development.sessions.some(session => session.id === id && session.project_id && session.task_id)).map(([id]) => id).sort().join('\n');
  useEffect(() => {
    let disposed = false; let timer: number | undefined;
    const epoch = development.epoch, runEpoch = runner.epoch;
    const generation = context.generation;
    setOtherApprovals([]);
    async function poll() {
      const rows = await Promise.all(otherWaiting ? otherWaiting.split('\n').map(async sessionId => {
        try {
          const [snapshot, host] = await Promise.all([invoke<AgentSession>('get_agent_session', { sessionId }), listCapabilityConfirmations(sessionId)]);
          return { sessionId, count: snapshot.status === 'closed' ? 0 : snapshot.pending.length + host.filter(item => item.session_id === sessionId).length };
        } catch { return { sessionId, count: 0 }; }
      }) : []);
      if (disposed || epoch !== useDevelopment.getState().epoch || runEpoch !== useAgentRunner.getState().epoch || generation !== contextGeneration()) return;
      setOtherApprovals(rows.filter(row => row.count > 0));
      if (otherWaiting) timer = window.setTimeout(() => void poll(), 700);
    }
    void poll(); return () => { disposed = true; window.clearTimeout(timer); };
  }, [otherWaiting, development.epoch, runner.epoch, context.generation]);
  async function navigateApproval(sessionId: string) {
    const saved = development.sessions.find(item => item.id === sessionId);
    if (!saved?.project_id || !saved.task_id) return;
    const epoch = development.epoch;
    const result = await chooseContext({ project_id: saved.project_id, task_id: saved.task_id, session_id: saved.id });
    if (!result.ok || epoch !== useDevelopment.getState().epoch || useDevelopment.getState().selection.session_id !== saved.id) return;
    revealAgent(true); await useAgentRunner.getState().attach(saved.id);
  }
  const failureCode = failure ?? development.error ?? runner.error;
  const processes = hostRuntime.taskId === task?.id ? hostRuntime.processes : [];
  const activeProcesses = processes.filter(processActive).length;
  const failedProcesses = processes.filter(item => item.status === 'failed').length;
  const unknownProcesses = processes.filter(item => ['unknown', 'cleanup_pending'].includes(item.status)).length;
  const unresolvedOperations = hostRuntime.operations.filter(item => item.taskId === task?.id && ['unknown', 'pending', 'confirming'].includes(item.phase)).length;
  return <section ref={root} className={`agent-workbench dev-workspace dev-layout-agent-${layout.agentWidth} dev-layout-runtime-${layout.runtimeHeight}`} data-compact={compact}>
    <span className="sr-only" role="status" data-testid="agent-session-announcement">{surfaceActive && current ? `${t(`agent.engine.${current.engine}`)}: ${t(`agent.state.${current.status}`)}. ${t('layout.approvalCounts', { native: current.pending.length, host: confirmations.pending.length })}` : ''}</span>
    <header className="dev-workspace-header" inert={compact && drawerOpen}>
      <Button ref={navigationTrigger} size="sm" variant="ghost" data-testid="dev-toggle-navigation" aria-expanded={navigationOpen} aria-controls="dev-navigation" onClick={() => compact ? setDrawerOpen(value => !value) : setLayout(value => ({ ...value, navigation: !value.navigation }))}>{t('dev.projects')}</Button>
      <div className="dev-workspace-context"><h1>{project?.name ?? t('dev.title')}</h1><p title={task?.goal}>{task?.goal ?? t('dev.noTaskSelected')}</p>
        <div className="dev-context-status" data-testid="dev-context-status"><span>{taskWorkspace?.taskId === task?.id && taskWorkspace ? `${taskWorkspace.branch ?? t('dev.kindLabel.directory')} / ${taskWorkspace.kind === 'original' ? t('layout.original') : 'worktree'}` : t('layout.directoryUnresolved')}</span><span>{current ? `${t(`agent.engine.${current.engine}`)}: ${t(`agent.state.${current.status}`)}` : t('layout.noRuntime')}</span>{workspaceError ? <span className="text-status-error-text">{t('layout.contextFailed')}: {errorText(workspaceError)}</span> : null}</div>
      </div>
      <details className="dev-context-details" data-testid="dev-context-details"><summary>{t('layout.contextDetails')}</summary><div className="dev-detail"><p>{t('review.cwd')}: {taskWorkspace?.path ?? t('layout.directoryUnresolved')}</p><p>{t('dev.selected')}: {project?.id ?? '—'}</p><p>{t('dev.tasks')}: {task?.id ?? '—'}</p><p>{t('dev.sessions')}: {current?.id ?? selectedSaved?.id ?? '—'}</p><p>{t('layout.head')}: {taskWorkspace?.head ?? '—'}</p><Button size="xs" variant="secondary" onClick={() => setWorkspaceRefresh(value => value + 1)}>{t('dev.inspect')}</Button></div></details>
      <Button ref={launchTrigger} size="sm" variant="secondary" data-testid="dev-start-task" aria-expanded={Boolean(launch)} aria-controls="dev-task-launch-panel" onClick={() => setLaunch(value => value ? null : { taskId: task?.id ?? null, revision: null, nonce: Date.now() })}>{t(task ? 'workflow.continueTask' : 'workflow.startTask')}</Button>
      <div className="dev-view-switch" role="group" aria-label={t('dev.workspaceView')}>
        <Button size="sm" variant={codeVisible ? 'secondary' : 'ghost'} aria-pressed={codeVisible} onClick={() => setLayout(value => ({ ...value, compactView: 'code' }))}>{t('dev.projectTools')}</Button>
        <Button ref={agentTrigger} size="sm" variant={agentVisible ? 'secondary' : 'ghost'} data-testid="dev-toggle-agent" aria-expanded={agentVisible} aria-controls="dev-panel-agent" onClick={() => agentVisible && !compact ? (setLayout(value => ({ ...value, agent: false })), agentTrigger.current?.focus()) : revealAgent()}>{t('dev.conversation')}{pendingCount ? ` (${pendingCount})` : ''}</Button>
      </div>
    </header>
    {failureCode ? <p role="alert" className="dev-workspace-error text-sm text-status-error">{errorText(failureCode)}</p> : null}
    <Dialog.Root open={Boolean(launch)} onOpenChange={open => { if (!open) setLaunch(null); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="dev-launch-overlay" />
        <Dialog.Content id="dev-task-launch-panel" data-testid="dev-task-launch-drawer" className="dev-task-launch-panel" aria-modal={true} aria-describedby={undefined} onInteractOutside={event => event.preventDefault()} onCloseAutoFocus={event => { event.preventDefault(); if (drawerOpen) navigation.current?.querySelector<HTMLElement>('button, input, summary')?.focus(); else launchTrigger.current?.focus(); }}>
          <Dialog.Title className="sr-only">{t('taskLaunch.title')}</Dialog.Title>
          {launch ? <TaskLaunch key={launch.nonce} initialTaskId={launch.taskId} requestedRevision={launch.revision} onInspectSession={inspectSession} onManageProjects={manageProjects} onContinue={(saved, trust) => resume(saved, trust)} onStart={async (input, projectId, taskId) => { const result = await start(input, projectId, taskId, null); if (result.ok) setLaunch(null); return result; }} /> : null}
          <Dialog.Close asChild><Button size="sm" variant="secondary" data-testid="dev-task-launch-close">{t('common.close')}</Button></Dialog.Close>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
    <div className={`dev-workspace-body${navigationOpen && !compact ? '' : ' dev-navigation-hidden'}`}>
      <aside ref={navigation} id="dev-navigation" className="dev-navigation" aria-label={t('dev.projects')} role={compact && drawerOpen ? 'dialog' : undefined} aria-modal={compact && drawerOpen ? true : undefined} hidden={!navigationOpen} onKeyDown={event => {
        if (!compact) return;
        if (event.key === 'Escape' && !event.defaultPrevented) { event.preventDefault(); closeNavigation(); }
        if (event.key === 'Tab') {
          const nodes = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"], summary')).filter(node => node.getClientRects().length > 0 && !node.closest('[inert]'));
          const first = nodes[0], last = nodes[nodes.length - 1];
          if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
          else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
        }
      }}>
        {compact ? <Button size="xs" variant="secondary" onClick={closeNavigation}>{t('common.close')}</Button> : null}
        <ProjectPanel />
        {project ? <>
          <TaskPanel project={project} />
          <SessionPanel project={project} task={task} workspacePath={taskWorkspace && taskWorkspace.taskId === task?.id ? taskWorkspace.path : null} sessions={sessions} live={runner.live} selectedId={runner.selected}
            trusted={trusted} onTrusted={setTrusted} engine={engine} onEngine={setEngine} model={model} onModel={setModel} busy={context.pending || busy || runner.busy}
            projectCapabilities={projectCapabilities.value} selectedCapabilities={selectedCapabilities} capabilitiesLoading={projectCapabilities.loading} capabilitiesError={projectCapabilities.error ? errorText(hostCode(projectCapabilities.error)) : null}
            onCapabilities={revision => setCapabilityChoice(revision && project ? { projectId: project.id, epoch: development.epoch, revision } : null)}
            onStart={startNew} onResume={resume} />
          <UnclassifiedPanel sessions={unclassified} taskId={task?.id ?? null} busy={context.pending || busy || runner.busy} onClassify={session => void classify(session)} />
        </> : <UnclassifiedPanel sessions={unclassified} taskId={null} busy={context.pending || busy || runner.busy} onClassify={() => {}} />}
      </aside>
      <div className="dev-workspace-main" inert={compact && drawerOpen}>
        {task ? <details className="dev-task-brief" data-testid="dev-task-summary"><summary>{t('dev.taskFor', { goal: task.goal })}<span>{t(`dev.taskStatus.${task.status}`)}</span></summary><p className="m-0 whitespace-pre-wrap text-sm">{task.requirements}</p></details> : null}
        {pendingCount || otherApprovals.length ? <div className="dev-approval-navigation">
          {pendingCount ? <Button size="xs" variant="secondary" onClick={() => revealAgent(true)}>{t('dev.pendingApprovals', { count: pendingCount })} · {t('layout.approvalCounts', { native: current?.pending.length ?? 0, host: confirmations.pending.length })}</Button> : null}
          {otherApprovals.map(item => <Button key={item.sessionId} size="xs" variant="ghost" onClick={() => void navigateApproval(item.sessionId)}>{t('dev.otherApprovals', { count: item.count, task: development.tasks.find(task => task.id === development.sessions.find(session => session.id === item.sessionId)?.task_id)?.goal ?? item.sessionId })}</Button>)}
        </div> : null}
        <div className={`dev-collaboration${agentVisible && !compact ? ' dev-agent-open' : ''}`}>
        <div id="dev-panel-code" data-testid="dev-panel-code" className="dev-tools-pane" hidden={!codeVisible}>
          {project && !context.pending ? <ProjectReview key={`${development.epoch}:${context.generation}:${project.id}:${task?.id ?? ''}`} projectId={project.id} taskId={task?.id ?? null} sessions={sessions} live={runner.live} capabilityRevision={selectedCapabilities?.revision ?? 0} view={layout.codeView} onViewChange={codeView => setLayout(value => ({ ...value, codeView }))} fileTarget={fileTarget} snapshot={current?.capabilities} onUseConfiguration={useConfiguration} onOpenRuntime={showRuntime} workspacePath={taskWorkspace?.taskId === task?.id ? taskWorkspace?.path : ''} onWorkspaceChange={()=>{setWorkspaceRefresh(value => value + 1);setTrusted(false);}} onHandoff={(sessionId,summary)=>{revealAgent();void acceptHandoff(sessionId,summary);}} /> : <div className="dev-workspace-empty"><h2>{t('dev.welcome')}</h2><p>{t('dev.none')}</p></div>}
        </div>
        {agentVisible && !compact ? <PanelResizer axis="agent" value={layout.agentWidth} onChange={agentWidth => setLayout(value => ({ ...value, agentWidth }))} label={t('layout.resizeAgent')} testId="dev-agent-resizer" controls="dev-panel-agent" /> : null}
        <div ref={agentPanel} id="dev-panel-agent" data-testid="dev-panel-agent" className="dev-conversation-pane" hidden={!agentVisible}>
          {current ? <AgentConversation key={`${development.epoch}:${current.id}`} session={current} confirmations={confirmations} drafts={drafts} setDraft={(id, value) => { if (useDevelopment.getState().epoch === development.epoch) setDrafts(state => ({ ...state, [id]: value })); }} busy={busy || runner.busy} error={Boolean(runner.error)} onOpenReference={openReference} onSelectRun={runId => { setRunTarget(runId); setLayout(value => ({ ...value, runtime: true, runtimeView: 'runs' })); }} />
            : !context.pending && selectedSaved && selectedSaved.task_id ? <div className="dev-workspace-empty" data-testid="dev-resume-entry"><h2>{t('dev.resume')}</h2><p>{t('dev.resumeNotStarted')}</p>
              <details><summary>{t('capabilities.actual')}</summary><SessionCapabilitySnapshot key={selectedSaved.id} sessionId={selectedSaved.id} snapshot={undefined} /></details>
              <label className="dev-checkbox"><input type="checkbox" checked={trusted} onChange={event => setTrusted(event.target.checked)} />{t('dev.trustProject')}</label>
              <Button size="sm" variant="secondary" disabled={busy || !trusted} onClick={() => resume(selectedSaved)}>{t('dev.trustResume')}</Button></div>
            : <div className="dev-workspace-empty"><h2>{t(project ? task ? 'dev.startConversation' : 'dev.newTask' : 'dev.welcome')}</h2><p>{t(project ? task ? 'dev.startConversationHint' : 'dev.noTasks' : 'dev.none')}</p>{project && task ? <p className="text-xs text-ink-secondary">{t('dev.sessionSetupHint')}</p> : null}</div>}
        </div>
        </div>
        <div className="dev-runtime-bar"><Button ref={runtimeTrigger} size="xs" variant="ghost" data-testid="dev-toggle-runtime" aria-expanded={layout.runtime} aria-controls="dev-panel-runtime" onClick={() => { setLayout(value => ({ ...value, runtime: !value.runtime })); runtimeTrigger.current?.focus(); }}>{t('layout.runtime')} · {t('layout.runtimeCounts', { active: activeProcesses, failed: failedProcesses, unknown: unknownProcesses + unresolvedOperations })}</Button>{hostRuntime.read.status === 'error' || hostRuntime.read.status === 'ready' && hostRuntime.read.refresh.status !== 'current' ? <span className="text-xs text-status-warning-text">{t('direct.lastKnownHistory')}</span> : null}</div>
        {layout.runtime ? <PanelResizer axis="runtime" value={layout.runtimeHeight} onChange={runtimeHeight => setLayout(value => ({ ...value, runtimeHeight }))} label={t('layout.resizeRuntime')} testId="dev-runtime-resizer" controls="dev-panel-runtime" /> : null}
        <div id="dev-panel-runtime" data-testid="dev-panel-runtime" className="dev-bottom-runtime" hidden={!layout.runtime}>
          <div className="dev-runtime-tabs" role="group" aria-label={t('layout.runtime')}><Button size="xs" data-testid="dev-runtime-processes" variant={layout.runtimeView === 'processes' ? 'secondary' : 'ghost'} aria-pressed={layout.runtimeView === 'processes'} onClick={() => setLayout(value => ({ ...value, runtimeView: 'processes' }))}>{t('direct.title')}</Button><Button size="xs" data-testid="dev-runtime-runs" variant={layout.runtimeView === 'runs' ? 'secondary' : 'ghost'} aria-pressed={layout.runtimeView === 'runs'} onClick={() => setLayout(value => ({ ...value, runtimeView: 'runs' }))}>{t('review.runs')}</Button></div>
          {project && task && !context.pending ? <><div hidden={layout.runtimeView !== 'processes'}><DevelopmentRuntime key={`${development.epoch}:${project.id}:${task.id}`} projectId={project.id} taskId={task.id} visible={layout.runtime && layout.runtimeView === 'processes'} /></div><div hidden={layout.runtimeView !== 'runs'}>{layout.runtime && layout.runtimeView === 'runs' ? <Runs key={`${development.epoch}:${task.id}`} taskId={task.id} targetRunId={runTarget} /> : null}</div></> : <p>{t('dev.noTaskSelected')}</p>}
        </div>
      </div>
    </div>
  </section>;
}
