import { useEffect, useMemo, useRef, useState } from 'react';
import { open as openDirectoryDialog } from '@tauri-apps/plugin-dialog';
import { basename, join } from '@tauri-apps/api/path';
import { Button, DropdownMenu, Icon, Input, Modal } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { chooseContext, contextGeneration } from '../../stores/developmentSelection';
import { useDevelopment, type Project, type ProjectInspection } from '../../stores/developmentStore';
import { confirmAction, errorText } from './shared';
import { discardDevelopmentDraft } from '../../stores/developmentEditor';
import { guardWorkbenchDrafts, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { processActive, useDevelopmentRuntime } from '../../stores/developmentRuntime';
import { useAgentRunner } from '../../stores/agentRunnerStore';

export function ProjectPanel() {
  const { t } = useLocale();
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  function operationOwner() {
    const epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    return () => mounted.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
  }
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
  const [cloneOpen, setCloneOpen] = useState(false);
  const [folderOpen, setFolderOpen] = useState(false);
  const [projectName, setProjectName] = useState('');
  const [parentPath, setParentPath] = useState('');
  const [repository, setRepository] = useState('');
  const [projectBusy, setProjectBusy] = useState(false);
  const projectLock = useRef(false);
  const [projectFailure, setProjectFailure] = useState<string | null>(null);
  const [pickerFailure, setPickerFailure] = useState(false);
  const visible = projects.filter(project => project.id === selectedId || matchingIds.has(project.id));
  const active = visible.filter(project => !project.archived).sort((a, b) => (b.last_opened_at ?? 0) - (a.last_opened_at ?? 0));
  const archived = visible.filter(project => project.archived);
  const selected = projects.find(project => project.id === selectedId) ?? null;
  const unfinished = operations.filter(operation => operation.status !== 'succeeded');
  const completed = operations.filter(operation => operation.status === 'succeeded');
  async function pickParent() {
    const valid = operationOwner();
    setPickerFailure(false);
    try {
      const selected = await openDirectoryDialog({ directory: true, multiple: false, title: t('dev.chooseParent') });
      if (valid() && typeof selected === 'string') setParentPath(selected);
    } catch { if (valid()) setPickerFailure(true); }
  }
  async function submitProject(kind: 'create' | 'clone') {
    const name = projectName.trim() || (kind === 'clone' ? repositoryName(repository) : '');
    if (projectLock.current || !validFolderName(name) || !parentPath || (kind === 'clone' && !repository.trim())) return;
    const valid = operationOwner();
    projectLock.current = true;
    setProjectBusy(true); setProjectFailure(null);
    try {
      const path = await join(parentPath, name);
      if (!valid()) return;
      const result = await useDevelopment.getState().createProject({ kind, name, path, source: kind === 'clone' ? repository.trim() : null });
      if (!valid()) return;
      if (!result.ok) { setProjectFailure(result.code); return; }
      setFolderOpen(false); setCloneOpen(false);
      const selected = await chooseContext({ project_id: result.value.id, task_id: null, session_id: null });
      if (!selected.ok) setProjectFailure(selected.code);
    } catch { if (valid()) setProjectFailure('unknown'); }
    finally { projectLock.current = false; setProjectBusy(false); }
  }
  async function importFolder() {
    if (projectLock.current) return;
    const valid = operationOwner();
    projectLock.current = true;
    setProjectBusy(true); setPickerFailure(false); setProjectFailure(null);
    let selected: string | string[] | null;
    try {
      selected = await openDirectoryDialog({ directory: true, multiple: false, title: t('dev.chooseProjectFolder') });
    } catch {
      if (valid()) setPickerFailure(true); projectLock.current = false; setProjectBusy(false); return;
    }
    try {
      if (!valid() || typeof selected !== 'string') return;
      const name = await basename(selected);
      if (!valid()) return;
      const result = await useDevelopment.getState().createProject({ kind: 'import', name, path: selected, source: null });
      if (!valid()) return;
      if (!result.ok) { setProjectFailure(result.code); return; }
      const chosen = await chooseContext({ project_id: result.value.id, task_id: null, session_id: null });
      if (!chosen.ok) setProjectFailure(chosen.code);
    } catch { if (valid()) setProjectFailure('unknown'); }
    finally { projectLock.current = false; setProjectBusy(false); }
  }
  return <section className="dev-panel dev-nav-panel" aria-label={t('dev.projects')} data-testid="dev-project-list">
    <header className="dev-panel-header"><h2 className="m-0 text-sm font-semibold">{t('dev.projects')}</h2>
      <DropdownMenu label={t('dev.add')} trigger={<Button size="xs" variant="secondary" disabled={projectBusy} aria-label={t('dev.add')}><Icon name="plus" />{t('dev.add')}</Button>} items={[
        { kind: 'item', id: 'open-folder', label: t('dev.openFolder'), icon: 'folder-open', onSelect: () => void importFolder() },
        { kind: 'item', id: 'create-folder', label: t('dev.createFolder'), icon: 'plus', onSelect: () => { setProjectName(''); setParentPath(''); setProjectFailure(null); setPickerFailure(false); setFolderOpen(true); } },
        { kind: 'item', id: 'clone-repository', label: t('dev.cloneRepository'), icon: 'code', onSelect: () => { setProjectName(''); setParentPath(''); setRepository(''); setProjectFailure(null); setPickerFailure(false); setCloneOpen(true); } },
      ]} />
    </header>
    <Input aria-label={t('workbenchNavigation.searchProjects')} placeholder={t('workbenchNavigation.searchProjects')} value={search} onChange={event => setSearch(event.target.value)} />
    {!folderOpen && !cloneOpen && pickerFailure ? <p role="alert" className="dev-feedback dev-feedback-error">{t('dev.folderPickerFailed')}</p> : null}
    {!folderOpen && !cloneOpen && projectFailure ? <p role="alert" className="dev-feedback dev-feedback-error">{errorText(projectFailure)}</p> : null}
    {query && !matchingIds.size ? <p role="status" className="m-0 text-xs text-ink-secondary">{t('workbenchNavigation.noMatches')}</p> : null}
    {!projects.length ? <p className="text-sm text-ink-secondary">{t('dev.none')}</p> : null}
    {active.length ? <ProjectGroup label={t('dev.recent')} projects={active} selectedId={selectedId} activity={activity} /> : null}
    {archived.length ? <details className="dev-disclosure" open={Boolean(query) || archived.some(project => project.id === selectedId)}><summary className="dev-disclosure-summary">{t('dev.archived')} <span className="dev-nav-status">{archived.length}</span></summary><div className="dev-disclosure-body"><ProjectGroup label={t('dev.archived')} projects={archived} selectedId={selectedId} activity={activity} /></div></details> : null}
    {unfinished.length ? <div className="dev-operations dev-nav-operations"><strong className="text-xs">{t('dev.operations')}</strong><p className="text-xs text-ink-secondary">{t('dev.operationHint')}</p><ul className="m-0 grid list-none gap-1 p-0">{unfinished.map(operation => <li key={operation.id} className="break-all text-xs" data-testid={`dev-operation-${operation.id}`}>{t(`dev.operationStatus.${operation.status}`)} · {operation.path}</li>)}</ul></div> : null}
    {completed.length ? <details className="dev-disclosure"><summary className="dev-disclosure-summary">{t('dev.operationHistory')} <span className="dev-nav-status">{completed.length}</span></summary><ul className="dev-disclosure-body m-0 grid list-none gap-1 p-0">{completed.map(operation => <li key={operation.id} className="break-all text-xs" data-testid={`dev-operation-${operation.id}`}>{t('dev.operationStatus.succeeded')} · {operation.path}</li>)}</ul></details> : null}
    {selected ? <ProjectDetail key={`${selected.id}:${selected.name}:${selected.path}`} project={selected} /> : null}
    <ProjectDialog open={folderOpen || cloneOpen} onOpenChange={open => { if (!projectLock.current && !open) { setFolderOpen(false); setCloneOpen(false); } }} kind={cloneOpen ? 'clone' : 'create'} name={projectName} onName={setProjectName} parent={parentPath} repository={repository} onRepository={setRepository} busy={projectBusy} failure={projectFailure} pickerFailure={pickerFailure} onPick={pickParent} onSubmit={() => submitProject(cloneOpen ? 'clone' : 'create')} />
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
function repositoryName(source: string): string {
  return source.trim().split(/[?#]/, 1)[0].replace(/\/+$/, '').split(/[/:]/).pop()?.replace(/\.git$/, '') ?? '';
}

function validFolderName(name: string): boolean {
  return Boolean(name) && name !== '.' && name !== '..' && !/[\\/:\x00]/.test(name);
}

type ProjectDialogProps = {
  open: boolean; onOpenChange(value: boolean): void; kind: 'create' | 'clone';
  name: string; onName(value: string): void; parent: string;
  repository: string; onRepository(value: string): void;
  busy: boolean; failure: string | null; pickerFailure: boolean;
  onPick(): Promise<void>; onSubmit(): Promise<void>;
};

function ProjectDialog(props: ProjectDialogProps) {
  const { t } = useLocale();
  const clone = props.kind === 'clone';
  const name = props.name.trim() || (clone ? repositoryName(props.repository) : '');
  const valid = validFolderName(name);
  const path = props.parent && valid ? `${props.parent.replace(/[\\/]+$/, '')}/${name}` : '';
  const ready = Boolean(path) && (!clone || Boolean(props.repository.trim()));
  return <Modal open={props.open} onOpenChange={props.onOpenChange} title={t(clone ? 'dev.cloneRepositoryTitle' : 'dev.createFolderTitle')} description={t(clone ? 'dev.cloneHint' : 'dev.createFolderHint')} kind="form" footer={<>
    <Button variant="secondary" disabled={props.busy} onClick={() => props.onOpenChange(false)}>{t('dev.cancel')}</Button>
    <Button disabled={!ready || props.busy} loading={props.busy} onClick={() => void props.onSubmit()}>{t(clone ? 'dev.cloneRepositoryTitle' : 'dev.create')}</Button>
  </>}>
    <form className="dev-project-dialog-form" aria-busy={props.busy} onSubmit={event => { event.preventDefault(); if (ready && !props.busy) void props.onSubmit(); }}>
      <fieldset disabled={props.busy} className="m-0 grid min-w-0 gap-4 border-0 p-0">
        {clone ? <Input autoFocus label={t('dev.source')} value={props.repository} onChange={event => props.onRepository(event.target.value)} placeholder={t('dev.repositoryUrlHint')} /> : null}
        <Input autoFocus={!clone} label={t('dev.name')} value={props.name} onChange={event => props.onName(event.target.value)} placeholder={clone ? t('dev.projectNameFromRepository') : undefined} />
        <div className="dev-project-path-field"><span>{t('dev.parentFolder')}</span><div className="dev-project-path-row"><Input aria-label={t('dev.parentFolder')} value={props.parent} readOnly placeholder={t('dev.chooseParentHint')} /><Button type="button" variant="secondary" size="md" iconOnly aria-label={t('dev.chooseParent')} onClick={() => void props.onPick()}><Icon name="folder-open" /></Button></div></div>
        <div className="dev-project-destination"><span>{t('dev.destination')}</span><code>{path || t('dev.destinationPreview')}</code></div>
        {name && !valid ? <p role="alert" className="dev-feedback dev-feedback-error">{t('dev.folderNameInvalid')}</p> : null}
        {props.pickerFailure ? <p role="alert" className="dev-feedback dev-feedback-error">{t('dev.folderPickerFailed')}</p> : null}
        {props.failure ? <p role="alert" className="dev-feedback dev-feedback-error">{errorText(props.failure)}</p> : null}
        <button type="submit" hidden disabled={!ready || props.busy} />
      </fieldset>
    </form>
  </Modal>;
}
