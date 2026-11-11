import { forwardRef, useCallback, useEffect, useId, useImperativeHandle, useRef, useState } from 'react';
import { Button, Input, Modal, ResourceDetail, ResourceList, Select, StatusBadge, Tabs } from '@dgos/ui';
import { BookOpen, ChevronLeft, Plus, RefreshCw, Search, Server } from 'lucide-react';
import { t } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { nativeSelectionIdentity, useNativeExtensions } from '../../stores/nativeExtensions';
import { nativeLibraryKey, useNativeSkillLibrary } from '../../stores/nativeSkillLibrary';
import { nativeProbeRecordKey, useNativeMcpCatalog } from '../../stores/nativeMcpCatalog';
import type { NativeDraft, NativeDraftField, NativeField, NativeMcpChange, NativeMcpInstance, NativePatchMode, NativePending, NativeResult, NativeSelection, NativeServerPatch, NativeSkillRuleIntentState, PreviewNativeSkill } from '../../types/nativeExtensions';
import { resourceView, type ResourceReadState } from '../../resourceState';
import { useSurfaceCloseGuard, useSurfaceId, useSurfaceStatus } from '../../workspace/SurfaceContext';
import { confirmDiscard } from '../../workspace/closeCoordinator';
import NativeSessionPanel from './NativeSessionPanel';
import { NativeSkillInspector, NativeSkillLibrary, NativeSkillVersionRecovery } from './NativeSkillLibrary';
import NativeMcpMarket from './NativeMcpMarket';
import NativeMcpConfiguration from './NativeMcpConfiguration';
import NativeMcpProbe, { nativeDraftProbeInput, NativeMcpProbeRecovery } from './NativeMcpProbe';
import { NativeManagerLifecycle, type NativeManagerGuard } from './NativeManagerLifecycle';
import './native-manager.css';

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
      <dt>{t('nativeExtensions.actualTargetPath')}</dt><dd className="m-0 break-all">{base.target_path}</dd>
      {operation.selection.project_id && <><dt>{t('nativeExtensions.ruleProjectDirectory')}</dt><dd className="m-0 break-all">{operation.selection.task_id ? t('nativeExtensions.taskWorkspace') : projectPath ?? t('nativeExtensions.unknown')}</dd></>}
      <dt>{t('nativeExtensions.expires')}</dt><dd className="m-0">{new Date(base.expires_at * 1000).toLocaleString()}</dd>
      <dt>{t('nativeExtensions.operationId')}</dt><dd className="m-0 break-all">{receipt.operation_id}</dd>
      <dt>{t('nativeExtensions.operationState')}</dt><dd className="m-0">{t(`nativeExtensions.state.${receipt.state}`)}</dd>
      <dt>{t('nativeExtensions.pendingActivation')}</dt><dd className="m-0">{t(receipt.pending_activation ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</dd>
      <dt>{t('nativeExtensions.recoveryRetained')}</dt><dd className="m-0">{t(receipt.recovery_material_retained ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</dd>
    </dl>
    {operation.kind === 'mcp' && operation.draft && <section><h4>{t('nativeExtensions.requestedFieldChanges')}</h4><p>{t('nativeExtensions.safeFieldChangeNotice')}</p><dl>{operation.draft.name && <><dt>{t('nativeExtensions.transport')}</dt><dd>{operation.draft.transport}</dd></>}{Object.entries(operation.draft.modes).filter(([, mode]) => mode !== 'keep').map(([field, mode]) => <div key={field}><dt>{t(`nativeExtensions.${field}`)}</dt><dd>{t(`nativeExtensions.field.${mode}`)}</dd></div>)}</dl></section>}
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

export interface NativeManagerHandle { requestClose(): Promise<boolean> }
export interface NativeManagerProps { kind: 'skills' | 'mcp'; initialSelection?: NativeSelection; initialSessionId?: string; initialResourceId?: string; onClose?: () => void }
type ManagerTab = 'installed' | 'discover' | 'sources' | 'configured' | 'market' | 'manual';

const NativeManager = forwardRef<NativeManagerHandle, NativeManagerProps>(function NativeManager({ kind, initialSelection, initialSessionId, initialResourceId, onClose }, ref) {
  const account = useDevelopment(s => s.epoch), projects = useDevelopment(s => s.projects);
  const api = useNativeExtensions();
  const libraryDrafts = useNativeSkillLibrary(state => state.drafts);
  const versionRecords = useNativeSkillLibrary(state => state.versions);
  const probeRecords = useNativeMcpCatalog(state => state.probes);
  const surfaceId = useSurfaceId(), instanceId = useId();
  const observationsRef = useRef<HTMLDivElement>(null);
  const managerRef = useRef<HTMLElement>(null);
  const previewRef = useRef<HTMLElement>(null), operationRef = useRef<HTMLElement>(null);
  const observationsId = useId();
  const ownerKey = JSON.stringify([account, surfaceId ?? instanceId, kind]);
  const ownerPrefix = `[${JSON.stringify(ownerKey)},`;
  const ownedOperations = useRef(new Set<string>());
  const ownerPrefixes = useRef(new Set<string>());
  ownerPrefixes.current.add(ownerPrefix);
  const managedSelections = useRef(new Set<string>());
  const managedLibraryKeys = useRef(new Set<string>());
  const childGuards = useRef(new Set<NativeManagerGuard>());
  const registerGuard = useCallback((guard: NativeManagerGuard) => { childGuards.current.add(guard); return () => { childGuards.current.delete(guard); }; }, []);
  const closing = useRef<Promise<boolean> | null>(null);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const [target, setTarget] = useState<'Codex' | 'Claude'>(initialSelection?.target ?? 'Codex');
  const [scope, setScope] = useState<'OsUser' | 'ProjectShared' | 'ProjectLocal'>(initialSelection?.target === 'Codex' && initialSelection.scope === 'ProjectLocal' ? 'OsUser' : initialSelection?.scope ?? 'OsUser');
  const [project, setProject] = useState<string | null>(initialSelection?.scope === 'OsUser' ? null : initialSelection?.project_id ?? null), [query, setQuery] = useState('');
  const [taskId, setTaskId] = useState<string | undefined>(initialSelection?.task_id ?? undefined);
  const [tabs, setTabs] = useState<Record<string, ManagerTab>>({});
  const [narrowDetails, setNarrowDetails] = useState(Boolean(initialResourceId));
  const [previewOpen, setPreviewOpen] = useState(false);
  const [detailTab, setDetailTab] = useState('configuration');
  const [observationsOpen, setObservationsOpen] = useState(false);
  const [rulesOpen, setRulesOpen] = useState(false);
  const rulesRef = useRef<HTMLDetailsElement>(null);
  const [views, setViews] = useState<Record<string, { selected: string | null }>>({});
  const [messages, setMessages] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, Partial<Record<NativeDraftField, string>>>>({});
  const generation = useRef(0);
  const entryKey = initialSelection ? JSON.stringify([initialSelection.target, initialSelection.scope, initialSelection.project_id, initialSelection.task_id ?? null]) : '';
  useEffect(() => {
    if (!initialSelection) return;
    const nextScope = initialSelection.target === 'Codex' && initialSelection.scope === 'ProjectLocal' ? 'OsUser' : initialSelection.scope;
    setTarget(initialSelection.target); setScope(nextScope);
    setProject(nextScope === 'OsUser' ? null : initialSelection.project_id);
    setTaskId(initialSelection.task_id ?? undefined);
  }, [entryKey]);
  const selection: NativeSelection = { target, scope, project_id: scope === 'OsUser' ? null : project, ...(scope !== 'OsUser' && taskId ? { task_id: taskId } : {}) };
  const selectionKey = JSON.stringify([account, target, scope, selection.project_id, selection.task_id ?? null]);
  managedSelections.current.add(selectionKey);
  if (kind === 'skills') managedLibraryKeys.current.add(nativeLibraryKey(selection));
  const rootIdentity = useDevelopment(() => nativeSelectionIdentity(selection));
  const workspaceKey = JSON.stringify([selectionKey, rootIdentity]);
  const currentWorkspace = useRef(workspaceKey); currentWorkspace.current = workspaceKey;
  const viewKey = JSON.stringify([workspaceKey, kind]), view = views[viewKey];
  const tab = tabs[viewKey] ?? (kind === 'skills' ? 'installed' : 'configured');
  const changeTab = (next: ManagerTab) => { void transition(() => { setTabs(current => ({ ...current, [viewKey]: next })); setNarrowDetails(false); }); };
  const creating = kind === 'mcp' && tab === 'manual';
  const busy = Boolean(api.busy[selectionKey]);
  const claudeRules = kind === 'skills' && target === 'Claude';
  const resourceUnsupported = kind === 'skills' && scope === 'ProjectLocal';
  const skillState = resourceUnsupported ? undefined : api.skills[`${selectionKey}:`], mcpState = api.mcp[selectionKey];
  const rulesState = api.rules[selectionKey];
  const state = kind === 'skills' ? skillState : mcpState;
  const filter = query.trim().toLocaleLowerCase();
  const skills = kind === 'skills' ? (skillState?.value?.instances ?? []).filter(item => [item.metadata.name, item.metadata.description, item.slug, item.source?.commit ?? '', t(`nativeExtensions.owner.${item.owner}`)].some(value => value.toLocaleLowerCase().includes(filter))) : [];
  const readonlyEntries = kind === 'skills' ? (skillState?.value?.readonly_entries ?? []).filter(item => item.slug.toLocaleLowerCase().includes(filter)) : [];
  const servers = kind === 'mcp' ? (mcpState?.value?.instances ?? []).filter(item => [item.native_key, item.transport, t(`nativeExtensions.owner.${item.owner}`)].some(value => value.toLocaleLowerCase().includes(filter))) : [];
  const serverSnapshots = useRef(new Map<string, NativeMcpInstance>());
  for (const item of mcpState?.value?.instances ?? []) serverSnapshots.current.set(JSON.stringify([workspaceKey, item.native_key]), item);
  const ids = kind === 'skills' ? skills.map(item => item.instance.id) : servers.map(item => item.native_key);
  const requested = view?.selected ?? (entryKey === JSON.stringify([target, scope, selection.project_id, selection.task_id ?? null]) ? initialResourceId : undefined);
  const chosen = requested ? ids.includes(requested) || kind === 'mcp' && api.drafts[JSON.stringify([ownerKey, workspaceKey, requested])] ? requested : null : ids[0] ?? null;
  const skill = skills.find(item => item.instance.id === chosen), server = servers.find(item => item.native_key === chosen) ?? (kind === 'mcp' && chosen ? serverSnapshots.current.get(JSON.stringify([workspaceKey, chosen])) : undefined);
  const draftKey = JSON.stringify([ownerKey, workspaceKey, creating ? 'create' : chosen]);
  const draft: NativeDraft = api.drafts[draftKey] ?? emptyDraft, operation = api.pending[selectionKey];
  const ruleDraftKey = JSON.stringify([ownerKey, workspaceKey, 'claude_name_rule']);
  const ruleDraft = api.drafts[ruleDraftKey] ?? emptyDraft;
  const ruleState = ruleDraft.visibilityState ?? 'on';
  const blocked = busy || Boolean(operation && operation.mode !== 'result');
  useEffect(() => {
    if (!previewOpen || !operation) return;
    const frame = requestAnimationFrame(() => { previewRef.current?.focus(); previewRef.current?.scrollIntoView?.({ block: 'nearest' }); });
    return () => cancelAnimationFrame(frame);
  }, [previewOpen, operation?.receipt.operation_id, workspaceKey]);
  const validSelection = scope === 'OsUser' || projects.some(item => item.id === project && !item.archived);
  const ownsDraft = (id: string) => [...ownerPrefixes.current].some(prefix => id.startsWith(prefix));
  const dirty = Object.keys(api.drafts).some(ownsDraft) || [...managedLibraryKeys.current].some(key => libraryDrafts[key]);
  const running = ownedOperations.current.size > 0;
  const isBlocked = () => {
    const current = useNativeExtensions.getState();
    return ownedOperations.current.size > 0 || kind === 'skills' && Object.values(useNativeSkillLibrary.getState().versions).some(record => record.busy || !record.result.cleanup_confirmed) || [...managedSelections.current].some(id => current.busy[id] || current.pending[id] && current.pending[id].mode !== 'result');
  };
  const canClose = async () => {
    if (isBlocked()) { message(t('nativeExtensions.closeBlocked')); return false; }
    const drafts = Object.fromEntries(Object.entries(useNativeExtensions.getState().drafts).filter(([id]) => ownsDraft(id)));
    const sourceDrafts = Object.fromEntries(Object.entries(useNativeSkillLibrary.getState().drafts).filter(([key]) => managedLibraryKeys.current.has(key)));
    for (const guard of childGuards.current) if (!await guard('close')) return false;
    if ((Object.keys(drafts).length || Object.keys(sourceDrafts).length) && !await confirmDiscard()) return false;
    if (!mounted.current || useDevelopment.getState().epoch !== account || isBlocked()) return false;
    const current = Object.keys(useNativeExtensions.getState().drafts).filter(ownsDraft);
    if (current.length !== Object.keys(drafts).length) return false;
    const currentSources = useNativeSkillLibrary.getState().drafts;
    if (Object.entries(sourceDrafts).some(([key, draft]) => currentSources[key] !== draft) || Object.keys(currentSources).filter(key => managedLibraryKeys.current.has(key)).length !== Object.keys(sourceDrafts).length) return false;
    if (!api.discardDrafts(drafts)) return false;
    const remaining = { ...currentSources }; for (const key of Object.keys(sourceDrafts)) delete remaining[key];
    useNativeSkillLibrary.setState({ drafts: remaining }); return true;
  };
  const collapsePreview = () => { setPreviewOpen(false); requestAnimationFrame(() => operationRef.current?.querySelector('button')?.focus()); };
  const requestClose = () => {
    if (operation && previewOpen && !['Committed', 'Cancelled', 'Conflict'].includes(operation.receipt.state)) { collapsePreview(); return Promise.resolve(false); }
    if (closing.current) return closing.current;
    const flight = canClose().then(allowed => { if (allowed) onClose?.(); return allowed; }).finally(() => { if (closing.current === flight) closing.current = null; });
    closing.current = flight; return flight;
  };
  useImperativeHandle(ref, () => ({ requestClose }));
  useSurfaceStatus({ dirty, running, runningLabel: running ? t('nativeExtensions.working') : undefined });
  useSurfaceCloseGuard(canClose);
  useEffect(() => {
    ownedOperations.current.clear(); managedSelections.current.clear(); managedSelections.current.add(selectionKey);
    managedLibraryKeys.current.clear(); if (kind === 'skills') managedLibraryKeys.current.add(nativeLibraryKey(selection));
    ownerPrefixes.current.clear(); ownerPrefixes.current.add(ownerPrefix);
    serverSnapshots.current.clear();
    setViews({}); setTabs({}); setMessages({}); setErrors({}); setPreviewOpen(false); setNarrowDetails(Boolean(initialResourceId)); setObservationsOpen(false); setRulesOpen(false); setQuery('');
  }, [account]);
  useEffect(() => { if (!useDevelopment.getState().loaded) void useDevelopment.getState().load(); }, []);
  const message = (text: string) => { if (useDevelopment.getState().epoch === account) setMessages(current => ({ ...current, [viewKey]: text })); };
  useEffect(() => {
    if (!validSelection) return;
    const seq = ++generation.current;
    const pending = claudeRules ? Promise.all([
      api.discoverSkillRules(selection),
      ...(resourceUnsupported ? [] : [api.discoverSkills({ ...selection, query: '' })]),
    ]) : Promise.all([kind === 'skills' ? api.discoverSkills({ ...selection, query: '' }) : api.discoverMcp(selection)]);
    void pending.then(results => { const failure = results.find(result => !result.ok); if (seq === generation.current && failure && !failure.ok) message(t(`nativeExtensions.error.${failure.code}`)); });
    return () => { generation.current++; };
  }, [workspaceKey, kind, validSelection]);
  const refresh = () => claudeRules ? Promise.all([api.discoverSkillRules(selection), ...(resourceUnsupported ? [] : [api.discoverSkills({ ...selection, query: '' })])]) : kind === 'skills' ? api.discoverSkills({ ...selection, query: '' }) : api.discoverMcp(selection);
  const changeView = (creating: boolean, selected: string | null = chosen) => { void transition(() => { setViews(current => ({ ...current, [viewKey]: { selected } })); setDetailTab('configuration'); setNarrowDetails(!creating); }); };
  const saveDraft = (next: NativeDraft) => { api.setDraft(draftKey, next); if (!creating && chosen) setViews(current => ({ ...current, [viewKey]: { selected: chosen } })); setErrors(current => ({ ...current, [draftKey]: {} })); };
  const validate = (fields: Partial<Record<NativeDraftField, string>>) => {
    setErrors(current => ({ ...current, [draftKey]: fields }));
    if (Object.keys(fields).length) {
      message(t('nativeExtensions.correctFields'));
      requestAnimationFrame(() => {
        const invalid = Array.from(managerRef.current?.querySelectorAll<HTMLElement>('[aria-invalid="true"]') ?? []).find(element => !element.closest('[hidden]'));
        for (let parent = invalid?.parentElement; parent; parent = parent.parentElement) if (parent instanceof HTMLDetailsElement) parent.open = true;
        invalid?.focus(); invalid?.scrollIntoView?.({ block: 'nearest' });
      });
      return false;
    }
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
      setPreviewOpen(true);
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
        if (operation.kind === 'skills' && !nameRule) setTabs(current => ({ ...current, [JSON.stringify([workspaceKey, 'skills'])]: 'installed' }));
        if (!nameRule) {
          const read = await (operation.kind === 'skills' ? api.discoverSkills({ ...operation.selection, query: '' }) : api.discoverMcp(operation.selection));
          if (!read.ok) message(t('nativeExtensions.writtenReadPending'));
          if (read.ok && currentWorkspace.current === workspaceKey) {
            const slug = 'slug' in operation.preview ? operation.preview.slug : null;
            const installed = 'readonly_entries' in read.value ? read.value.instances.find(item => item.slug === slug)?.instance.id : operation.draft?.name.trim();
            if (installed) { setQuery(''); setViews(current => ({ ...current, [viewKey]: { selected: installed } })); setNarrowDetails(true); }
            if (operation.kind === 'mcp') setTabs(current => ({ ...current, [viewKey]: 'configured' }));
          }
        }
      }
    } finally { ownedOperations.current.delete(selectionKey); if (useDevelopment.getState().epoch === account) api.end(selectionKey); }
  };
  const previewSkillInput = (input: PreviewNativeSkill) => {
    if (JSON.stringify([input.target, input.scope, input.project_id, input.task_id ?? null]) !== JSON.stringify([selection.target, selection.scope, selection.project_id, selection.task_id ?? null])) return;
    void preview(() => api.previewSkill(input), t(`nativeExtensions.skillAction.${input.action}`), false);
  };
  const removeSkill = () => { if (skill) previewSkillInput({ ...selection, action: 'remove', slug: skill.slug, expected: skill.instance, source: null }); };
  const saveRuleDraft = (name: string, state: NativeSkillRuleIntentState = ruleState) => {
    api.setDraft(ruleDraftKey, { ...ruleDraft, name, visibilityState: state });
    setErrors(current => ({ ...current, [ruleDraftKey]: {} })); setRulesOpen(true); setNarrowDetails(true);
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
      forwarded_env: field('forwarded_env', refs), ...(target === 'Codex' ? { cwd: field('cwd', draft.cwd.trim()) } : {}),
    } : { url: field('url', draft.url.trim()), header_env: field('header_env', headers ?? []), ...(target === 'Codex' ? { bearer_env: field('bearer_env', draft.bearer_env.trim()) } : {}) };
    if (creating) fields.transport = { kind: 'set', value: draft.transport };
    void previewMcp({ kind: 'upsert', name: creating ? draft.name.trim() : server!.native_key, fields });
  };
  const input = (name: NativeDraftField, patch = false) => {
    const props = { label: t(`nativeExtensions.${name}`), value: draft[name], disabled: busy || (patch && mode(name as keyof NativeServerPatch) === 'remove'), error: errors[draftKey]?.[name], onChange: (event: { target: { value: string } }) => saveDraft({ ...draft, [name]: event.target.value, modes: patch ? { ...draft.modes, [name]: 'set' } : draft.modes }) };
    return <div key={name} className="native-manager-field">
      {patch && !creating && <Select
        label={`${t(`nativeExtensions.${name}`)}: ${t('nativeExtensions.fieldAction')}`}
        value={mode(name as keyof NativeServerPatch)} disabled={busy}
        onChange={value => saveDraft({ ...draft, modes: { ...draft.modes, [name]: value as NativePatchMode } })}
        options={['keep', 'set', ...(name === 'header_env' && target === 'Claude' ? [] : ['remove'])].map(value => ({ value, label: t(`nativeExtensions.field.${value}`) }))}
      />}
      {name === 'args' || name === 'forwarded_env' || name === 'header_env' ? <Input multiline {...props}/> : <Input {...props}/>}
    </div>;
  };
  const serverInputs = (transport: string) => transport === 'Stdio' ? <>{input('program', true)}{input('args', true)}<details className="native-manager-disclosure"><summary>{t('nativeExtensions.optionalConfiguration')}</summary><div className="native-manager-form-fields">{target === 'Codex' && input('cwd', true)}{input('forwarded_env', true)}</div></details></> : transport === 'Http' || transport === 'Sse' ? <>{input('url', true)}<details className="native-manager-disclosure"><summary>{t('nativeExtensions.optionalConfiguration')}</summary><div className="native-manager-form-fields">{target === 'Codex' && input('bearer_env', true)}{input('header_env', true)}</div></details></> : null;
  const readState: ResourceReadState = !state ? { status: 'idle' } : state.loading && !state.value ? { status: 'loading' } : state.error && !state.value ? { status: 'error', error: t(`nativeExtensions.error.${state.error}`) } : { status: 'ready', refresh: state.stale ? { status: 'error', error: t('nativeExtensions.stale') } : { status: 'current' } };
  const projectOptions = projects.filter(item => !item.archived).map(item => ({ value: item.id, label: item.name }));
  const scopeOptions = ['OsUser', 'ProjectShared', ...(target === 'Claude' ? ['ProjectLocal'] : [])].map(value => ({ value, label: t(`nativeExtensions.scope.${value}`) }));
  const editDisabled = blocked || !validSelection;
  const resourceEditDisabled = editDisabled || resourceUnsupported;
  const transition = async (apply: () => void, reason: 'context' | 'view' = 'view') => {
    for (const guard of childGuards.current) if (!await guard(reason)) return;
    if (mounted.current && useDevelopment.getState().epoch === account && currentWorkspace.current === workspaceKey) apply();
  };
  const configureMarketDraft = async (next: NativeDraft): Promise<boolean> => {
    if (editDisabled) return false;
    const id = JSON.stringify([ownerKey, workspaceKey, 'create']);
    const previous = useNativeExtensions.getState().drafts[id];
    if (previous && !await confirmDiscard()) return false;
    if (!mounted.current || useDevelopment.getState().epoch !== account || currentWorkspace.current !== workspaceKey || useNativeExtensions.getState().drafts[id] !== previous || isBlocked()) return false;
    api.setDraft(id, next);
    setViews(current => ({ ...current, [viewKey]: { selected: null } }));
    setTabs(current => ({ ...current, [viewKey]: 'manual' }));
    setNarrowDetails(false); return true;
  };
  const tabsForKind: ManagerTab[] = kind === 'skills' ? ['installed', 'discover', 'sources'] : ['configured', 'market', 'manual'];
  const showInstalled = tab === 'installed' || tab === 'configured';
  const probeDraft = api.drafts[JSON.stringify([ownerKey, workspaceKey, 'create'])] ?? emptyDraft;
  const displayedProbeKey = kind === 'mcp' ? nativeProbeRecordKey(showInstalled && server ? { selection, source: { kind: 'existing', server_name: server.native_key } } : creating ? nativeDraftProbeInput(selection, probeDraft) : null, probeRecords) : null;
  const startServer = () => { changeView(true, null); changeTab('manual'); };
  const showObservations = () => {
    if (kind === 'mcp') {
      const reveal = () => { setTabs(current => ({ ...current, [viewKey]: 'configured' })); setDetailTab('session'); setNarrowDetails(true); requestAnimationFrame(() => observationsRef.current?.focus()); };
      if (showInstalled) reveal(); else void transition(reveal);
    } else {
      setObservationsOpen(true);
      requestAnimationFrame(() => { observationsRef.current?.focus(); observationsRef.current?.scrollIntoView?.({ block: 'nearest' }); });
    }
  };
  const operationActions = operation?.mode === 'preview' ? <><Button disabled={busy || !validSelection} onClick={() => void resolveOperation('commit')}>{t('nativeExtensions.confirm')}</Button><Button variant="secondary" disabled={busy || !validSelection} onClick={() => void resolveOperation('cancel')}>{t('nativeExtensions.cancel')}</Button></> : operation?.mode === 'inspect' ? <Button variant="secondary" disabled={busy || !validSelection} onClick={() => void resolveOperation('inspect')}>{t('nativeExtensions.verify')}</Button> : null;
  const sessionPanel = <div id={observationsId} ref={observationsRef} tabIndex={-1}><NativeSessionPanel target={target} domain={kind} configSelection={selection} initialProjectId={selection.project_id ?? undefined} initialSessionId={target === initialSelection?.target && (selection.project_id === initialSelection.project_id || scope === 'OsUser') && (scope === 'OsUser' || (selection.task_id ?? null) === (initialSelection.task_id ?? null)) ? initialSessionId : undefined}/></div>;
  return <NativeManagerLifecycle.Provider value={registerGuard}><section ref={managerRef} className="native-manager" aria-label={t(kind === 'skills' ? 'nativeExtensions.skills' : 'nativeExtensions.mcp')}>
    <Tabs className="native-manager-tabs" value={tab} onValueChange={value => changeTab(value as ManagerTab)}>
    <header className="native-manager-header">
      <span className="native-resource-icon" aria-hidden="true">{kind === 'skills' ? <BookOpen /> : <Server />}</span>
      <div className="native-manager-title"><h2>{t(kind === 'skills' ? 'nativeExtensions.skills' : 'nativeExtensions.mcp')}</h2><p>{t('nativeExtensions.managerSubtitle')}</p></div>
      <Tabs.List aria-label={t('nativeExtensions.managementTabs')}>{tabsForKind.map(value => <Tabs.Trigger key={value} value={value} disabled={!validSelection}>{t(`nativeExtensions.tab.${value}`)}</Tabs.Trigger>)}</Tabs.List>
    </header>
    <div className="native-manager-context">
      <Select label={t('nativeExtensions.target')} value={target} onChange={value => void transition(() => { setTarget(value as typeof target); setQuery(''); setNarrowDetails(false); if (value === 'Codex' && scope === 'ProjectLocal') setScope('OsUser'); }, 'context')} options={[{ value: 'Codex', label: 'Codex' }, { value: 'Claude', label: 'Claude Code' }]}/>
      <Select label={t('nativeExtensions.scope')} value={scope} onChange={value => void transition(() => { const next = value as typeof scope; setScope(next); setQuery(''); setNarrowDetails(false); if (next !== 'OsUser' && !project) { setProject(projectOptions[0]?.value ?? null); setTaskId(undefined); } }, 'context')} options={scopeOptions}/>
      {scope !== 'OsUser' && <Select label={t('nativeExtensions.project')} value={project ?? ''} onChange={value => void transition(() => { setProject(value); setTaskId(undefined); setQuery(''); setNarrowDetails(false); }, 'context')} options={projectOptions}/>}
      <div className="native-manager-context-hint"><span>{scope === 'OsUser' ? t('nativeExtensions.sharedOs') : taskId ? t('nativeExtensions.taskWorkspace') : t('nativeExtensions.projectScopeHint')}</span>{scope !== 'OsUser' && !taskId && <span className="native-manager-path" title={projects.find(item => item.id === project)?.path}>{projects.find(item => item.id === project)?.path}</span>}</div>
    </div>
    {!validSelection && <p role="alert" className="native-manager-message">{t('nativeExtensions.error.project_not_found')}</p>}
    {messages[viewKey] && <p role="status" aria-live="polite" className="native-manager-message">{messages[viewKey]}</p>}
    {operation && <section ref={operationRef} className="native-manager-operation" aria-label={t('nativeExtensions.operationStatus')}><span>{operation.summary} · {t(`nativeExtensions.state.${operation.receipt.state}`)}</span><Button variant="secondary" onClick={() => previewOpen ? collapsePreview() : setPreviewOpen(true)}>{t(previewOpen ? 'nativeExtensions.hidePreview' : 'nativeExtensions.reviewPreview')}</Button>{!previewOpen && operationActions}</section>}
      <Tabs.Content forceMount value={kind === 'skills' ? 'installed' : 'configured'} hidden={!showInstalled} className="native-manager-installed">
        <div className="native-manager-list-toolbar"><div className="native-manager-search"><Search aria-hidden="true" /><Input id={instanceId} type="search" aria-label={t('nativeExtensions.search')} placeholder={t('nativeExtensions.search')} value={query} onChange={event => setQuery(event.target.value)}/></div><Button variant="secondary" disabled={busy || !validSelection} onClick={() => void refresh()}><RefreshCw aria-hidden="true" />{t('nativeExtensions.refresh')}</Button><Button disabled={resourceEditDisabled} onClick={() => kind === 'skills' ? changeTab('discover') : startServer()}><Plus aria-hidden="true" />{t(kind === 'skills' ? 'nativeExtensions.installSkill' : 'nativeExtensions.createServer')}</Button></div>
        {state?.loading && state.value && <p role="status" className="native-manager-message">{t('nativeExtensions.loading')}</p>}
        {state?.value?.incomplete && <p role="status" className="native-manager-message">{t('nativeExtensions.incomplete')}</p>}
        {state?.error && state.value && <p role="alert" className="native-manager-message">{t(`nativeExtensions.error.${state.error}`)} {t('nativeExtensions.stale')}</p>}
        {resourceUnsupported && <p className="native-manager-message">{t('nativeExtensions.localRulesOnly')}</p>}
        <div className="native-manager-split" data-details={narrowDetails}>
          <div className="native-manager-list" aria-label={t('nativeExtensions.instances')}>
            {!resourceUnsupported && <ResourceList state={resourceView(readState, !ids.length && !readonlyEntries.length && !state?.value?.incomplete && !state?.error && !state?.stale)} label={t('nativeExtensions.instances')} loadingLabel={t('nativeExtensions.loading')} emptyContent={<div className="native-manager-empty"><p>{t('nativeExtensions.empty')}</p>{!filter && <Button variant="secondary" disabled={resourceEditDisabled} onClick={() => kind === 'skills' ? changeTab('discover') : changeTab('market')}>{t(kind === 'skills' ? 'nativeExtensions.tab.discover' : 'nativeExtensions.browseMarket')}</Button>}</div>} retry={<Button variant="secondary" onClick={() => void refresh()}>{t('nativeExtensions.refresh')}</Button>}>
              {kind === 'skills' ? skills.map(item => <button key={item.instance.id} type="button" aria-pressed={item.instance.id === chosen} onClick={() => changeView(false, item.instance.id)} className="native-manager-row">
                <span className="native-resource-icon" aria-hidden="true"><BookOpen /></span><span className="native-manager-row-body"><span className="native-manager-row-heading"><strong>{item.metadata.name}</strong>{item.owner !== 'Loose' && <StatusBadge status="neutral" label={t(`nativeExtensions.owner.${item.owner}`)}/>}</span><span className="native-manager-row-description">{item.metadata.description}</span><span className="native-manager-row-meta">{t(`nativeExtensions.scope.${item.scope}`)} · {item.source?.origin ? `${item.source.origin.repository.owner}/${item.source.origin.repository.repo}` : item.source?.commit ? item.source.commit.slice(0, 12) : t('nativeExtensions.sourceUnavailable')}</span></span>
              </button>) : servers.map(item => <button key={item.native_key} type="button" aria-pressed={item.native_key === chosen} onClick={() => changeView(false, item.native_key)} className="native-manager-row">
                <span className="native-resource-icon" aria-hidden="true"><Server /></span><span className="native-manager-row-body"><span className="native-manager-row-heading"><strong>{item.native_key}</strong>{item.owner !== 'Loose' && <StatusBadge status="neutral" label={t(`nativeExtensions.owner.${item.owner}`)}/>}</span><span className="native-manager-row-description">{item.transport} · {t(`nativeExtensions.scope.${scope}`)}</span><span className="native-manager-row-meta">{t('nativeExtensions.configEnabled')}: {t(item.enabled === null ? 'nativeExtensions.notReported' : item.enabled ? 'nativeExtensions.yes' : 'nativeExtensions.no')} · {t('nativeExtensions.sessionNotRead')}</span></span>
              </button>)}
              {readonlyEntries.length > 0 && <section aria-label={t('nativeExtensions.readonlyLinks')} className="native-manager-linked"><h3>{t('nativeExtensions.readonlyLinks')}</h3><p>{t('nativeExtensions.linkReadOnly')}</p><ul>{readonlyEntries.map(entry => <li key={entry.id}><strong>{entry.slug}</strong><StatusBadge status="neutral" label={t('nativeExtensions.owner.Symlink')}/><dl><dt>{t('nativeExtensions.linkPath')}</dt><dd>{entry.link_path}</dd><dt>{t('nativeExtensions.linkTarget')}</dt><dd>{entry.target}</dd>{entry.status === 'within_authorized_root' && entry.resolved_target !== null && <><dt>{t('nativeExtensions.resolvedTarget')}</dt><dd>{entry.resolved_target}</dd></>}</dl><p>{t(`nativeExtensions.linkStatus.${entry.status}`)}</p></li>)}</ul></section>}
            </ResourceList>}
            {claudeRules && <Button variant="secondary" disabled={!validSelection} onClick={() => { setRulesOpen(true); setNarrowDetails(true); requestAnimationFrame(() => rulesRef.current?.querySelector('summary')?.focus()); }}>{t('nativeExtensions.nameRules')}</Button>}
          </div>
          <Tabs className="native-manager-detail" value={kind === 'skills' ? 'configuration' : detailTab} onValueChange={setDetailTab}>
            <div className="native-manager-back"><Button variant="ghost" onClick={() => { setNarrowDetails(false); requestAnimationFrame(() => document.getElementById(instanceId)?.focus()); }}><ChevronLeft aria-hidden="true" />{t('nativeExtensions.backToList')}</Button></div>
            {kind === 'mcp' && <Tabs.List aria-label={t('nativeExtensions.details')}><Tabs.Trigger value="configuration">{t('nativeExtensions.configuration')}</Tabs.Trigger><Tabs.Trigger value="session">{t('nativeExtensions.currentSession')}</Tabs.Trigger><Tabs.Trigger value="test" disabled={!server}>{t('nativeMcpCatalog.probe')}</Tabs.Trigger></Tabs.List>}
            <Tabs.Content value="configuration" forceMount hidden={kind === 'mcp' && detailTab !== 'configuration'}>
            <ResourceDetail state={{ status: 'ready', empty: !skill && !server, refresh: { status: 'current' } }} label={t('nativeExtensions.details')} emptyContent={<p>{t('nativeExtensions.select')}</p>}>
              {kind === 'skills' ? skill && <div className="native-manager-detail-content">
                <header><h3>{skill.metadata.name}</h3><p>{skill.metadata.description}</p></header>
                <NativeSkillInspector key={JSON.stringify([workspaceKey, skill.instance.id, skill.instance.revision])} selection={selection} skill={skill} disabled={resourceEditDisabled || !showInstalled} onPreview={previewSkillInput}/>
                <details className="native-manager-disclosure"><summary>{t('nativeExtensions.configurationRules')}</summary><p>{t('nativeExtensions.sessionUnavailable')}</p>{claudeRules ? <p>{t('nativeExtensions.ruleResourceHint')}</p> : skill.owner === 'Loose' && <div className="native-manager-actions">{[true, false].map(visible => <Button key={String(visible)} variant="secondary" disabled={resourceEditDisabled} onClick={() => void preview(() => api.previewVisibility({ ...selection, intent: { kind: 'codex_path', instance: skill.instance, visible } }), t(visible ? 'nativeExtensions.enableSkill' : 'nativeExtensions.disableSkill'), false)}>{t(visible ? 'nativeExtensions.enableSkill' : 'nativeExtensions.disableSkill')}</Button>)}</div>}</details>
                <div className="native-manager-detail-actions">{claudeRules && skill.owner !== 'Plugin' && <Button variant="secondary" disabled={editDisabled} onClick={() => saveRuleDraft(skill.metadata.name)}>{t('nativeExtensions.useRuleName')}</Button>}{skill.owner === 'Loose' ? <details className="native-manager-more"><summary>{t('nativeExtensions.moreActions')}</summary><Button variant="ghost" disabled={resourceEditDisabled} onClick={removeSkill}>{t('nativeExtensions.remove')}</Button></details> : <p>{t('nativeExtensions.readOnly')}</p>}</div>
              </div> : server && <div className="native-manager-detail-content">
                <header><h3>{server.native_key}</h3><p>{server.transport} · {t(`nativeExtensions.owner.${server.owner}`)} · {t(`nativeExtensions.scope.${scope}`)}</p></header>
                <dl><dt>{t('nativeExtensions.configEnabled')}</dt><dd>{t(server.enabled === null ? 'nativeExtensions.notReported' : server.enabled ? 'nativeExtensions.yes' : 'nativeExtensions.no')}</dd><dt>{t('nativeExtensions.auth')}</dt><dd>{t(server.has_auth_configuration ? 'nativeExtensions.authConfigured' : 'nativeExtensions.notReported')}</dd></dl><p>{t('nativeExtensions.sessionUnavailable')}</p>
                <NativeMcpConfiguration key={JSON.stringify([workspaceKey, server.native_key])} selection={selection} name={server.native_key} disabled={editDisabled || !showInstalled}/>
                {server.owner === 'Loose' ? <details className="native-manager-disclosure"><summary>{t('nativeExtensions.editConfiguration')}</summary><div className="native-manager-form-fields">{showInstalled && serverInputs(server.transport)}<p>{t('nativeExtensions.fieldKeepHint')}</p>{server.unknown_fields_present && <p>{t('nativeExtensions.unknownFieldsKept')}</p>}<footer className="native-manager-detail-actions"><Button disabled={editDisabled || !['Stdio', 'Http', 'Sse'].includes(server.transport) || !Object.values(draft.modes).some(value => value !== 'keep')} onClick={previewServer}>{t('nativeExtensions.previewChange')}</Button></footer></div></details> : <p>{t('nativeExtensions.readOnly')}</p>}
                {server.owner === 'Loose' && <div className="native-manager-detail-actions">{target === 'Codex' && server.enabled !== null && <Button variant="secondary" disabled={editDisabled} onClick={() => void previewMcp({ kind: 'set_enabled', name: server.native_key, enabled: !server.enabled })}>{t('nativeExtensions.visibility')}</Button>}<details className="native-manager-more"><summary>{t('nativeExtensions.moreActions')}</summary><Button variant="ghost" disabled={editDisabled || !['Stdio', 'Http', 'Sse'].includes(server.transport)} onClick={() => void previewMcp({ kind: 'remove', name: server.native_key })}>{t('nativeExtensions.remove')}</Button></details></div>}
              </div>}
            </ResourceDetail>
            {claudeRules && <details ref={rulesRef} className="native-manager-disclosure" open={rulesOpen || resourceUnsupported} onToggle={event => setRulesOpen(event.currentTarget.open)}><summary>{t('nativeExtensions.nameRules')}</summary><section aria-label={t('nativeExtensions.nameRules')} className="native-manager-rule-editor">
              <h3>{t('nativeExtensions.nameRules')}</h3><p>{t('nativeExtensions.ruleNameImpact')}</p><p>{t('nativeExtensions.settingsLayer')}: {t(`nativeExtensions.scope.${scope}`)} · {scope === 'OsUser' ? t('nativeExtensions.ruleOsScope') : t('nativeExtensions.ruleProjectScope')}</p><p>{t('nativeExtensions.ruleLayerCoverage')}</p>
              {rulesState?.loading && <p role="status">{t('nativeExtensions.loading')}</p>}{rulesState?.error && <p role="alert">{t(`nativeExtensions.error.${rulesState.error}`)}</p>}{rulesState?.stale && !rulesState.loading && <p role="status">{t('nativeExtensions.refreshRequired')}</p>}
              {rulesState?.value && (rulesState.value.rules.length ? <ul>{rulesState.value.rules.map(rule => <li key={rule.name}><strong>{rule.name}</strong> {t(`nativeExtensions.ruleState.${rule.state}`)}<div className="native-manager-actions"><Button variant="secondary" disabled={editDisabled} onClick={() => saveRuleDraft(rule.name, rule.state)}>{t('nativeExtensions.editRule')}</Button><Button variant="secondary" disabled={editDisabled} onClick={() => previewRule('inherit', rule.name, false)}>{t('nativeExtensions.restoreInherit')}</Button></div></li>)}</ul> : <p>{t('nativeExtensions.noNameRules')}</p>)}
              <Input label={t('nativeExtensions.ruleName')} value={ruleDraft.name} disabled={busy} error={errors[ruleDraftKey]?.name} onChange={event => saveRuleDraft(event.target.value)}/><Select label={t('nativeExtensions.ruleState')} value={ruleState} disabled={busy} onChange={value => saveRuleDraft(ruleDraft.name, value as NativeSkillRuleIntentState)} options={ruleStates.map(value => ({ value, label: t(`nativeExtensions.ruleState.${value}`) }))}/><dl>{ruleStates.map(value => <div key={value}><dt>{t(`nativeExtensions.ruleState.${value}`)}</dt><dd>{t(`nativeExtensions.ruleMeaning.${value}`)}</dd></div>)}</dl><p>{t('nativeExtensions.rulePluginsExcluded')}</p><p>{t('nativeExtensions.rulePrecedenceWarning')}</p><p>{t('nativeExtensions.ruleHookWarning')}</p><Button disabled={editDisabled} onClick={() => previewRule()}>{t('nativeExtensions.previewRule')}</Button>
            </section></details>}
            </Tabs.Content>
            {kind === 'mcp' && <><Tabs.Content value="session" forceMount hidden={detailTab !== 'session'}>{sessionPanel}</Tabs.Content><Tabs.Content value="test" forceMount hidden={detailTab !== 'test'}>{server && <NativeMcpProbe key={JSON.stringify([workspaceKey, server.native_key])} selection={selection} serverName={server.native_key} disabled={editDisabled || !showInstalled} active={showInstalled}/>}</Tabs.Content></>}
          </Tabs>
        </div>
      </Tabs.Content>
      {kind === 'skills' ? <>{(['discover', 'sources'] as const).map(value => <Tabs.Content forceMount hidden={tab !== value} key={value} value={value} className="native-manager-library"><NativeSkillLibrary selection={selection} tab={value} disabled={resourceEditDisabled || tab !== value} onPreview={previewSkillInput}/></Tabs.Content>)}</> : <>
        <Tabs.Content forceMount hidden={tab !== 'market'} value="market" className="native-manager-library"><NativeMcpMarket selection={selection} disabled={editDisabled || tab !== 'market'} onConfigure={configureMarketDraft}/></Tabs.Content>
        <Tabs.Content forceMount hidden={tab !== 'manual'} value="manual" className="native-manager-manual">{creating && <section aria-label={t('nativeExtensions.createServer')} className="native-manager-form">
          <header><h3>{t('nativeExtensions.createServer')}</h3><p>{t('nativeExtensions.manualNotice')}</p></header>{input('name')}<Select label={t('nativeExtensions.transport')} value={draft.transport} disabled={busy} onChange={value => saveDraft({ ...draft, transport: value as NativeDraft['transport'] })} options={(target === 'Codex' ? ['Stdio', 'Http'] : ['Stdio', 'Http', 'Sse']).map(value => ({ value, label: value }))}/>{serverInputs(draft.transport)}<p>{t('nativeExtensions.fieldKeepHint')}</p>
        </section>}<details className="native-manager-manual-test native-manager-disclosure"><summary>{t('nativeMcpCatalog.probe')}</summary><NativeMcpProbe key={workspaceKey} selection={selection} draft={probeDraft} disabled={editDisabled || tab !== 'manual'} active={creating}/></details><footer className="native-manager-form-actions"><Button disabled={editDisabled} onClick={previewServer}>{t('nativeExtensions.previewChange')}</Button><Button variant="secondary" onClick={() => changeTab('configured')}>{t('nativeExtensions.closeEditor')}</Button></footer></Tabs.Content>
      </>}
      <footer className="native-manager-footer"><span>{t('nativeExtensions.configurationNotice')}</span>{kind === 'mcp' && <Button variant="ghost" size="xs" onClick={showObservations}>{t('nativeExtensions.currentSession')}</Button>}</footer>
    </Tabs>
    {kind === 'skills' && Object.values(versionRecords).filter(record => !record.result.cleanup_confirmed && !(showInstalled && skill?.slug === record.skill.slug && JSON.stringify([selection.target, selection.scope, selection.project_id, selection.task_id ?? null]) === JSON.stringify([record.skill.selection.target, record.skill.selection.scope, record.skill.selection.project_id, record.skill.selection.task_id ?? null]))).map(record => <NativeSkillVersionRecovery key={record.plan.plan_id} record={record} />)}
    {kind === 'mcp' && Object.entries(probeRecords).filter(([key, record]) => key !== displayedProbeKey && ['running', 'unknown'].includes(record.snapshot.state)).map(([key, record]) => <NativeMcpProbeRecovery key={key} probeKey={key} record={record} />)}
    {kind === 'skills' && <details className="native-manager-session native-manager-disclosure" open={observationsOpen} onToggle={event => setObservationsOpen(event.currentTarget.open)}><summary>{t('nativeExtensions.currentSession')}</summary>{sessionPanel}</details>}
    <Modal open={Boolean(operation && previewOpen)} onOpenChange={open => { if (!open) collapsePreview(); }} title={t('nativeExtensions.previewLayer')} footer={<>{operationActions}<Button variant="ghost" onClick={collapsePreview}>{t('nativeExtensions.hidePreview')}</Button></>}>
      {operation && <section ref={previewRef} tabIndex={-1} className="native-manager-preview"><PreviewDetails operation={operation} observationsId={observationsId} showObservations={() => { setPreviewOpen(false); requestAnimationFrame(() => requestAnimationFrame(showObservations)); }}/></section>}
    </Modal>
  </section></NativeManagerLifecycle.Provider>;
});

export default NativeManager;
