import { useEffect, useState } from 'react';
import { Button, Input, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { chooseContext } from '../../stores/developmentSelection';
import { TASK_STATUSES, useDevelopment, type DevelopmentTask, type Project, type TaskStatus } from '../../stores/developmentStore';
import { errorText } from './shared';

// Task list and editor for the selected project. The editor is keyed by the selected task id, so switching tasks
// never shows a draft from another task, and a failed save keeps what the user typed.
export function TaskPanel({ project }: { project: Project }) {
  const { t } = useLocale();
  const tasks = useDevelopment(state => state.tasks.filter(task => task.project_id === project.id));
  const selectedTaskId = useDevelopment(state => state.selection.task_id);
  const selected = tasks.find(task => task.id === selectedTaskId) ?? null;
  return <section className="dev-panel" aria-label={t('dev.tasks')} data-testid="dev-task-list">
    <header className="dev-panel-header"><h2 className="m-0 text-sm font-semibold">{t('dev.tasks')}</h2>
      <Button size="xs" variant="secondary" data-testid="dev-task-new" onClick={() => void chooseContext({ project_id: project.id, task_id: null, session_id: null })}>{t('dev.newTask')}</Button></header>
    {!tasks.length ? <p className="text-sm text-muted-foreground">{t('dev.noTasks')}</p> : null}
    <nav aria-label={t('dev.tasks')} className="grid gap-1">
      {tasks.map(task => <Button key={task.id} size="xs" variant={selectedTaskId === task.id ? 'primary' : 'ghost'} className="justify-start" data-testid={`dev-task-item-${task.id}`} aria-pressed={selectedTaskId === task.id} onClick={() => void chooseContext({ project_id: project.id, task_id: task.id, session_id: null })}>
        <span className="truncate">{task.goal}</span></Button>)}
    </nav>
    <TaskEditor key={`${project.id}:${selected?.id ?? 'new'}`} project={project} task={selected} />
  </section>;
}

function TaskEditor({ project, task }: { project: Project; task: DevelopmentTask | null }) {
  const { t } = useLocale();
  const [goal, setGoal] = useState(task?.goal ?? '');
  const [requirements, setRequirements] = useState(task?.requirements ?? '');
  const [status, setStatus] = useState<TaskStatus>(task?.status ?? 'todo');
  const [statusEdited, setStatusEdited] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  // Reloads of the same task refresh the editor only while nothing has been typed into it.
  useEffect(() => {
    if (task && !goal && !requirements) { setGoal(task.goal); setRequirements(task.requirements); setStatus(task.status); }
  }, [task?.id, task?.goal, task?.requirements, task?.status]);
  useEffect(() => { if (task && !statusEdited) setStatus(task.status); }, [task?.status, statusEdited]);
  async function save() {
    setBusy(true);
    const result = await useDevelopment.getState().saveTask({ id: task?.id ?? null, project_id: project.id, goal, requirements, status });
    setBusy(false);
    if (!result.ok) { setFailure(result.code); return; }
    setFailure(null);
    await chooseContext({ project_id: project.id, task_id: result.value.id, session_id: null });
  }
  return <form className="dev-form" data-testid="dev-task-form" onSubmit={event => { event.preventDefault(); void save(); }}>
    <Input multiline rows={2} data-testid="dev-task-goal" label={t('dev.goal')} value={goal} onChange={event => setGoal(event.target.value)} />
    <Input multiline rows={3} data-testid="dev-task-requirements" label={t('dev.requirements')} value={requirements} onChange={event => setRequirements(event.target.value)} />
    <Select label={t('dev.status')} value={status} onChange={value => { setStatusEdited(true); setStatus(TASK_STATUSES.find(item => item === value) ?? 'todo'); }} options={TASK_STATUSES.map(item => ({ value: item, label: t(`dev.taskStatus.${item}`) }))} />
    {failure ? <p role="alert" className="m-0 text-xs text-destructive">{errorText(failure)}</p> : null}
    <Button type="submit" size="xs" data-testid="dev-task-save" disabled={busy || !goal.trim() || !requirements.trim()}>{busy ? t('dev.busy') : t('dev.saveTask')}</Button>
  </form>;
}
