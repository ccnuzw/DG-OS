import { useEffect, useRef, useState } from 'react';
import { Button, Input, Modal, Select } from '@dgos/ui';
import { t, useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import type { AgentSession } from '../../stores/agentRunnerStore';
import { codexOperationError, codexOperationsKey, emptyCodexOperations, useCodexOperations } from '../../stores/codexOperations';
import type { CodexChildAgent, CodexOperationAction, CodexOperationReceipt, CodexOperationScope, CodexReviewTarget } from './codexOperationsService';
import { codexOperationsService } from './codexOperationsService';

export interface CodexOperationsProps {
  session: AgentSession;
  disabled?: boolean;
  onSessionChanged?: (session: AgentSession) => void;
  /** Parent owns navigation and unsaved-draft guards. Fork never switches the source implicitly. */
  onOpenSession?: (sessionId: string) => void;
}

export default function CodexOperations(props: CodexOperationsProps) {
  useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const accountKey = useDevelopment(state => state.workspaceOwner);
  const metadata = useDevelopment(state => state.sessions.find(item => item.id === props.session.id));
  const project = useDevelopment(state => state.projects.find(item => item.id === metadata?.project_id));
  const scope: CodexOperationScope | null = accountKey && metadata?.owner && metadata.engine === 'codex' && project?.owner === metadata.owner && !project.archived
    ? { epoch, accountKey, owner: metadata.owner, projectId: project.id, taskId: metadata.task_id, sessionId: props.session.id, generation: props.session.generation } : null;
  if (props.session.engine !== 'codex') return null;
  if (!scope) return <p data-testid="codex-operations-unavailable" data-scope-ready="false" role="status" className="m-0 text-sm text-ink-secondary">{t('codexOperations.unavailable')}</p>;
  return <OperationsPanel key={codexOperationsKey(scope)} {...props} scope={scope} />;
}

function OperationsPanel({ session, disabled = false, onSessionChanged, onOpenSession, scope }: CodexOperationsProps & { scope: CodexOperationScope }) {
  const state = useCodexOperations(store => store.entries[codexOperationsKey(scope)] ?? emptyCodexOperations);
  const store = useCodexOperations.getState();
  const [dialog, setDialog] = useState<'review' | 'fork' | 'revert' | null>(null);
  const [selectedChild, setSelectedChild] = useState('');
  const [editing, setEditing] = useState<string | null>(null);
  const [reviewType, setReviewType] = useState<CodexReviewTarget['type']>('uncommittedChanges');
  const [turn, setTurn] = useState('');
  const [forkTurn, setForkTurn] = useState('');
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  // Every callback rechecks the scope after awaits rather than applying old-account results.
  const currentScope = () => {
    const development = useDevelopment.getState();
    return live.current && development.epoch === scope.epoch && development.workspaceOwner === scope.accountKey;
  };
  const unavailable = disabled || ['closed', 'starting', 'unknown'].includes(session.status);
  const blocked = unavailable || state.busy || !!state.pending;
  const snapshot = !unavailable && !state.stale ? state.snapshot : null;
  const supported = (kind: string) => !!snapshot && !snapshot.unsupported.includes(kind);
  const canAct = (kind: string) => !blocked && supported(kind);
  const idle = session.status !== 'running' && !session.active_run && !session.persistence_error;
  const draft = (id: string) => state.drafts[id] ?? '';
  const updateDraft = (id: string, value: string) => store.draft(scope, id, value);
  const refresh = () => { if (!unavailable) void store.observe(scope); };
  const finish = async (receipt: CodexOperationReceipt | null) => {
    if (!currentScope() || receipt?.state !== 'applied') return;
    await store.observe(scope);
    if (!onSessionChanged || !currentScope()) return;
    try {
      const value = await codexOperationsService.session(scope);
      if (currentScope() && value.id === scope.sessionId && value.generation === scope.generation && value.engine === 'codex') onSessionChanged(value);
    } catch { /* The accepted operation is not retried when its read-only refresh fails. */ }
  };
  const act = async (action: CodexOperationAction) => {
    if (blocked || !canAct(action.kind)) return;
    const receipt = await store.act(scope, action);
    if (!currentScope()) return;
    await finish(receipt);
    if (receipt?.state === 'applied') { setDialog(null); setEditing(null); }
  };
  const inspect = async () => {
    const receipt = await store.inspect(scope);
    if (!currentScope() || receipt?.state !== 'applied') return;
    await finish(receipt);
  };
  const history = selectedChild ? state.histories[selectedChild] : null;
  const child = snapshot?.children.find(item => item.thread_id === selectedChild);
  const reviewValue = draft(`review:${reviewType}`);
  const reviewValid = reviewType === 'uncommittedChanges' || !!reviewValue.trim();
  const reviewTarget: CodexReviewTarget = reviewType === 'baseBranch' ? { type: reviewType, branch: reviewValue.trim() }
    : reviewType === 'commit' ? { type: reviewType, sha: reviewValue.trim() }
      : reviewType === 'custom' ? { type: reviewType, instructions: reviewValue } : { type: reviewType };
  const selectedTurn = snapshot?.turns.find(item => item.id === turn);
  const queue = snapshot?.queue ?? [];
  const orderedChildren: { child: CodexChildAgent; depth: number }[] = [];
  const seen = new Set<string>();
  const append = (item: CodexChildAgent, depth: number) => {
    if (seen.has(item.thread_id)) return;
    seen.add(item.thread_id); orderedChildren.push({ child: item, depth });
    snapshot?.children.filter(other => other.parent_thread_id === item.thread_id).forEach(other => append(other, depth + 1));
  };
  snapshot?.children.filter(item => !snapshot.children.some(other => other.thread_id === item.parent_thread_id)).forEach(item => append(item, 0));
  snapshot?.children.forEach(item => append(item, 0));
  return <details data-testid="codex-operations" data-scope-ready="true" data-reading={state.reading} data-stale={state.stale} data-pending={!!state.pending} data-operation-error={state.error ? state.error === 'unknown' ? 'unknown' : codexOperationError(state.error) : undefined} className="min-w-0 border-t border-border-subtle pt-3" onToggle={event => { if (event.currentTarget.open && !state.snapshot && !state.reading) refresh(); }}>
    <summary className="cursor-pointer py-2 text-sm font-medium focus-visible:outline-2 focus-visible:outline-border-focus">{t('codexOperations.title')}</summary>
    <div className="flex min-w-0 flex-col gap-4 py-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <Button data-testid="codex-operations-refresh" variant="secondary" loading={state.reading} disabled={unavailable || state.busy} onClick={refresh}>{t('codexOperations.refresh')}</Button>
        <span className="text-ink-secondary">{t('codexOperations.snapshotNotice')}</span>
      </div>
      <div role="status" aria-live="polite" className="min-w-0 break-words">
        {state.pending ? t('codexOperations.unknown') : state.error ? t(`codexOperations.error.${state.error}`) : unavailable ? t('codexOperations.unavailable') : state.receipt ? t('codexOperations.accepted') : state.reading ? t('codexOperations.loading') : ''}
        {state.receipt?.run_id && <p className="m-0 break-all">{t('codexOperations.run')}: {state.receipt.run_id}</p>}
      </div>
      {state.pending && <div className="flex min-w-0 flex-wrap items-center gap-2"><span className="break-all">{t(`codexOperations.operation.${state.pending.action.kind}`)} · {t('codexOperations.request')}: {state.pending.request_id}</span><Button data-testid="codex-operation-inspect" variant="secondary" disabled={state.busy || session.status === 'starting'} onClick={() => void inspect()}>{t('codexOperations.inspect')}</Button></div>}
      {state.receipt?.fork_session_id && <div className="flex min-w-0 flex-wrap items-center gap-2"><span className="break-all">{t('codexOperations.forkCreated')}: {state.receipt.fork_session_id}</span><Button data-testid="codex-fork-open" variant="secondary" disabled={!onOpenSession || unavailable || state.receipt.state !== 'applied'} onClick={() => { if (currentScope() && state.receipt?.fork_session_id) onOpenSession?.(state.receipt.fork_session_id); }}>{t('codexOperations.openFork')}</Button></div>}
      {snapshot?.incomplete && <p className="m-0 text-ink-secondary">{t('codexOperations.incomplete')}</p>}
      {!!snapshot?.unsupported.length && <p className="m-0 break-words text-ink-secondary">{t('codexOperations.unsupported')}: {snapshot.unsupported.map(kind => t(`codexOperations.operation.${kind}`)).join(', ')}</p>}
      <section aria-label={t('codexOperations.conversation')} className="flex min-w-0 flex-col gap-2">
        <h4 className="m-0 text-sm font-medium">{t('codexOperations.conversation')}</h4>
        <p className="m-0 text-ink-secondary">{t('codexOperations.historyOnly')}</p>
        <div className="flex flex-wrap gap-2">
          <Button data-testid="codex-review-open" variant="secondary" disabled={!idle || !canAct('review')} onClick={() => setDialog('review')}>{t('codexOperations.review')}</Button>
          <Button data-testid="codex-compact" variant="secondary" disabled={!idle || !canAct('compact')} onClick={() => void act({ kind: 'compact' })}>{t('codexOperations.compact')}</Button>
          <Button data-testid="codex-fork" variant="secondary" disabled={!idle || !canAct('fork')} onClick={() => setDialog('fork')}>{t('codexOperations.fork')}</Button>
          <Button data-testid="codex-revert-open" variant="secondary" disabled={!idle || !canAct('revert') || !snapshot?.turns.length || snapshot.incomplete} onClick={() => setDialog('revert')}>{t('codexOperations.revert')}</Button>
        </div>
      </section>
      <section data-testid="codex-agents" aria-label={t('codexOperations.agents')} className="flex min-w-0 flex-col gap-3 border-t border-border-subtle pt-3">
        <h4 className="m-0 text-sm font-medium">{t('codexOperations.agents')}</h4>
        {!orderedChildren.length && <p className="m-0 text-ink-secondary">{t('codexOperations.noAgents')}</p>}
        <ul className="m-0 flex min-w-0 list-none flex-col gap-2 p-0" aria-label={t('codexOperations.agentTree')}>
          {orderedChildren.map(({ child: item, depth }) => <li key={item.thread_id} className="flex min-w-0 flex-wrap items-center gap-2">
            <Button className="h-auto min-h-8 max-w-full whitespace-normal py-1 text-left" variant={selectedChild === item.thread_id ? 'primary' : 'secondary'} aria-pressed={selectedChild === item.thread_id} disabled={unavailable} onClick={() => setSelectedChild(item.thread_id)}><span className="break-all">{depth > 0 ? `${t('codexOperations.childLevel', { depth: String(depth + 1) })}: ` : ''}{item.thread_id}</span></Button>
            <span>{t('codexOperations.state')}: {item.status}</span><span className="break-all text-ink-secondary">{t('codexOperations.parent')}: {item.parent_thread_id}</span>
          </li>)}
        </ul>
        {child && <div className="flex min-w-0 flex-col gap-2 border-l border-border-subtle pl-3">
          <p className="m-0 break-all">{t('codexOperations.selectedAgent')}: {child.thread_id}</p>
          <div className="flex flex-wrap gap-2">
            <Button data-testid="codex-agent-history" variant="secondary" disabled={state.reading || unavailable || state.stale} onClick={() => void store.history(scope, child.thread_id)}>{t('codexOperations.history')}</Button>
            <Button data-testid="codex-agent-interrupt" variant="secondary" disabled={!canAct('interrupt_child') || !child.turn_id || snapshot?.incomplete} onClick={() => child.turn_id && void act({ kind: 'interrupt_child', child_thread_id: child.thread_id, turn_id: child.turn_id })}>{t('codexOperations.interrupt')}</Button>
            <Button data-testid="codex-agent-resume" variant="secondary" disabled={!canAct('resume_child') || snapshot?.incomplete} onClick={() => void act({ kind: 'resume_child', child_thread_id: child.thread_id })}>{t('codexOperations.resume')}</Button>
            <Button data-testid="codex-agent-close" variant="secondary" disabled={!canAct('close_child') || snapshot?.incomplete} onClick={() => void act({ kind: 'close_child', child_thread_id: child.thread_id })}>{t('codexOperations.closeAgent')}</Button>
          </div>
          {snapshot?.unsupported.includes('close_child') && <p className="m-0 text-ink-secondary">{t('codexOperations.childCloseUnavailable')}</p>}
          <form className="flex min-w-0 flex-col gap-2" onSubmit={event => { event.preventDefault(); if (draft(`child:${child.thread_id}`).trim()) void act({ kind: 'send_child', child_thread_id: child.thread_id, text: draft(`child:${child.thread_id}`) }); }}>
            <Input multiline label={t('codexOperations.childMessage')} value={draft(`child:${child.thread_id}`)} onChange={event => updateDraft(`child:${child.thread_id}`, event.target.value)} disabled={unavailable} />
            <Button data-testid="codex-agent-send" type="submit" disabled={!canAct('send_child') || snapshot?.incomplete || !draft(`child:${child.thread_id}`).trim()}>{t('codexOperations.sendChild')}</Button>
          </form>
          {history && <div data-testid="codex-agent-history-content" className="max-h-80 min-w-0 overflow-y-auto"><p>{t('codexOperations.historySnapshot')}</p>{history.incomplete && <p>{t('codexOperations.incomplete')}</p>}{history.turns.map(item => <article key={item.id} className="min-w-0 border-t border-border-subtle py-2"><h5 className="m-0 break-all text-sm">{item.id} · {item.status}</h5>{item.items.map(entry => <p key={entry.id} className="whitespace-pre-wrap break-words">{entry.text ?? entry.type}</p>)}</article>)}</div>}
        </div>}
      </section>
      <section data-testid="codex-background-commands" aria-label={t('codexOperations.background')} className="flex min-w-0 flex-col gap-2 border-t border-border-subtle pt-3">
        <h4 className="m-0 text-sm font-medium">{t('codexOperations.background')}</h4>
        {!snapshot?.terminals.length && <p className="m-0 text-ink-secondary">{t('codexOperations.noBackground')}</p>}
        {snapshot?.terminals.map(item => <div key={item.process_id} className="flex min-w-0 flex-wrap items-center gap-2"><code className="min-w-0 whitespace-pre-wrap break-all">{item.command}</code><span className="break-all">{item.process_id} · {item.status}</span><Button data-testid="codex-background-stop" variant="secondary" disabled={!canAct('terminate_terminal') || snapshot.incomplete || ['completed', 'exited', 'terminated'].includes(item.status)} onClick={() => void act({ kind: 'terminate_terminal', process_id: item.process_id })}>{t('codexOperations.stopCommand')}</Button></div>)}
      </section>
      <section data-testid="codex-queue" aria-label={t('codexOperations.queue')} className="flex min-w-0 flex-col gap-3 border-t border-border-subtle pt-3">
        <h4 className="m-0 text-sm font-medium">{t('codexOperations.queue')}</h4>
        <p className="m-0 text-ink-secondary">{t('codexOperations.queueNotice')}</p>
        {!queue.length && <p className="m-0 text-ink-secondary">{t('codexOperations.noQueue')}</p>}
        <ol className="m-0 flex min-w-0 list-none flex-col gap-3 p-0">
          {queue.map((item, index) => <li key={item.id} data-submission-id={item.id} className="flex min-w-0 flex-col gap-2 border-l border-border-subtle pl-3">
            <p className="m-0 whitespace-pre-wrap break-words">{index + 1}. {item.text}</p>
            {editing === item.id && <Input data-testid="codex-queue-edit-text" multiline label={t('codexOperations.editText')} value={draft(`queue:${item.id}`)} onChange={event => updateDraft(`queue:${item.id}`, event.target.value)} />}
            <div className="flex flex-wrap gap-2">
              <Button data-testid="codex-queue-edit" variant="secondary" disabled={!canAct('queue_update') || snapshot?.incomplete} onClick={() => { updateDraft(`queue:${item.id}`, state.drafts[`queue:${item.id}`] ?? item.text); setEditing(item.id); }}>{t('codexOperations.edit')}</Button>
              {editing === item.id && <><Button data-testid="codex-queue-save" disabled={!canAct('queue_update') || !draft(`queue:${item.id}`).trim()} onClick={() => void act({ kind: 'queue_update', submission_id: item.id, text: draft(`queue:${item.id}`) })}>{t('codexOperations.save')}</Button><Button variant="secondary" onClick={() => setEditing(null)}>{t('codexOperations.cancel')}</Button></>}
              <Button data-testid="codex-queue-delete" variant="secondary" disabled={!canAct('queue_delete') || snapshot?.incomplete} onClick={() => void act({ kind: 'queue_delete', submission_id: item.id })}>{t('codexOperations.delete')}</Button>
              <Button data-testid="codex-queue-up" variant="secondary" aria-label={t('codexOperations.moveUpItem', { position: String(index + 1) })} disabled={!canAct('queue_reorder') || snapshot?.incomplete || index === 0} onClick={() => { const ids = queue.map(entry => entry.id); [ids[index - 1], ids[index]] = [ids[index], ids[index - 1]]; void act({ kind: 'queue_reorder', submission_ids: ids }); }}>{t('codexOperations.moveUp')}</Button>
              <Button data-testid="codex-queue-down" variant="secondary" aria-label={t('codexOperations.moveDownItem', { position: String(index + 1) })} disabled={!canAct('queue_reorder') || snapshot?.incomplete || index === queue.length - 1} onClick={() => { const ids = queue.map(entry => entry.id); [ids[index], ids[index + 1]] = [ids[index + 1], ids[index]]; void act({ kind: 'queue_reorder', submission_ids: ids }); }}>{t('codexOperations.moveDown')}</Button>
              <Button data-testid="codex-queue-start" disabled={!idle || !canAct('queue_start') || snapshot?.incomplete} onClick={() => void act({ kind: 'queue_start', submission_id: item.id })}>{t('codexOperations.start')}</Button>
            </div>
          </li>)}
        </ol>
        <form className="flex min-w-0 flex-col gap-2" onSubmit={event => { event.preventDefault(); if (draft('queue:new').trim()) void act({ kind: 'queue_add', text: draft('queue:new') }); }}>
          <Input data-testid="codex-queue-text" multiline label={t('codexOperations.newQueueText')} value={draft('queue:new')} onChange={event => updateDraft('queue:new', event.target.value)} disabled={unavailable} />
          <Button data-testid="codex-queue-add" type="submit" disabled={!canAct('queue_add') || !draft('queue:new').trim()}>{t('codexOperations.addQueue')}</Button>
        </form>
      </section>
    </div>
    <Modal open={dialog === 'review' && !unavailable} onOpenChange={open => { if (!open) setDialog(null); }} title={t('codexOperations.review')} description={t('codexOperations.reviewNotice')} kind="form" footer={<div className="flex flex-wrap gap-2"><Button variant="secondary" disabled={state.busy} onClick={() => setDialog(null)}>{t('codexOperations.cancel')}</Button><Button data-testid="codex-review-start" disabled={!idle || !canAct('review') || !reviewValid} onClick={() => void act({ kind: 'review', target: reviewTarget })}>{t('codexOperations.startReview')}</Button></div>}>
      <div className="flex min-w-0 flex-col gap-3"><Select id="codex-review-target" label={t('codexOperations.reviewTarget')} value={reviewType} onChange={value => setReviewType(value as CodexReviewTarget['type'])} options={(['uncommittedChanges', 'baseBranch', 'commit', 'custom'] as const).map(value => ({ value, label: t(`codexOperations.target.${value}`) }))} />{reviewType !== 'uncommittedChanges' && <Input data-testid="codex-review-value" label={t(`codexOperations.targetValue.${reviewType}`)} value={reviewValue} onChange={event => updateDraft(`review:${reviewType}`, event.target.value)} />}</div>
    </Modal>
    <Modal open={dialog === 'fork' && !unavailable} onOpenChange={open => { if (!open) setDialog(null); }} title={t('codexOperations.fork')} description={t('codexOperations.forkNotice')} kind="form" footer={<div className="flex flex-wrap gap-2"><Button variant="secondary" disabled={state.busy} onClick={() => setDialog(null)}>{t('codexOperations.cancel')}</Button><Button data-testid="codex-fork-confirm" disabled={!idle || !canAct('fork') || !!forkTurn && (!snapshot?.turns.some(item => item.id === forkTurn) || snapshot.incomplete)} onClick={() => void act({ kind: 'fork', new_session_id: crypto.randomUUID(), ...(forkTurn ? { last_turn_id: forkTurn } : {}) })}>{t('codexOperations.fork')}</Button></div>}>
      <Select id="codex-fork-turn" label={t('codexOperations.forkPoint')} value={forkTurn || 'latest'} onChange={value => setForkTurn(value === 'latest' ? '' : value)} options={[{ value: 'latest', label: t('codexOperations.latestHistory') }, ...(snapshot?.turns ?? []).map(item => ({ value: item.id, label: `${item.id} · ${item.status}` }))]} />
    </Modal>
    <Modal open={dialog === 'revert' && !unavailable} onOpenChange={open => { if (!open) setDialog(null); }} title={t('codexOperations.revert')} description={t('codexOperations.revertNotice')} kind="form" footer={<div className="flex flex-wrap gap-2"><Button variant="secondary" disabled={state.busy} onClick={() => setDialog(null)}>{t('codexOperations.cancel')}</Button><Button data-testid="codex-revert-confirm" variant="danger" disabled={!idle || !canAct('revert') || !selectedTurn || snapshot?.incomplete} onClick={() => selectedTurn && void act({ kind: 'revert', before_turn_id: selectedTurn.id })}>{t('codexOperations.confirmRevert')}</Button></div>}>
      <Select id="codex-revert-turn" label={t('codexOperations.beforeTurn')} value={selectedTurn?.id ?? ''} onChange={setTurn} options={(snapshot?.turns ?? []).map(item => ({ value: item.id, label: `${item.id} · ${item.status}` }))} />
    </Modal>
  </details>;
}
