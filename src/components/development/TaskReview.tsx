import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment, type DevelopmentTask, type Outcome, type ProjectOperation } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { listDevelopmentRuns, readDevelopmentReview, type DevReview, type DevRun } from '../../stores/developmentReview';
import { deliveryRequests, readDevelopmentWorkspaces, type WorkspaceView } from '../../stores/developmentDelivery';
import { readDevelopmentProjectGit, type ProjectGitOverview } from '../../stores/developmentExplorer';
import { processActive, runtimeCall, useDevelopmentRuntime, type DevelopmentProcess, type RuntimeOperation } from '../../stores/developmentRuntime';
import { developmentDraftKey, useDevelopmentDrafts, useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { errorText } from './shared';

export interface TaskReviewProps {
  projectId: string;
  taskId: string;
  onOpenDiff: (path: string) => void;
  onOpenRuntime: () => void;
  onOpenDelivery: () => void;
}
interface Metadata { tasks: DevelopmentTask[]; operations: ProjectOperation[] }
interface Snapshot {
  task: DevelopmentTask;
  review: DevReview;
  runs: DevRun[];
  git: ProjectGitOverview;
  workspaces: WorkspaceView;
  processes: DevelopmentProcess[];
  projectOperations: ProjectOperation[];
}
type Verification = 'not_requested' | 'observed_stable' | 'stale' | 'failed' | 'running' | 'incomplete' | 'changed' | 'unknown';
const verificationStates: readonly string[] = ['not_requested', 'observed_stable', 'stale', 'failed', 'running', 'incomplete', 'changed', 'unknown'];
function verification(process: DevelopmentProcess, review: DevReview): Verification {
  if (process.verification_status === 'observed_stable') {
    if (!process.verification_complete || review.truncated) return 'incomplete';
    if (!process.fingerprint || process.fingerprint !== review.fingerprint) return 'stale';
    if (process.mutation_observed || process.start_fingerprint !== process.fingerprint) return 'changed';
    if (process.exit_code !== 0 || processActive(process) || process.reason !== null) return 'unknown';
    return 'observed_stable';
  }
  return verificationStates.includes(process.verification_status) ? process.verification_status as Verification : 'unknown';
}
function latestVerification(snapshot: Snapshot): DevelopmentProcess | undefined {
  let latest: DevelopmentProcess | undefined;
  for (const process of snapshot.processes) {
    if (process.verification_status === 'not_requested') continue;
    // Host timestamps have second precision. A tied failure must not be hidden by a pass.
    if (!latest || process.started_at > latest.started_at || process.started_at === latest.started_at && verification(latest, snapshot.review) === 'observed_stable' && verification(process, snapshot.review) !== 'observed_stable') latest = process;
  }
  return latest;
}
function evidenceKey(snapshot: Snapshot): string {
  return JSON.stringify({ task: snapshot.task, fingerprint: snapshot.review.fingerprint, truncated: snapshot.review.truncated,
    baseline: snapshot.review.captured_at, late: snapshot.review.baseline_late, git: snapshot.git.snapshot_token,
    workspace: snapshot.workspaces.selected_workspace_id,
    processes: snapshot.processes.map(process => [process.id, process.status, process.verification_status, process.start_fingerprint, process.fingerprint, process.verification_complete, process.mutation_observed, process.exit_code, process.reason, process.finished_at]),
    runs: snapshot.runs.map(run => [run.id, run.status, run.fingerprint]) });
}
async function readSnapshot(projectId: string, taskId: string): Promise<Outcome<Snapshot>> {
  const [review, runs, git, workspaces, processes] = await Promise.all([
    readDevelopmentReview(taskId), listDevelopmentRuns(taskId), readDevelopmentProjectGit(projectId, taskId),
    readDevelopmentWorkspaces({ project_id: projectId, task_id: taskId }),
    runtimeCall<DevelopmentProcess[]>('list_development_processes', { taskId }),
  ]);
  const metadata = await runtimeCall<Metadata>('read_development_workspace', {});
  if (!metadata.ok) return metadata;
  if (!review.ok) return review;
  if (!runs.ok) return runs;
  if (!git.ok) return git;
  if (!workspaces.ok) return workspaces;
  if (!processes.ok) return processes;
  const task = metadata.value.tasks.find(item => item.id === taskId && item.project_id === projectId);
  if (!task) return { ok: false, code: 'task_not_found' };
  if (review.value.task_id !== taskId || runs.value.some(run => run.task_id !== taskId) || processes.value.some(process => process.task_id !== taskId || process.project_id !== projectId)) return { ok: false, code: 'unknown' };
  return { ok: true, value: { task, review: review.value, runs: runs.value, git: git.value, workspaces: workspaces.value, processes: processes.value, projectOperations: metadata.value.operations.filter(operation => operation.project_id === projectId) } };
}
function unresolved(snapshot: Snapshot, operations: RuntimeOperation[], epoch: number) {
  const git = snapshot.workspaces.operations.filter(operation => operation.status === 'unknown' || operation.status === 'pending');
  let outstanding = false;
  try {
    outstanding = deliveryRequests(snapshot.task.owner).some(request => request.project_id === snapshot.task.project_id && request.task_id === snapshot.task.id && !snapshot.workspaces.operations.some(operation => operation.id === request.id && ['succeeded', 'failed'].includes(operation.status)));
  } catch { outstanding = true; }
  return {
    delivery: git.length > 0 || outstanding,
    project: snapshot.projectOperations.some(operation => ['unknown', 'pending'].includes(operation.status)),
    runtime: operations.some(operation => operation.epoch === epoch && operation.projectId === snapshot.task.project_id && operation.taskId === snapshot.task.id && (['unknown', 'pending', 'confirming'].includes(operation.phase) || operation.action === 'stop' && operation.phase === 'accepted' || operation.refreshing)),
    execution: snapshot.processes.some(process => processActive(process)) || snapshot.runs.some(run => ['unknown', 'running', 'waiting'].includes(run.status)),
    conflicts: snapshot.git.conflicts.length > 0,
  };
}

// Context invalidation disposes pending UI work without clearing the scoped acceptance explanation.
export function TaskReview(props: TaskReviewProps) {
  const epoch = useDevelopment(state => state.epoch);
  const selection = useContextSelection();
  return selection.pending ? null : <TaskReviewContent key={`${epoch}:${selection.generation}:${props.projectId}:${props.taskId}`} {...props} />;
}
function TaskReviewContent({ projectId, taskId, onOpenDiff, onOpenRuntime, onOpenDelivery }: TaskReviewProps) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const operations = useDevelopmentRuntime(state => state.operations);
  const taskDraftKey = developmentDraftKey('task', projectId, taskId);
  const taskDraft = useDevelopmentDrafts(state => state.entries[taskDraftKey]);
  const explanation = useScopedDevelopmentDraft('task', [projectId, taskId, 'review'], { explanation: '' });
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [accepted, setAccepted] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const ticket = useRef(0);
  const pending = useRef(false);
  const refresh = useCallback(async () => {
    const current = ++ticket.current;
    const generation = contextGeneration();
    setBusy(true); setError(null); setNotice(null); setAccepted(false); setConfirmed(false);
    const result = await readSnapshot(projectId, taskId);
    if (current !== ticket.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    setBusy(false);
    setSnapshot(result.ok ? result.value : null);
    if (!result.ok) setError(result.code);
  }, [epoch, projectId, taskId]);
  useEffect(() => { void refresh(); return () => { ++ticket.current; }; }, [refresh]);
  const latest = snapshot ? latestVerification(snapshot) : undefined;
  const state = snapshot && latest ? verification(latest, snapshot.review) : 'not_requested';
  const blocks = snapshot ? unresolved(snapshot, operations, epoch) : null;
  const blocked = !blocks || Object.values(blocks).some(Boolean) || Boolean(taskDraft);
  async function save(status: 'review' | 'completed') {
    if (pending.current || !snapshot || busy || blocked || status === 'completed' && (!confirmed || state !== 'observed_stable' && (!accepted || !explanation.value.explanation.trim()))) return;
    pending.current = true;
    const current = ++ticket.current;
    const generation = contextGeneration();
    const active = () => current === ticket.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
    const oldKey = evidenceKey(snapshot);
    const submitted = explanation.value;
    setBusy(true); setError(null); setNotice(null);
    try {
      // A displayed result is never authorization to complete against a later host snapshot.
      const result = await readSnapshot(projectId, taskId);
      if (!active()) return;
      if (!result.ok) { setError(result.code); setSnapshot(null); setAccepted(false); setConfirmed(false); return; }
      const fresh = result.value;
      setSnapshot(fresh);
      const nowBlocked = Object.values(unresolved(fresh, useDevelopmentRuntime.getState().operations, epoch)).some(Boolean);
      if (nowBlocked || useDevelopmentDrafts.getState().entries[taskDraftKey]) { setNotice('blocked'); return; }
      if (evidenceKey(fresh) !== oldKey) { setAccepted(false); setConfirmed(false); setNotice('changed'); return; }
      const verified = latestVerification(fresh);
      const freshState = verified ? verification(verified, fresh.review) : 'not_requested';
      if (status === 'completed' && freshState !== 'observed_stable' && (!accepted || !submitted.explanation.trim())) { setNotice('acceptanceRequired'); return; }
      const note = status === 'completed' && freshState !== 'observed_stable'
        ? `\n\n[DG-OS task review acceptance]\nTask: ${taskId}\nRecorded at: ${new Date().toISOString()}\nVerification: ${freshState}\nVerification process: ${verified?.id ?? 'none'}\nCurrent fingerprint: ${fresh.review.fingerprint}\nExplicit user acceptance of completion without current passing verification:\n${submitted.explanation.trim()}\n[/DG-OS task review acceptance]`
        : '';
      if (!active()) return;
      const saved = await useDevelopment.getState().saveTask({ id: taskId, project_id: projectId, goal: fresh.task.goal, requirements: fresh.task.requirements + note, status });
      if (!active()) return;
      if (!saved.ok) { setError(saved.code); return; }
      setSnapshot({ ...fresh, task: saved.value }); setAccepted(false); setConfirmed(false);
      if (status === 'completed') explanation.clear(submitted);
      setNotice(status === 'completed' ? 'completed' : 'inReview');
    } finally {
      pending.current = false;
      if (active()) setBusy(false);
    }
  }
  return <section className="dev-review-stack" aria-label={t('taskReview.title')} data-testid="dev-task-review">
    <header className="dev-review-toolbar"><h2 className="m-0 text-sm font-semibold">{t('taskReview.title')}</h2><Button size="xs" variant="secondary" disabled={busy} onClick={() => void refresh()}>{t('review.refresh')}</Button></header>
    {busy ? <p role="status" className="m-0 text-xs text-ink-secondary">{t('dev.busy')}</p> : null}
    {error ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(error)}</p> : null}
    {notice ? <p role="status" className="dev-evidence-warning">{t(`taskReview.${notice}`)}</p> : null}
    {snapshot ? <>
      <p className="m-0 text-xs">{snapshot.task.goal} · {t(`dev.taskStatus.${snapshot.task.status}`)}</p>
      <section className="dev-review-stack" aria-label={t('review.changes')}>
        <strong className="text-xs">{t('review.changes')}</strong><p className="m-0 text-xs text-ink-secondary">{t('review.uncertainty')}</p>
        {snapshot.review.captured_at === null ? <p className="dev-evidence-warning">{t('review.noBaseline')}</p> : null}
        {snapshot.review.baseline_late ? <p className="dev-evidence-warning">{t('review.baselineLate')}</p> : null}
        {snapshot.review.truncated ? <p className="dev-evidence-warning">{t('review.reviewTruncated')}</p> : null}
        {!snapshot.review.changes.length ? <p className="m-0 text-xs text-ink-secondary">{t('review.noChanges')}</p> : null}
        <nav className="dev-change-list" aria-label={t('taskReview.openDiff')}>{snapshot.review.changes.map(change => <Button key={change.path} size="xs" variant="ghost" className="dev-change-entry" onClick={() => onOpenDiff(change.path)}><span className="break-all">{change.path}</span><span>{t(`review.change.${change.kind}`)} · {t(change.origin === 'external' ? 'review.external' : 'review.uncertain')}</span></Button>)}</nav>
        {snapshot.review.excluded?.length ? <p className="dev-evidence-warning">{t('taskReview.excluded', { count: snapshot.review.excluded.length })}</p> : null}
      </section>
      <section className="dev-review-stack" aria-label={t('direct.verification')}>
        <strong className="text-xs" data-testid="dev-task-review-verification" data-verification={state}>{t('direct.verification')}: {t(`direct.verification.${state}`)}</strong>
        <p className="m-0 text-xs text-ink-secondary">{t('taskReview.verificationScope')}</p>
        {snapshot.processes.filter(process => process.verification_status !== 'not_requested').map(process => <div key={process.id} className="dev-review-stack"><span className="break-all text-xs">{process.program} {process.args.join(' ')}</span><span className="text-xs">{t(`direct.verification.${verification(process, snapshot.review)}`)}</span></div>)}
        <p className="m-0 text-xs text-ink-secondary">{t('taskReview.nativeRuns', { count: snapshot.runs.length })}</p>
        {snapshot.runs.filter(run => ['unknown', 'running', 'waiting'].includes(run.status)).map(run => <p key={run.id} className="m-0 break-all text-xs">{run.id}: {t(`review.run.${run.status}`)}</p>)}
        {operations.filter(operation => operation.epoch === epoch && operation.projectId === projectId && operation.taskId === taskId && ['unknown', 'pending', 'confirming', 'accepted'].includes(operation.phase)).map(operation => <p key={operation.id} className="m-0 break-all text-xs">{operation.targetId}: {t(`direct.operation.${operation.action}.${operation.phase}`)}</p>)}
        <Button size="xs" variant="secondary" onClick={onOpenRuntime}>{t('taskReview.revalidate')}</Button>
      </section>
      <section className="dev-review-stack" aria-label={t('taskReview.delivery')}>
        <strong className="text-xs">{t('taskReview.delivery')}</strong>
        <p className="m-0 text-xs text-ink-secondary">{t(snapshot.git.kind === 'directory' ? 'taskReview.noGit' : 'taskReview.gitSeparate')}</p>
        {snapshot.git.conflicts.length ? <><p role="alert" className="dev-evidence-warning">{t('taskReview.conflicts')}</p><ul className="m-0 pl-4 text-xs">{[...new Set(snapshot.git.conflicts.map(conflict => conflict.path))].map(path => <li key={path} className="break-all">{path}</li>)}</ul></> : null}
        {snapshot.workspaces.operations.filter(operation => ['unknown', 'pending'].includes(operation.status)).map(operation => <p key={operation.id} className="m-0 break-all text-xs">{operation.id}: {t(`delivery.operation.${operation.status}`)}</p>)}
        {snapshot.projectOperations.filter(operation => ['unknown', 'pending'].includes(operation.status)).map(operation => <p key={operation.id} className="m-0 break-all text-xs">{operation.id}: {t(`delivery.operation.${operation.status}`)}</p>)}
        <Button size="xs" variant="secondary" onClick={onOpenDelivery}>{t(blocks?.delivery ? 'taskReview.reconcileDelivery' : 'taskReview.openDelivery')}</Button>
      </section>
      {blocks?.delivery || blocks?.project ? <p role="alert" className="dev-evidence-warning">{t('taskReview.unresolvedDelivery')}</p> : null}
      {blocks?.runtime || blocks?.execution ? <div className="dev-review-stack"><p role="alert" className="dev-evidence-warning">{t('taskReview.unresolvedRuntime')}</p><Button size="xs" variant="secondary" onClick={onOpenRuntime}>{t('taskReview.reconcileRuntime')}</Button></div> : null}
      {taskDraft ? <p role="alert" className="dev-evidence-warning">{t('taskReview.taskDraft')}</p> : null}
      {snapshot.task.status !== 'completed' ? <form className="dev-form" onSubmit={event => { event.preventDefault(); void save('completed'); }}>
        {state !== 'observed_stable' ? <><label className="flex items-center gap-2 text-xs"><input type="checkbox" data-testid="dev-task-review-accept" checked={accepted} disabled={busy} onChange={event => setAccepted(event.target.checked)} />{t('taskReview.acceptMissing')}</label><Input multiline rows={3} data-testid="dev-task-review-explanation" label={t('taskReview.explanation')} value={explanation.value.explanation} disabled={busy} onChange={event => explanation.setValue({ explanation: event.target.value })} /><p className="m-0 text-xs text-ink-secondary">{t('taskReview.recordLocation')}</p></> : null}
        <label className="flex items-center gap-2 text-xs"><input type="checkbox" data-testid="dev-task-review-confirm" checked={confirmed} disabled={busy} onChange={event => setConfirmed(event.target.checked)} />{t('taskReview.confirm')}</label>
        <div className="flex flex-wrap gap-2"><Button type="submit" size="xs" data-testid="dev-task-review-complete" disabled={busy || blocked || !confirmed || state !== 'observed_stable' && (!accepted || !explanation.value.explanation.trim())}>{t('taskReview.complete')}</Button>{snapshot.task.status !== 'review' ? <Button type="button" size="xs" variant="secondary" disabled={busy || blocked} onClick={() => void save('review')}>{t('taskReview.markReview')}</Button> : null}</div>
      </form> : <p role="status" className="m-0 text-xs">{t('taskReview.completed')}</p>}
    </> : null}
  </section>;
}
