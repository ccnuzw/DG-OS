import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { useProjectCapabilities } from '../../stores/developmentCapabilities';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { readDevelopmentWorkspaces, saveDevelopmentWorkspace, readDevelopmentGit, executeDevelopmentGit, reconcileDevelopmentGit, listDevelopmentHandoffs, prepareDevelopmentHandoff, acceptDevelopmentHandoff, type WorkspaceView, type GitReview, type GitOperation, type StoredHandoff, type GitAction } from '../../stores/developmentDelivery';
import { deliveryRequests, dispatchDeliveryRequest, forgetDeliveryRequests } from '../../stores/developmentDelivery';
import { confirmAction, errorText } from './shared';
import { developmentDraftKey, useDevelopmentDrafts, useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { GitHistoryPull } from './GitHistoryPull';
import { readDevelopmentProjectGit, type ProjectGitOverview } from '../../stores/developmentExplorer';
import './workbench-tree.css';
import { GitChangesTree } from './GitChangesTree';

interface Props {
  projectId: string; taskId: string | null; sessions: { id: string; engine: 'codex' | 'claude' }[];
  live: Record<string, string>; capabilityRevision?: number; workspacePath?: string;
  ensureTask?: () => Promise<string | null>;
  onHandoff?: (sessionId: string) => void;
  onWorkspaceChange?: (path: string) => void;
  mode?: 'delivery' | 'git';
  onOpenDiff?: (path: string, source?: 'index' | 'worktree') => void;
  refreshKey?: string | number;
}
export function DeliveryPanel(props: Props) {
  const epoch = useDevelopment(state => state.epoch);
  return <ScopedDeliveryPanel key={`${epoch}:${props.projectId}:${props.taskId}`} {...props} />;
}

function ScopedDeliveryPanel({ projectId, taskId: initialTaskId, ensureTask, sessions, live, onHandoff, onWorkspaceChange, mode = 'delivery', onOpenDiff, refreshKey }: Props) {
  const resolvedTask = useRef<string | null>(initialTaskId);
  const taskId = resolvedTask.current ?? '';
  const { t } = useLocale();
  const capabilities = useProjectCapabilities(projectId);
  const [reconcileId, setReconcileId] = useState('');
  const [workspaces, setWorkspaces] = useState<WorkspaceView | null>(null);
  const [git, setGit] = useState<ProjectGitOverview | null>(null);
  const [taskReview, setTaskReview] = useState<GitReview | null>(null);
  const [history, setHistory] = useState<StoredHandoff[]>([]);
  const [operation, setOperation] = useState<GitOperation | null>(null);
  const [busy, setBusy] = useState(false); const [error, setError] = useState<string | null>(null);
  const [path, setPath] = useState(''); const [branch, setBranch] = useState(''); const [base, setBase] = useState('');
  const [retainBranch, setRetainBranch] = useState('');
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [includeExisting, setIncludeExisting] = useState(false); const [fullIndex, setFullIndex] = useState(false);
  const [remote, setRemote] = useState(''); const [branchTarget, setBranchTarget] = useState('');
  const deliveryDraft = useScopedDevelopmentDraft('delivery', [projectId, taskId], { sourceId: '', remaining: '' });
  const { sourceId, remaining } = deliveryDraft.value;
  const handoffKey = developmentDraftKey('handoff', projectId, taskId);
  const handoffId = useDevelopmentDrafts(state => state.handoffs[handoffKey]);
  const handoff = history.find(item => item.id === handoffId) ?? null;
  function setHandoff(item: StoredHandoff | null) { useDevelopmentDrafts.setState(state => ({ handoffs: { ...state.handoffs, [handoffKey]: item?.id ?? '' } })); }
  const summaryDraft = useScopedDevelopmentDraft('handoff', [projectId, taskId, handoffId ?? ''], handoff?.summary ?? '');
  const summary = summaryDraft.value;
  const [targetRevision, setTargetRevision] = useState<string>(''); const [trustedTarget, setTrustedTarget] = useState(false);
  const mounted = useRef(true); const ticket = useRef(0);
  const busyRef = useRef(false); const pendingRefresh = useRef(false); const reviewPath = useRef<string | null>(null);
  busyRef.current = busy;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++ticket.current; }; }, []);
  useEffect(() => { setTrustedTarget(false); }, [workspaces?.selected_workspace_id]);
  function guard() { const epoch = useDevelopment.getState().epoch, generation = contextGeneration(); return () => mounted.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration(); }
  const refresh = useCallback(async (deferIfBusy = false) => {
    if (deferIfBusy && busyRef.current) { pendingRefresh.current = true; return; }
    const n = ++ticket.current, epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    setBusy(true); setError(null);
    const currentTask = resolvedTask.current;
    const [w, g, h] = await Promise.all([readDevelopmentWorkspaces({ project_id: projectId, task_id: currentTask }), readDevelopmentProjectGit(projectId, currentTask), mode === 'git' || !currentTask ? Promise.resolve({ ok: true as const, value: [] as StoredHandoff[] }) : listDevelopmentHandoffs(currentTask)]);
    if (!mounted.current || n !== ticket.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    if (w.ok) setWorkspaces(w.value);
    if (g.ok) {
      setGit(g.value);
      setTaskReview(current => current?.path === g.value.path && current.snapshot_token === g.value.snapshot_token ? current : null);
      const sameWorkspace = reviewPath.current === g.value.path;
      reviewPath.current = g.value.path;
      setSelectedPaths(rows => sameWorkspace ? rows.filter(p => g.value.files.some(file => file.path === p)) : []);
    }
    setHistory(h.ok ? h.value : []);
    if (w.ok) {
      const receipts = [...(w.value.operations ?? [])];
      const owner = useDevelopment.getState().projects.find(item => item.id === projectId)?.owner;
      try {
        if (owner) {
          forgetDeliveryRequests(owner, receipts);
          const known = new Set(receipts.map(item => item.id));
          receipts.push(...deliveryRequests(owner).filter(item => item.project_id === projectId && item.task_id === taskId && !known.has(item.id)).map(item => ({ ...item, status: 'unknown' as const, workspace_id: null, request_fingerprint: '', details: {} })));
        }
        setWorkspaces({ ...w.value, operations: receipts });
        setOperation(current => receipts.find(item => item.status === 'pending' || item.status === 'unknown') ?? receipts.find(item => item.id === current?.id) ?? null);
      } catch { setError('invalid_request'); }
    }
    setError(!w.ok ? w.code : !g.ok ? g.code : !h.ok ? h.code : null); setBusy(false);
    setFullIndex(false); setIncludeExisting(false);
  }, [projectId, taskId, mode]);
  useEffect(() => { void refresh(true); }, [refresh, refreshKey]);
  useEffect(() => {
    if (!busy && pendingRefresh.current) { pendingRefresh.current = false; void refresh(); }
  }, [busy, refresh]);
  const selected = workspaces?.workspaces.find(item => workspaces.selected_workspace_id ? item.id === workspaces.selected_workspace_id : item.kind === 'original');
  const commitDraft = useScopedDevelopmentDraft('commit', [projectId, initialTaskId ?? '', selected?.id ?? git?.path ?? ''], '');
  const message = commitDraft.value;
  const source = sessions.find(item => item.id === sourceId);
  const target = handoff?.target_engine ?? (source?.engine === 'codex' ? 'claude' : 'codex');
  const compatible = capabilities.value?.configuration.engine === target ? capabilities.value : null;
  const unresolved = operation?.status === 'unknown' || operation?.status === 'pending';
  async function save(action: 'create' | 'import' | 'select' | 'remove', extra: Record<string, string | null> = {}) {
    const valid = guard();
    const removalDraft = action === 'remove' ? useDevelopmentDrafts.getState().entries[developmentDraftKey('commit', projectId, taskId, extra.workspace_id ?? '')] : null;
    if (action === 'remove' && !await confirmAction(t('delivery.remove'), t('delivery.removeWarning') + (removalDraft ? '\n' + t('dev.discardDraft.commit') : ''), t('delivery.remove'))) return;
    if (!valid() || action === 'remove' && useDevelopmentDrafts.getState().entries[developmentDraftKey('commit', projectId, taskId, extra.workspace_id ?? '')] !== removalDraft) return;
    setBusy(true); setError(null);
    const requestId = crypto.randomUUID();
    setOperation({ id: requestId, project_id: projectId, task_id: taskId, kind: `workspace_${action}`, status: 'pending', workspace_id: null, request_fingerprint: '', details: {} });
    const result = await dispatchDeliveryRequest({ id: requestId, project_id: projectId, task_id: taskId, kind: `workspace_${action}` }, () => saveDevelopmentWorkspace({ request_id: requestId, task_id: taskId, action, ...extra }));
    if (!valid()) return;
    if (!result.ok) { await refresh(); if (valid()) setError(result.code); return; }
    if (removalDraft && result.value.operation?.status === 'succeeded' && useDevelopmentDrafts.getState().entries[developmentDraftKey('commit', projectId, taskId, extra.workspace_id ?? '')] === removalDraft) {
      useDevelopmentDrafts.setState(state => { const entries = { ...state.entries }; delete entries[developmentDraftKey('commit', projectId, taskId, extra.workspace_id ?? '')]; return { entries }; });
    }
    setWorkspaces(result.value);
    if (result.value.operation) setOperation(result.value.operation as GitOperation);
    const current = result.value.workspaces.find(item => item.id === result.value.selected_workspace_id);
    if (current) onWorkspaceChange?.(current.path);
    await refresh();
  }
  async function act(requested: GitAction | { kind: 'discard_selected'; paths: string[] } | null) {
    if (!git || busy || unresolved) return;
    const valid = guard(), expected = git; setBusy(true); setError(null);
    try {
      const writeTask = resolvedTask.current ?? await ensureTask?.() ?? null;
      if (!writeTask || !valid()) return;
      const [review, workspace] = await Promise.all([readDevelopmentGit(writeTask), readDevelopmentWorkspaces({ project_id: projectId, task_id: writeTask })]);
      if (!valid()) return;
      if (!review.ok || !workspace.ok) { setError(!review.ok ? review.code : !workspace.ok ? workspace.code : 'unknown'); return; }
      if (review.value.path !== expected.path) { setError('git_review_stale'); return; }
      resolvedTask.current = writeTask;
      setGit(review.value); setTaskReview(review.value); setWorkspaces(workspace.value);
      if (review.value.snapshot_token !== expected.snapshot_token) { setFullIndex(false); setIncludeExisting(false); setError('git_review_stale'); return; }
      const owner = useDevelopment.getState().projects.find(item => item.id === projectId)?.owner;
      let remembered = false;
      try {
        if (owner) { forgetDeliveryRequests(owner, workspace.value.operations); remembered = deliveryRequests(owner).some(item => item.project_id === projectId && item.task_id === writeTask); }
      } catch { remembered = true; }
      if (remembered || workspace.value.operations.some(item => item.status === 'pending' || item.status === 'unknown')) { await refresh(); return; }
      if (!requested) return;
      let action: GitAction;
      if (requested.kind === 'discard_selected') {
        if (!review.value.baseline_version) { setError('baseline_missing'); return; }
        action = { kind: 'discard', baseline_version: review.value.baseline_version, files: requested.paths.map(path => ({ path, expected_hash: review.value.file_versions[path]?.split(':', 1)[0] ?? '' })), confirmed: true };
      } else action = requested;
      if (action.kind === 'commit' && review.value.initial_staged_paths.length && !action.confirm_preexisting_staged) { setError('git_preexisting_staged_confirmation_required'); return; }
      const requestId = crypto.randomUUID();
      setOperation({ id: requestId, project_id: projectId, task_id: writeTask, kind: `git_${action.kind}`, status: 'pending', workspace_id: null, request_fingerprint: '', details: {} });
      const result = await dispatchDeliveryRequest({ id: requestId, project_id: projectId, task_id: writeTask, kind: `git_${action.kind}` }, () => executeDevelopmentGit({ id: requestId, task_id: writeTask, review_token: review.value.review_token, action }));
      if (action.kind === 'commit' && result.ok && result.value.status === 'succeeded') commitDraft.clear(action.message);
      if (!valid()) return;
      if (!result.ok) { await refresh(); if (valid()) setError(result.code); return; }
      setOperation(result.value); await refresh();
      if (valid() && result.value.details.error) setError(result.value.details.error);
    } finally { if (valid()) setBusy(false); }
  }
  async function reconcile() {
    if (!operation) return; const valid = guard();
    const push = operation.kind === 'git_push';
    const trusted = !push || await confirmAction(t('delivery.reconcile'), t('delivery.pushTrust'), t('delivery.reconcile'));
    if (!trusted || !valid()) return;
    setBusy(true); setError(null);
    const result = await reconcileDevelopmentGit({ id: operation.id, trust_directory: push, trust_network: push });
    await useDevelopment.getState().load();
    if (!valid()) return;
    if (!result.ok) { setError(result.code); setBusy(false); return; }
    setOperation(result.value); await refresh();
  }
  async function prepare() {
    if (!source) return; const valid = guard(); setBusy(true); setError(null);
    const result = await prepareDevelopmentHandoff({ task_id: taskId, source_session_id: source.id, target_engine: source.engine === 'codex' ? 'claude' : 'codex', remaining });
    if (!valid()) return;
    if (result.ok) { setHandoff(result.value); setTrustedTarget(false); setTargetRevision(''); setHistory(rows => [result.value, ...rows]); } else setError(result.code);
    setBusy(false);
  }
  async function accept() {
    const provider = capabilities.value;
    if (!handoff || !selected || !provider || provider.configuration.engine !== target || !trustedTarget || targetRevision !== String(provider.revision)) return;
    const valid = guard(); setBusy(true); setError(null);
    const result = await acceptDevelopmentHandoff({ handoff_id: handoff.id, summary, confirmed: true, project_revision: provider.revision, provider: provider.configuration.provider, options: { workspace: selected.path, resume_id: null, executable: null, model: null, trust_workspace: true } });
    if (!valid()) return;
    if (!result.ok) { setError(result.code); setBusy(false); await useDevelopment.getState().load(); return; }
    summaryDraft.clear(summary);
    deliveryDraft.clear();
    setHistory(rows => rows.map(item => item.id === result.value.handoff.id ? result.value.handoff : item));
    // The host has already opened the fresh runtime. The parent attaches/selects it and preserves the reviewed draft.
    onHandoff?.(result.value.session.id);
    setBusy(false);
  }
  return <div className="dev-review-stack" data-testid="dev-delivery">
    <div className="dev-review-toolbar"><h3 className="m-0 text-sm">{t(mode === 'git' ? 'workbenchGit.title' : 'delivery.title')}</h3><Button size="xs" variant="secondary" disabled={busy} onClick={() => void refresh()}>{t('review.refresh')}</Button></div>
    {busy ? <p role="status" className="text-xs">{t('review.loading')}</p> : null}{error ? <p role="alert" className="text-xs text-status-error">{errorText(error)}</p> : null}
    {mode !== 'git' && taskId ? <>
    <section className="dev-panel" aria-label={t('delivery.workspaces')}><h4 className="m-0 text-xs font-semibold">{t('delivery.workspaces')}</h4><p className="m-0 break-all text-xs">{selected?.path ?? t('delivery.unknown')}</p>
      <p className="dev-evidence-warning">{t('delivery.cleanCopyWarning')}</p>
      <Input label={t('delivery.directory')} value={path} onChange={e => setPath(e.target.value)} />
      <Input label={t('delivery.branch')} value={branch} onChange={e => setBranch(e.target.value)} /><Input label={t('delivery.baseCommit')} value={base} onChange={e => setBase(e.target.value)} />
      <Input label={t('delivery.retainBranch')} value={retainBranch} onChange={e => setRetainBranch(e.target.value)} />
      <div className="flex flex-wrap gap-2"><Button size="xs" disabled={busy || unresolved || !path || !branch || !base} onClick={() => void save('create', { path, branch, base_commit: base })}>{t('delivery.create')}</Button><Button size="xs" variant="secondary" disabled={busy || unresolved || !path} onClick={() => void save('import', { path })}>{t('delivery.import')}</Button></div>
      {workspaces?.workspaces.map(item => <div key={item.id} className="flex flex-wrap items-center gap-2 text-xs"><span className="min-w-0 flex-1 break-all">{item.path} · {t(`delivery.kind.${item.kind}`)}</span><Button size="xs" variant="secondary" disabled={busy || unresolved || item.id === selected?.id} aria-pressed={item.id === selected?.id} onClick={() => void save('select', { workspace_id: item.id })}>{t('delivery.select')}</Button>{item.kind === 'managed' ? <Button size="xs" variant="secondary" disabled={busy || unresolved || !retainBranch} onClick={() => void save('remove', { workspace_id: item.id, retain_branch: retainBranch })}>{t('delivery.remove')}</Button> : null}</div>)}
      {workspaces?.operation ? <details className="dev-technical-details"><summary>{t('agent.details')}</summary><pre className="dev-diff" tabIndex={0}>{JSON.stringify(workspaces.operation, null, 2)}</pre></details> : null}
    </section>
    </> : null}
    <section className="dev-panel" aria-label={t('delivery.git')}><h4 className="m-0 text-xs font-semibold">{t('delivery.git')}</h4>
      {!git ? <p className="text-xs">{t('delivery.unknown')}</p> : git.kind !== 'git' ? <p className="text-xs">{t('delivery.plainDirectory')}</p> : <>
        <p className="m-0 break-all text-xs">{git.branch ?? t('delivery.detached')}</p><details className="dev-technical-details"><summary>{t('layout.head')}</summary><code>{git.head ?? t('delivery.unknown')}</code></details>
        <p className="m-0 break-all text-xs">{git.path}</p>
        <p className="m-0 text-xs text-ink-secondary">{t('workbenchGit.workspaceScope')}</p>
        {!taskId ? <><p className="text-xs text-ink-secondary">{t('workbenchGit.projectScope')}</p>{ensureTask ? <Button size="xs" variant="secondary" disabled={busy || unresolved} onClick={() => void act(null)}>{t('workbenchGit.enableWrites')}</Button> : null}</> : null}
        {taskReview ? <p className="dev-evidence-warning">{t(taskReview.baseline_version ? 'workbenchGit.taskBaseline' : 'workbenchGit.noTaskBaseline')}</p> : null}
        {taskReview?.initial_staged_paths.length ? <fieldset className="dev-runtime-fields"><legend>{t('workbenchGit.existingStaged')}</legend>{taskReview.initial_staged_paths.map(p => <p key={p} className="m-0 break-all text-xs">{p}</p>)}<label className="dev-checkbox"><input type="checkbox" checked={includeExisting} onChange={e => setIncludeExisting(e.target.checked)} />{t('delivery.includeExisting')}</label></fieldset> : null}
        <GitChangesTree review={git} selected={selectedPaths} onSelect={(path, checked) => setSelectedPaths(rows => checked ? [...new Set([...rows, path])] : rows.filter(item => item !== path))} onOpenDiff={onOpenDiff} />
        {git.conflicts.length ? <section className="dev-review-stack"><p role="alert" className="dev-evidence-warning">{t('delivery.conflicts')}</p>{git.conflicts.map(entry => <p className="m-0 break-all text-xs" key={`${entry.path}:${entry.stage}`}>{entry.path} · {t('delivery.conflictStage', { stage: entry.stage })} · {entry.oid}</p>)}<p className="text-xs">{t('delivery.editConflict')}</p></section> : null}
        <div className="flex flex-wrap gap-2"><Button size="xs" disabled={busy || unresolved || !selectedPaths.length} onClick={() => void act({ kind: 'stage', paths: selectedPaths })}>{t('delivery.stage')}</Button><Button size="xs" variant="secondary" disabled={busy || unresolved || !selectedPaths.length} onClick={() => void act({ kind: 'unstage', paths: selectedPaths })}>{t('delivery.unstage')}</Button></div>
        <div className="dev-danger-actions"><Button size="xs" variant="danger" disabled={busy || unresolved || !selectedPaths.length || (!taskId && !ensureTask)} onClick={async () => { const valid = guard(), paths = [...selectedPaths]; if (await confirmAction(t('delivery.discard'), t('delivery.discardWarning') + '\n' + paths.join('\n'), t('delivery.discardSelected')) && valid()) void act({ kind: 'discard_selected', paths }); }}>{t('delivery.discard')}</Button></div>
        <div className="dev-git-commit-area">
        <Input label={t('delivery.commitMessage')} value={message} onChange={e => commitDraft.setValue(e.target.value)} /><label className="dev-checkbox"><input type="checkbox" checked={fullIndex} onChange={e => setFullIndex(e.target.checked)} />{t('delivery.confirmAllIndex')}</label><Button size="xs" disabled={busy || unresolved || !message.trim() || !fullIndex || (!git.files.some(file => file.index.trim() !== '' && file.index !== '?') && !git.merge_heads.length) || ((taskReview?.initial_staged_paths.length ?? 0) > 0 && !includeExisting)} onClick={() => void act({ kind: 'commit', message, confirm_full_index: fullIndex, confirm_preexisting_staged: includeExisting })}>{t('delivery.commit')}</Button>
        {commitDraft.dirty ? <Button size="xs" variant="secondary" disabled={busy} onClick={() => void commitDraft.discard()}>{t('dev.discardDraft')}</Button> : null}
        </div>
        <GitHistoryPull key={`${taskId}:${git.path}`} projectId={projectId} taskId={resolvedTask.current} review={git} busy={busy} blocked={unresolved} act={act} refreshKey={refreshKey} />
        <details className="dev-technical-details"><summary>{t('workbenchGit.repositoryActions')}</summary>
        <Input label={t('delivery.remote')} value={remote} onChange={e => setRemote(e.target.value)} /><Button size="xs" disabled={busy || unresolved || !remote} onClick={async () => { const valid = guard(); if (await confirmAction(t('delivery.push'), t('delivery.pushTrust') + '\n' + git.path + '\n' + remote + ' / ' + git.branch, t('delivery.push')) && valid()) void act({ kind: 'push', remote, trust_directory: true, trust_network: true }); }}>{t('delivery.push')}</Button>
        <Input label={t('delivery.branch')} value={branchTarget} onChange={e => setBranchTarget(e.target.value)} /><div className="flex gap-2"><Button size="xs" variant="secondary" disabled={busy || unresolved || !branchTarget} onClick={() => void act({ kind: 'switch', branch: branchTarget })}>{t('delivery.switch')}</Button><Button size="xs" variant="secondary" disabled={busy || unresolved || !branchTarget} onClick={() => void act({ kind: 'merge', branch: branchTarget })}>{t('delivery.merge')}</Button></div>
        </details>
      </>}
      {operation ? <div className="dev-review-stack"><p role="status" className={`dev-feedback dev-feedback-${operation.status === 'failed' ? 'error' : operation.status === 'succeeded' ? 'success' : 'info'}`}>{t(`delivery.operation.${operation.status}`)}</p>{unresolved ? <><p className="dev-evidence-warning">{t('delivery.unknownOperation')}</p><Button size="xs" disabled={busy} onClick={() => void reconcile()}>{t('delivery.reconcile')}</Button></> : null}<details className="dev-technical-details"><summary>{t('agent.details')}</summary><code>{operation.id}</code><pre className="dev-diff" tabIndex={0}>{JSON.stringify(operation.details, null, 2)}</pre></details></div> : null}
      <details className="dev-technical-details"><summary>{t('delivery.reconcile')}</summary>
      <fieldset className="dev-runtime-fields"><legend>{t('delivery.reconcile')}</legend>{workspaces?.operations?.filter(item => item.status === 'pending' || item.status === 'unknown').map(item => <Button key={item.id} size="xs" variant="secondary" onClick={() => setOperation(item)}>{item.id} · {t(`delivery.operation.${item.status}`)}</Button>)}<Input label={t('delivery.operationId')} value={reconcileId} onChange={e => setReconcileId(e.target.value)} /><Button size="xs" disabled={busy || !reconcileId} onClick={() => setOperation({ id: reconcileId, project_id: projectId, kind: 'git_push', status: 'unknown', task_id: taskId, workspace_id: null, request_fingerprint: '', details: {} })}>{t('delivery.inspectOperation')}</Button></fieldset>
      </details>
    </section>
    {mode !== 'git' && taskId ? <>
    <section className="dev-panel" aria-label={t('delivery.handoffs')}><h4 className="m-0 text-xs font-semibold">{t('delivery.handoffs')}</h4><p className="text-xs">{t('delivery.handoffHint')}</p>
      <label className="dev-field">{t('delivery.sourceSession')}<select value={sourceId} onChange={event => { deliveryDraft.setValue({ ...deliveryDraft.value, sourceId: event.target.value }); setHandoff(null); setTrustedTarget(false); }}><option value="">{t('delivery.chooseSource')}</option>{sessions.map(s => <option key={s.id} value={s.id}>{s.engine} · {s.id.slice(0, 8)}</option>)}</select></label>
      {source ? <p className="text-xs">{t('delivery.targetEngine')}: {target}</p> : null}
      {sourceId && live[sourceId] ? <><p role="status" className="dev-evidence-warning">{t('delivery.closeSource')}</p><Button size="xs" disabled={busy} onClick={async () => { const valid = guard(); setBusy(true); const ok = await useAgentRunner.getState().action('close_agent_session', { sessionId: sourceId }); if (valid()) { if (!ok) setError(useAgentRunner.getState().error ?? 'unknown'); setBusy(false); } }}>{t('delivery.closeSourceAction')}</Button></> : null}
      <Input label={t('delivery.remaining')} value={remaining} onChange={e => deliveryDraft.setValue({ ...deliveryDraft.value, remaining: e.target.value })} /><Button size="xs" disabled={busy || !sourceId || Boolean(live[sourceId])} onClick={() => void prepare()}>{t('delivery.prepareHandoff')}</Button>
      {handoff ? <><label className="dev-field">{t('delivery.summary')}<textarea className="dev-editor type-reading" value={summary} onChange={e => summaryDraft.setValue(e.target.value)} /></label><label className="dev-field">{t('delivery.capabilityRevision')}<select value={targetRevision} onChange={e => setTargetRevision(e.target.value)}><option value="">{t('delivery.chooseCapability')}</option>{compatible ? <option value={compatible.revision}>{t('capabilities.projectRevision', { revision: compatible.revision })}</option> : null}</select></label><p className="m-0 break-all text-xs">{selected?.path ?? t('delivery.unknown')}</p><label className="dev-checkbox"><input type="checkbox" checked={trustedTarget} onChange={e => setTrustedTarget(e.target.checked)} />{t('delivery.trustWorkspace')}</label><Button size="xs" disabled={busy || !selected || !trustedTarget || !compatible || targetRevision !== String(compatible.revision) || Boolean(handoff.target_session_id)} onClick={() => void accept()}>{t('delivery.acceptTrust')}</Button><Button size="xs" variant="secondary" disabled={busy || !summary.trim() || !useAgentRunner.getState().selected} onClick={() => { const runner = useAgentRunner.getState(); if (runner.session && runner.session.id === runner.selected) void runner.action('send_agent_message', { input: { session_id: runner.session.id, generation: runner.session.generation, request_id: crypto.randomUUID(), text: summary } }); }}>{t('delivery.sendSummary')}</Button></> : null}
      {summaryDraft.dirty ? <Button size="xs" variant="secondary" disabled={busy} onClick={() => void summaryDraft.discard()}>{t('dev.discardDraft')}</Button> : null}
      {history.map(item => <details key={item.id} className="dev-run"><summary className="text-xs">{item.target_engine} · {item.id.slice(0, 8)} · {item.target_session_id ? t('delivery.accepted') : t('delivery.awaitingReview')}</summary><pre className="dev-diff type-reading" tabIndex={0}>{item.summary}</pre><p className="break-all text-xs">{item.fingerprint}</p>{!item.target_session_id ? <Button size="xs" variant="secondary" onClick={() => { setHandoff(item); setTargetRevision(''); setTrustedTarget(false); }}>{t('delivery.useDraft')}</Button> : null}</details>)}
    </section>
    </> : null}
  </div>;
}
