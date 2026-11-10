import { useEffect, useId, useRef, useState } from 'react';
import { Button, Input, ResourceDetail, ResourceLayout, ResourceList, Select, StatusBadge } from '@dgos/ui';
import { t } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { useNativeExtensions } from '../../stores/nativeExtensions';
import type { NativeDraft, NativeDraftField, NativeField, NativeMcpChange, NativePatchMode, NativePending, NativeResult, NativeSelection, NativeServerPatch, NativeSkillRuleIntentState } from '../../types/nativeExtensions';
import { resourceView, type ResourceReadState } from '../../resourceState';
import { useSurfaceCloseGuard, useSurfaceId, useSurfaceStatus } from '../../workspace/SurfaceContext';
import { confirmDiscard } from '../../workspace/closeCoordinator';
import NativeSessionPanel from './NativeSessionPanel';

const lines = (value: string) => value.split('\n').map(item => item.trim()).filter(Boolean);
const envName = (value: string) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(value);
const emptyDraft: NativeDraft = { slug: '', source: '', name: '', transport: 'Stdio', program: '', args: '', url: '', cwd: '', bearer_env: '', forwarded_env: '', header_env: '', modes: {} };
const ruleStates: NativeSkillRuleIntentState[] = ['on', 'name-only', 'user-invocable-only', 'off', 'inherit'];
function headerPairs(value: string): [string, string][] | null {
  const entries: [string, string][] = [], names = new Set<string>();
  for (const line of lines(value)) {
    const index = line.indexOf('='), key = line.slice(0, index).trim(), variable = line.slice(index + 1).trim();
    if (index <= 0 || !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(key) || !envName(variable) || names.has(key.toLowerCase())) return null;
    names.add(key.toLowerCase()); entries.push([key, variable]);
  }
  return entries;
}
function PreviewDetails({ operation, observationsId, showObservations }: { operation: NativePending; observationsId: string; showObservations: () => void }) {
  const value = operation.preview, base = 'preview' in value ? value.preview : value, receipt = operation.receipt;
  const projectPath = useDevelopment(s => s.projects.find(project => project.id === operation.selection.project_id)?.path);
  return <section aria-label={t('nativeExtensions.preview')} className="mt-3 flex min-w-0 flex-col gap-2 border-t border-border-subtle pt-3 text-sm">
    <h3 className="m-0 text-base">{t('nativeExtensions.preview')}</h3>
    <p className="m-0 break-all">{operation.summary}</p>
    <dl className="grid grid-cols-2 gap-2">
      <dt>{t('nativeExtensions.target')}</dt><dd className="m-0">{base.target}</dd>
      <dt>{t('nativeExtensions.scope')}</dt><dd className="m-0">{t(`nativeExtensions.scope.${base.scope}`)}</dd>
      <dt>{t('nativeExtensions.project')}</dt><dd className="m-0 break-all">{operation.selection.project_id ?? '—'}</dd>
      {operation.selection.project_id && <><dt>{t('nativeExtensions.ruleProjectDirectory')}</dt><dd className="m-0 break-all">{projectPath ?? t('nativeExtensions.unknown')}</dd></>}
      <dt>{t('nativeExtensions.expires')}</dt><dd className="m-0">{new Date(base.expires_at * 1000).toLocaleString()}</dd>
      <dt>{t('nativeExtensions.operationId')}</dt><dd className="m-0 break-all">{receipt.operation_id}</dd>
      <dt>{t('nativeExtensions.operationState')}</dt><dd className="m-0">{t(`nativeExtensions.state.${receipt.state}`)}</dd>
      <dt>{t('nativeExtensions.pendingActivation')}</dt><dd className="m-0">{t(receipt.pending_activation ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</dd>
      <dt>{t('nativeExtensions.recoveryRetained')}</dt><dd className="m-0">{t(receipt.recovery_material_retained ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</dd>
    </dl>
    {'slug' in value && <>
      <p className="m-0 break-all">{value.slug}</p>
      {value.metadata && <><h4 className="m-0">{value.metadata.name}</h4><p className="m-0">{value.metadata.description}</p><p className="m-0">{t('nativeExtensions.license')}: {value.metadata.license ?? t('nativeExtensions.unknown')}</p><p className="m-0">{t('nativeExtensions.dynamicCommands')}: {t(value.metadata.dynamic_commands ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</p><p className="m-0">{t('nativeExtensions.dependenciesNotChecked')}: {[...value.metadata.dependencies.tools.map(tool => [tool.program, tool.version_requirement].filter(Boolean).join(' ')), ...value.metadata.dependencies.packages, ...value.metadata.dependencies.required_tools].join(', ') || t('nativeExtensions.unknown')}</p></>}
      <dl className="grid grid-cols-2 gap-2"><dt>{t('nativeExtensions.beforeHash')}</dt><dd className="m-0 break-all">{value.before_hash ?? '—'}</dd><dt>{t('nativeExtensions.afterHash')}</dt><dd className="m-0 break-all">{value.after_hash ?? '—'}</dd><dt>{t('nativeExtensions.sourceHash')}</dt><dd className="m-0 break-all">{value.source?.source_hash ?? t('nativeExtensions.unknown')}</dd><dt>{t('nativeExtensions.commit')}</dt><dd className="m-0 break-all">{value.source?.commit ?? t('nativeExtensions.unknown')}</dd></dl>
      <h4 className="m-0">{t('nativeExtensions.resources')} ({value.resources.length})</h4>
      <ul className="m-0 pl-5">{value.resources.map(resource => <li key={resource.path} className="break-all">{resource.path} · {resource.hash} · {t('nativeExtensions.bytes', { count: resource.bytes })} · {t('nativeExtensions.executable')}: {t(resource.executable ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</li>)}</ul>
      <p className="m-0">{t('nativeExtensions.scriptsNotRun')}</p>
      {value.impact && <>
        <h4 className="m-0">{t('nativeExtensions.resourceImpact')}</h4>
        <p className="m-0 break-all">{t('nativeExtensions.declaredName')}: {value.impact.native_name}</p>
        {value.impact.previous_name && <p className="m-0 break-all">{t('nativeExtensions.previousName')}: {value.impact.previous_name}</p>}
        <p className="m-0">{t('nativeExtensions.looseImpactCoverage')}</p>
        {value.impact.incomplete && <p className="m-0" role="status">{t('nativeExtensions.incomplete')}</p>}
        <ul className="m-0 pl-5">{value.impact.same_name.map(item => <li key={JSON.stringify([item.scope, item.slug])} className="break-all">{item.slug} · {item.native_name} · {t(`nativeExtensions.scope.${item.scope}`)} · {t(`nativeExtensions.owner.${item.owner}`)}</li>)}</ul>
        {!value.impact.visibility_writable && <p className="m-0">{t('nativeExtensions.visibilityUnavailable')}</p>}
      </>}
    </>}
    {'affected' in value && <>
      <h4 className="m-0">{t('nativeExtensions.affectedVisibility')}</h4>
      <p className="m-0">{t(`nativeExtensions.coverage.${value.coverage}`)}</p>
      {value.rule && <>
        <dl className="grid grid-cols-2 gap-2">
          <dt>{t('nativeExtensions.ruleName')}</dt><dd className="m-0 break-all">{value.rule.name}</dd>
          <dt>{t('nativeExtensions.previousRule')}</dt><dd className="m-0">{t(`nativeExtensions.ruleState.${value.rule.previous ?? 'inherit'}`)}</dd>
          <dt>{t('nativeExtensions.ruleState')}</dt><dd className="m-0">{t(`nativeExtensions.ruleState.${value.rule.state}`)}</dd>
          <dt>{t('nativeExtensions.settingsLayer')}</dt><dd className="m-0">{t(`nativeExtensions.scope.${value.rule.scope}`)}</dd>
        </dl>
        <p className="m-0">{t(`nativeExtensions.ruleMeaning.${value.rule.state}`)}</p>
        <p className="m-0">{t('nativeExtensions.ruleNameImpact')}</p>
        <p className="m-0">{t('nativeExtensions.rulePluginsExcluded')}</p>
        <p className="m-0">{t('nativeExtensions.rulePrecedenceWarning')}</p>
        <p className="m-0">{t('nativeExtensions.ruleHookWarning')}</p>
        <p className="m-0">{t('nativeExtensions.ruleObservationsNotCaptured')}</p>
        <p className="m-0">{t('nativeExtensions.ruleObservationsLink')}</p>
        <Button variant="secondary" aria-controls={observationsId} onClick={showObservations}>{t('nativeExtensions.reviewRuleObservations')}</Button>
      </>}
      <ul className="m-0 pl-5">{value.affected.map(item => <li key={JSON.stringify([item.selection, item.instance.id])} className="break-all">{item.slug} · {item.selection.target} · {t(`nativeExtensions.scope.${item.selection.scope}`)} · {item.selection.project_id ?? '—'} · {item.instance.id} · {item.instance.revision}</li>)}</ul>
    </>}
  </section>;
}

export default function NativeManager({ kind, initialSelection, initialSessionId }: { kind: 'skills' | 'mcp'; initialSelection?: NativeSelection; initialSessionId?: string }) {
  const account = useDevelopment(s => s.epoch), projects = useDevelopment(s => s.projects);
  const api = useNativeExtensions();
  const surfaceId = useSurfaceId(), instanceId = useId();
  const observationsRef = useRef<HTMLDivElement>(null);
  const observationsId = useId();
  const ownerKey = JSON.stringify([account, surfaceId ?? instanceId, kind]);
  const ownerPrefix = `[${JSON.stringify(ownerKey)},`;
  const ownedOperations = useRef(new Set<string>());
  const [target, setTarget] = useState<'Codex' | 'Claude'>(initialSelection?.target ?? 'Codex');
  const [scope, setScope] = useState<'OsUser' | 'ProjectShared' | 'ProjectLocal'>(initialSelection?.target === 'Codex' && initialSelection.scope === 'ProjectLocal' ? 'OsUser' : initialSelection?.scope ?? 'OsUser');
  const [project, setProject] = useState<string | null>(initialSelection?.scope === 'OsUser' ? null : initialSelection?.project_id ?? null), [query, setQuery] = useState('');
  const [views, setViews] = useState<Record<string, { creating: boolean; selected: string | null }>>({});
  const [messages, setMessages] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, Partial<Record<NativeDraftField, string>>>>({});
  const generation = useRef(0);
  const entryKey = initialSelection ? JSON.stringify([initialSelection.target, initialSelection.scope, initialSelection.project_id]) : '';
  useEffect(() => {
    if (!initialSelection) return;
    const nextScope = initialSelection.target === 'Codex' && initialSelection.scope === 'ProjectLocal' ? 'OsUser' : initialSelection.scope;
    setTarget(initialSelection.target); setScope(nextScope);
    setProject(nextScope === 'OsUser' ? null : initialSelection.project_id);
  }, [entryKey]);
  const selection: NativeSelection = { target, scope, project_id: scope === 'OsUser' ? null : project };
  const selectionKey = JSON.stringify([account, target, scope, selection.project_id]);
  const viewKey = JSON.stringify([selectionKey, kind]), view = views[viewKey];
  const creating = view?.creating ?? false;
  const busy = Boolean(api.busy[selectionKey]);
  const claudeRules = kind === 'skills' && target === 'Claude';
  const resourceUnsupported = kind === 'skills' && scope === 'ProjectLocal';
  const skillState = resourceUnsupported ? undefined : api.skills[`${selectionKey}:${query}`], mcpState = api.mcp[selectionKey];
  const rulesState = api.rules[selectionKey];
  const state = kind === 'skills' ? skillState : mcpState;
  const skills = kind === 'skills' ? skillState?.value?.instances ?? [] : [];
  const readonlyEntries = kind === 'skills' ? skillState?.value?.readonly_entries ?? [] : [];
  const servers = kind === 'mcp' ? mcpState?.value?.instances ?? [] : [];
  const ids = kind === 'skills' ? skills.map(item => item.instance.id) : servers.map(item => item.native_key);
  const chosen = view?.selected && ids.includes(view.selected) ? view.selected : ids[0] ?? null;
  const skill = skills.find(item => item.instance.id === chosen), server = servers.find(item => item.native_key === chosen);
  const draftKey = JSON.stringify([ownerKey, selectionKey, creating ? 'create' : chosen]);
  const draft: NativeDraft = api.drafts[draftKey] ?? emptyDraft, operation = api.pending[selectionKey];
  const ruleDraftKey = JSON.stringify([ownerKey, selectionKey, 'claude_name_rule']);
  const ruleDraft = api.drafts[ruleDraftKey] ?? emptyDraft;
  const ruleState = ruleDraft.visibilityState ?? 'on';
  const blocked = busy || Boolean(operation && operation.mode !== 'result');
  const validSelection = scope === 'OsUser' || projects.some(item => item.id === project && !item.archived);
  const dirty = Object.keys(api.drafts).some(id => id.startsWith(ownerPrefix));
  const running = ownedOperations.current.size > 0;
  useSurfaceStatus({ dirty, running, runningLabel: running ? t('nativeExtensions.working') : undefined });
  useSurfaceCloseGuard(async () => {
    if (ownedOperations.current.size > 0) return false;
    const drafts = Object.fromEntries(Object.entries(useNativeExtensions.getState().drafts).filter(([id]) => id.startsWith(ownerPrefix)));
    if (!Object.keys(drafts).length) return true;
    if (!await confirmDiscard() || useDevelopment.getState().epoch !== account || ownedOperations.current.size > 0) return false;
    if (Object.keys(useNativeExtensions.getState().drafts).filter(id => id.startsWith(ownerPrefix)).length !== Object.keys(drafts).length) return false;
    return api.discardDrafts(drafts);
  });
  useEffect(() => { ownedOperations.current.clear(); setViews({}); setMessages({}); setErrors({}); }, [account]);
  useEffect(() => { if (!useDevelopment.getState().loaded) void useDevelopment.getState().load(); }, []);
  const message = (text: string) => { if (useDevelopment.getState().epoch === account) setMessages(current => ({ ...current, [viewKey]: text })); };
  useEffect(() => {
    if (!validSelection) return;
    const seq = ++generation.current;
    const pending = claudeRules ? Promise.all([
      api.discoverSkillRules(selection),
      ...(resourceUnsupported ? [] : [api.discoverSkills({ ...selection, query })]),
    ]) : Promise.all([kind === 'skills' ? api.discoverSkills({ ...selection, query }) : api.discoverMcp(selection)]);
    void pending.then(results => { const failure = results.find(result => !result.ok); if (seq === generation.current && failure && !failure.ok) message(t(`nativeExtensions.error.${failure.code}`)); });
    return () => { generation.current++; };
  }, [selectionKey, query, kind, validSelection]);
  const refresh = () => claudeRules ? Promise.all([api.discoverSkillRules(selection), ...(resourceUnsupported ? [] : [api.discoverSkills({ ...selection, query })])]) : kind === 'skills' ? api.discoverSkills({ ...selection, query }) : api.discoverMcp(selection);
  const changeView = (creating: boolean, selected: string | null = chosen) => setViews(current => ({ ...current, [viewKey]: { creating, selected } }));
  const saveDraft = (next: NativeDraft) => { api.setDraft(draftKey, next); setErrors(current => ({ ...current, [draftKey]: {} })); };
  const validate = (fields: Partial<Record<NativeDraftField, string>>) => {
    setErrors(current => ({ ...current, [draftKey]: fields }));
    if (Object.keys(fields).length) { message(t('nativeExtensions.correctFields')); return false; }
    return true;
  };
  const preview = async (call: () => Promise<NativeResult<NativePending['preview']>>, summary: string, consumesDraft = true, previewDraftKey = draftKey) => {
    if (!validSelection || !api.begin(selectionKey, true)) return;
    ownedOperations.current.add(selectionKey);
    message(t('nativeExtensions.working'));
    const snapshot = consumesDraft ? api.drafts[previewDraftKey] : undefined;
    try {
      const result = await call();
      if (useDevelopment.getState().epoch !== account) return;
      if (!result.ok) { message(t(`nativeExtensions.error.${result.code}`)); return; }
      const receipt = ('preview' in result.value ? result.value.preview : result.value).receipt;
      api.setPending(selectionKey, { kind, selection, mode: 'preview', receipt, preview: result.value, draft_key: previewDraftKey, draft: snapshot, summary });
      message(t('nativeExtensions.previewReady'));
    } finally { ownedOperations.current.delete(selectionKey); if (useDevelopment.getState().epoch === account) api.end(selectionKey); }
  };
  const resolveOperation = async (action: 'commit' | 'cancel' | 'inspect') => {
    if (!validSelection || !operation || !api.begin(selectionKey, false)) return;
    ownedOperations.current.add(selectionKey);
    try {
      const result = await api[action]({ operation_id: operation.receipt.operation_id });
      if (useDevelopment.getState().epoch !== account) return;
      if (!result.ok) {
        api.setPending(selectionKey, { ...operation, mode: 'inspect' });
        message(`${t(`nativeExtensions.error.${result.code}`)} ${t('nativeExtensions.needsVerification')}`); return;
      }
      const receipt = result.value;
      const terminal = receipt.state === 'Committed' || receipt.state === 'Cancelled' || receipt.state === 'Conflict';
      api.setPending(selectionKey, { ...operation, receipt, mode: terminal ? 'result' : receipt.state === 'Previewed' ? 'preview' : 'inspect' });
      const nameRule = 'affected' in operation.preview && operation.preview.rule !== null;
      message(receipt.state === 'Unknown' ? t('nativeExtensions.needsVerification') : receipt.state === 'Committed' && nameRule ? t('nativeExtensions.savedRefreshExplicit') : t(`nativeExtensions.state.${receipt.state}`));
      if (receipt.state === 'Committed') {
        api.clearCommittedDraft(operation);
        if (!nameRule) {
          const read = await (operation.kind === 'skills' ? api.discoverSkills({ ...operation.selection, query }) : api.discoverMcp(operation.selection));
          if (!read.ok) message(t('nativeExtensions.writtenReadPending'));
        }
      }
    } finally { ownedOperations.current.delete(selectionKey); if (useDevelopment.getState().epoch === account) api.end(selectionKey); }
  };
  const previewSkill = (action: 'install' | 'update' | 'remove') => {
    if (!validate({ ...(action === 'install' && !draft.slug.trim() ? { slug: t('nativeExtensions.required') } : {}), ...(action !== 'remove' && !draft.source.trim() ? { source: t('nativeExtensions.required') } : {}) })) return;
    void preview(() => api.previewSkill({ ...selection, action, slug: action === 'install' ? draft.slug.trim() : skill!.slug, expected: action === 'install' ? null : skill!.instance, source: action === 'remove' ? null : { kind: 'github', input: draft.source.trim() } }), t(`nativeExtensions.skillAction.${action}`), action !== 'remove');
  };
  const saveRuleDraft = (name: string, state: NativeSkillRuleIntentState = ruleState) => {
    api.setDraft(ruleDraftKey, { ...ruleDraft, name, visibilityState: state });
    setErrors(current => ({ ...current, [ruleDraftKey]: {} }));
  };
  const previewRule = (state = ruleState, name = ruleDraft.name, consumesDraft = true) => {
    if (!name.trim()) {
      setErrors(current => ({ ...current, [ruleDraftKey]: { name: t('nativeExtensions.required') } }));
      message(t('nativeExtensions.correctFields')); return;
    }
    void preview(() => api.previewVisibility({ ...selection, intent: { kind: 'claude_name', name: name.trim(), state } }), `${name.trim()} · ${t(`nativeExtensions.ruleState.${state}`)}`, consumesDraft, ruleDraftKey);
  };
  const previewMcp = (change: NativeMcpChange) => preview(async () => {
    // Upsert has no expected-instance token. Re-read before preview so an edit
    // cannot silently create a row that disappeared since discovery.
    const current = await api.discoverMcp(selection);
    if (!current.ok) return current;
    if (current.value.incomplete) return { ok: false, code: 'native_extension_Limit' };
    const exists = current.value.instances.some(item => item.native_key === change.name);
    const create = change.kind === 'upsert' && change.fields.transport?.kind === 'set';
    if (create && exists) return { ok: false, code: 'native_extension_Conflict' };
    if (!create && !exists) return { ok: false, code: 'native_extension_NotFound' };
    return api.previewMcp({ ...selection, change });
  }, `${target} · ${t(`nativeExtensions.scope.${scope}`)} · ${change.name} · ${t(`nativeExtensions.change.${change.kind}`)}`, change.kind === 'upsert');
  const mode = (name: keyof NativeServerPatch): NativePatchMode => draft.modes[name] ?? (creating && Boolean(draft[name as NativeDraftField]) ? 'set' : 'keep');
  const previewServer = () => {
    const stdio = (creating ? draft.transport : server?.transport) === 'Stdio';
    const validation: Partial<Record<NativeDraftField, string>> = {};
    if (creating && !draft.name.trim()) validation.name = t('nativeExtensions.required');
    if (creating && servers.some(item => item.native_key === draft.name.trim())) validation.name = t('nativeExtensions.duplicateServer');
    const endpoint = stdio ? 'program' : 'url';
    if ((creating || mode(endpoint) === 'set') && !draft[endpoint].trim()) validation[endpoint] = t('nativeExtensions.required');
    const headers = headerPairs(draft.header_env), refs = lines(draft.forwarded_env);
    if (!stdio && mode('header_env') === 'set' && (!headers || !headers.length)) validation.header_env = t('nativeExtensions.invalidHeaderReference');
    if (stdio && mode('forwarded_env') === 'set' && (refs.some(value => !envName(value)) || new Set(refs).size !== refs.length)) validation.forwarded_env = t('nativeExtensions.invalidEnvironmentReference');
    if (!stdio && mode('bearer_env') === 'set' && !envName(draft.bearer_env.trim())) validation.bearer_env = t('nativeExtensions.invalidEnvironmentReference');
    if (!validate(validation)) return;
    const field = <T,>(name: keyof NativeServerPatch, value: T): NativeField<T> => mode(name) === 'set' ? { kind: 'set', value } : { kind: mode(name) as 'keep' | 'remove' };
    const fields: NativeServerPatch = stdio ? {
      program: field('program', draft.program.trim()), args: field('args', draft.args === '' ? [] : draft.args.split('\n')),
      ...(target === 'Codex' ? { cwd: field('cwd', draft.cwd.trim()), forwarded_env: field('forwarded_env', refs) } : {}),
    } : { url: field('url', draft.url.trim()), header_env: field('header_env', headers ?? []), ...(target === 'Codex' ? { bearer_env: field('bearer_env', draft.bearer_env.trim()) } : {}) };
    if (creating) fields.transport = { kind: 'set', value: draft.transport };
    void previewMcp({ kind: 'upsert', name: creating ? draft.name.trim() : server!.native_key, fields });
  };
  const input = (name: NativeDraftField, patch = false) => {
    const props = { label: t(`nativeExtensions.${name}`), value: draft[name], disabled: busy || (patch && mode(name as keyof NativeServerPatch) === 'remove'), error: errors[draftKey]?.[name], onChange: (event: { target: { value: string } }) => saveDraft({ ...draft, [name]: event.target.value, modes: patch ? { ...draft.modes, [name]: 'set' } : draft.modes }) };
    return <div key={name} className="flex min-w-0 flex-col gap-2">{patch && !creating && <Select label={`${t(`nativeExtensions.${name}`)}: ${t('nativeExtensions.fieldAction')}`} value={mode(name as keyof NativeServerPatch)} disabled={busy} onChange={value => saveDraft({ ...draft, modes: { ...draft.modes, [name]: value as NativePatchMode } })} options={['keep', 'set', ...(name === 'header_env' && target === 'Claude' ? [] : ['remove'])].map(value => ({ value, label: t(`nativeExtensions.field.${value}`) }))}/ >}{name === 'args' || name === 'forwarded_env' || name === 'header_env' ? <Input multiline {...props}/> : <Input {...props}/>}</div>;
  };
  const serverInputs = (transport: string) => transport === 'Stdio' ? <>{input('program', true)}{input('args', true)}{target === 'Codex' && <>{input('cwd', true)}{input('forwarded_env', true)}</>}</> : transport === 'Http' || transport === 'Sse' ? <>{input('url', true)}{target === 'Codex' && input('bearer_env', true)}{input('header_env', true)}</> : null;
  const readState: ResourceReadState = !state ? { status: 'idle' } : state.loading && !state.value ? { status: 'loading' } : state.error && !state.value ? { status: 'error', error: t(`nativeExtensions.error.${state.error}`) } : { status: 'ready', refresh: state.stale ? { status: 'error', error: t('nativeExtensions.stale') } : { status: 'current' } };
  const projectOptions = projects.filter(item => !item.archived).map(item => ({ value: item.id, label: item.name }));
  const scopeOptions = ['OsUser', 'ProjectShared', ...(target === 'Claude' ? ['ProjectLocal'] : [])].map(value => ({ value, label: t(`nativeExtensions.scope.${value}`) }));
  const editDisabled = blocked || !validSelection;
  const resourceEditDisabled = editDisabled || resourceUnsupported;
  return <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto bg-surface-canvas p-3 text-ink-primary" aria-label={t(kind === 'skills' ? 'nativeExtensions.skills' : 'nativeExtensions.mcp')}>
    <header className="flex flex-wrap items-start justify-between gap-3 border-b border-border-subtle pb-3"><div className="min-w-0"><h2 className="m-0 text-base font-semibold">{t(kind === 'skills' ? 'nativeExtensions.skills' : 'nativeExtensions.mcp')}</h2><p className="m-0 text-sm text-ink-secondary">{t('nativeExtensions.configurationNotice')}</p></div><Button variant="secondary" disabled={busy || !validSelection} onClick={() => void refresh()}>{t('nativeExtensions.refresh')}</Button></header>
    <div className="my-3 flex flex-wrap items-end gap-3"><Select label={t('nativeExtensions.target')} value={target} onChange={value => { setTarget(value as typeof target); if (value === 'Codex' && scope === 'ProjectLocal') { setScope('OsUser'); setProject(null); } }} options={['Codex', 'Claude'].map(value => ({ value, label: value }))}/><Select label={t('nativeExtensions.scope')} value={scope} onChange={value => { const next = value as typeof scope; setScope(next); setProject(next === 'OsUser' ? null : project ?? projectOptions[0]?.value ?? null); }} options={scopeOptions}/>{scope !== 'OsUser' && <Select label={t('nativeExtensions.project')} value={project ?? ''} onChange={setProject} options={projectOptions}/ >}{scope === 'OsUser' && <span className="text-xs text-ink-secondary">{t('nativeExtensions.sharedOs')}</span>}{kind === 'skills' && <Input label={t('nativeExtensions.search')} value={query} onChange={event => setQuery(event.target.value)}/>}</div>
    {messages[viewKey] && <p role="status" aria-live="polite" className="text-sm text-ink-secondary">{messages[viewKey]}</p>}
    {state?.value?.incomplete && <p className="text-sm">{t('nativeExtensions.incomplete')}</p>}
    {claudeRules && <section aria-label={t('nativeExtensions.nameRules')} className="mb-3 flex min-w-0 flex-col gap-3 border-b border-border-subtle pb-3 text-sm">
      <h3 className="m-0 text-base font-semibold">{t('nativeExtensions.nameRules')}</h3>
      <p className="m-0">{t('nativeExtensions.ruleNameImpact')}</p>
      <p className="m-0 break-all">{t('nativeExtensions.settingsLayer')}: {t(`nativeExtensions.scope.${scope}`)} · {scope === 'OsUser' ? t('nativeExtensions.ruleOsScope') : `${projects.find(item => item.id === project)?.path ?? '—'} · ${t('nativeExtensions.ruleProjectScope')}`}</p>
      <p className="m-0 text-ink-secondary">{t('nativeExtensions.ruleLayerCoverage')}</p>
      {rulesState?.loading && <p role="status" className="m-0">{t('nativeExtensions.loading')}</p>}
      {rulesState?.error && <p role="alert" className="m-0">{t(`nativeExtensions.error.${rulesState.error}`)}</p>}
      {rulesState?.stale && !rulesState.loading && <p role="status" className="m-0">{t('nativeExtensions.refreshRequired')}</p>}
      {rulesState?.value && (rulesState.value.rules.length ? <ul className="m-0 list-none p-0">{rulesState.value.rules.map(rule => <li key={rule.name} className="flex flex-wrap items-center gap-2 border-t border-border-subtle py-2">
        <strong className="break-all">{rule.name}</strong><span>{t(`nativeExtensions.ruleState.${rule.state}`)}</span>
        <Button variant="secondary" disabled={editDisabled} onClick={() => saveRuleDraft(rule.name, rule.state)}>{t('nativeExtensions.editRule')}</Button>
        <Button variant="secondary" disabled={editDisabled} onClick={() => previewRule('inherit', rule.name, false)}>{t('nativeExtensions.restoreInherit')}</Button>
      </li>)}</ul> : <p className="m-0">{t('nativeExtensions.noNameRules')}</p>)}
      <Input label={t('nativeExtensions.ruleName')} value={ruleDraft.name} disabled={busy} error={errors[ruleDraftKey]?.name} onChange={event => saveRuleDraft(event.target.value)}/>
      <Select label={t('nativeExtensions.ruleState')} value={ruleState} disabled={busy} onChange={value => saveRuleDraft(ruleDraft.name, value as NativeSkillRuleIntentState)} options={ruleStates.map(value => ({ value, label: t(`nativeExtensions.ruleState.${value}`) }))}/>
      <dl className="m-0 grid gap-2">{ruleStates.map(value => <div key={value}><dt className="font-semibold">{t(`nativeExtensions.ruleState.${value}`)}</dt><dd className="m-0">{t(`nativeExtensions.ruleMeaning.${value}`)}</dd></div>)}</dl>
      <p className="m-0 text-ink-secondary">{t('nativeExtensions.rulePluginsExcluded')}</p>
      <p className="m-0 text-ink-secondary">{t('nativeExtensions.rulePrecedenceWarning')}</p>
      <p className="m-0 text-ink-secondary">{t('nativeExtensions.ruleHookWarning')}</p>
      <Button disabled={editDisabled} onClick={() => previewRule()}>{t('nativeExtensions.previewRule')}</Button>
    </section>}
    {resourceUnsupported && <p className="text-sm">{t('nativeExtensions.localRulesOnly')}</p>}
    <div className="mb-3 flex flex-wrap gap-2"><Button variant="secondary" disabled={busy || resourceUnsupported} onClick={() => changeView(!creating)}>{t(creating ? 'nativeExtensions.closeEditor' : kind === 'skills' ? 'nativeExtensions.installSkill' : 'nativeExtensions.createServer')}</Button></div>
    {creating && <section className="mb-3 flex flex-col gap-3 border-b border-border-subtle pb-3" aria-label={t(kind === 'skills' ? 'nativeExtensions.installSkill' : 'nativeExtensions.createServer')}>
      {kind === 'skills' ? <>{input('slug')}{input('source')}<p className="m-0 text-xs text-ink-secondary">{t('nativeExtensions.scriptsNotRun')}</p><Button disabled={resourceEditDisabled} onClick={() => previewSkill('install')}>{t('nativeExtensions.previewInstall')}</Button></> : <>{input('name')}<Select label={t('nativeExtensions.transport')} value={draft.transport} disabled={busy} onChange={value => saveDraft({ ...draft, transport: value as NativeDraft['transport'] })} options={(target === 'Codex' ? ['Stdio', 'Http'] : ['Stdio', 'Http', 'Sse']).map(value => ({ value, label: value }))}/>{serverInputs(draft.transport)}<p className="m-0 text-xs text-ink-secondary">{t('nativeExtensions.fieldKeepHint')}</p><Button disabled={editDisabled} onClick={previewServer}>{t('nativeExtensions.previewChange')}</Button></>}
    </section>}
    <ResourceLayout className="min-h-0 flex-1" listLabel={t('nativeExtensions.instances')} detailLabel={t('nativeExtensions.details')} list={<ResourceList state={resourceView(readState, !ids.length && !readonlyEntries.length)} label={t('nativeExtensions.instances')} loadingLabel={t('nativeExtensions.loading')} emptyContent={<p>{t('nativeExtensions.empty')}</p>} retry={<Button variant="secondary" onClick={() => void refresh()}>{t('nativeExtensions.refresh')}</Button>}>
      {kind === 'skills' ? skills.map(item => <button key={item.instance.id} type="button" aria-pressed={!creating && item.instance.id === chosen} onClick={() => changeView(false, item.instance.id)} className="block w-full border-b border-border-subtle p-3 text-left focus-visible:outline focus-visible:outline-2"><strong>{item.metadata.name}</strong><span className="ml-2"><StatusBadge status={item.owner === 'Loose' ? 'success' : 'neutral'} label={t(`nativeExtensions.owner.${item.owner}`)}/></span></button>) : servers.map(item => <button key={item.native_key} type="button" aria-pressed={!creating && item.native_key === chosen} onClick={() => changeView(false, item.native_key)} className="block w-full border-b border-border-subtle p-3 text-left focus-visible:outline focus-visible:outline-2"><strong>{item.native_key}</strong><span className="ml-2">{item.transport}</span><span className="ml-2"><StatusBadge status={item.owner === 'Loose' ? 'success' : 'neutral'} label={t(`nativeExtensions.owner.${item.owner}`)}/></span></button>)}
      {readonlyEntries.length > 0 && <section aria-label={t('nativeExtensions.readonlyLinks')} className="p-3 text-sm">
        <h3 className="m-0 text-sm font-semibold">{t('nativeExtensions.readonlyLinks')}</h3>
        <p>{t('nativeExtensions.linkReadOnly')}</p>
        <ul className="m-0 list-none p-0">{readonlyEntries.map(entry => <li key={entry.id} className="space-y-2 border-t border-border-subtle py-3">
          <strong className="break-all">{entry.slug}</strong><StatusBadge status="neutral" label={t('nativeExtensions.owner.Symlink')}/>
          <dl className="m-0 space-y-1">
            <div><dt>{t('nativeExtensions.linkPath')}</dt><dd className="m-0 break-all">{entry.link_path}</dd></div>
            <div><dt>{t('nativeExtensions.linkTarget')}</dt><dd className="m-0 break-all">{entry.target}</dd></div>
            {entry.status === 'within_authorized_root' && entry.resolved_target !== null && <div><dt>{t('nativeExtensions.resolvedTarget')}</dt><dd className="m-0 break-all">{entry.resolved_target}</dd></div>}
          </dl>
          <p className="m-0">{t(`nativeExtensions.linkStatus.${entry.status}`)}</p>
        </li>)}</ul>
      </section>}
    </ResourceList>} detail={<ResourceDetail state={{ status: 'ready', empty: creating || (!skill && !server), refresh: { status: 'current' } }} label={t('nativeExtensions.details')} emptyContent={<p>{t('nativeExtensions.select')}</p>}>
      {!creating && (kind === 'skills' ? skill && <div className="flex min-w-0 flex-col gap-3">
        <h3 className="m-0 text-base">{skill.metadata.name}</h3><p className="m-0 text-sm">{skill.metadata.description}</p>
        <dl className="grid grid-cols-2 gap-2 text-sm"><dt>{t('nativeExtensions.revision')}</dt><dd className="m-0 break-all">{skill.instance.revision}</dd><dt>{t('nativeExtensions.sourceHash')}</dt><dd className="m-0 break-all">{skill.source?.source_hash ?? t('nativeExtensions.unknown')}</dd><dt>{t('nativeExtensions.resources')}</dt><dd className="m-0">{skill.resources.length}</dd></dl>
        <p className="m-0 text-sm text-ink-secondary">{t('nativeExtensions.dependenciesNotChecked')}: {skill.metadata.dependencies.tools.map(item => item.program).join(', ') || t('nativeExtensions.unknown')}</p>
        {skill.can_update && input('source')}
        <div className="flex flex-wrap gap-2">
          <Button disabled={resourceEditDisabled || skill.owner !== 'Loose' || !skill.can_update} onClick={() => previewSkill('update')}>{t('nativeExtensions.update')}</Button>
          <Button variant="danger" disabled={resourceEditDisabled || skill.owner !== 'Loose'} onClick={() => previewSkill('remove')}>{t('nativeExtensions.remove')}</Button>
          {target === 'Codex' && [true, false].map(visible => <Button key={String(visible)} variant="secondary" disabled={resourceEditDisabled || skill.owner !== 'Loose'} onClick={() => void preview(() => api.previewVisibility({ ...selection, intent: { kind: 'codex_path', instance: skill.instance, visible } }), t(visible ? 'nativeExtensions.enableSkill' : 'nativeExtensions.disableSkill'), false)}>{t(visible ? 'nativeExtensions.enableSkill' : 'nativeExtensions.disableSkill')}</Button>)}
          {claudeRules && skill.owner !== 'Plugin' && <Button variant="secondary" disabled={editDisabled} onClick={() => saveRuleDraft(skill.metadata.name)}>{t('nativeExtensions.useRuleName')}</Button>}
        </div>
        {claudeRules && <p className="m-0 text-sm text-ink-secondary">{t('nativeExtensions.ruleResourceHint')}</p>}
        <p className="m-0 text-xs text-ink-secondary">{t('nativeExtensions.sessionUnavailable')}</p>
      </div> : server && <div className="flex min-w-0 flex-col gap-3"><h3 className="m-0 break-all text-base">{server.native_key}</h3><p className="m-0 text-sm">{server.transport} · {t(`nativeExtensions.owner.${server.owner}`)}</p><p className="m-0 text-sm">{t('nativeExtensions.auth')}: {server.authentication.authenticated === null ? t('nativeExtensions.unknown') : t(server.authentication.authenticated ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</p><p className="m-0 text-sm">{t('nativeExtensions.connection')}: {server.authentication.connected === null ? t('nativeExtensions.unknown') : t(server.authentication.connected ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</p><p className="m-0 text-xs text-ink-secondary">{t('nativeExtensions.sessionUnavailable')}</p>{server.owner === 'Loose' && serverInputs(server.transport)}<p className="m-0 text-xs text-ink-secondary">{t('nativeExtensions.fieldKeepHint')}</p>{server.unknown_fields_present && <p className="m-0 text-xs text-ink-secondary">{t('nativeExtensions.unknownFieldsKept')}</p>}<div className="flex flex-wrap gap-2"><Button disabled={editDisabled || server.owner !== 'Loose' || !['Stdio', 'Http', 'Sse'].includes(server.transport) || !Object.values(draft.modes).some(value => value !== 'keep')} onClick={previewServer}>{t('nativeExtensions.previewChange')}</Button><Button variant="danger" disabled={editDisabled || server.owner !== 'Loose' || !['Stdio', 'Http', 'Sse'].includes(server.transport)} onClick={() => void previewMcp({ kind: 'remove', name: server.native_key })}>{t('nativeExtensions.remove')}</Button>{target === 'Codex' && server.enabled !== null && <Button variant="secondary" disabled={editDisabled || server.owner !== 'Loose'} onClick={() => void previewMcp({ kind: 'set_enabled', name: server.native_key, enabled: !server.enabled })}>{t('nativeExtensions.visibility')}</Button>}</div></div>)}
    </ResourceDetail>}/>
    {operation && <PreviewDetails operation={operation} observationsId={observationsId} showObservations={() => { observationsRef.current?.focus(); observationsRef.current?.scrollIntoView?.({ block: 'nearest' }); }}/ >}
    {operation?.mode === 'preview' && <footer className="mt-3 flex flex-wrap gap-2 border-t border-border-subtle pt-3"><Button disabled={busy || !validSelection} onClick={() => void resolveOperation('commit')}>{t('nativeExtensions.confirm')}</Button><Button variant="secondary" disabled={busy || !validSelection} onClick={() => void resolveOperation('cancel')}>{t('nativeExtensions.cancel')}</Button></footer>}
    {operation?.mode === 'inspect' && <footer className="mt-3 flex flex-wrap gap-2 border-t border-border-subtle pt-3"><Button variant="secondary" disabled={busy || !validSelection} onClick={() => void resolveOperation('inspect')}>{t('nativeExtensions.verify')}</Button></footer>}
    <div id={observationsId} ref={observationsRef} tabIndex={-1} className="min-w-0 focus-visible:outline focus-visible:outline-2">
      <NativeSessionPanel target={target} domain={kind} configSelection={selection} initialProjectId={selection.project_id ?? undefined} initialSessionId={target === initialSelection?.target && (selection.project_id === initialSelection.project_id || scope === 'OsUser') ? initialSessionId : undefined} />
    </div>
  </section>;
}
