import { useEffect, useMemo, useState } from 'react';
import { Button, Input, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { chooseContext, contextGeneration } from '../../stores/developmentSelection';
import { TASK_STATUSES, useDevelopment, type DevelopmentTask, type Project, type TaskStatus } from '../../stores/developmentStore';
import { errorText } from './shared';
import { useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { processActive, useDevelopmentRuntime } from '../../stores/developmentRuntime';

// Collapsing task details keeps the mounted editor and its failed-save draft intact.
export function TaskPanel({ project, onContinue }: { project: Project; onContinue?: (sessionId: string) => void }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const allTasks = useDevelopment(state => state.tasks);
  const tasks = allTasks.filter(task => task.project_id === project.id);
  const selectedTaskId = useDevelopment(state => state.selection.task_id);
  const selected = tasks.find(task => task.id === selectedTaskId) ?? null;
  const sessions = useDevelopment(state => state.sessions);
  const live = useAgentRunner(state => state.live);
  const processes = useDevelopmentRuntime(state => state.processes);
  const runtimeTaskId = useDevelopmentRuntime(state => state.taskId);
  const [search, setSearch] = useState('');
  const query = search.trim().toLocaleLowerCase();
  const matchingIds = useMemo(() => {
    const ids = new Set(allTasks.filter(task => task.project_id === project.id && [task.goal, task.requirements, task.id, project.path].some(value => value.toLocaleLowerCase().includes(query))).map(task => task.id));
    if (query) for (const session of sessions) {
      if (session.project_id === project.id && session.task_id && session.workspace.toLocaleLowerCase().includes(query)) ids.add(session.task_id);
    }
    return ids;
  }, [allTasks, sessions, project.id, project.path, query]);
  const visible = tasks.filter(task => task.id === selectedTaskId || matchingIds.has(task.id));
  const activity = useMemo(() => {
    const result = new Map<string, { live: number; running: number; processes: number }>();
    const entry = (id: string) => {
      let value = result.get(id);
      if (!value) { value = { live: 0, running: 0, processes: 0 }; result.set(id, value); }
      return value;
    };
    for (const session of sessions) {
      if (session.project_id !== project.id || !session.task_id || !live[session.id]) continue;
      const value = entry(session.task_id);
      value.live += 1;
      if (live[session.id] === 'running') value.running += 1;
    }
    for (const process of processes) {
      if (process.project_id === project.id && process.task_id === runtimeTaskId && processActive(process)) entry(process.task_id).processes += 1;
    }
    return result;
  }, [sessions, live, processes, project.id, runtimeTaskId]);
  const [creating, setCreating] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  useEffect(() => { setCreating(false); setFailure(null); setSearch(''); }, [project.id]);
  useEffect(() => { if (selectedTaskId) setCreating(false); }, [selectedTaskId]);
  async function choose(taskId: string | null) {
    if (taskId && onContinue) {
      const saved = sessions.find(session => session.project_id === project.id && session.task_id === taskId);
      if (saved) { onContinue(saved.id); return; }
      if (taskId === selectedTaskId) return;
    }
    setPending(true);
    const result = await chooseContext({ project_id: project.id, task_id: taskId, session_id: null });
    setPending(false);
    setFailure(result.ok ? null : result.code);
    if (result.ok) setCreating(taskId === null);
  }
  return <section className="dev-panel dev-nav-panel" aria-label={t('dev.tasks')} data-testid="dev-task-list">
    <header className="dev-panel-header"><h2 className="m-0 text-sm font-semibold">{t('dev.tasks')}</h2>
      <Button size="xs" variant="secondary" data-testid="dev-task-new" disabled={pending} onClick={() => void choose(null)}>{t('dev.newTask')}</Button></header>
    {tasks.length ? <div className="dev-nav-search"><Input type="search" data-testid="dev-task-search" label={t('workbenchNavigation.searchTasks')} placeholder={t('workbenchNavigation.taskSearchHint')} value={search} onChange={event => setSearch(event.target.value)} />{search ? <Button size="xs" variant="ghost" onClick={() => setSearch('')}>{t('patterns.clearSearch')}</Button> : null}</div> : null}
    {query && !matchingIds.size ? <p role="status" className="m-0 text-xs text-ink-secondary">{t('workbenchNavigation.noMatches')}</p> : null}
    {!tasks.length ? <p className="text-sm text-ink-secondary">{t('dev.noTasks')}</p> : null}
    <nav aria-label={t('dev.tasks')} className="dev-nav-list grid gap-1">
      {visible.map(task => <Button key={task.id} size="xs" variant={selectedTaskId === task.id ? 'secondary' : 'ghost'} className="dev-nav-item justify-start" data-testid={`dev-task-item-${task.id}`} aria-pressed={selectedTaskId === task.id} disabled={pending} onClick={() => void choose(task.id)}>
        <span className="dev-nav-item-label truncate" title={task.goal}>{task.goal}</span><span className="dev-nav-status">{t(`dev.taskStatus.${task.status}`)}</span>
        {selectedTaskId === task.id ? <span className="dev-nav-status">{t('workbenchNavigation.current')}</span> : null}
        {activity.get(task.id)?.running ? <span className="dev-nav-status">{t('workbenchNavigation.running', { count: activity.get(task.id)!.running })}</span> : null}
        {activity.get(task.id)?.live ? <span className="dev-nav-status">{t('workbenchNavigation.liveCli', { count: activity.get(task.id)!.live })}</span> : null}
        {activity.get(task.id)?.processes ? <span className="dev-nav-status">{t('workbenchNavigation.activeProcesses', { count: activity.get(task.id)!.processes })}</span> : null}
      </Button>)}
    </nav>
    {failure ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(failure)}</p> : null}
    {selected && onContinue ? <nav className="dev-nav-list" aria-label={t('dev.sessions')}>{sessions.filter(session => session.task_id === selected.id && session.project_id === project.id).map(session => <Button key={session.id} size="xs" variant="ghost" onClick={() => onContinue(session.id)}>{t('workbenchFiles.continueSession', { engine: session.engine, name: useDevelopment.getState().sessionNames[session.id] ?? selected.goal })}</Button>)}</nav> : null}
    {selected ? <details key={`${epoch}:${selected.id}`} className="dev-disclosure"><summary className="dev-disclosure-summary">{t('dev.taskDetails')}</summary><div className="dev-disclosure-body"><TaskEditor key={`${epoch}:${project.id}:${selected.id}`} project={project} task={selected} /></div></details>
      : creating || !tasks.length && !onContinue ? <TaskEditor key={`${epoch}:${project.id}:new`} project={project} task={null} />
      : <p className="m-0 text-xs text-ink-secondary">{t('dev.noTaskSelected')}</p>}
  </section>;
}

function TaskEditor({ project, task }: { project: Project; task: DevelopmentTask | null }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const draft = useScopedDevelopmentDraft('task', [project.id, task?.id ?? 'new'], { goal: task?.goal ?? '', requirements: task?.requirements ?? '', status: task?.status ?? 'todo' as TaskStatus });
  const { goal, requirements, status } = draft.value;
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  async function save() {
    if (busy) return;
    const generation = contextGeneration();
    setBusy(true);
    const result = await useDevelopment.getState().saveTask({ id: task?.id ?? null, project_id: project.id, goal, requirements, status });
    if (epoch !== useDevelopment.getState().epoch) return;
    setBusy(false);
    if (!result.ok) { setFailure(result.code); return; }
    setFailure(null);
    draft.clear({ goal, requirements, status });
    if (generation === contextGeneration()) await chooseContext({ project_id: project.id, task_id: result.value.id, session_id: null });
  }
  return <form className="dev-form" data-testid="dev-task-form" onSubmit={event => { event.preventDefault(); void save(); }}>
    <Input multiline rows={2} data-testid="dev-task-goal" label={t('dev.goal')} value={goal} onChange={event => draft.setValue({ ...draft.value, goal: event.target.value })} />
    <Input multiline rows={3} data-testid="dev-task-requirements" label={t('dev.requirements')} value={requirements} onChange={event => draft.setValue({ ...draft.value, requirements: event.target.value })} />
    <Select label={t('dev.status')} value={status} onChange={value => draft.setValue({ ...draft.value, status: TASK_STATUSES.find(item => item !== 'completed' && item === value) ?? task?.status ?? 'todo' })} options={TASK_STATUSES.filter(item => item !== 'completed' || task?.status === 'completed').map(item => ({ value: item, label: t(`dev.taskStatus.${item}`) }))} />
    <p className="m-0 text-xs text-ink-secondary">{t('taskReview.completeInReview')}</p>
    {failure ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(failure)}</p> : null}
    <Button type="submit" size="xs" data-testid="dev-task-save" disabled={busy || !goal.trim() || !requirements.trim()}>{busy ? t('dev.busy') : t('dev.saveTask')}</Button>
    {draft.dirty ? <Button size="xs" variant="secondary" onClick={() => void draft.discard()}>{t('dev.discardDraft')}</Button> : null}
  </form>;
}
