import { useEffect, useId, useMemo, useRef } from 'react';
import { Button } from '@dgos/ui';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { formatNumber, t, useLocale } from '../../i18n';
import { useChatStore, type ChatModelSegment, type ChatRun, type ChatRunStep, type ChatToolProposal } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import RunSteps from './RunSteps';
import Markdown from './Markdown';
import ManualResolution from './ManualResolution';
import { SourcePreview } from './SourcePreview';
import { executionSteps, operationFailed, trustedSources, webActivity } from './sources';
import './chatRecovery.css';

/** Reading a visible historical reply is lazy, bounded and read-only. Never inspect the whole message page. */
export function useReplyRun(sessionId: string, taskId: string | null, messageId: string) {
  const row = useRef<HTMLElement>(null);
  const account = useDevelopment(state => state.epoch);
  const active = useSurfaceActive();
  const load = useChatStore(state => state.loadReplyRun);
  const retain = useChatStore(state => state.retainReplyRun);
  const cached = useChatStore(state => taskId ? state.replyRuns[taskId] : undefined);
  const stale = useChatStore(state => state.activeStale);
  useEffect(() => {
    if (!active || stale || !taskId || !row.current) return;
    const element = row.current;
    let release: (() => void) | undefined;
    const visible = (intersecting: boolean) => {
      if (!intersecting) { release?.(); release = undefined; return; }
      if (release) return;
      release = retain(taskId);
      void load(sessionId, taskId, messageId);
    };
    if (typeof IntersectionObserver === 'undefined') {
      visible(true);
      return () => release?.();
    }
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) if (entry.target === element) visible(entry.isIntersecting);
    }, { root: element.closest('.chat-message-scroll'), rootMargin: '160px' });
    observer.observe(element);
    return () => { observer.disconnect(); release?.(); };
  }, [active, stale, account, sessionId, taskId, messageId, load, retain]);
  const projection = !stale && cached?.sessionId === sessionId && cached.messageId === messageId ? cached : undefined;
  return { row, projection };
}

export default function ReplyExecution({ taskId, steps, segments = [], run, proposal, unavailable = false, settled = false, showRecovery = true }: { taskId: string; steps: ChatRunStep[]; segments?: ChatModelSegment[]; run?: ChatRun | null; proposal?: ChatToolProposal | null; unavailable?: boolean; settled?: boolean; showRecovery?: boolean }) {
  useLocale();
  const id = useId();
  const expandedItems = useChatStore(state => state.replyExpanded);
  const expanded = expandedItems[taskId] ?? false;
  const setExpanded = useChatStore(state => state.setReplyExpanded);
  const stage = useChatStore(state => state.stageContinuation);
  const busy = useChatStore(state => !!state.preparingSessionId || !!state.generation || state.activeStale);
  const account = useDevelopment(state => state.epoch);
  const operations = useMemo(() => executionSteps(steps), [steps]);
  const sources = useMemo(() => trustedSources(steps), [steps]);
  const intermediate = useMemo(() => {
    const values = new Map(segments.map(segment => [segment.round, segment]));
    for (const step of steps) if (step.kind === 'model_segment' && typeof step.receipt?.content === 'string') { const round = typeof step.receipt.round === 'number' ? step.receipt.round : step.ordinal; values.set(round, { round, content: step.receipt.content, truncated: step.receipt.truncated === true }); }
    return [...values.values()].sort((a, b) => a.round - b.round);
  }, [segments, steps]);
  const failures = operations.filter(operationFailed);
  const terminal = settled || (!!run && !['running', 'generating', 'pending'].includes(run.status));
  const unknown = terminal && operations.some(step => step.result_unknown || (step.kind === 'tool' && (step.status === 'dispatched' || (step.dispatched && !step.receipt))));
  const runningOperations = terminal ? [] : operations.filter(step => webActivity(step)?.status === 'running' || ['running', 'dispatched', 'pending'].includes(step.status));
  const running = runningOperations[runningOperations.length - 1];
  const blocked = !run || !!run.audit_warning || unknown || ['running', 'generating', 'pending'].includes(run.status);
  const summary = proposal ? t('chat.replyNeedsConfirmation') : unknown ? t('chat.replyOutcomeUnknown') : running ? t(webActivity(running)?.operation === 'search' ? 'chat.webSearching' : webActivity(running)?.operation === 'fetch' ? 'chat.webReading' : 'chat.operationRunning') : failures.length ? t('chat.replyPartialFailure', { count: formatNumber(failures.length) }) : t('chat.replyOperations', { count: formatNumber(operations.length) });
  if (!operations.length && !intermediate.length && !proposal && !unavailable && (!run || ['completed', 'succeeded'].includes(run.status))) return null;
  return <section className="chat-reply-execution" aria-label={t('chat.executionActivity')}>
    {proposal ? <div className="chat-reply-approval"><strong>{t('chat.replyNeedsConfirmation')}</strong><span>{proposal.server_name} / {proposal.tool_name}</span><Button type="button" size="sm" variant="secondary" onClick={() => document.getElementById('chat-proposal-heading')?.focus({ preventScroll: false })}>{t('chat.replyGoToConfirmation')}</Button></div> : null}
    {operations.length || intermediate.length ? <>
      <Button type="button" variant="ghost" size="sm" className="chat-execution-disclosure" aria-expanded={expanded} aria-controls={id} onClick={() => setExpanded(taskId, !expanded)}>{expanded ? <ChevronDown size={15} aria-hidden="true" /> : <ChevronRight size={15} aria-hidden="true" />}<span>{operations.length ? summary : t('chat.replyIntermediate')}</span><span className="text-ink-secondary">{t('chat.replyViewProcess')}</span></Button>
      {failures.length ? <p className="chat-execution-impact text-status-warning-text">{t('chat.replyFailureImpact')}</p> : null}
      {!expanded && failures.length ? <ul className="chat-execution-failures">{failures.map(step => { const activity = webActivity(step); return <li key={activity?.id ?? step.id}>{activity ? `${t(activity.operation === 'search' ? 'chat.webSearchAction' : 'chat.webFetchAction')} · ${(activity.query || activity.url || '').slice(0, 240)}` : typeof step.receipt?.tool_name === 'string' ? step.receipt.tool_name : t('chat.operationFailedState')}</li>; })}</ul> : null}
      <div id={id} hidden={!expanded}>
        {intermediate.map(segment => <details className="chat-intermediate" key={segment.round} open={expandedItems[`${taskId}:round:${segment.round}`] ?? false} onToggle={event => setExpanded(`${taskId}:round:${segment.round}`, event.currentTarget.open)}><summary>{t('chat.replyModelRound', { round: formatNumber(segment.round) })}</summary><Markdown source={segment.content} sources={sources} />{segment.truncated ? <p>{t('chat.resultExcerpt')}</p> : null}</details>)}
        <RunSteps steps={steps} active={!terminal} taskId={taskId} />
      </div>
    </> : null}
    {sources.length ? <div className="chat-reply-sources"><strong>{t('chat.replySources', { count: formatNumber(sources.length) })}</strong>{sources.map(source => <SourcePreview key={source.id} source={source} />)}</div> : null}
    {unavailable ? <p className="text-xs text-ink-secondary">{t('chat.activityUnavailable')}</p> : null}
    {run?.audit_warning ? <p className="text-sm text-status-warning-text">{t('chat.activityAuditBlocked')}</p> : null}
    {showRecovery && run && !['running', 'generating', 'pending'].includes(run.status) ? <>
      {run.steps.filter(step => step.kind === 'tool' && step.result_unknown).map(step => <ManualResolution key={`${account}:${run.task_id}:${step.id}`} run={run} step={step} disabled={busy || !!run.audit_warning} onResolved={updated => { if (useDevelopment.getState().epoch !== account || useChatStore.getState().activeSessionId !== run.session_id || updated.task_id !== run.task_id || updated.assistant_message_id !== run.assistant_message_id) return; void useChatStore.getState().loadReplyRun(run.session_id, run.task_id, run.assistant_message_id, true); }} />)}
      <Button type="button" size="sm" variant="ghost" disabled={busy || blocked} onClick={() => stage(run)}>{t('chat.activityContinue')}</Button>
    </> : null}
  </section>;
}
