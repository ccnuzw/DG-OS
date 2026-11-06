import { useEffect, useMemo, useState } from 'react';
import { Button, Input, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { chooseContext } from '../../stores/developmentSelection';
import { useDevelopment, type CreateProjectInput, type Project, type ProjectInspection } from '../../stores/developmentStore';
import { confirmAction, errorText } from './shared';
import { discardDevelopmentDraft } from '../../stores/developmentEditor';
import { guardWorkbenchDrafts, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { processActive, useDevelopmentRuntime } from '../../stores/developmentRuntime';
import { useAgentRunner } from '../../stores/agentRunnerStore';

export function ProjectPanel() {
  const { t } = useLocale();
  const projects = useDevelopment(state => state.projects);
  const operations = useDevelopment(state => state.operations);
  const selectedId = useDevelopment(state => state.selection.project_id);
  const sessions = useDevelopment(state => state.sessions);
  const live = useAgentRunner(state => state.live);
  const processes = useDevelopmentRuntime(state => state.processes);
  const runtimeTaskId = useDevelopmentRuntime(state => state.taskId);
  const [search, setSearch] = useState('');
  const query = search.trim().toLocaleLowerCase();
  const matchingIds = new Set(projects.filter(project => [project.name, project.path, project.id].some(value => value.toLocaleLowerCase().includes(query))).map(project => project.id));
  const activity = useMemo(() => {
    const result = new Map<string, { live: number; running: number; processes: number }>();
    const entry = (id: string) => {
      let value = result.get(id);
      if (!value) { value = { live: 0, running: 0, processes: 0 }; result.set(id, value); }
      return value;
    };
    for (const session of sessions) {
      if (!session.project_id || !live[session.id]) continue;
      const value = entry(session.project_id);
      value.live += 1;
      if (live[session.id] === 'running') value.running += 1;
    }
    for (const process of processes) {
      if (process.task_id === runtimeTaskId && processActive(process)) entry(process.project_id).processes += 1;
    }
    return result;
  }, [sessions, live, processes, runtimeTaskId]);
  const [creating, setCreating] = useState(false);
  const [createOpened, setCreateOpened] = useState(false);
  const visible = projects.filter(project => project.id === selectedId || matchingIds.has(project.id));
  const active = visible.filter(project => !project.archived).sort((a, b) => (b.last_opened_at ?? 0) - (a.last_opened_at ?? 0));
  const archived = visible.filter(project => project.archived);
  const selected = projects.find(project => project.id === selectedId) ?? null;
  const unfinished = operations.filter(operation => operation.status !== 'succeeded');
  const completed = operations.filter(operation => operation.status === 'succeeded');
  return <section className="dev-panel dev-nav-panel" aria-label={t('dev.projects')} data-testid="dev-project-list">
    <header className="dev-panel-header"><h2 className="m-0 text-sm font-semibold">{t('dev.projects')}</h2>
      <Button size="xs" variant="secondary" data-testid="dev-project-create-open" aria-expanded={creating} aria-controls="dev-project-create" onClick={() => { setCreateOpened(true); setCreating(value => !value); }}>{t('dev.add')}</Button></header>
    <div id="dev-project-create" hidden={!creating}>{createOpened ? <CreateForm onDone={() => { setCreating(false); setCreateOpened(false); }} /> : null}</div>
    {projects.length ? <div className="dev-nav-search"><Input type="search" data-testid="dev-project-search" label={t('workbenchNavigation.searchProjects')} placeholder={t('workbenchNavigation.projectSearchHint')} value={search} onChange={event => setSearch(event.target.value)} />{search ? <Button size="xs" variant="ghost" onClick={() => setSearch('')}>{t('patterns.clearSearch')}</Button> : null}</div> : null}
    {query && !matchingIds.size ? <p role="status" className="m-0 text-xs text-ink-secondary">{t('workbenchNavigation.noMatches')}</p> : null}
    {!projects.length ? <p className="text-sm text-ink-secondary">{t('dev.none')}</p> : null}
    {active.length ? <ProjectGroup label={t('dev.recent')} projects={active} selectedId={selectedId} activity={activity} /> : null}
    {archived.length ? <details className="dev-disclosure" open={Boolean(query) || archived.some(project => project.id === selectedId)}><summary className="dev-disclosure-summary">{t('dev.archived')} <span className="dev-nav-status">{archived.length}</span></summary><div className="dev-disclosure-body"><ProjectGroup label={t('dev.archived')} projects={archived} selectedId={selectedId} activity={activity} /></div></details> : null}
    {unfinished.length ? <div className="dev-operations dev-nav-operations"><strong className="text-xs">{t('dev.operations')}</strong><p className="text-xs text-ink-secondary">{t('dev.operationHint')}</p>
      <ul className="m-0 grid list-none gap-1 p-0">{unfinished.map(operation => <li key={operation.id} className="break-all text-xs" data-testid={`dev-operation-${operation.id}`}>{t(`dev.operationStatus.${operation.status}`)} · {operation.path}</li>)}</ul></div> : null}
    {completed.length ? <details className="dev-disclosure"><summary className="dev-disclosure-summary">{t('dev.operationHistory')} <span className="dev-nav-status">{completed.length}</span></summary><ul className="dev-disclosure-body m-0 grid list-none gap-1 p-0">{completed.map(operation => <li key={operation.id} className="break-all text-xs" data-testid={`dev-operation-${operation.id}`}>{t('dev.operationStatus.succeeded')} · {operation.path}</li>)}</ul></details> : null}
    {selected ? <ProjectDetail key={`${selected.id}:${selected.name}:${selected.path}`} project={selected} /> : null}
  </section>;
}

function ProjectGroup({ label, projects, selectedId, activity }: { label: string; projects: Project[]; selectedId: string | null; activity: Map<string, { live: number; running: number; processes: number }> }) {
  const { t } = useLocale();
  const [pending, setPending] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  async function choose(project: Project) {
    setPending(project.id);
    const result = await chooseContext({ project_id: project.id, task_id: null, session_id: null });
    setFailure(result.ok ? null : result.code);
    setPending(null);
  }
  return <nav aria-label={label} className="dev-nav-list grid gap-1">
    <span className="text-xs text-ink-secondary">{label}</span>
    {projects.map(project => <Button key={project.id} variant={selectedId === project.id ? 'secondary' : 'ghost'} size="xs" className="dev-nav-item justify-start" data-testid={`dev-project-item-${project.id}`} aria-pressed={selectedId === project.id} disabled={pending !== null} onClick={() => void choose(project)}>
      <span className="dev-nav-item-label truncate" title={project.path}>{project.name}{project.archived ? ` · ${t('dev.projectStatus.archived')}` : ''}</span>
      {selectedId === project.id ? <span className="dev-nav-status">{t('workbenchNavigation.current')}</span> : null}
      {activity.get(project.id)?.running ? <span className="dev-nav-status">{t('workbenchNavigation.running', { count: activity.get(project.id)!.running })}</span> : null}
      {activity.get(project.id)?.live ? <span className="dev-nav-status">{t('workbenchNavigation.liveCli', { count: activity.get(project.id)!.live })}</span> : null}
      {activity.get(project.id)?.processes ? <span className="dev-nav-status">{t('workbenchNavigation.activeProcesses', { count: activity.get(project.id)!.processes })}</span> : null}
    </Button>)}
    {failure ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(failure)}</p> : null}
  </nav>;
}

// Keeps every typed value when a submission fails. The form only clears after the host commits the project.
function CreateForm({ onDone }: { onDone: () => void }) {
  const { t } = useLocale();
  const [kind, setKind] = useState<CreateProjectInput['kind']>('import');
  const [name, setName] = useState('');
  const [path, setPath] = useState('');
  const [source, setSource] = useState('');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  async function submit() {
    setBusy(true);
    const result = await useDevelopment.getState().createProject({ kind, name, path, source: kind === 'clone' ? source : null });
    setBusy(false);
    if (!result.ok) { setFailure(result.code); return; }
    setFailure(null);
    await chooseContext({ project_id: result.value.id, task_id: null, session_id: null });
    onDone();
  }
  return <form className="dev-form" data-testid="dev-project-form" onSubmit={event => { event.preventDefault(); void submit(); }}>
    <Select label={t('dev.kind')} value={kind} onChange={value => setKind(value === 'create' || value === 'clone' ? value : 'import')} options={[{ value: 'import', label: t('dev.kind.import') }, { value: 'create', label: t('dev.kind.create') }, { value: 'clone', label: t('dev.kind.clone') }]} />
    <Input data-testid="dev-project-name" label={t('dev.name')} value={name} onChange={event => setName(event.target.value)} />
    <Input data-testid="dev-project-path" label={t('dev.path')} value={path} onChange={event => setPath(event.target.value)} />
    {kind === 'clone' ? <Input data-testid="dev-project-source" label={t('dev.source')} value={source} onChange={event => setSource(event.target.value)} /> : null}
    {failure ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(failure)}</p> : null}
    <div className="flex gap-2"><Button type="submit" size="xs" data-testid="dev-project-submit" disabled={busy}>{busy ? t('dev.busy') : t('dev.submit')}</Button><Button size="xs" variant="secondary" disabled={busy} onClick={onDone}>{t('dev.cancel')}</Button></div>
  </form>;
}

function ProjectDetail({ project }: { project: Project }) {
  const { t } = useLocale();
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [name, setName] = useState(project.name);
  const [path, setPath] = useState('');
  const [inspection, setInspection] = useState<ProjectInspection | null>(null);
  const [inspectFailure, setInspectFailure] = useState<string | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  useEffect(() => {
    let current = true;
    setInspection(null); setInspectFailure(null);
    void useDevelopment.getState().inspect(project.id).then(result => {
      if (!current) return;
      if (result.ok) setInspection(result.value); else setInspectFailure(result.code);
    });
    return () => { current = false; };
  }, [project.id]);
  async function run(action: 'rename' | 'archive' | 'restore' | 'unlink' | 'relocate', value: string | null) {
    if (['archive', 'unlink', 'relocate'].includes(action) && !await discardDevelopmentDraft()) return false;
    const epoch = useDevelopment.getState().epoch;
    if (action === 'unlink' && !await guardWorkbenchDrafts(project.id)) return false;
    if (epoch !== useDevelopment.getState().epoch) return false;
    setBusy(true);
    const result = await useDevelopment.getState().updateProject(project.id, action, value);
    if (result.ok && action === 'unlink' && epoch === useDevelopment.getState().epoch) useDevelopmentDrafts.setState(state => ({
      entries: Object.fromEntries(Object.entries(state.entries).filter(([key]) => JSON.parse(key)[2] !== project.id)),
      handoffs: Object.fromEntries(Object.entries(state.handoffs).filter(([key]) => JSON.parse(key)[2] !== project.id)),
    }));
    if (result.ok && action === 'unlink' && epoch === useDevelopment.getState().epoch) useDevelopmentRuntime.setState(state => ({ drafts: Object.fromEntries(Object.entries(state.drafts).filter(([key]) => JSON.parse(key)[1] !== project.id)) }));
    setBusy(false);
    setFailure(result.ok ? null : result.code);
    return result.ok;
  }
  async function unlink() {
    if (!await confirmAction(t('dev.unlink'), t('dev.unlinkConfirm'), t('dev.unlink'))) return;
    await run('unlink', null);
  }
  const relocateNeeded = inspection !== null && inspection.path_state !== 'available';
  return <article className="dev-nav-detail" data-testid="dev-project-detail">
    {relocateNeeded ? <div className="dev-nav-warning"><p role="alert" className="m-0 text-xs text-status-error">{t('dev.relocateNeeded')}</p><Button size="xs" variant="secondary" onClick={() => setDetailsOpen(true)}>{t('dev.repair')}</Button></div> : null}
    {inspectFailure ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(inspectFailure)}</p> : null}
    <details className="dev-disclosure" open={detailsOpen} onToggle={event => setDetailsOpen(event.currentTarget.open)}>
      <summary className="dev-disclosure-summary">{t('dev.projectDetails')}</summary>
      <div className="dev-detail dev-disclosure-body">
    <h3 className="m-0 text-sm font-semibold">{project.name}</h3>
    <p className="m-0 break-all text-xs text-ink-secondary">{project.path}</p>
    <section aria-label={t('dev.repository')} className="grid gap-1 text-xs">
      <strong>{t('dev.repository')}</strong>
      {inspectFailure ? <p role="alert" className="m-0 text-status-error">{errorText(inspectFailure)}</p> : null}
      {inspection ? <RepositoryInfo inspection={inspection} /> : <p className="m-0 text-ink-secondary">{t('dev.inspecting')}</p>}
      <Button size="xs" variant="secondary" className="justify-self-start" onClick={() => void useDevelopment.getState().inspect(project.id).then(result => { if (result.ok) { setInspection(result.value); setInspectFailure(null); } else setInspectFailure(result.code); })}>{t('dev.inspect')}</Button>
    </section>
    <div className="grid gap-2">
      <Input data-testid="dev-project-rename-input" label={t('dev.renameField')} value={name} onChange={event => setName(event.target.value)} />
      <Button size="xs" variant="secondary" data-testid="dev-project-rename" disabled={busy || !name.trim()} onClick={() => void run('rename', name)}>{t('dev.rename')}</Button>
    </div>
    <div className="grid gap-2">
      <Input data-testid="dev-project-relocate-input" label={t('dev.relocateField')} value={path} onChange={event => setPath(event.target.value)} />
      <Button size="xs" variant="secondary" data-testid="dev-project-relocate" disabled={busy || !path.trim()} onClick={() => void run('relocate', path)}>{t('dev.relocate')}</Button>
    </div>
    <div className="flex flex-wrap gap-2">
      {project.archived
        ? <Button size="xs" variant="secondary" data-testid="dev-project-restore" disabled={busy} onClick={() => void run('restore', null)}>{t('dev.restore')}</Button>
        : <Button size="xs" variant="secondary" data-testid="dev-project-archive" disabled={busy} onClick={() => void run('archive', null)}>{t('dev.archive')}</Button>}
      <Button size="xs" variant="danger" data-testid="dev-project-unlink" disabled={busy} onClick={() => void unlink()}>{t('dev.unlink')}</Button>
    </div>
    {failure ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(failure)}</p> : null}
      </div>
    </details>
  </article>;
}

function RepositoryInfo({ inspection }: { inspection: ProjectInspection }) {
  const { t } = useLocale();
  const kind = t(`dev.kindLabel.${inspection.kind}`);
  const state = t(`dev.pathState.${inspection.path_state}`);
  if (inspection.error) return <p role="alert" className="m-0 text-status-error">{t('dev.inspectError')}</p>;
  return <dl className="dev-repository m-0 grid gap-x-2 gap-y-1">
    <dt>{t('dev.status')}</dt><dd className="m-0" data-testid="dev-path-state">{state} · {kind}</dd>
    {inspection.repository_root ? <><dt>{t('dev.repoRoot')}</dt><dd className="m-0 break-all">{inspection.repository_root}</dd></> : null}
    {inspection.branch ? <><dt>{t('dev.branch')}</dt><dd className="m-0">{inspection.branch}</dd></> : null}
    {inspection.kind === 'git' ? <><dt>{t('dev.changes')}</dt><dd className="m-0" data-testid="dev-git-changes">{inspection.changes.length ? t('dev.changesCount', { count: inspection.changes.length }) : t('dev.clean')}</dd></> : null}
    {inspection.truncated ? <dd className="col-span-2 m-0 text-ink-secondary">{t('dev.truncated')}</dd> : null}
    {inspection.changes.length ? <dd className="col-span-2 m-0"><ul className="m-0 list-none p-0 font-mono">{inspection.changes.map(change => <li key={change} className="break-all">{change}</li>)}</ul></dd> : null}
  </dl>;
}
