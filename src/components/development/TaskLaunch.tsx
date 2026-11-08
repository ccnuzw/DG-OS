import { useEffect, useMemo, useRef, useState } from 'react';
import { Button, Checkbox, Input, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment, type DevelopmentSession, type Outcome, type TaskStatus } from '../../stores/developmentStore';
import { useAgentRunner, type AgentOpenInput, type Engine } from '../../stores/agentRunnerStore';
import { readDevelopmentWorkspaces } from '../../stores/developmentDelivery';
import { developmentDraftKey, useDevelopmentDrafts, useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { readCliProviderRevision, useProjectCapabilities } from '../../stores/developmentCapabilities';
import { chooseContext, contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { errorText } from './shared';
import { CliCleanupRecheck } from './CliCleanupRecheck';
import { hostCode } from '../../stores/hostError';

interface LaunchDraft {
  goal: string; requirements: string; status: TaskStatus;
  taskId: string | null; sessionId: string | null; taskIdsBeforeCreate: string[] | null;
  taskCreateAttempted: boolean; taskCreateUnknown: boolean;
  sessionOpenAttempted: boolean; sessionOpenUnknown: boolean; sessionOpened: boolean;
  engine?: Engine; model?: string; revision?: number | null;
}
interface Props {
  initialTaskId?: string | null;
  requestedRevision?: number | null;
  onStart(input: AgentOpenInput, projectId: string, taskId: string): Promise<Outcome<null>>;
  onContinue(saved: DevelopmentSession, trust: boolean): Promise<void> | void;
  onInspectSession?(sessionId: string): Promise<void>;
  onManageProjects(): void;
}
interface ResolvedDirectory { projectId: string; taskId: string | null; path: string; identity: string }
const isLive = (status: string | undefined) => Boolean(status && !['closed', 'unknown', 'error'].includes(status));

export function TaskLaunch({ initialTaskId, requestedRevision = null, onStart, onContinue, onInspectSession, onManageProjects }: Props) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const projects = useDevelopment(state => state.projects);
  const tasks = useDevelopment(state => state.tasks);
  const sessions = useDevelopment(state => state.sessions);
  const selection = useDevelopment(state => state.selection);
  const live = useAgentRunner(state => state.live);
  const project = projects.find(item => item.id === selection.project_id) ?? null;
  const projectTasks = useMemo(() => tasks.filter(task => task.project_id === project?.id), [tasks, project?.id]);
  const [chosenTask, setChosenTask] = useState<string | null | undefined>(initialTaskId);
  const selectedTask = projectTasks.find(task => task.id === (chosenTask === undefined ? selection.task_id : chosenTask)) ?? null;
  const initial: LaunchDraft = { goal: selectedTask?.goal ?? '', requirements: selectedTask?.requirements ?? '', status: selectedTask?.status ?? 'todo', taskId: null, sessionId: null, taskIdsBeforeCreate: null, taskCreateAttempted: false, taskCreateUnknown: false, sessionOpenAttempted: false, sessionOpenUnknown: false, sessionOpened: false };
  const scope = [project?.id ?? '', selectedTask?.id ?? 'new'];
  const draft = useScopedDevelopmentDraft<LaunchDraft>('task', scope, initial);
  // TaskPanel owns the same goal/requirements/status object; launch only extends that draft.
  const value: LaunchDraft = { ...initial, ...draft.value };
  function updateDraft(next: Partial<LaunchDraft>) {
    if (epoch !== useDevelopment.getState().epoch) return;
    const current = useDevelopmentDrafts.getState().entries[developmentDraftKey('task', ...scope)]?.value as Partial<LaunchDraft> | undefined;
    draft.setValue({ ...initial, ...current, ...next });
  }
  const activeTask = selectedTask ?? projectTasks.find(task => task.id === value.taskId) ?? null;
  const savedCapabilities = useProjectCapabilities(project?.id ?? null);
  const [revision, setRevision] = useState<number | null>(requestedRevision ?? value.revision ?? null);
  const [directory, setDirectory] = useState<ResolvedDirectory | null>(null);
  const [trustKey, setTrustKey] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const operation = useRef(false);
  const completed = useRef(false);
  const mounted = useRef(true);
  const previousProject = useRef(project?.id);
  const currentDirectory = directory && directory.projectId === project?.id && directory.taskId === (activeTask?.id ?? null) ? directory : null;
  const directoryKey = currentDirectory ? JSON.stringify([epoch, currentDirectory.projectId, currentDirectory.path, currentDirectory.identity]) : '';
  const trusted = Boolean(directoryKey && trustKey === directoryKey);
  const current = () => mounted.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    if (previousProject.current !== project?.id) { setChosenTask(undefined); setRevision(null); setTrustKey(null); previousProject.current = project?.id; }
  }, [project?.id]);
  useEffect(() => {
    let active = true;
    if (!project || pending) return;
    const taskId = activeTask?.id ?? null;
    void readDevelopmentWorkspaces({ project_id: project.id, task_id: taskId }).then(result => {
      if (!active || !current()) return;
      if (!result.ok) { setDirectory(null); setError(result.code); return; }
      const workspace = result.value.workspaces.find(item => result.value.selected_workspace_id ? item.id === result.value.selected_workspace_id : item.kind === 'original');
      if (!workspace) { setDirectory(null); setError('workspace_not_found'); return; }
      setDirectory({ projectId: project.id, taskId, path: workspace.path, identity: workspace.directory_identity });
    });
    return () => { active = false; };
  }, [project?.id, project?.path, activeTask?.id, epoch, generation, pending]);

  async function reconcileTask() {
    if (operation.current) return;
    operation.current = true; setBusy(true); setError(null);
    try { if (!await useDevelopment.getState().load() && current()) setError('unknown'); }
    finally { operation.current = false; if (mounted.current) setBusy(false); }
  }
  async function start() {
    if (!project || !currentDirectory || !trusted || pending || operation.current || completed.current || project.archived || value.taskCreateUnknown || value.sessionOpenUnknown || value.sessionOpened) return;
    operation.current = true; setBusy(true); setError(null);
    let taskId = activeTask?.id ?? value.taskId;
    let sessionId = value.sessionId;
    let openDispatched = false;
    try {
      const capabilities = await readCliProviderRevision(project.id);
      if (!current()) return;
      if (!capabilities || (capabilities.revision !== savedCapabilities.value?.revision || capabilities.permission_epoch !== savedCapabilities.value?.permission_epoch) || revision !== capabilities.revision) { setError('capability_conflict'); return; }
      if (!taskId) {
        if (value.taskCreateAttempted || !value.goal.trim()) return;
        const before = projectTasks.map(task => task.id);
        // Treat a pending request as unknown immediately; remount never replays creation.
        updateDraft({ taskIdsBeforeCreate: before, taskCreateAttempted: true, taskCreateUnknown: true });
        const result = await useDevelopment.getState().saveTask({ id: null, project_id: project.id, goal: value.goal, requirements: value.requirements, status: value.status });
        if (epoch !== useDevelopment.getState().epoch) return;
        if (!result.ok) { updateDraft({ taskCreateUnknown: result.code === 'unknown', taskCreateAttempted: result.code === 'unknown' }); if (current()) setError(result.code); return; }
        taskId = result.value.id;
        updateDraft({ taskId, taskCreateUnknown: false, taskCreateAttempted: true });
        if (!current()) return;
      }
      const result = await readDevelopmentWorkspaces({ project_id: project.id, task_id: taskId });
      if (!current()) return;
      if (!result.ok) { setError(result.code); return; }
      const workspace = result.value.workspaces.find(item => result.value.selected_workspace_id ? item.id === result.value.selected_workspace_id : item.kind === 'original');
      if (!workspace || workspace.path !== currentDirectory.path || workspace.directory_identity !== currentDirectory.identity) { setTrustKey(null); setError('workspace_changed'); return; }
      sessionId ??= crypto.randomUUID();
      updateDraft({ taskId, sessionId, sessionOpenAttempted: true, sessionOpenUnknown: true });
      openDispatched = true;
      const opened = await onStart({ intent: 'create', id: sessionId, engine: capabilities.configuration.engine, task_id: taskId, project_revision: capabilities.revision, provider: capabilities.configuration.provider, options: { workspace: workspace.path, resume_id: null, executable: null, model: null, trust_workspace: true } }, project.id, taskId);
      if (epoch !== useDevelopment.getState().epoch) return;
      if (opened.ok) {
        completed.current = true;
        const key = developmentDraftKey('task', ...scope);
        useDevelopmentDrafts.setState(state => {
          const entry = state.entries[key];
          if (!entry) return state;
          const content = entry.value as LaunchDraft;
          const entries = { ...state.entries };
          delete entries[key];
          if (selectedTask) {
            const remaining = { goal: content.goal, requirements: content.requirements, status: content.status };
            const baseline = { goal: selectedTask.goal, requirements: selectedTask.requirements, status: selectedTask.status };
            if (JSON.stringify(remaining) !== JSON.stringify(baseline)) entries[key] = { ...entry, value: remaining, baseline };
          }
          return { entries };
        });
      } else {
        updateDraft({ taskId, sessionId, sessionOpened: false, sessionOpenUnknown: ['unknown', 'cleanup_pending', 'session_exists'].includes(opened.code) });
        if (current()) setError(opened.code);
      }
    } catch (failure) {
      const code = hostCode(failure);
      if (epoch === useDevelopment.getState().epoch && openDispatched) updateDraft({ taskId, sessionId, sessionOpenUnknown: ['unknown', 'cleanup_pending', 'session_exists'].includes(code) });
      if (current()) setError(code);
    } finally { operation.current = false; if (mounted.current) setBusy(false); }
  }
  async function continueSession(session: DevelopmentSession) {
    if (operation.current || ['unknown', 'error'].includes(live[session.id]) || !isLive(live[session.id]) && (!trusted || session.workspace !== currentDirectory?.path || !session.native_id)) return;
    operation.current = true; setBusy(true);
    try { await onContinue(session, trusted); }
    catch { if (current()) setError('unknown'); }
    finally { operation.current = false; if (mounted.current) setBusy(false); }
  }
  const candidates = value.taskCreateUnknown ? projectTasks.filter(task => !(value.taskIdsBeforeCreate ?? []).includes(task.id)) : [];
  const taskSessions = sessions.filter(session => session.project_id === project?.id && session.task_id === activeTask?.id);
  return <section className="dev-review-stack" aria-label={t('taskLaunch.title')} data-testid="dev-task-launch">
    <div className="dev-panel-header"><h2 className="m-0 text-sm font-semibold">{t('taskLaunch.title')}</h2><Button size="xs" variant="secondary" onClick={onManageProjects}>{t('taskLaunch.manageProjects')}</Button></div>
    {!projects.length ? <p className="text-sm text-ink-secondary">{t('taskLaunch.noProjects')}</p> : <>
      <Select id="task-launch-project" label={t('dev.projects')} value={project?.id ?? ''} disabled={busy} placeholder={t('taskLaunch.chooseProject')} options={projects.filter(item => !item.archived).map(item => ({ value: item.id, label: `${item.name} · ${item.path}` }))} onChange={id => { void chooseContext({ project_id: id, task_id: null, session_id: null }); }} />
      {project ? <>
        <p data-testid="task-launch-directory" className="m-0 break-all text-xs">{currentDirectory?.path ?? t('capabilities.loading')}</p>
        <Select id="task-launch-task" label={t('dev.tasks')} value={selectedTask?.id ?? 'new'} disabled={busy} options={[{ value: 'new', label: t('taskLaunch.newTask') }, ...projectTasks.map(task => ({ value: task.id, label: task.goal }))]} onChange={id => { setChosenTask(id === 'new' ? null : id); setTrustKey(null); }} />
        {!activeTask ? <><Input id="task-launch-goal" data-testid="task-launch-goal" className="type-reading" label={t('dev.goal')} value={value.goal} disabled={busy || value.taskCreateUnknown} onChange={event => updateDraft({ goal: event.target.value })} /><label className="dev-field">{t('dev.requirements')}<textarea id="task-launch-requirements" data-testid="task-launch-requirements" className="dev-editor type-reading" value={value.requirements} disabled={busy || value.taskCreateUnknown} onChange={event => updateDraft({ requirements: event.target.value })} /></label></> : <><p className="type-reading">{activeTask.goal}</p><p className="type-reading whitespace-pre-wrap">{activeTask.requirements}</p></>}
        {value.taskCreateUnknown ? <><p role="alert">{t('taskLaunch.taskOutcomeUnknown')}</p><Button size="xs" disabled={busy} onClick={() => void reconcileTask()}>{t('taskLaunch.recheckTasks')}</Button>{candidates.map(task => <Button key={task.id} size="xs" onClick={() => { updateDraft({ taskId: task.id, taskCreateUnknown: false }); }}>{task.goal} · {task.id}</Button>)}</> : null}
        <Checkbox id="task-launch-trust" label={t('dev.trustProject')} checked={trusted} disabled={busy || !currentDirectory} onChange={checked => setTrustKey(checked ? directoryKey : null)} />
        <p className="m-0 text-xs text-ink-secondary">{t('dev.trustHint')}</p>
        <Select id="task-launch-capabilities" label={t('capabilities.newSessionSource')} value={revision === null ? '' : String(revision)} placeholder={t('capabilities.chooseProvider')} disabled={busy || savedCapabilities.loading} options={savedCapabilities.value ? [{ value: String(savedCapabilities.value.revision), label: t('capabilities.projectRevision', { revision: savedCapabilities.value.revision }) }] : []} onChange={choice => { const next = Number(choice); setRevision(next); updateDraft({ revision: next }); }} />
        {revision === null ? <p role="alert" className="text-xs text-status-error">{t('capabilities.providerRequired')}</p> : <p className="text-xs">{t('capabilities.projectRevision', { revision })}</p>}
        {error || savedCapabilities.error ? <p role="alert" className="text-xs text-status-error">{errorText(error ?? 'unknown')}</p> : null}
        {error === 'cleanup_pending' ? <CliCleanupRecheck /> : null}
        <Button size="xs" data-testid="task-launch-start" disabled={busy || completed.current || pending || !trusted || !revision || project.archived || !activeTask && !value.goal.trim() || value.taskCreateUnknown || value.sessionOpenUnknown || value.sessionOpened || savedCapabilities.loading || Boolean(savedCapabilities.error) || savedCapabilities.value?.revision !== revision} onClick={() => void start()}>{t('taskLaunch.start')}</Button>
        {(value.sessionOpenUnknown || value.sessionOpened) && value.sessionId ? <Button size="xs" disabled={busy || !onInspectSession} onClick={() => { if (onInspectSession && value.sessionId) void onInspectSession(value.sessionId); }}>{t('taskLaunch.inspectSession')} · {value.sessionId}</Button> : null}
        {taskSessions.map(session => <Button key={session.id} size="xs" variant="secondary" data-testid={`task-launch-continue-${session.id}`} disabled={busy || ['unknown', 'error'].includes(live[session.id]) || !isLive(live[session.id]) && (!trusted || session.workspace !== currentDirectory?.path || !session.native_id)} onClick={() => void continueSession(session)}>{t(isLive(live[session.id]) ? 'taskLaunch.connectLive' : 'taskLaunch.continueSaved')} · {session.engine} · {session.id.slice(0, 8)} · {session.workspace}</Button>)}
      </> : null}
    </>}
  </section>;
}
