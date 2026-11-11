import { useEffect, useId, useRef, useState } from 'react';
import { Button, Modal } from '@dgos/ui';
import { formatDateTime, useLocale } from '../../i18n';
import type { AgentSession } from '../../stores/agentRunnerStore';
import { readDevelopmentSessionSummaries, type SessionSummary } from '../../stores/developmentExplorer';
import { listDevelopmentRuns, type DevRun } from '../../stores/developmentReview';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useDevelopment, type DevelopmentSession, type Outcome } from '../../stores/developmentStore';
import { useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { errorText } from './shared';
import { activateRecoveredConversationDraft, hasUnclassifiedRecoveryDraft, preserveClassifiedConversationDraft, readRecoverySession, readRecoveryTarget, recoveryMessages, unclassifiedRecoveryDraft, type RecoveryTarget } from './unclassifiedRecovery';

export interface UnclassifiedSessionRecoveryProps {
  session: DevelopmentSession;
  onClose(): void;
  onClassified(saved: DevelopmentSession): void;
  onManageProjects(): void;
  onManageTasks(projectId: string): void;
}
interface RecoveryHistory {
  summary: SessionSummary | null;
  summaryFailed: boolean;
  snapshot: Outcome<AgentSession>;
  runs: Outcome<DevRun[]> | null;
}

/** A read-only recovery browser. Only explicit classification mutates the existing session index. */
export function UnclassifiedSessionRecovery({ session, onClose, onClassified, onManageProjects, onManageTasks }: UnclassifiedSessionRecoveryProps) {
  const { t, locale } = useLocale();
  const { epoch, workspaceOwner, projects, tasks, sessions } = useDevelopment();
  const { generation, pending } = useContextSelection();
  const surfaceActive = useSurfaceActive();
  const identity = JSON.stringify([epoch, workspaceOwner, generation, session.id, session.owner, session.engine, session.workspace]);
  const origin = useRef(identity);
  const latest = useRef({ identity, surfaceActive, pending });
  latest.current = { identity, surfaceActive, pending };
  const mounted = useRef(false);
  const initialFocus = useRef<HTMLHeadingElement>(null);
  const flight = useRef(false);
  const [history, setHistory] = useState<RecoveryHistory | null>(null);
  const [targets, setTargets] = useState<RecoveryTarget[] | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [uncertainTaskId, setUncertainTaskId] = useState<string | null>(null);
  const [copyFailed, setCopyFailed] = useState(false);
  const draftEntries = useDevelopmentDrafts(state => state.entries);
  const groupId = useId();
  const savedSource = sessions.find(item => item.id === session.id && item.owner === session.owner && item.engine === session.engine && item.workspace === session.workspace);
  const draftConflict = savedSource ? unclassifiedRecoveryDraft(savedSource, draftEntries) : null;
  const valid = identity === origin.current && surfaceActive && !pending && Boolean(savedSource);
  const unclassified = savedSource?.project_id === null && savedSource.task_id === null;
  const selected = targets?.find(item => item.task.id === selectedTaskId && !item.reason);
  const ownedProjects = projects.filter(item => item.owner === session.owner);
  const availableProjects = ownedProjects.filter(item => !item.archived);

  function current() {
    const state = useDevelopment.getState();
    return mounted.current && latest.current.identity === origin.current && latest.current.surfaceActive && !latest.current.pending
      && state.epoch === epoch && state.workspaceOwner === workspaceOwner && contextGeneration() === generation
      && state.sessions.some(item => item.id === session.id && item.owner === session.owner && item.engine === session.engine && item.workspace === session.workspace);
  }

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => { if (!valid) onClose(); }, [valid, onClose]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      initialFocus.current?.focus({ preventScroll: true });
      const scrollport = initialFocus.current?.closest('.ui-modal-body');
      if (scrollport) scrollport.scrollTop = 0;
    });
    return () => window.cancelAnimationFrame(frame);
  }, []);

  useEffect(() => {
    if (!valid) return;
    let active = true;
    setHistory(null);
    void Promise.all([
      readDevelopmentSessionSummaries(),
      readRecoverySession(session),
      session.task_id ? listDevelopmentRuns(session.task_id) : Promise.resolve(null),
    ]).then(([summaries, snapshot, runs]) => {
      if (!active || !current()) return;
      setHistory({ summary: summaries.ok ? summaries.value.find(item => item.session_id === session.id) ?? null : null, summaryFailed: !summaries.ok, snapshot, runs });
    });
    return () => { active = false; };
  // Read results belong to this account, context generation and saved session, never a newly selected context.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identity, valid, session.task_id, refresh]);

  useEffect(() => {
    if (!valid) return;
    let active = true;
    setTargets(null);
    const candidates = tasks.flatMap(task => {
      const project = projects.find(item => item.id === task.project_id && item.owner === session.owner);
      return project && task.owner === session.owner ? [{ project, task }] : [];
    });
    const rows: RecoveryTarget[] = new Array(candidates.length);
    let cursor = 0;
    void Promise.all(Array.from({ length: Math.min(3, candidates.length) }, async () => {
      while (active && current() && cursor < candidates.length) {
        const index = cursor++, candidate = candidates[index];
        rows[index] = await readRecoveryTarget(session, candidate.project, candidate.task);
      }
    })).then(() => { if (active && current()) setTargets(rows); });
    return () => { active = false; };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identity, valid, projects, tasks, refresh]);

  function finish() {
    flight.current = false;
    if (current()) setBusy(false);
  }
  function acceptSaved(taskId: string): boolean {
    const state = useDevelopment.getState();
    const task = state.tasks.find(item => item.id === taskId && item.owner === session.owner);
    const saved = state.sessions.find(item => item.id === session.id && item.owner === session.owner && item.engine === session.engine && item.workspace === session.workspace);
    if (state.epoch !== epoch || state.workspaceOwner !== workspaceOwner || !task || !saved || saved.task_id !== task.id || saved.project_id !== task.project_id) return false;
    // This record-level reindex follows host-confirmed ownership even after navigation; it cannot reveal a late UI result.
    preserveClassifiedConversationDraft(session, saved);
    if (current() && !hasUnclassifiedRecoveryDraft(saved)) onClassified(saved);
    return true;
  }
  async function classify() {
    if (!current() || flight.current || uncertainTaskId || !unclassified || !selected) return;
    const state = useDevelopment.getState();
    const source = state.sessions.find(item => item.id === session.id);
    const project = state.projects.find(item => item.id === selected.project.id && item.owner === session.owner);
    const task = state.tasks.find(item => item.id === selected.task.id && item.owner === session.owner && item.project_id === project?.id);
    if (!source || source.project_id !== null || source.task_id !== null || !project || !task) return;
    flight.current = true; setBusy(true); setError(null);
    const checked = await readRecoveryTarget(session, project, task);
    if (!current()) { finish(); return; }
    const now = useDevelopment.getState();
    const currentProject = now.projects.find(item => item.id === project.id && item.owner === session.owner);
    const currentTask = now.tasks.find(item => item.id === task.id && item.owner === session.owner && item.project_id === project.id);
    const currentSource = now.sessions.find(item => item.id === session.id && item.owner === session.owner);
    if (!currentTask || !currentProject || currentProject.archived || currentProject.path !== project.path || currentProject.directory_identity !== project.directory_identity || currentSource?.project_id !== null || currentSource.task_id !== null) {
      setError(currentProject?.archived ? 'project_archived' : 'workspace_changed');
      setRefresh(value => value + 1); finish(); return;
    }
    if (checked.reason || checked.workspace?.id !== selected.workspace?.id || checked.workspace?.directory_identity !== selected.workspace?.directory_identity) {
      setError(checked.error ?? (checked.reason === 'archived' ? 'project_archived' : 'workspace_changed'));
      setRefresh(value => value + 1); finish(); return;
    }
    // The host retains final ownership, directory identity, archived-project and active-session admission checks.
    const result = await useDevelopment.getState().classify(session.id, task.id);
    const applied = result.ok && acceptSaved(task.id);
    if (!current() || applied) { finish(); return; }
    if (result.ok || result.code === 'unknown') setUncertainTaskId(task.id);
    else setError(result.code);
    finish();
  }
  async function inspectClassification() {
    if (!current() || flight.current || !uncertainTaskId) return;
    flight.current = true; setBusy(true); setError(null);
    const loaded = await useDevelopment.getState().load();
    if (loaded) acceptSaved(uncertainTaskId);
    if (!current()) { finish(); return; }
    if (!loaded) setError(useDevelopment.getState().error ?? 'unknown');
    finish();
  }
  function navigate(action: () => void) {
    if (!current() || flight.current) return;
    onClose(); action();
  }

  function activateOriginalDraft() {
    if (!current() || flight.current || !savedSource || !draftConflict) return;
    if (activateRecoveredConversationDraft(savedSource, draftConflict)) onClassified(savedSource);
  }
  async function copyOriginalDraft() {
    if (!current() || flight.current || !savedSource || !draftConflict) return;
    flight.current = true; setBusy(true); setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(draftConflict.original);
      if (current()) onClassified(savedSource);
    } catch { if (current()) setCopyFailed(true); }
    finish();
  }

  if (!valid) return null;
  const durableRuns = history?.runs?.ok ? history.runs.value.filter(run => run.session_id === session.id).sort((a, b) => a.started_at - b.started_at) : [];
  const groups = durableRuns.length ? durableRuns.map(run => ({ id: run.id, truncated: run.truncated, messages: recoveryMessages(run.events) }))
    : history?.snapshot.ok ? [{ id: session.id, truncated: Boolean(history.snapshot.value.history_truncated), messages: recoveryMessages(history.snapshot.value.events) }] : [];
  const hasMessages = groups.some(group => group.messages.length);
  return <Modal open title={t('workbenchNavigation.recoveryTitle')} description={t('workbenchNavigation.recoveryReadOnly')} onOpenChange={open => { if (!open && !flight.current) onClose(); }} footer={<>
    <Button variant="secondary" disabled={busy} onClick={onClose}>{t('common.close')}</Button>
    {savedSource?.project_id && savedSource.task_id ? <Button disabled={busy} onClick={() => { if (current()) onClassified(savedSource); }}>{t('workbenchNavigation.recoveryOpenCurrent')}</Button>
      : uncertainTaskId ? <Button disabled={busy} onClick={() => void inspectClassification()}>{t('workbenchNavigation.recoveryInspect')}</Button>
        : <Button disabled={busy || !unclassified || !selected} onClick={() => void classify()}>{t(busy ? 'dev.busy' : 'workbenchNavigation.recoveryClassify')}</Button>}
  </>}>
    <div className="dev-review-stack" data-testid="unclassified-session-recovery" aria-busy={busy}>
      <section className="dev-review-stack" aria-label={t('workbenchNavigation.recoveryMetadata')}>
        <h3 ref={initialFocus} tabIndex={-1} className="m-0 text-sm">{t('workbenchNavigation.recoveryMetadata')}</h3>
        <dl className="m-0 grid gap-2 text-xs">
          <div><dt>{t('workbenchNavigation.copySessionId')}</dt><dd className="m-0 break-all">{session.id}</dd></div>
          <div><dt>{t('workbenchNavigation.recoveryNativeId')}</dt><dd className="m-0 break-all">{session.native_id ?? t('workbenchNavigation.recoveryUnknown')}</dd></div>
          <div><dt>{t('workbenchNavigation.recoveryCliModel')}</dt><dd className="m-0 break-all">{session.engine === 'codex' ? 'Codex' : 'Claude'} · {session.model ?? t('workbenchNavigation.recoveryUnknown')}</dd></div>
          <div><dt>{t('workbenchNavigation.recoveryExecutable')}</dt><dd className="m-0 break-all">{session.executable ?? t('workbenchNavigation.recoveryUnknown')}</dd></div>
          <div><dt>{t('workbenchNavigation.recoveryWorkspace')}</dt><dd className="m-0 break-all">{session.workspace}</dd></div>
          <div><dt>{t('workbenchNavigation.recoveryOwnership')}</dt><dd className="m-0 break-all">{savedSource?.project_id ?? t('workbenchNavigation.recoveryUnlinked')} / {savedSource?.task_id ?? t('workbenchNavigation.recoveryUnlinked')}</dd></div>
          <div><dt>{t('workbenchNavigation.recoveryActivity')}</dt><dd className="m-0">{history?.summary?.last_activity_at ? <time dateTime={new Date(history.summary.last_activity_at * 1000).toISOString()}>{formatDateTime(new Date(history.summary.last_activity_at * 1000), locale)}</time> : t('workbenchNavigation.recoveryUnknown')}</dd></div>
        </dl>
      </section>
      <section className="dev-review-stack" aria-label={t('workbenchNavigation.recoveryHistory')}>
        <h3 className="m-0 text-sm">{t('workbenchNavigation.recoveryHistory')}</h3>
        {!history ? <p role="status" className="m-0 text-xs">{t('review.loading')}</p> : <>
          {history.summary?.preview ? <blockquote className="m-0 whitespace-pre-wrap break-words text-xs">{history.summary.preview}</blockquote> : <p className="m-0 text-xs text-ink-secondary">{t(history.summaryFailed ? 'workbenchNavigation.metadataUnavailable' : 'workbenchNavigation.noPreview')}</p>}
          <p className="m-0 text-xs text-ink-secondary">{t('workbenchNavigation.recoveryHistoryBoundary')}</p>
          {!session.task_id ? <p className="dev-evidence-warning">{t('workbenchNavigation.recoveryNoTaskHistory')}</p> : null}
          {history.runs && !history.runs.ok ? <p className="m-0 text-xs">{t('workbenchNavigation.recoveryHistoryUnavailable')} {errorText(history.runs.code)}</p> : null}
          {!history.snapshot.ok ? <p className="m-0 text-xs">{t('workbenchNavigation.recoverySnapshotUnavailable')} {errorText(history.snapshot.code)}</p> : null}
          {!hasMessages ? <p className="m-0 text-xs">{t('workbenchNavigation.recoveryNoMessages')}</p> : groups.map(group => <div key={group.id} className="dev-review-stack">
            {group.truncated ? <p className="dev-evidence-warning">{t('agent.truncated')}</p> : null}
            {group.messages.map(message => <article key={message.seq} className="dev-review text-xs"><strong>{t(message.role === 'user' ? 'agent.you' : 'agent.assistant')}</strong><p className="m-0 whitespace-pre-wrap break-words">{message.text}</p></article>)}
          </div>)}
        </>}
      </section>
      <section className="dev-review-stack" aria-label={t('workbenchNavigation.recoveryTarget')}>
        <h3 className="m-0 text-sm">{t('workbenchNavigation.recoveryTarget')}</h3>
        <p className="m-0 text-xs text-ink-secondary">{t('workbenchNavigation.recoveryMatchBoundary')}</p>
        {!unclassified ? <p className="dev-evidence-warning">{t('workbenchNavigation.recoveryLinkedBoundary')}</p> : null}
        {targets === null ? <p role="status" className="m-0 text-xs">{t('review.loading')}</p> : <fieldset className="dev-runtime-fields" disabled={busy || Boolean(uncertainTaskId) || !unclassified}>
          <legend>{t('workbenchNavigation.recoveryTarget')}</legend>
          {targets.map(target => <div key={target.task.id} className="dev-review-stack">
            <label className="dev-checkbox"><input type="radio" name={groupId} value={target.task.id} checked={selectedTaskId === target.task.id} disabled={Boolean(target.reason)} aria-describedby={`${groupId}-${target.task.id}`} onChange={() => setSelectedTaskId(target.task.id)} />{target.project.name} / {target.task.goal}</label>
            <p id={`${groupId}-${target.task.id}`} className="m-0 break-words text-xs text-ink-secondary">{target.workspace?.path ?? t('workbenchNavigation.recoveryUnknownWorkspace')}{target.reason ? <> — {t(`workbenchNavigation.recoveryTarget.${target.reason}`)}{target.error ? ` ${errorText(target.error)}` : ''}</> : null}</p>
          </div>)}
          {!targets.length || !targets.some(target => !target.reason) ? <p className="m-0 text-xs">{t('workbenchNavigation.recoveryNoTarget')}</p> : null}
        </fieldset>}
        <div className="flex flex-wrap gap-2">
          <Button size="xs" variant="secondary" disabled={busy} onClick={() => navigate(onManageProjects)}>{t('workbenchNavigation.recoveryManageProjects')}</Button>
          {availableProjects.map(project => <Button key={project.id} size="xs" variant="secondary" disabled={busy} onClick={() => navigate(() => onManageTasks(project.id))}>{t('workbenchNavigation.recoveryManageTasks', { name: project.name })}</Button>)}
          <Button size="xs" variant="ghost" disabled={busy} onClick={() => setRefresh(value => value + 1)}>{t('review.refresh')}</Button>
        </div>
      </section>
      {draftConflict ? <section className="dev-review-stack" aria-label={t('workbenchNavigation.recoveryDraftConflict')}>
        <h3 className="m-0 text-sm">{t('workbenchNavigation.recoveryDraftConflict')}</h3>
        <p className="dev-evidence-warning">{t('workbenchNavigation.recoveryDraftNotice')}</p>
        <label className="dev-field">{t('workbenchNavigation.recoveryDraftOriginal')}<textarea className="dev-editor" rows={3} readOnly value={draftConflict.original} /></label>
        <label className="dev-field">{t('workbenchNavigation.recoveryDraftCurrent')}<textarea className="dev-editor" rows={3} readOnly value={draftConflict.current} /></label>
        <div className="flex flex-wrap gap-2">
          <Button size="xs" variant="secondary" disabled={busy} onClick={activateOriginalDraft}>{t('workbenchNavigation.recoveryUseOriginal')}</Button>
          <Button size="xs" variant="secondary" disabled={busy} onClick={() => void copyOriginalDraft()}>{t('workbenchNavigation.recoveryCopyOriginal')}</Button>
        </div>
        {copyFailed ? <p role="alert" className="m-0 text-xs">{t('workbenchNavigation.copyFailed')}</p> : null}
      </section> : null}
      {uncertainTaskId ? <p role="status" className="dev-evidence-warning">{t('workbenchNavigation.recoveryUnknownResult')}</p> : null}
      {error ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(error)}</p> : null}
    </div>
  </Modal>;
}
