import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { readDevelopmentWorkspaces, saveDevelopmentWorkspace, reconcileDevelopmentGit, listDevelopmentHandoffs, type WorkspaceView, type GitOperation, type StoredHandoff } from '../../stores/developmentDelivery';
import { deliveryRequests, dispatchDeliveryRequest, forgetDeliveryRequests } from '../../stores/developmentDelivery';
import { confirmAction, errorText } from './shared';
import { developmentDraftKey, useDevelopmentDrafts, useScopedDevelopmentDraft } from '../../stores/developmentDrafts';

interface Props {
  projectId: string; taskId: string | null; sessions: { id: string; engine: 'codex' | 'claude' }[];
  onContinueModel?: (sourceSessionId: string, preparedHandoffId?: string) => void;
  onOpenGit?: () => void;
  onWorkspaceChange?: (path: string) => void;
  refreshKey?: string | number;
}
export function DeliveryPanel(props: Props) {
  const epoch = useDevelopment(state => state.epoch);
  return <ScopedDeliveryPanel key={`${epoch}:${props.projectId}:${props.taskId}`} {...props} />;
}

function ScopedDeliveryPanel({ projectId, taskId: initialTaskId, sessions, onContinueModel, onOpenGit, onWorkspaceChange, refreshKey }: Props) {
  const taskId = initialTaskId ?? '';
  const { t } = useLocale();
  const [reconcileId, setReconcileId] = useState('');
  const [workspaces, setWorkspaces] = useState<WorkspaceView | null>(null);
  const [history, setHistory] = useState<StoredHandoff[]>([]);
  const [operation, setOperation] = useState<GitOperation | null>(null);
  const [busy, setBusy] = useState(false); const [error, setError] = useState<string | null>(null);
  const [path, setPath] = useState(''); const [branch, setBranch] = useState(''); const [base, setBase] = useState('');
  const [retainBranch, setRetainBranch] = useState('');
  const deliveryDraft = useScopedDevelopmentDraft('delivery', [projectId, taskId], { sourceId: '', remaining: '' });
  const { sourceId, remaining } = deliveryDraft.value;
  const handoffKey = developmentDraftKey('handoff', projectId, taskId);
  const handoffId = useDevelopmentDrafts(state => state.handoffs[handoffKey]);
  const handoff = history.find(item => item.id === handoffId) ?? null;
  function setHandoff(item: StoredHandoff | null) { useDevelopmentDrafts.setState(state => ({ handoffs: { ...state.handoffs, [handoffKey]: item?.id ?? '' } })); }
  const summaryDraft = useScopedDevelopmentDraft('handoff', [projectId, taskId, handoffId ?? ''], handoff?.summary ?? '');
  const summary = summaryDraft.value;
  const mounted = useRef(true); const ticket = useRef(0);
  const busyRef = useRef(false); const pendingRefresh = useRef(false);
  busyRef.current = busy;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++ticket.current; }; }, []);
  function guard() { const epoch = useDevelopment.getState().epoch, generation = contextGeneration(); return () => mounted.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration(); }
  const refresh = useCallback(async (deferIfBusy = false) => {
    if (deferIfBusy && busyRef.current) { pendingRefresh.current = true; return; }
    const n = ++ticket.current, epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    setBusy(true); setError(null);
    const [w, h] = await Promise.all([readDevelopmentWorkspaces({ project_id: projectId, task_id: initialTaskId }), taskId ? listDevelopmentHandoffs(taskId) : Promise.resolve({ ok: true as const, value: [] as StoredHandoff[] })]);
    if (!mounted.current || n !== ticket.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    if (w.ok) setWorkspaces(w.value);
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
    setError(!w.ok ? w.code : !h.ok ? h.code : null); setBusy(false);
  }, [projectId, taskId, initialTaskId]);
  useEffect(() => { void refresh(true); }, [refresh, refreshKey]);
  useEffect(() => {
    if (!busy && pendingRefresh.current) { pendingRefresh.current = false; void refresh(); }
  }, [busy, refresh]);
  const selected = workspaces?.workspaces.find(item => workspaces.selected_workspace_id ? item.id === workspaces.selected_workspace_id : item.kind === 'original');
  const source = sessions.find(item => item.id === sourceId);
  const unresolved = operation?.status === 'unknown' || operation?.status === 'pending';
  async function save(action: 'create' | 'import' | 'select' | 'remove', extra: Record<string, string | null> = {}) {
    const valid = guard();
    const removedWorkspace = workspaces?.workspaces.find(item => item.id === extra.workspace_id);
    const removalKeys = action === 'remove' ? [...new Set([extra.workspace_id ?? '', ...(removedWorkspace ? [removedWorkspace.path] : [])])].map(scope => developmentDraftKey('commit', projectId, taskId, scope)) : [];
    const removalDrafts = removalKeys.map(key => ({ key, entry: useDevelopmentDrafts.getState().entries[key] }));
    if (action === 'remove' && !await confirmAction(t('delivery.remove'), t('delivery.removeWarning') + (removalDrafts.some(item => item.entry) ? '\n' + t('dev.discardDraft.commit') : ''), t('delivery.remove'))) return;
    if (!valid() || removalDrafts.some(item => useDevelopmentDrafts.getState().entries[item.key] !== item.entry)) return;
    setBusy(true); setError(null);
    const requestId = crypto.randomUUID();
    setOperation({ id: requestId, project_id: projectId, task_id: taskId, kind: `workspace_${action}`, status: 'pending', workspace_id: null, request_fingerprint: '', details: {} });
    const result = await dispatchDeliveryRequest({ id: requestId, project_id: projectId, task_id: taskId, kind: `workspace_${action}` }, () => saveDevelopmentWorkspace({ request_id: requestId, task_id: taskId, action, ...extra }));
    if (!valid()) return;
    if (!result.ok) { await refresh(); if (valid()) setError(result.code); return; }
    if (result.value.operation?.status === 'succeeded' && removalDrafts.length) {
      useDevelopmentDrafts.setState(state => { const entries = { ...state.entries }; for (const item of removalDrafts) if (entries[item.key] === item.entry) delete entries[item.key]; return { entries }; });
    }
    setWorkspaces(result.value);
    if (result.value.operation) setOperation(result.value.operation as GitOperation);
    const current = result.value.workspaces.find(item => item.id === result.value.selected_workspace_id);
    if (current) onWorkspaceChange?.(current.path);
    await refresh();
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
  return <div className="dev-review-stack" data-testid="dev-delivery">
    <div className="dev-review-toolbar"><h3 className="m-0 text-sm">{t('delivery.title')}</h3><Button size="xs" variant="secondary" disabled={busy} onClick={() => void refresh()}>{t('review.refresh')}</Button></div>
    {busy ? <p role="status" className="text-xs">{t('review.loading')}</p> : null}{error ? <p role="alert" className="text-xs text-status-error">{errorText(error)}</p> : null}
    {taskId ? <>
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
      <p className="text-xs">{t('delivery.gitHint')}</p><Button size="xs" variant="secondary" disabled={!onOpenGit} onClick={() => onOpenGit?.()}>{t('delivery.openGit')}</Button>
      {operation ? <div className="dev-review-stack"><p role="status" className={`dev-feedback dev-feedback-${operation.status === 'failed' ? 'error' : operation.status === 'succeeded' ? 'success' : 'info'}`}>{t(`delivery.operation.${operation.status}`)}</p>{unresolved ? <><p className="dev-evidence-warning">{t('delivery.unknownOperation')}</p><Button size="xs" disabled={busy} onClick={() => void reconcile()}>{t('delivery.reconcile')}</Button></> : null}<details className="dev-technical-details"><summary>{t('agent.details')}</summary><code>{operation.id}</code><pre className="dev-diff" tabIndex={0}>{JSON.stringify(operation.details, null, 2)}</pre></details></div> : null}
      <details className="dev-technical-details"><summary>{t('delivery.reconcile')}</summary>
      <fieldset className="dev-runtime-fields"><legend>{t('delivery.reconcile')}</legend>{workspaces?.operations?.filter(item => item.status === 'pending' || item.status === 'unknown').map(item => <Button key={item.id} size="xs" variant="secondary" onClick={() => setOperation(item)}>{item.id} · {t(`delivery.operation.${item.status}`)}</Button>)}<Input label={t('delivery.operationId')} value={reconcileId} onChange={e => setReconcileId(e.target.value)} /><Button size="xs" disabled={busy || !reconcileId} onClick={() => setOperation({ id: reconcileId, project_id: projectId, kind: 'git_push', status: 'unknown', task_id: taskId, workspace_id: null, request_fingerprint: '', details: {} })}>{t('delivery.inspectOperation')}</Button></fieldset>
      </details>
    </section>
    {taskId ? <>
    <section className="dev-panel" aria-label={t('delivery.handoffs')}><h4 className="m-0 text-xs font-semibold">{t('delivery.handoffs')}</h4><p className="text-xs">{t('delivery.handoffHint')}</p>
      <label className="dev-field">{t('delivery.sourceSession')}<select value={sourceId} onChange={event => { deliveryDraft.setValue({ ...deliveryDraft.value, sourceId: event.target.value }); setHandoff(null); }}><option value="">{t('delivery.chooseSource')}</option>{sessions.map(s => <option key={s.id} value={s.id}>{s.engine} · {s.id.slice(0, 8)}</option>)}</select></label>
      <Input label={t('delivery.remaining')} value={remaining} onChange={e => deliveryDraft.setValue({ ...deliveryDraft.value, remaining: e.target.value })} />
      <Button size="xs" disabled={busy || !source || !onContinueModel} onClick={() => source && onContinueModel?.(source.id)}>{t('delivery.continueModel')}</Button>
      {handoff ? <><label className="dev-field">{t('delivery.summary')}<textarea className="dev-editor type-reading" value={summary} onChange={e => summaryDraft.setValue(e.target.value)} /></label><p className="text-xs">{t('delivery.summaryHint')}</p><Button size="xs" variant="secondary" disabled={busy || !onContinueModel || Boolean(handoff.target_session_id)} onClick={() => onContinueModel?.(handoff.source_session_id, handoff.id)}>{t('delivery.continueSummary')}</Button></> : null}
      {summaryDraft.dirty ? <Button size="xs" variant="secondary" disabled={busy} onClick={() => void summaryDraft.discard()}>{t('dev.discardDraft')}</Button> : null}
      {history.map(item => <details key={item.id} className="dev-run"><summary className="text-xs">{item.target_engine} · {item.id.slice(0, 8)} · {item.target_session_id ? t('delivery.accepted') : t('delivery.awaitingReview')}</summary><pre className="dev-diff type-reading" tabIndex={0}>{item.summary}</pre><p className="break-all text-xs">{item.fingerprint}</p>{!item.target_session_id ? <Button size="xs" variant="secondary" onClick={() => { setHandoff(item); deliveryDraft.setValue({ ...deliveryDraft.value, sourceId: item.source_session_id }); }}>{t('delivery.useDraft')}</Button> : null}</details>)}
    </section>
    </> : null}
  </div>;
}
