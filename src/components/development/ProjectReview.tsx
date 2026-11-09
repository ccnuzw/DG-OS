import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Tabs } from '@dgos/ui';
import './workbench-tree.css';
import { formatDateTime, useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { readDevelopmentProjectGit, readDevelopmentProjectDiff } from '../../stores/developmentExplorer';
import { contextGeneration } from '../../stores/developmentSelection';
import { readDevelopmentReview, readDevelopmentDiff, listDevelopmentRuns, type DevDiff, type DevRun } from '../../stores/developmentReview';
import type { Outcome } from '../../stores/developmentStore';
import { errorText } from './shared';
import type { CodeView } from './WorkbenchLayout';
import { discardDevelopmentDraft } from '../../stores/developmentEditor';
import { ProjectCapabilities } from './ProjectCapabilities';
import { SessionConfigurationViewDisplay } from './CapabilitySnapshotView';
import { DeliveryPanel } from './DeliveryPanel';
import { TaskReview } from './TaskReview';
import type { CliProviderRevision, CliProviderSnapshot } from '../../stores/developmentCapabilities';
import type { ConversationReference } from '../../stores/developmentReferences';

import { Files, type FileTarget } from './ProjectFiles';
import { useWorkbenchLocation } from './workbenchLocations';

// Each resource has an independent request generation. Unmount, account reset and context intent all invalidate it.
function useRead<T>(load: () => Promise<Outcome<T>>) {
  const [value, setValue] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ticket = useRef(0);
  const refresh = useCallback(async () => {
    const current = ++ticket.current;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    setBusy(true); setError(null); setValue(null);
    const result = await load();
    if (ticket.current !== current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    if (result.ok) setValue(result.value); else setError(result.code);
    setBusy(false);
  }, [load]);
  useEffect(() => { void refresh(); return () => { ++ticket.current; }; }, [refresh]);
  return { value, error, busy, refresh };
}

function Feedback({ busy, error }: { busy: boolean; error: string | null }) {
  const { t } = useLocale();
  return <>{busy ? <p role="status" className="m-0 text-xs text-ink-secondary">{t('review.loading')}</p> : null}{error ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(error)}</p> : null}</>;
}

function ContentReason({ reason }: { reason: string | null }) {
  const { t } = useLocale();
  const known = ['file_sensitive', 'file_binary', 'file_too_large', 'file_access_denied', 'file_not_found', 'file_changed', 'file_symlink', 'scan_limit', 'baseline_unavailable', 'baseline_missing'];
  return <p className="m-0 text-xs text-ink-secondary">{t(`review.reason.${reason && known.includes(reason) ? reason : 'unavailable'}`)}</p>;
}

export function SourceLines({ text, label }: { text: string; label: string }) {
  return <div className="dev-source" tabIndex={0} role="region" aria-label={label}><ol>{text.split('\n').map((line, index) => <li key={index}><code>{line || ' '}</code></li>)}</ol></div>;
}

export function ProjectReview({ projectId, taskId, sessions = [], live = {}, capabilityRevision = 0, workspacePath = '', onHandoff, onWorkspaceChange, view, views = ['files', 'changes', 'review', 'delivery', 'capabilities'], onViewChange, fileTarget, snapshot, onUseConfiguration, onOpenRuntime, onOpenFile }: { projectId: string; taskId: string | null; sessions?: {id:string;engine:'codex'|'claude'}[]; live?: Record<string,string>; capabilityRevision?: number; workspacePath?: string; onHandoff?: (sessionId:string)=>void; onWorkspaceChange?: (path:string)=>void; view?: CodeView; views?: readonly CodeView[]; onViewChange?: (view: CodeView)=>void; fileTarget?: FileTarget | null; snapshot?: CliProviderSnapshot | null; onUseConfiguration?: (revision: CliProviderRevision)=>void; onOpenRuntime?: ()=>void; onOpenFile?: (target: FileTarget) => void }) {
  const { t } = useLocale();
  const [localTab, setTab] = useState<CodeView>('files');
  const tab = view ?? localTab;
  const [visited, setVisited] = useState<Set<string>>(() => new Set([tab]));
  useEffect(() => { setVisited(previous => previous.has(tab) ? previous : new Set([...previous, tab])); }, [tab]);
  function changeTab(next: string) {
    if (!['files', 'changes', 'capabilities', 'delivery', 'review'].includes(next)) return;
    setTab(next as CodeView); onViewChange?.(next as CodeView);
  }
  const [reviewTarget, setReviewTarget] = useState<FileTarget | null>(null);
  const target = fileTarget && (!reviewTarget || fileTarget.nonce > reviewTarget.nonce) ? fileTarget : reviewTarget;
  return <section className="dev-review" aria-label={t('review.title')}>
    <Tabs value={tab} activationMode="manual" onValueChange={changeTab}>
      <Tabs.List aria-label={t('review.title')}>
        {views.map(item => <Tabs.Trigger key={item} value={item} data-testid={`dev-review-tab-${item}`}>{t(item === 'review' ? 'workflow.review' : item === 'capabilities' ? 'capabilities.title' : item === 'delivery' ? 'delivery.title' : `review.${item}`)}</Tabs.Trigger>)}
      </Tabs.List>
      <Tabs.Content forceMount hidden={tab !== 'files'} value="files">{visited.has('files') ? <Files projectId={projectId} taskId={taskId} workspacePath={workspacePath} target={target?.mode === 'file' ? target : null} /> : null}</Tabs.Content>
      <Tabs.Content forceMount hidden={tab !== 'changes'} value="changes">{visited.has('changes') ? taskId ? <Changes projectId={projectId} taskId={taskId} target={target?.mode === 'diff' ? target : null} /> : <ProjectChanges projectId={projectId} target={target?.mode === 'diff' ? target : null} /> : null}</Tabs.Content>
      <Tabs.Content forceMount hidden={tab !== 'review'} value="review">{visited.has('review') && taskId ? <TaskReview projectId={projectId} taskId={taskId} onOpenDiff={async path => { const target: FileTarget = { path, mode: 'diff', nonce: Date.now() }; if (onOpenFile) { onOpenFile(target); return; } if (await discardDevelopmentDraft()) { setReviewTarget(target); changeTab('changes'); } }} onOpenRuntime={() => onOpenRuntime?.()} onOpenDelivery={() => changeTab('delivery')} /> : null}</Tabs.Content>
      <Tabs.Content forceMount hidden={tab !== 'capabilities'} value="capabilities">{visited.has('capabilities') ? <ProjectCapabilities projectId={projectId} snapshot={snapshot} onUseConfiguration={onUseConfiguration} /> : null}</Tabs.Content>
      <Tabs.Content forceMount hidden={tab !== 'delivery'} value="delivery">{visited.has('delivery') ? <DeliveryPanel projectId={projectId} taskId={taskId} sessions={sessions} live={live} capabilityRevision={capabilityRevision} workspacePath={workspacePath} onHandoff={onHandoff} onWorkspaceChange={onWorkspaceChange} /> : null}</Tabs.Content>
    </Tabs>
  </section>;
}


function Evidence({ capturedAt, late, truncated, stale }: { capturedAt: number | null; late: boolean; truncated: boolean; stale: boolean }) {
  const { t } = useLocale();
  return <div className="dev-review-stack" role="status"><p className="m-0 text-xs text-ink-secondary">{capturedAt === null ? t('review.noBaseline') : t('review.baselineAt', { time: formatDateTime(capturedAt * 1000) })}</p>{late ? <p className="dev-evidence-warning">{t('review.baselineLate')}</p> : null}{truncated ? <p className="dev-evidence-warning">{t('review.reviewTruncated')}</p> : null}{stale ? <p className="dev-evidence-warning">{t('review.stale')}</p> : null}</div>;
}

function ProjectChanges({ projectId, target }: { projectId: string; target?: FileTarget | null }) {
  const { t } = useLocale();
  const git = useRead(useCallback(() => readDevelopmentProjectGit(projectId), [projectId]));
  const [selected, setSelected] = useWorkbenchLocation<string | null>(`${projectId}:project-change`, null);
  useEffect(() => { if (target) setSelected(target.path); }, [target?.nonce]);
  return <div className="dev-review-stack"><div className="dev-review-toolbar"><p className="m-0 text-xs text-ink-secondary">{t('workbenchGit.projectScope')}</p><Button size="xs" variant="secondary" disabled={git.busy} onClick={() => { setSelected(null); void git.refresh(); }}>{t('review.refresh')}</Button></div><Feedback busy={git.busy} error={git.error} />
    {git.value ? <><p className="m-0 break-all text-xs">{git.value.branch ?? t('delivery.detached')} · {git.value.head ?? t('delivery.unknown')}</p>
      {!git.value.files.length ? <p className="text-xs text-ink-secondary">{t('workbenchGit.noChanges')}</p> : <nav className="dev-change-list" aria-label={t('review.changes')}>{git.value.files.map(file => <div key={file.path} className="dev-change-tree-group"><Button size="xs" variant={selected === file.path ? 'secondary' : 'ghost'} className="dev-change-entry" onClick={() => setSelected(file.path)}><span className="break-all">{file.path}</span><span className="dev-evidence-tags">{file.index !== ' ' ? t('delivery.stagedDiff') : null}{file.worktree !== ' ' ? t('delivery.worktreeDiff') : null}</span></Button></div>)}</nav>}
      {selected ? <ChangeDiff taskId={null} projectId={projectId} path={selected} source="worktree" /> : null}</> : null}
  </div>;
}

function Changes({ projectId, taskId, target }: { projectId: string; taskId: string; target?: FileTarget | null }) {
  const { t } = useLocale();
  const review = useRead(useCallback(() => readDevelopmentReview(taskId), [taskId]));
  const [selected, setSelected] = useWorkbenchLocation<string | null>(`${taskId}:change`, null);
  useEffect(() => { if (target) setSelected(target.path); }, [target?.nonce]);
  return <div className="dev-review-stack"><div className="dev-review-toolbar"><p className="m-0 text-xs text-ink-secondary">{t('review.uncertainty')}</p><Button size="xs" variant="secondary" data-testid="dev-review-refresh" disabled={review.busy} onClick={() => { setSelected(null); void review.refresh(); }}>{t('review.refresh')}</Button></div><Feedback busy={review.busy} error={review.error} />
    {review.value ? <><Evidence capturedAt={review.value.captured_at} late={review.value.baseline_late} truncated={review.value.truncated} stale={review.value.runs_stale} />
      {review.value.excluded?.length ? <section className="dev-review-stack" aria-label={t('review.excluded')}><strong className="text-xs">{t('review.excluded')}</strong><ul className="m-0 grid gap-2 pl-4">{review.value.excluded.map((entry, index) => <li key={`${entry.path}:${index}`}><span className="break-all text-xs">{entry.path}</span><ContentReason reason={entry.reason} /></li>)}</ul></section> : null}
      {!review.value.changes.length ? <p className="text-xs text-ink-secondary">{t('review.noChanges')}</p> : null}
      <nav className="dev-change-list" aria-label={t('review.changes')}>{review.value.changes.map(change => <Button key={`${change.kind}:${change.path}`} size="xs" variant={selected === change.path ? 'secondary' : 'ghost'} className="dev-change-entry" data-testid={`dev-change-item-${change.path}`} aria-pressed={selected === change.path} onClick={() => setSelected(change.path)}>
        <span className="break-all">{change.path}</span><span className="dev-evidence-tags"><span>{t(`review.change.${change.kind}`)}</span>{change.pre_existing && change.kind !== 'pre_existing' ? <span>{t('review.change.pre_existing')}</span> : null}<span>{t(change.origin === 'external' ? 'review.external' : 'review.uncertain')}</span>{!change.previewable ? <span>{t('review.noPreview')}</span> : null}</span>
      </Button>)}</nav>{selected ? <ChangeDiff key={selected} taskId={taskId} projectId={projectId} path={selected} /> : null}</> : null}
  </div>;
}

export function ChangeDiff({ taskId, path, projectId, source: requestedSource, onReference }: { taskId: string | null; path: string; projectId: string; source?: 'task' | 'index' | 'worktree'; onReference?: (reference: ConversationReference) => void }) {
  const source = taskId ? requestedSource ?? 'task' : requestedSource === 'index' ? 'index' : 'worktree';
  const { t } = useLocale();
  const diff = useRead<DevDiff>(useCallback(() => taskId ? readDevelopmentDiff({ task_id: taskId, path, source }) : readDevelopmentProjectDiff({ project_id: projectId, path, source: source === 'index' ? 'index' : 'worktree' }), [taskId, projectId, path, source]));
  const [mode, setMode] = useWorkbenchLocation(`${taskId ?? projectId}:${path}:${source}:diff`, 'unified');
  const [referenceBusy, setReferenceBusy] = useState(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  async function attach() {
    if (!diff.value || diff.value.reason || !diff.value.diff || referenceBusy) return;
    const epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    setReferenceBusy(true);
    try {
      const hash = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(diff.value.diff));
      if (!mounted.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      onReference?.({ project_id: projectId, task_id: taskId, path, kind: 'diff', diff_source: source, expected_hash: Array.from(new Uint8Array(hash), byte => byte.toString(16).padStart(2, '0')).join('') });
    } finally { if (mounted.current) setReferenceBusy(false); }
  }
  const added = diff.value?.diff.split('\n').filter(line => line.startsWith('+') && !line.startsWith('+++')).length ?? 0;
  const deleted = diff.value?.diff.split('\n').filter(line => line.startsWith('-') && !line.startsWith('---')).length ?? 0;
  return <article className="dev-review-stack" data-testid="dev-file-diff">
    <div className="dev-review-toolbar"><strong className="break-all text-xs">{path}</strong><Button size="xs" variant="secondary" disabled={diff.busy} onClick={() => void diff.refresh()}>{t('review.refresh')}</Button>{onReference ? <Button size="xs" variant="secondary" disabled={!diff.value?.diff || Boolean(diff.value.reason) || referenceBusy} onClick={() => void attach()}>{t('workbenchFiles.referenceDiff')}</Button> : null}</div>
    <p className="text-xs">{t(source === 'task' ? 'workbenchFiles.taskDiff' : source === 'index' ? 'workbenchFiles.indexDiff' : 'workbenchFiles.worktreeDiff')}</p>
    <Feedback busy={diff.busy} error={diff.error} />
    {diff.value ? <><p className="m-0 text-xs text-ink-secondary">{t('review.diffCounts', { added, deleted })}</p>{diff.value.reason ? <ContentReason reason={diff.value.reason} /> : null}<Tabs value={mode} onValueChange={setMode}><Tabs.List aria-label={t('review.diffView')}>{['unified', 'before', 'after'].map(item => <Tabs.Trigger key={item} value={item}>{t(`review.${item}`)}</Tabs.Trigger>)}</Tabs.List><Tabs.Content value="unified">{diff.value.diff ? <pre className="dev-diff" tabIndex={0} aria-label={t('review.unified')}>{diff.value.diff.split('\n').map((line, index) => <span key={index} className={line.startsWith('+') ? 'dev-diff-added' : line.startsWith('-') ? 'dev-diff-deleted' : line.startsWith('@@') ? 'dev-diff-hunk' : undefined}>{line}{'\n'}</span>)}</pre> : <p className="text-xs">{t('delivery.emptyDiff')}</p>}</Tabs.Content><Tabs.Content value="before">{diff.value.before !== null ? <SourceLines text={diff.value.before} label={t('review.before')} /> : <p className="text-xs">{t('delivery.emptyDiff')}</p>}</Tabs.Content><Tabs.Content value="after">{diff.value.after !== null ? <SourceLines text={diff.value.after} label={t('review.after')} /> : <p className="text-xs">{t('delivery.emptyDiff')}</p>}</Tabs.Content></Tabs></> : null}
  </article>;
}

export function Runs({ taskId, targetRunId }: { taskId: string; targetRunId?: string | null }) {
  const { t } = useLocale();
  const runs = useRead(useCallback(() => listDevelopmentRuns(taskId), [taskId]));
  const review = useRead(useCallback(() => readDevelopmentReview(taskId), [taskId]));
  const viewport = useRef<HTMLDivElement>(null);
  useEffect(() => { if (!targetRunId || !runs.value) return; const item = Array.from(viewport.current?.querySelectorAll<HTMLElement>('[data-run-id]') ?? []).find(node => node.dataset.runId === targetRunId); item?.focus(); item?.scrollIntoView?.({ block: 'nearest' }); }, [targetRunId, runs.value]);
  return <div ref={viewport} className="dev-review-stack" data-testid="dev-run-history"><div className="dev-review-toolbar"><p className="m-0 text-xs text-ink-secondary">{t('review.persistedRuns')}</p><Button size="xs" variant="secondary" data-testid="dev-review-refresh" disabled={runs.busy || review.busy} onClick={() => { void runs.refresh(); void review.refresh(); }}>{t('review.refresh')}</Button></div><Feedback busy={runs.busy || review.busy} error={runs.error ?? review.error} />{targetRunId && runs.value && !runs.value.some(run => run.id === targetRunId) ? <p role="status" className="text-xs dev-evidence-warning">{t('agent.referenceRunUnavailable')}</p> : null}{review.value ? <Evidence capturedAt={review.value.captured_at} late={review.value.baseline_late} truncated={review.value.truncated} stale={review.value.runs_stale} /> : null}{runs.value ? runs.value.length ? runs.value.map(run => <Run key={run.id} run={run} stale={Boolean(review.value && run.fingerprint !== review.value.fingerprint)} />) : <p className="text-xs text-ink-secondary">{t('review.noRuns')}</p> : null}</div>;
}

function Run({ run, stale }: { run: DevRun; stale: boolean }) {
  const { t } = useLocale();
  return <article className="dev-run dev-review-stack" tabIndex={-1} data-run-id={run.id}>
    <header className="dev-review-toolbar"><strong className="text-xs">{t('direct.source.native_agent')} · {run.engine === 'codex' ? 'Codex' : 'Claude'}</strong><span className="text-xs">{t(`review.run.${run.status}`)}</span></header>
    <p className="m-0 text-xs text-ink-secondary">{formatDateTime(run.started_at * 1000)}{run.finished_at !== null ? ` — ${formatDateTime(run.finished_at * 1000)}` : ''}</p>
    <details className="dev-technical-details"><summary>{t('agent.details')}</summary><p className="m-0 break-all text-xs text-ink-secondary">{run.id}</p></details>
    {run.summary ? <p className="m-0 whitespace-pre-wrap text-sm">{run.summary}</p> : null}
    {run.reason ? <p role="status" className="m-0 whitespace-pre-wrap text-xs text-status-error">{run.reason}</p> : null}
    {stale ? <p className="dev-evidence-warning">{t('review.runStale')}</p> : null}
    {run.truncated ? <p className="dev-evidence-warning">{t('review.runTruncated')}</p> : null}
    <SessionConfigurationViewDisplay value={run.configuration ?? null} />
    <details><summary className="text-xs">{t('review.events', { count: run.events.length })}</summary><div className="dev-review-stack">
      {run.events.map(event => {
        const raw = event.data && typeof event.data === 'object' && !Array.isArray(event.data) ? event.data : {};
        const item = raw.item && typeof raw.item === 'object' && !Array.isArray(raw.item) ? raw.item : {};
        const data = { ...raw, ...item };
        const command = typeof data.command === 'string' ? data.command : Array.isArray(data.command) ? data.command.join(' ') : null;
        const cwd = typeof data.cwd === 'string' ? data.cwd : null;
        const output = [data.aggregatedOutput, data.aggregated_output, data.output, data.stdout, data.stderr].filter(value => value !== undefined && value !== null).map(value => typeof value === 'string' ? value : JSON.stringify(value)).join('\n');
        const exitCode = data.exitCode ?? data.exit_code;
        const commandEvent = /command|exec|bash|shell/i.test(event.kind) || typeof data.type === 'string' && /command|exec|bash|shell/i.test(data.type) || typeof data.name === 'string' && /command|exec|bash|shell/i.test(data.name);
        return <article key={event.seq} className="dev-run-event">
          <strong className="text-xs">{event.kind}{typeof data.name === 'string' ? `: ${data.name}` : ''}</strong>
          {event.text ? <pre className="dev-event-text">{event.text}</pre> : null}
          {command ? <pre className="dev-event-text">{command}</pre> : null}
          {cwd ? <p className="break-all text-xs">{t('review.cwd')}: {cwd}</p> : null}
          {output ? <pre className="dev-event-text">{output}</pre> : null}
          {commandEvent || command ? <p className="text-xs">{t('review.exitCode')}: {typeof exitCode === 'number' ? exitCode : t('review.run.unknown')}</p> : null}
          {event.data !== null ? <details><summary className="text-xs">{t('review.eventData')}</summary><pre className="dev-event-text">{JSON.stringify(event.data, null, 2)}</pre></details> : null}
        </article>;
      })}
    </div></details>
  </article>;
}
