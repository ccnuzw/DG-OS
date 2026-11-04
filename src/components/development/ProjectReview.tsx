import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Input, Tabs } from '@dgos/ui';
import { formatDateTime, useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { listDevelopmentFiles, readDevelopmentReview, readDevelopmentDiff, listDevelopmentRuns, type DevDiff, type DevRun } from '../../stores/developmentReview';
import type { Outcome } from '../../stores/developmentStore';
import { errorText } from './shared';
import { discardDevelopmentDraft } from '../../stores/developmentEditor';
import { FileEditor } from './FileEditor';
import { DevelopmentRuntime } from './DevelopmentRuntime';
import { ProjectCapabilities } from './ProjectCapabilities';
import { CapabilitySnapshotView } from './CapabilitySnapshotView';

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
  return <>{busy ? <p role="status" className="m-0 text-xs text-muted-foreground">{t('review.loading')}</p> : null}{error ? <p role="alert" className="m-0 text-xs text-destructive">{errorText(error)}</p> : null}</>;
}

function ContentReason({ reason }: { reason: string | null }) {
  const { t } = useLocale();
  const known = ['file_sensitive', 'file_binary', 'file_too_large', 'file_access_denied', 'file_not_found', 'file_changed', 'file_symlink', 'scan_limit', 'baseline_unavailable', 'baseline_missing'];
  return <p className="m-0 text-xs text-muted-foreground">{t(`review.reason.${reason && known.includes(reason) ? reason : 'unavailable'}`)}</p>;
}

export function SourceLines({ text, label }: { text: string; label: string }) {
  return <div className="dev-source" tabIndex={0} role="region" aria-label={label}><ol>{text.split('\n').map((line, index) => <li key={index}><code>{line || ' '}</code></li>)}</ol></div>;
}

export function ProjectReview({ projectId, taskId }: { projectId: string; taskId: string | null }) {
  const { t } = useLocale();
  const [tab, setTab] = useState('files');
  const changingTab = useRef(false);
  async function changeTab(next: string) {
    if (next === tab || changingTab.current) return;
    changingTab.current = true;
    try {
      if (await discardDevelopmentDraft()) setTab(next);
    } finally {
      changingTab.current = false;
    }
  }
  return <section className="dev-review" aria-label={t('review.title')}>
    {/* Focus returns here when discard is cancelled; focusing an inactive tab must not reopen it. */}
    <Tabs value={tab} activationMode="manual" onValueChange={next => { void changeTab(next); }}>
      <Tabs.List aria-label={t('review.title')}>
        {['files', 'changes', 'runs', 'develop', 'capabilities'].map(item => <Tabs.Trigger key={item} value={item} data-testid={`dev-review-tab-${item}`}>{t(item === 'develop' ? 'direct.title' : item === 'capabilities' ? 'capabilities.title' : `review.${item}`)}</Tabs.Trigger>)}
      </Tabs.List>
      <Tabs.Content value="files"><Files projectId={projectId} taskId={taskId} /></Tabs.Content>
      <Tabs.Content value="changes">{taskId ? <Changes taskId={taskId} /> : <p className="text-xs text-muted-foreground">{t('dev.noTaskSelected')}</p>}</Tabs.Content>
      <Tabs.Content value="runs">{taskId ? <Runs taskId={taskId} /> : <p className="text-xs text-muted-foreground">{t('dev.noTaskSelected')}</p>}</Tabs.Content>
      <Tabs.Content value="develop">{taskId ? <DevelopmentRuntime projectId={projectId} taskId={taskId} /> : <p className="text-xs text-muted-foreground">{t('dev.noTaskSelected')}</p>}</Tabs.Content>
      <Tabs.Content value="capabilities"><ProjectCapabilities projectId={projectId} /></Tabs.Content>
    </Tabs>
  </section>;
}

function Files({ projectId, taskId }: { projectId: string; taskId: string | null }) {
  const { t } = useLocale();
  const [path, setPath] = useState('');
  const [query, setQuery] = useState('');
  const [ignored, setIgnored] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const listing = useRead(useCallback(() => listDevelopmentFiles({ project_id: projectId, path, query, include_ignored: ignored }), [projectId, path, query, ignored]));
  async function change(action: () => void) { if (await discardDevelopmentDraft()) action(); }
  function navigate(next: string) { void change(() => { setPath(next); setSelected(null); setQuery(''); }); }
  const segments = path.split('/').filter(Boolean);
  return <div className="dev-review-stack">
    <div className="dev-review-toolbar"><Input type="search" data-testid="dev-file-search" label={t('review.search')} value={query} onChange={event => { const value = event.target.value; void change(() => { setQuery(value); setSelected(null); }); }} />
      <Button size="xs" variant="secondary" data-testid="dev-review-refresh" disabled={listing.busy} onClick={() => void change(() => { setSelected(null); void listing.refresh(); })}>{t('review.refresh')}</Button></div>
    <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={ignored} onChange={event => { const checked = event.target.checked; void change(() => { setIgnored(checked); setSelected(null); }); }} />{t('review.includeIgnored')}</label>
    <nav className="dev-review-breadcrumb" aria-label={t('review.directory')}><Button size="xs" variant="ghost" onClick={() => navigate('')}>{t('review.root')}</Button>{segments.map((segment, index) => <Button key={index} size="xs" variant="ghost" onClick={() => navigate(segments.slice(0, index + 1).join('/'))}>{segment}</Button>)}</nav>
    <Feedback busy={listing.busy} error={listing.error} />
    {listing.value ? <><p className="m-0 text-xs text-muted-foreground">{query ? t('review.searchHint') : t('direct.editHint')}</p>
      {listing.value.truncated ? <p role="status" className="dev-evidence-warning">{t('review.filesTruncated')}</p> : null}
      <div className="dev-file-layout"><nav aria-label={t('review.files')} className="dev-file-list">
        {!listing.value.entries.length ? <p className="text-xs text-muted-foreground">{t('review.noFiles')}</p> : null}
        {listing.value.entries.map(entry => <Button key={entry.path} size="xs" variant={selected === entry.path ? 'secondary' : 'ghost'} className="dev-file-entry" data-testid={`dev-file-item-${entry.path}`} aria-pressed={selected === entry.path} disabled={entry.kind === 'symlink' || entry.kind === 'blocked'} onClick={() => entry.kind === 'directory' ? navigate(entry.path) : entry.path !== selected && void change(() => setSelected(entry.path))}>
          <span aria-hidden="true">{entry.kind === 'directory' ? '▸' : '·'}</span><span className="break-all">{query ? entry.path : entry.name}</span><span className="text-muted-foreground">{entry.ignored ? t('review.ignored') : ''}{entry.kind === 'symlink' || entry.kind === 'blocked' ? t(`review.kind.${entry.kind}`) : ''}</span>
        </Button>)}
      </nav><div className="min-w-0">{selected ? <FileEditor key={selected} projectId={projectId} taskId={taskId} path={selected} /> : <p className="text-xs text-muted-foreground">{t('review.selectFile')}</p>}</div></div></> : null}
  </div>;
}


function Evidence({ capturedAt, late, truncated, stale }: { capturedAt: number | null; late: boolean; truncated: boolean; stale: boolean }) {
  const { t } = useLocale();
  return <div className="dev-review-stack" role="status"><p className="m-0 text-xs text-muted-foreground">{capturedAt === null ? t('review.noBaseline') : t('review.baselineAt', { time: formatDateTime(capturedAt * 1000) })}</p>{late ? <p className="dev-evidence-warning">{t('review.baselineLate')}</p> : null}{truncated ? <p className="dev-evidence-warning">{t('review.reviewTruncated')}</p> : null}{stale ? <p className="dev-evidence-warning">{t('review.stale')}</p> : null}</div>;
}

function Changes({ taskId }: { taskId: string }) {
  const { t } = useLocale();
  const review = useRead(useCallback(() => readDevelopmentReview(taskId), [taskId]));
  const [selected, setSelected] = useState<string | null>(null);
  return <div className="dev-review-stack"><div className="dev-review-toolbar"><p className="m-0 text-xs text-muted-foreground">{t('review.uncertainty')}</p><Button size="xs" variant="secondary" data-testid="dev-review-refresh" disabled={review.busy} onClick={() => { setSelected(null); void review.refresh(); }}>{t('review.refresh')}</Button></div><Feedback busy={review.busy} error={review.error} />
    {review.value ? <><Evidence capturedAt={review.value.captured_at} late={review.value.baseline_late} truncated={review.value.truncated} stale={review.value.runs_stale} />
      {review.value.excluded?.length ? <section className="dev-review-stack" aria-label={t('review.excluded')}><strong className="text-xs">{t('review.excluded')}</strong><ul className="m-0 grid gap-2 pl-4">{review.value.excluded.map((entry, index) => <li key={`${entry.path}:${index}`}><span className="break-all text-xs">{entry.path}</span><ContentReason reason={entry.reason} /></li>)}</ul></section> : null}
      {!review.value.changes.length ? <p className="text-xs text-muted-foreground">{t('review.noChanges')}</p> : null}
      <nav className="dev-change-list" aria-label={t('review.changes')}>{review.value.changes.map(change => <Button key={`${change.kind}:${change.path}`} size="xs" variant={selected === change.path ? 'secondary' : 'ghost'} className="dev-change-entry" data-testid={`dev-change-item-${change.path}`} aria-pressed={selected === change.path} onClick={() => setSelected(change.path)}>
        <span className="break-all">{change.path}</span><span className="dev-evidence-tags"><span>{t(`review.change.${change.kind}`)}</span>{change.pre_existing && change.kind !== 'pre_existing' ? <span>{t('review.change.pre_existing')}</span> : null}<span>{t(change.origin === 'external' ? 'review.external' : 'review.uncertain')}</span>{!change.previewable ? <span>{t('review.noPreview')}</span> : null}</span>
      </Button>)}</nav>{selected ? <ChangeDiff key={selected} taskId={taskId} path={selected} /> : null}</> : null}
  </div>;
}

function ChangeDiff({ taskId, path }: { taskId: string; path: string }) {
  const { t } = useLocale();
  const diff = useRead<DevDiff>(useCallback(() => readDevelopmentDiff({ task_id: taskId, path }), [taskId, path]));
  const [mode, setMode] = useState('unified');
  const added = diff.value?.diff.split('\n').filter(line => line.startsWith('+') && !line.startsWith('+++')).length ?? 0;
  const deleted = diff.value?.diff.split('\n').filter(line => line.startsWith('-') && !line.startsWith('---')).length ?? 0;
  return <article className="dev-review-stack" data-testid="dev-file-diff"><strong className="break-all text-xs">{path}</strong><Feedback busy={diff.busy} error={diff.error} />{diff.value ? <><p className="m-0 text-xs text-muted-foreground">{t('review.diffCounts', { added, deleted })}</p>{diff.value.reason ? <ContentReason reason={diff.value.reason} /> : null}<Tabs value={mode} onValueChange={setMode}><Tabs.List aria-label={t('review.diffView')}>{['unified', 'before', 'after'].map(item => <Tabs.Trigger key={item} value={item}>{t(`review.${item}`)}</Tabs.Trigger>)}</Tabs.List><Tabs.Content value="unified">{diff.value.diff ? <pre className="dev-diff" tabIndex={0} aria-label={t('review.unified')}>{diff.value.diff.split('\n').map((line, index) => <span key={index} className={line.startsWith('+') ? 'dev-diff-added' : line.startsWith('-') ? 'dev-diff-deleted' : line.startsWith('@@') ? 'dev-diff-hunk' : undefined}>{line}{'\n'}</span>)}</pre> : <p className="text-xs text-muted-foreground">{t('review.noDiff')}</p>}</Tabs.Content><Tabs.Content value="before">{diff.value.before !== null ? <SourceLines text={diff.value.before} label={t('review.before')} /> : <ContentReason reason={diff.value.reason} />}</Tabs.Content><Tabs.Content value="after">{diff.value.after !== null ? <SourceLines text={diff.value.after} label={t('review.after')} /> : <ContentReason reason={diff.value.reason} />}</Tabs.Content></Tabs></> : null}</article>;
}

function Runs({ taskId }: { taskId: string }) {
  const { t } = useLocale();
  const runs = useRead(useCallback(() => listDevelopmentRuns(taskId), [taskId]));
  const review = useRead(useCallback(() => readDevelopmentReview(taskId), [taskId]));
  return <div className="dev-review-stack" data-testid="dev-run-history"><div className="dev-review-toolbar"><p className="m-0 text-xs text-muted-foreground">{t('review.persistedRuns')}</p><Button size="xs" variant="secondary" data-testid="dev-review-refresh" disabled={runs.busy || review.busy} onClick={() => { void runs.refresh(); void review.refresh(); }}>{t('review.refresh')}</Button></div><Feedback busy={runs.busy || review.busy} error={runs.error ?? review.error} />{review.value ? <Evidence capturedAt={review.value.captured_at} late={review.value.baseline_late} truncated={review.value.truncated} stale={review.value.runs_stale} /> : null}{runs.value ? runs.value.length ? runs.value.map(run => <Run key={run.id} run={run} stale={Boolean(review.value && run.fingerprint !== review.value.fingerprint)} />) : <p className="text-xs text-muted-foreground">{t('review.noRuns')}</p> : null}</div>;
}

function Run({ run, stale }: { run: DevRun; stale: boolean }) {
  const { t } = useLocale();
  return <article className="dev-run dev-review-stack">
    <header className="dev-review-toolbar"><strong className="text-xs">{t('direct.source.native_agent')} · {run.engine === 'codex' ? 'Codex' : 'Claude'}</strong><span className="text-xs">{t(`review.run.${run.status}`)}</span></header>
    <p className="m-0 text-xs text-muted-foreground">{formatDateTime(run.started_at * 1000)}{run.finished_at !== null ? ` — ${formatDateTime(run.finished_at * 1000)}` : ''}</p>
    <p className="m-0 break-all text-xs text-muted-foreground">{run.id}</p>
    {run.summary ? <p className="m-0 whitespace-pre-wrap text-sm">{run.summary}</p> : null}
    {run.reason ? <p role="status" className="m-0 whitespace-pre-wrap text-xs text-destructive">{run.reason}</p> : null}
    {stale ? <p className="dev-evidence-warning">{t('review.runStale')}</p> : null}
    {run.truncated ? <p className="dev-evidence-warning">{t('review.runTruncated')}</p> : null}
    <CapabilitySnapshotView snapshot={run.capabilities} />
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
