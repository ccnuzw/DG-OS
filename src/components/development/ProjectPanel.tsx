import { useEffect, useState } from 'react';
import { Button, Input, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { chooseContext } from '../../stores/developmentSelection';
import { useDevelopment, type CreateProjectInput, type Project, type ProjectInspection } from '../../stores/developmentStore';
import { confirmAction, errorText } from './shared';
import { discardDevelopmentDraft } from '../../stores/developmentEditor';

export function ProjectPanel() {
  const { t } = useLocale();
  const projects = useDevelopment(state => state.projects);
  const operations = useDevelopment(state => state.operations);
  const selectedId = useDevelopment(state => state.selection.project_id);
  const [creating, setCreating] = useState(false);
  const active = projects.filter(project => !project.archived).sort((a, b) => (b.last_opened_at ?? 0) - (a.last_opened_at ?? 0));
  const archived = projects.filter(project => project.archived);
  const selected = projects.find(project => project.id === selectedId) ?? null;
  return <section className="dev-panel" aria-label={t('dev.projects')} data-testid="dev-project-list">
    <header className="dev-panel-header"><h2 className="m-0 text-sm font-semibold">{t('dev.projects')}</h2>
      <Button size="xs" variant="secondary" data-testid="dev-project-create-open" onClick={() => setCreating(value => !value)}>{t('dev.add')}</Button></header>
    {creating ? <CreateForm onDone={() => setCreating(false)} /> : null}
    {!projects.length ? <p className="text-sm text-muted-foreground">{t('dev.none')}</p> : null}
    {active.length ? <ProjectGroup label={t('dev.recent')} projects={active} selectedId={selectedId} /> : null}
    {archived.length ? <ProjectGroup label={t('dev.archived')} projects={archived} selectedId={selectedId} /> : null}
    {operations.length ? <div className="dev-operations"><strong className="text-xs">{t('dev.operations')}</strong><p className="text-xs text-muted-foreground">{t('dev.operationHint')}</p>
      <ul className="m-0 grid list-none gap-1 p-0">{operations.map(operation => <li key={operation.id} className="break-all text-xs" data-testid={`dev-operation-${operation.id}`}>{t(`dev.operationStatus.${operation.status}`)} · {operation.path}</li>)}</ul></div> : null}
    {selected ? <ProjectDetail key={`${selected.id}:${selected.name}:${selected.path}`} project={selected} /> : null}
  </section>;
}

function ProjectGroup({ label, projects, selectedId }: { label: string; projects: Project[]; selectedId: string | null }) {
  const { t } = useLocale();
  const [pending, setPending] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  async function choose(project: Project) {
    setPending(project.id);
    const result = await chooseContext({ project_id: project.id, task_id: null, session_id: null });
    setFailure(result.ok ? null : result.code);
    setPending(null);
  }
  return <nav aria-label={label} className="grid gap-1">
    <span className="text-xs text-muted-foreground">{label}</span>
    {projects.map(project => <Button key={project.id} variant={selectedId === project.id ? 'primary' : 'ghost'} size="xs" className="justify-start" data-testid={`dev-project-item-${project.id}`} aria-pressed={selectedId === project.id} disabled={pending !== null} onClick={() => void choose(project)}>
      <span className="truncate">{project.name}{project.archived ? ` · ${t('dev.projectStatus.archived')}` : ''}</span></Button>)}
    {failure ? <p role="alert" className="m-0 text-xs text-destructive">{errorText(failure)}</p> : null}
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
    {failure ? <p role="alert" className="m-0 text-xs text-destructive">{errorText(failure)}</p> : null}
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
    setBusy(true);
    const result = await useDevelopment.getState().updateProject(project.id, action, value);
    setBusy(false);
    setFailure(result.ok ? null : result.code);
    return result.ok;
  }
  async function unlink() {
    if (!await confirmAction(t('dev.unlink'), t('dev.unlinkConfirm'), t('dev.unlink'))) return;
    await run('unlink', null);
  }
  const relocateNeeded = inspection !== null && inspection.path_state !== 'available';
  return <article className="dev-detail" data-testid="dev-project-detail">
    <h3 className="m-0 text-sm font-semibold">{project.name}</h3>
    <p className="m-0 break-all text-xs text-muted-foreground">{project.path}</p>
    <section aria-label={t('dev.repository')} className="grid gap-1 text-xs">
      <strong>{t('dev.repository')}</strong>
      {inspectFailure ? <p role="alert" className="m-0 text-destructive">{errorText(inspectFailure)}</p> : null}
      {inspection ? <RepositoryInfo inspection={inspection} /> : <p className="m-0 text-muted-foreground">{t('dev.inspecting')}</p>}
      <Button size="xs" variant="secondary" className="justify-self-start" onClick={() => void useDevelopment.getState().inspect(project.id).then(result => { if (result.ok) { setInspection(result.value); setInspectFailure(null); } else setInspectFailure(result.code); })}>{t('dev.inspect')}</Button>
    </section>
    {relocateNeeded ? <p role="alert" className="m-0 text-xs text-destructive">{t('dev.relocateNeeded')}</p> : null}
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
    {failure ? <p role="alert" className="m-0 text-xs text-destructive">{errorText(failure)}</p> : null}
  </article>;
}

function RepositoryInfo({ inspection }: { inspection: ProjectInspection }) {
  const { t } = useLocale();
  const kind = t(`dev.kindLabel.${inspection.kind}`);
  const state = t(`dev.pathState.${inspection.path_state}`);
  if (inspection.error) return <p role="alert" className="m-0 text-destructive">{t('dev.inspectError')}</p>;
  return <dl className="dev-repository m-0 grid gap-x-2 gap-y-1">
    <dt>{t('dev.status')}</dt><dd className="m-0" data-testid="dev-path-state">{state} · {kind}</dd>
    {inspection.repository_root ? <><dt>{t('dev.repoRoot')}</dt><dd className="m-0 break-all">{inspection.repository_root}</dd></> : null}
    {inspection.branch ? <><dt>{t('dev.branch')}</dt><dd className="m-0">{inspection.branch}</dd></> : null}
    {inspection.kind === 'git' ? <><dt>{t('dev.changes')}</dt><dd className="m-0" data-testid="dev-git-changes">{inspection.changes.length ? t('dev.changesCount', { count: inspection.changes.length }) : t('dev.clean')}</dd></> : null}
    {inspection.truncated ? <dd className="col-span-2 m-0 text-muted-foreground">{t('dev.truncated')}</dd> : null}
    {inspection.changes.length ? <dd className="col-span-2 m-0"><ul className="m-0 list-none p-0 font-mono">{inspection.changes.map(change => <li key={change} className="break-all">{change}</li>)}</ul></dd> : null}
  </dl>;
}
