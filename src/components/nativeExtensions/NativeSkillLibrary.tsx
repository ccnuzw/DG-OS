import { useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Archive, FolderOpen, RefreshCw, Search } from 'lucide-react';
import { Button, Input, Select } from '@dgos/ui';
import { t } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { nativeLibraryKey, useNativeSkillLibrary, type NativeSourceDraft, type NativeVersionRecord } from '../../stores/nativeSkillLibrary';
import { useNativeExtensions } from '../../stores/nativeExtensions';
import type { NativePackage, NativeResult, NativeSelection, NativeSkill, PreviewNativeSkill } from '../../types/nativeExtensions';
import type { NativeDependencyReport, NativeLibraryResult, NativeLibrarySkill, NativePickedSkillSource, NativeSkillFile, NativeSkillInspection, NativeSkillSource, NativeVersionPlan } from '../../types/nativeSkillLibrary';
import { useNativeManagerGuard } from './NativeManagerLifecycle';
import { confirmDiscard } from '../../workspace/closeCoordinator';
import './native-skill-library.css';

const emptyDraft: NativeSourceDraft = { input: '', sourceId: null, revision: null };
const date = (value: number | null) => value ? new Date(value * 1000).toLocaleString() : t('nativeSkillLibrary.notIndexed');
const issue = (code: string) => t(`nativeExtensions.error.${code}`);
function LibraryMetadata({ skill }: { skill: NativeLibrarySkill }) {
  return <dl className="native-skill-library__facts">
    <dt>{t('nativeSkillLibrary.packagePath')}</dt><dd>{skill.path || '/'}</dd>
    <dt>{t('nativeSkillLibrary.contentHash')}</dt><dd>{skill.content_hash}</dd>
    <dt>{t('nativeSkillLibrary.files')}</dt><dd>{skill.file_count}</dd>
    <dt>{t('nativeSkillLibrary.scripts')}</dt><dd>{skill.script_count}</dd>
    <dt>{t('nativeSkillLibrary.executableFiles')}</dt><dd>{skill.executable_files}</dd>
    <dt>{t('nativeSkillLibrary.dynamicContent')}</dt><dd>{t(skill.dynamic_commands ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</dd>
  </dl>;
}

export function NativeSkillLibrary({ selection, tab, disabled, onPreview }: { selection: NativeSelection; tab: 'discover' | 'sources'; disabled: boolean; onPreview: (input: PreviewNativeSkill) => void }) {
  const api = useNativeSkillLibrary();
  const context = useDevelopment(() => nativeLibraryKey(selection));
  const sourceState = api.sources[context];
  const draft = api.drafts[context] ?? emptyDraft;
  const [mode, setMode] = useState<'catalog' | 'github' | 'local'>('catalog');
  const [query, setQuery] = useState('');
  const [sourceId, setSourceId] = useState('all');
  const [results, setResults] = useState<NativeLibraryResult[] | null>(null);
  const [picked, setPicked] = useState<NativePickedSkillSource | null>(null);
  const [selected, setSelected] = useState<{ skill: NativeLibrarySkill; source: NativePackage; repository: string | null; commit: string | null; stale: boolean } | null>(null);
  const [slug, setSlug] = useState('');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const flight = useRef(false), generation = useRef(0);
  const grant = useRef<{ selection: NativeSelection; context: string; handle: string } | null>(null);
  const current = useRef(context); current.current = context;
  useNativeManagerGuard(() => !flight.current);
  useEffect(() => {
    generation.current++; flight.current = false;
    setBusy(false); setMessage(''); setResults(null); setPicked(null); setSelected(null); setSlug(''); setQuery(''); setSourceId('all');
    void api.loadSources(selection);
    return () => {
      generation.current++;
      if (grant.current?.context === context) { void api.release(grant.current.selection, grant.current.handle); grant.current = null; }
    };
  }, [context]);

  async function perform<T>(work: () => Promise<NativeResult<T>>, accept: (value: T) => void, dispose?: (value: T) => void): Promise<void> {
    if (disabled || flight.current) return;
    const token = generation.current, key = context;
    flight.current = true; setBusy(true); setMessage('');
    try {
      const result = await work();
      if (generation.current !== token || current.current !== key) { if (result.ok) dispose?.(result.value); return; }
      if (result.ok) accept(result.value); else setMessage(issue(result.code));
    } finally {
      if (generation.current === token && current.current === key) { flight.current = false; setBusy(false); }
    }
  }
  function choose(skill: NativeLibrarySkill, source: NativePackage, repository: string | null, commit: string | null, stale = false) {
    setSelected({ skill, source, repository, commit, stale }); setSlug(skill.name); setMessage('');
  }
  const search = () => perform(() => api.search(selection, query, sourceId === 'all' ? null : sourceId), value => { setResults(value.results); setSelected(null); });
  const save = (index: boolean) => perform(async () => {
    const saved = await api.saveSource(selection, draft);
    if (!saved.ok || !index) return saved;
    return api.refreshSource(selection, saved.value);
  }, source => {
    setMessage(source.error ? issue(source.error) : t(index ? 'nativeSkillLibrary.indexed' : 'nativeSkillLibrary.sourceSaved'));
    if (index) { setSourceId(source.id); setMode('catalog'); setResults(source.skills.map(skill => ({ source_id: source.id, repository_url: source.repository.normalized_source, indexed_at: source.indexed_at ?? 0, commit: source.commit ?? '', stale: !!source.error, skill }))); }
  });
  const edit = (source: NativeSkillSource) => perform(async () => {
    const snapshot = useNativeSkillLibrary.getState().drafts[context];
    const accepted = !snapshot?.input || (snapshot.sourceId === source.id && snapshot.input === source.repository.normalized_source) || await confirmDiscard();
    return { ok: true, value: accepted && snapshot === useNativeSkillLibrary.getState().drafts[context] };
  }, accepted => { if (accepted) api.setDraft(selection, { input: source.repository.normalized_source, sourceId: source.id, revision: source.revision }); });
  const refresh = (source: NativeSkillSource) => perform(() => api.refreshSource(selection, source), value => { setResults(null); setSelected(null); setMessage(value.error ? issue(value.error) : t('nativeSkillLibrary.indexed')); });
  const remove = (source: NativeSkillSource) => perform(() => api.removeSource(selection, source), () => { setResults(null); setSelected(null); if (draft.sourceId === source.id && useNativeSkillLibrary.getState().drafts[context] === draft) api.setDraft(selection, null); setMessage(t('nativeSkillLibrary.sourceRemoved')); });
  const pick = (kind: 'zip' | 'directory') => perform(async () => {
    if (grant.current) { await api.release(grant.current.selection, grant.current.handle); grant.current = null; setPicked(null); setSelected(null); }
    return api.pick(selection, kind);
  }, value => { if (value) { grant.current = { selection, context, handle: value.handle }; setPicked(value); setSelected(null); } }, value => { if (value) void api.release(selection, value.handle); });
  const sourceForm = <form className="native-skill-library__source-form" onSubmit={event => { event.preventDefault(); void save(tab === 'discover'); }}>
    <Input label={t('nativeSkillLibrary.repositoryUrl')} value={draft.input} onChange={event => api.setDraft(selection, { ...draft, input: event.target.value })} placeholder="https://github.com/owner/repository" disabled={disabled || busy} />
    <p className="native-skill-library__hint">{t('nativeSkillLibrary.supportedGithub')}</p>
    <div className="native-skill-library__actions">
      <Button type="submit" disabled={disabled || busy || !draft.input.trim()}>{t(tab === 'discover' ? 'nativeSkillLibrary.addAndIndex' : draft.sourceId ? 'nativeSkillLibrary.saveSource' : 'nativeSkillLibrary.addSource')}</Button>
      {draft.input && <Button variant="secondary" disabled={disabled || busy} onClick={() => api.setDraft(selection, null)}>{t('nativeSkillLibrary.discardSourceDraft')}</Button>}
    </div>
  </form>;
  return <section className="native-skill-library" aria-label={t(tab === 'sources' ? 'nativeSkillLibrary.sources' : 'nativeSkillLibrary.discover')} aria-busy={busy}>
    {tab === 'sources' ? <>
      <p className="native-skill-library__hint">{t('nativeSkillLibrary.sourceBoundary')}</p>
      {sourceForm}
      {sourceState?.loading && <p role="status">{t('nativeSkillLibrary.loadingSources')}</p>}
      {sourceState?.error && <p role="alert">{issue(sourceState.error)}</p>}
      {!sourceState?.loading && sourceState?.loaded && !sourceState.error && !sourceState.value.length && <p>{t('nativeSkillLibrary.noSources')}</p>}
      <ul className="native-skill-library__sources">
        {(sourceState?.value ?? []).map(source => <li key={source.id}>
          <div><strong>{source.repository.owner}/{source.repository.repo}</strong><p>{source.repository.normalized_source}</p></div>
          <dl className="native-skill-library__facts">
            <dt>{t('nativeSkillLibrary.ref')}</dt><dd>{source.repository.requested_ref ?? t('nativeSkillLibrary.defaultRef')}</dd>
            <dt>{t('nativeSkillLibrary.skillCount')}</dt><dd>{source.skills.length}</dd>
            <dt>{t('nativeSkillLibrary.lastIndexed')}</dt><dd>{date(source.indexed_at)}</dd>
            {source.commit && <><dt>{t('nativeSkillLibrary.commit')}</dt><dd>{source.commit}</dd></>}
          </dl>
          {source.error && <p role="status">{t('nativeSkillLibrary.lastResults')} — {issue(source.error)}</p>}
          {source.rejected.length > 0 && <details><summary>{t('nativeSkillLibrary.rejectedPackages', { count: source.rejected.length })}</summary><ul>{source.rejected.map(([path, code]) => <li key={path}>{path || '/'}: {issue(code)}</li>)}</ul></details>}
          <div className="native-skill-library__actions">
            <Button variant="secondary" disabled={disabled || busy} onClick={() => void refresh(source)}><RefreshCw size={14} aria-hidden="true" />{t('nativeSkillLibrary.refreshIndex')}</Button>
            <Button variant="secondary" disabled={disabled || busy} onClick={() => edit(source)}>{t('nativeSkillLibrary.editSource')}</Button>
            <Button variant="danger" disabled={disabled || busy} onClick={() => void remove(source)}>{t('nativeSkillLibrary.removeSource')}</Button>
          </div>
        </li>)}
      </ul>
    </> : <>
      <div className="native-skill-library__actions" role="group" aria-label={t('nativeSkillLibrary.importMethod')}>
        {(['catalog', 'github', 'local'] as const).map(value => <Button key={value} variant={mode === value ? 'primary' : 'secondary'} aria-pressed={mode === value} disabled={disabled || busy} onClick={() => { setMode(value); setSelected(null); }}>{t(`nativeSkillLibrary.method.${value}`)}</Button>)}
      </div>
      {mode === 'github' ? sourceForm : mode === 'local' ? <>
        <p className="native-skill-library__hint">{t('nativeSkillLibrary.pickerBoundary')}</p>
        <div className="native-skill-library__actions">
          <Button variant="secondary" disabled={disabled || busy} onClick={() => void pick('zip')}><Archive size={16} aria-hidden="true" />{t('nativeSkillLibrary.pickZip')}</Button>
          <Button variant="secondary" disabled={disabled || busy} onClick={() => void pick('directory')}><FolderOpen size={16} aria-hidden="true" />{t('nativeSkillLibrary.pickDirectory')}</Button>
        </div>
        {picked && <>
          <p>{picked.label} — {t('nativeSkillLibrary.grantExpires')}: {date(picked.expires_at)}</p>
          {picked.rejected.length > 0 && <p role="status">{t('nativeSkillLibrary.rejectedPackages', { count: picked.rejected.length })}</p>}
          {!picked.skills.length && <p>{t('nativeSkillLibrary.noValidSkills')}</p>}
          <ul className="native-skill-library__results">{picked.skills.map(skill => <li key={skill.path}><button type="button" aria-pressed={selected?.skill === skill} disabled={disabled || busy} onClick={() => choose(skill, { kind: 'picker', handle: picked.handle, skill_path: skill.path }, null, null)}><strong>{skill.name}</strong>{' '}<span>{skill.description}</span>{' '}<small>{skill.path || '/'}</small></button></li>)}</ul>
        </>}
      </> : <>
        <form className="native-skill-library__search" onSubmit={event => { event.preventDefault(); void search(); }}>
          <Input label={t('nativeSkillLibrary.searchQuery')} value={query} onChange={event => setQuery(event.target.value)} disabled={disabled || busy} />
          <Select label={t('nativeSkillLibrary.sourceFilter')} value={sourceId} options={[{ value: 'all', label: t('nativeSkillLibrary.allSources') }, ...(sourceState?.value ?? []).map(source => ({ value: source.id, label: `${source.repository.owner}/${source.repository.repo}` }))]} onChange={setSourceId} disabled={disabled || busy} />
          <Button type="submit" disabled={disabled || busy}><Search size={15} aria-hidden="true" />{t('nativeSkillLibrary.search')}</Button>
        </form>
        <p className="native-skill-library__hint">{t('nativeSkillLibrary.explicitIndex')}</p>
        {sourceState?.error && <p role="alert">{issue(sourceState.error)}</p>}
        {results && !results.length && <p>{t('nativeSkillLibrary.noResults')}</p>}
        <ul className="native-skill-library__results">{results?.map(result => <li key={`${result.source_id}:${result.skill.path}`}><button type="button" aria-pressed={selected?.skill === result.skill} disabled={disabled || busy} onClick={() => choose(result.skill, { kind: 'repository', source_id: result.source_id, skill_path: result.skill.path }, result.repository_url, result.commit, result.stale)}><strong>{result.skill.name}</strong>{' '}<span>{result.skill.description}</span>{' '}<small>{result.repository_url} / {result.skill.path || '/'}{result.stale ? ` — ${t('nativeSkillLibrary.lastResults')}` : ''}</small></button></li>)}</ul>
      </>}
      {selected && <section className="native-skill-library__selection" aria-label={t('nativeSkillLibrary.selectedSkill')}>
        <h3>{selected.skill.name}</h3><p>{selected.skill.description}</p>
        {selected.repository && <p>{selected.repository}</p>}
        {selected.commit && <p>{t('nativeSkillLibrary.commit')}: <code>{selected.commit}</code></p>}
        {selected.stale && <p role="status">{t('nativeSkillLibrary.lastResults')}</p>}
        <LibraryMetadata skill={selected.skill} />
        <p className="native-skill-library__hint">{t('nativeSkillLibrary.installBoundary')}</p>
        <Input label={t('nativeSkillLibrary.installSlug')} value={slug} onChange={event => setSlug(event.target.value)} disabled={disabled || busy} />
        <Button disabled={disabled || busy || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(slug)} onClick={() => onPreview({ ...selection, action: 'install', slug, expected: null, source: selected.source })}>{t('nativeSkillLibrary.previewInstall')}</Button>
      </section>}
    </>}
    {busy && <p role="status">{t('nativeSkillLibrary.working')}</p>}
    {message && <p role="status">{message}</p>}
  </section>;
}

function VersionPlanDetails({ plan }: { plan: NativeVersionPlan }) {
  return <dl className="native-skill-library__facts"><dt>{t('nativeSkillLibrary.program')}</dt><dd>{plan.executable}</dd><dt>{t('nativeSkillLibrary.argv')}</dt><dd><code>{JSON.stringify(plan.argv)}</code></dd><dt>{t('nativeSkillLibrary.cwd')}</dt><dd>{plan.cwd}</dd><dt>{t('nativeSkillLibrary.contentHash')}</dt><dd>{plan.digest}</dd><dt>{t('nativeSkillLibrary.environment')}</dt><dd>{plan.environment}</dd><dt>{t('nativeSkillLibrary.grantExpires')}</dt><dd>{date(plan.expires_at)}</dd></dl>;
}

export function NativeSkillVersionRecovery({ record, disabled = false }: { record: NativeVersionRecord; disabled?: boolean }) {
  const { plan, result } = record;
  return <section className="native-skill-library__selection" aria-label={t('nativeSkillLibrary.versionApproval')}>
    <h4>{t('nativeSkillLibrary.versionApproval')}</h4><p>{record.skill.selection.target} · {t(`nativeExtensions.scope.${record.skill.selection.scope}`)} · {record.skill.slug} · {plan.plan_id}</p>
    <VersionPlanDetails plan={plan} />
    <p role="status">{t(`nativeSkillLibrary.versionStatus.${result.status}`)}</p>
    {result.output && <pre className="native-skill-library__body" tabIndex={0}>{result.output}</pre>}
    <p>{t(result.cleanup_confirmed ? 'nativeSkillLibrary.cleanupConfirmed' : 'nativeSkillLibrary.cleanupUnknown')}</p><p>{t('nativeSkillLibrary.requirementsUnknown')}</p>
    {record.error && <p role="alert">{issue(record.error)}</p>}
    <Button variant="secondary" disabled={disabled || record.busy} onClick={() => void useNativeSkillLibrary.getState().inspectVersion(plan.plan_id)}>{t('nativeSkillLibrary.inspectVersion')}</Button>
  </section>;
}

export function NativeSkillInspector({ selection, skill, disabled, onPreview }: { selection: NativeSelection; skill: NativeSkill; disabled: boolean; onPreview: (input: PreviewNativeSkill) => void }) {
  const api = useNativeSkillLibrary();
  const scopeContext = useDevelopment(() => nativeLibraryKey(selection));
  const context = JSON.stringify([scopeContext, skill.instance.id, skill.instance.revision]);
  const [inspection, setInspection] = useState<NativeSkillInspection | null>(null);
  const [rereadSkill, setRereadSkill] = useState<NativeSkill | null>(null);
  const activeSkill = rereadSkill ?? skill;
  const [file, setFile] = useState<NativeSkillFile | null>(null);
  const [report, setReport] = useState<NativeDependencyReport | null>(null);
  const [plan, setPlan] = useState<NativeVersionPlan | null>(null);
  const versionRecord = (plan ? api.versions[plan.plan_id] : undefined) ?? Object.values(api.versions).find(record => !record.result.cleanup_confirmed && record.skill.slug === skill.slug && JSON.stringify([record.skill.selection.target, record.skill.selection.scope, record.skill.selection.project_id, record.skill.selection.task_id ?? null]) === JSON.stringify([selection.target, selection.scope, selection.project_id, selection.task_id ?? null]));
  const [replacement, setReplacement] = useState<NativePickedSkillSource | null>(null);
  const replacementGrant = useRef<{ selection: NativeSelection; context: string; handle: string } | null>(null);
  const [busy, setBusy] = useState(false), [message, setMessage] = useState('');
  const flight = useRef(false), generation = useRef(0), current = useRef(context); current.current = context;
  const unresolved = Object.values(api.versions).some(record => record.busy || !record.result.cleanup_confirmed);
  useNativeManagerGuard(() => !flight.current && !unresolved);
  const request = { selection, slug: activeSkill.slug, expected: activeSkill.instance };
  useEffect(() => {
    generation.current++; flight.current = false; setBusy(false); setMessage(''); setInspection(null); setFile(null); setReport(null);
    setPlan(Object.values(useNativeSkillLibrary.getState().versions).find(record => !record.result.cleanup_confirmed && record.skill.slug === skill.slug && JSON.stringify([record.skill.selection.target, record.skill.selection.scope, record.skill.selection.project_id, record.skill.selection.task_id ?? null]) === JSON.stringify([selection.target, selection.scope, selection.project_id, selection.task_id ?? null]))?.plan ?? null);
    setRereadSkill(null);
    setReplacement(null);
    const token = generation.current;
    void api.inspect(request).then(result => {
      if (token !== generation.current || current.current !== context) return;
      if (result.ok) setInspection(result.value); else setMessage(issue(result.code));
    });
    return () => {
      generation.current++;
      if (replacementGrant.current?.context === context) { void api.release(replacementGrant.current.selection, replacementGrant.current.handle); replacementGrant.current = null; }
    };
  }, [context]);
  useEffect(() => {
    if (!plan) return;
    const close = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); if (!flight.current && !unresolved) setPlan(null); } };
    document.addEventListener('keydown', close, true);
    return () => document.removeEventListener('keydown', close, true);
  }, [plan, unresolved]);
  async function perform<T>(work: () => Promise<NativeResult<T>>, accept: (value: T) => void, dispose?: (value: T) => void) {
    if (disabled || flight.current || unresolved) return;
    const token = generation.current, key = context;
    flight.current = true; setBusy(true); setMessage('');
    try {
      const result = await work();
      if (generation.current !== token || current.current !== key) { if (result.ok) dispose?.(result.value); return; }
      if (result.ok) accept(result.value); else setMessage(issue(result.code));
    } finally { if (generation.current === token && current.current === key) { flight.current = false; setBusy(false); } }
  }
  const pickReplacement = (kind: 'zip' | 'directory') => perform(async () => {
    if (replacementGrant.current) { await api.release(replacementGrant.current.selection, replacementGrant.current.handle); replacementGrant.current = null; setReplacement(null); }
    return api.pick(selection, kind);
  }, value => { if (value) { replacementGrant.current = { selection, context, handle: value.handle }; setReplacement(value); } }, value => { if (value) void api.release(selection, value.handle); });
  const source = activeSkill.source;
  const blocked = disabled || busy || unresolved;
  const localUpdateReview = activeSkill.source_modified && !rereadSkill;
  return <section className="native-skill-library native-skill-inspector" aria-label={t('nativeSkillLibrary.inspector')} aria-busy={busy}>
    <dl className="native-skill-library__facts">
      <dt>{t('nativeSkillLibrary.actualTarget')}</dt><dd>{inspection?.target_path ?? t('nativeSkillLibrary.pathLoading')}</dd>
      <dt>{t('nativeSkillLibrary.commit')}</dt><dd>{source?.commit ?? t('nativeSkillLibrary.unversioned')}</dd>
      <dt>{t('nativeSkillLibrary.contentHash')}</dt><dd>{source?.source_hash ?? '—'}</dd>
      {source?.origin && <><dt>{t('nativeSkillLibrary.source')}</dt><dd><Button variant="secondary" disabled={disabled} onClick={() => void openUrl(`https://github.com/${encodeURIComponent(source.origin!.repository.owner)}/${encodeURIComponent(source.origin!.repository.repo)}`)}>{source.origin.repository.owner}/{source.origin.repository.repo}</Button> / {source.origin.skill_path || '/'}</dd></>}
    </dl>
    <h4>{t('nativeSkillLibrary.files')}</h4>
    <ul className="native-skill-library__files">{activeSkill.resources.map(resource => <li key={resource.path}><Button variant="secondary" disabled={blocked} onClick={() => void perform(() => api.readFile(request, resource.path), setFile)}>{resource.path}</Button><span>{t('nativeSkillLibrary.fileBytes', { count: resource.bytes })}{resource.executable ? ` — ${t('nativeSkillLibrary.executable')}` : ''}</span></li>)}</ul>
    {file && <section aria-label={t('nativeSkillLibrary.fileContent')}><h4>{file.path}</h4><p className="native-skill-library__hint">{file.hash}</p>{file.text !== null ? <pre className="native-skill-library__body" tabIndex={0}>{file.text}</pre> : <p>{t(`nativeSkillLibrary.fileReason.${file.reason ?? 'binary'}`)}</p>}</section>}
    <h4>{t('nativeSkillLibrary.dependencies')}</h4>
    <p className="native-skill-library__hint">{t('nativeSkillLibrary.staticBoundary')}</p>
    <Button variant="secondary" disabled={blocked} onClick={() => void perform(() => api.dependencies(request), value => { setReport(value); setPlan(null); })}>{t('nativeSkillLibrary.checkDependencies')}</Button>
    {report && <>
      {!report.declarations.declared && <p>{t('nativeSkillLibrary.noDeclarations')}</p>}
      <ul className="native-skill-library__dependencies">{report.tools.map(tool => <li key={tool.declaration.program}><strong>{tool.declaration.program}</strong><span>{tool.declaration.version_requirement ?? t('nativeSkillLibrary.noVersionRequirement')} — {t(`nativeSkillLibrary.availability.${tool.availability}`)}</span>{tool.candidate && <code>{tool.candidate}</code>}{tool.candidate && tool.executable && <Button variant="secondary" disabled={blocked} onClick={() => void perform(() => api.previewVersion(request, tool.declaration.program), setPlan)}>{t('nativeSkillLibrary.previewVersion')}</Button>}</li>)}</ul>
      {report.declarations.packages.length > 0 && <p>{t('nativeSkillLibrary.packagesUnverified')}: {report.declarations.packages.join(', ')}</p>}
      {report.declarations.required_tools.length > 0 && <p>{t('nativeSkillLibrary.apisUnverified')}: {report.declarations.required_tools.join(', ')}</p>}
      {report.unknown && <p className="native-skill-library__hint">{t('nativeSkillLibrary.requirementsUnknown')}</p>}
    </>}
    {versionRecord ? <NativeSkillVersionRecovery record={versionRecord} disabled={disabled} /> : plan && <section className="native-skill-library__selection" aria-label={t('nativeSkillLibrary.versionApproval')}>
      <h4>{t('nativeSkillLibrary.versionApproval')}</h4><VersionPlanDetails plan={plan} />
      <p>{t('nativeSkillLibrary.versionWarning')}</p>
      <div className="native-skill-library__actions"><Button disabled={blocked} onClick={() => void api.runVersion(request, plan)}>{t('nativeSkillLibrary.approveVersion')}</Button><Button variant="secondary" disabled={blocked} onClick={() => setPlan(null)}>{t('nativeSkillLibrary.cancelVersion')}</Button></div>
    </section>}
    {activeSkill.can_update && source?.origin && <>
      <h4>{t('nativeSkillLibrary.update')}</h4>
      <p className="native-skill-library__hint">{t('nativeSkillLibrary.updateBoundary')}</p>
      {activeSkill.source_modified && <p role="status">{t('nativeSkillLibrary.localChanges')}</p>}
      <Button disabled={blocked || localUpdateReview} onClick={() => void perform(() => api.prepareUpdate(request), value => { replacementGrant.current = { selection, context, handle: value.handle }; onPreview({ ...selection, action: 'update', slug: activeSkill.slug, expected: activeSkill.instance, source: { kind: 'picker', handle: value.handle, skill_path: value.skill_path } }); }, value => { void api.release(selection, value.handle); })}>{t('nativeSkillLibrary.previewUpdate')}</Button>
    </>}
    {activeSkill.can_update && !source?.origin && <section className="native-skill-library__selection" aria-label={t('nativeSkillLibrary.localUpdate')}>
      <h4>{t('nativeSkillLibrary.localUpdate')}</h4>
      <p>{t('nativeSkillLibrary.localUpdateBoundary')}</p>
      {activeSkill.source_modified && <p role="status">{t('nativeSkillLibrary.localChanges')}</p>}
      <div className="native-skill-library__actions"><Button variant="secondary" disabled={blocked || localUpdateReview} onClick={() => void pickReplacement('zip')}>{t('nativeSkillLibrary.pickZip')}</Button><Button variant="secondary" disabled={blocked || localUpdateReview} onClick={() => void pickReplacement('directory')}>{t('nativeSkillLibrary.pickDirectory')}</Button></div>
      {replacement && <ul className="native-skill-library__results">{replacement.skills.map(candidate => <li key={candidate.path}><Button variant="secondary" disabled={blocked || localUpdateReview} onClick={() => onPreview({ ...selection, action: 'update', slug: activeSkill.slug, expected: activeSkill.instance, source: { kind: 'picker', handle: replacement.handle, skill_path: candidate.path } })}>{t('nativeSkillLibrary.previewReplacement')}: {candidate.name} / {candidate.path || '/'}</Button></li>)}</ul>}
    </section>}
    {message && <p role="alert">{message}</p>}
    {(message || localUpdateReview) && <div className="native-skill-library__actions">
      <Button variant="secondary" disabled={blocked} onClick={() => { setMessage(''); setPlan(null); }}>{t('nativeSkillLibrary.keepLocal')}</Button>
      <Button variant="secondary" disabled={blocked} onClick={() => void perform(async () => {
        const result = await useNativeExtensions.getState().getSkill({ ...selection, slug: skill.slug });
        return result.ok ? api.inspect({ selection, slug: result.value.slug, expected: result.value.instance }) : result;
      }, value => {
        setRereadSkill(value.skill); setInspection(value); setFile(null); setReport(null); setPlan(null);
        setMessage(t('nativeSkillLibrary.rereadInstruction'));
      })}>{t('nativeSkillLibrary.reread')}</Button>
    </div>}
    {busy && <p role="status">{t('nativeSkillLibrary.working')}</p>}
  </section>;
}
