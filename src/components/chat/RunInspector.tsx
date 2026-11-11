import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Button, Modal } from '@dgos/ui';
import { History } from 'lucide-react';
import { formatDateTime, formatNumber, t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore, type ChatRun } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import RunSteps, { chatRunStatus } from './RunSteps';
import { ChatNotice } from './ChatNotice';
import Markdown from './Markdown';
import ManualResolution from './ManualResolution';
import ReplyExecution from './ReplyExecution';
import { trustedSources } from './sources';
import './chatRecovery.css';

interface RunInspectorProps {
  sessionId: string;
  /** Message entry points bind both identities; session history also retains orphan runs. */
  taskId?: string;
  messageId?: string;
  title?: string;
  disabled?: boolean;
}

export default function RunInspector({ sessionId, taskId, messageId, title, disabled = false }: RunInspectorProps) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const list = useChatStore(state => state.listRuns);
  const inspect = useChatStore(state => state.inspectRun);
  const stage = useChatStore(state => state.stageContinuation);
  const messages = useChatStore(state => state.messages);
  const unavailable = useChatStore(state => state.activeStale || !!state.preparingSessionId || Object.keys(state.generations).length > 0);
  const [open, setOpen] = useState(false);
  const [runs, setRuns] = useState<ChatRun[]>([]);
  const [selected, setSelected] = useState<ChatRun | null>(null);
  const sharedRun = useChatStore(state => selected ? state.replyRuns[selected.task_id]?.run : null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const current = (ticket: number) => ticket === revision.current && useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === sessionId;
  useLayoutEffect(() => { revision.current++; setOpen(false); setRuns([]); setSelected(null); setError(null); setLoading(false); }, [sessionId, account, taskId, messageId]);
  useEffect(() => () => { revision.current++; }, []);
  useEffect(() => {
    if (!sharedRun || !selected || sharedRun.session_id !== sessionId || sharedRun.task_id !== selected.task_id || sharedRun.assistant_message_id !== selected.assistant_message_id || sharedRun === selected) return;
    setSelected(sharedRun);
    setRuns(currentRuns => currentRuns.map(run => run.task_id === sharedRun.task_id ? sharedRun : run));
  }, [sharedRun, selected, sessionId]);
  const choose = async (id: string) => {
    const ticket = ++revision.current;
    setLoading(true); setSelected(null); setError(null);
    try {
      const result = await inspect(id);
      if (!current(ticket)) return;
      if (result?.session_id === sessionId && result.task_id === id && (!messageId || result.assistant_message_id === messageId)) setSelected(result);
      else setError(t('chat.activityUnavailable'));
    } catch (failure) { if (current(ticket)) setError(chatErrorText(failure)); }
    finally { if (current(ticket)) setLoading(false); }
  };
  const load = async () => {
    if (taskId) { await choose(taskId); return; }
    const ticket = ++revision.current;
    setLoading(true); setError(null);
    try {
      const result = await list(sessionId);
      if (current(ticket) && result) {
        setRuns(result.filter(run => run.session_id === sessionId));
        setSelected(null);
      }
    } catch (failure) { if (current(ticket)) setError(chatErrorText(failure)); }
    finally { if (current(ticket)) setLoading(false); }
  };
  const runTitle = (run: ChatRun) => {
    if (run.source_run_id) return t('chat.activityFollowUp');
    const index = messages.findIndex(message => message.session_id === sessionId && message.id === run.assistant_message_id && message.task_id === run.task_id);
    const request = index >= 0 ? messages.slice(0, index).reverse().find(message => message.session_id === sessionId && message.role === 'user') : null;
    return request?.content.trim().replace(/\s+/g, ' ').slice(0, 100) || title || t('chat.activityRequest');
  };
  const unresolvedTool = selected?.steps.some(step => step.kind === 'tool' && (step.result_unknown || step.status === 'dispatched' || (step.dispatched && !step.receipt)));
  const activeRun = !!selected && ['running', 'generating', 'pending'].includes(selected.status);
  const blocked = !selected || !!selected.audit_warning || activeRun || !!unresolvedTool;
  const controlsDisabled = disabled || unavailable;
  const close = () => { revision.current++; setOpen(false); setLoading(false); };
  const label = t(taskId ? 'chat.replyDiagnostics' : 'chat.activityHistory');
  return <>
    <Button type="button" size="sm" variant="ghost" onClick={() => { setOpen(true); void load(); }}><History size={15} aria-hidden="true" />{label}</Button>
    <Modal open={open} onOpenChange={next => { if (!next) close(); }} kind="alert" title={label} description={t('chat.activityHistoryHelp')} footer={<Button type="button" variant="secondary" onClick={close}>{t('chat.activityClose')}</Button>}>
      <section className="chat-activity-dialog" aria-busy={loading}>
        <div className="chat-activity-toolbar"><Button type="button" size="sm" variant="secondary" loading={loading} onClick={() => void (selected ? choose(selected.task_id) : load())}>{t('chat.refreshRuns')}</Button>{!taskId && selected ? <Button type="button" size="sm" variant="ghost" onClick={() => { revision.current++; setSelected(null); setLoading(false); }}>{t('chat.activityAll')}</Button> : null}</div>
        {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
        {!selected && !taskId ? <div className="chat-activity-list" role="group" aria-label={label}>{runs.map(run => <Button type="button" key={run.task_id} variant="ghost" className="chat-activity-choice" onClick={() => void choose(run.task_id)}><span className="chat-activity-choice-copy"><strong>{runTitle(run)}</strong><time dateTime={new Date(run.created_at * 1000).toISOString()}>{formatDateTime(new Date(run.created_at * 1000))}</time></span><span>{run.result_unknown ? t('chat.runUnknown') : chatRunStatus(run.status)}</span></Button>)}</div> : null}
        {!loading && !selected && !taskId && !runs.length ? <p className="text-sm text-ink-secondary">{t('chat.noRuns')}</p> : null}
        {selected ? <>
          <header className="chat-activity-heading"><h3>{runTitle(selected)}</h3><span>{chatRunStatus(selected.status)}</span><time dateTime={new Date(selected.created_at * 1000).toISOString()}>{formatDateTime(new Date(selected.created_at * 1000))}</time></header>
          {selected.audit_warning ? <ChatNotice tone="warning">{t('chat.activityAuditBlocked')}</ChatNotice> : null}
          {unresolvedTool ? <ChatNotice tone="warning">{t('chat.activityToolUnknown')}</ChatNotice> : selected.result_unknown ? <ChatNotice tone="warning">{t('chat.historicalUnknownContinuation')}</ChatNotice> : null}
          {selected.partial_content ? <details className="chat-activity-section"><summary>{t('chat.activityPartial')}</summary><p className="text-sm text-ink-secondary">{t('chat.activityPartialHelp')}</p><Markdown source={selected.partial_content} sources={trustedSources(selected.steps)} /></details> : null}
          {taskId ? <section className="chat-activity-section"><h4>{t('chat.activitySteps')}</h4>{selected.steps.length ? <RunSteps steps={selected.steps} diagnostic /> : <p className="text-sm text-ink-secondary">{t('chat.activityNoSteps')}</p>}</section> : <ReplyExecution taskId={selected.task_id} steps={selected.steps} run={selected} showRecovery={false} />}
          {!activeRun ? selected.steps.filter(step => step.kind === 'tool' && step.result_unknown).map(step => <ManualResolution key={`${account}:${selected.task_id}:${step.id}`} run={selected} step={step} disabled={controlsDisabled || loading || !!selected.audit_warning} onResolved={updated => { if (updated.session_id !== sessionId || updated.task_id !== selected.task_id || useDevelopment.getState().epoch !== account || useChatStore.getState().activeSessionId !== sessionId) return; setSelected(updated); setRuns(currentRuns => currentRuns.map(run => run.task_id === updated.task_id ? updated : run)); }} />) : null}
          <details className="chat-activity-section"><summary>{t('chat.activityTechnical')}</summary><dl className="chat-activity-metadata"><div><dt>{t('chat.activityTaskId')}</dt><dd>{selected.task_id}</dd></div><div><dt>{t('chat.activityMessageId')}</dt><dd>{selected.assistant_message_id}</dd></div>{selected.source_run_id ? <div><dt>{t('chat.activitySourceId')}</dt><dd>{selected.source_run_id}</dd></div> : null}<div><dt>{t('chat.dispatchState')}</dt><dd>{t(selected.dispatched ? 'chat.dispatched' : 'chat.notDispatched')}</dd></div><div><dt>{t('chat.runUsage')}</dt><dd>{selected.total_tokens === null ? t('chat.usageUnknown') : formatNumber(selected.total_tokens)}</dd></div></dl></details>
          <section className="chat-activity-continue"><h4>{t('chat.activityContinue')}</h4><p>{t('chat.continueComposerHelp')}</p>{blocked ? <ChatNotice tone="warning">{t(activeRun ? 'chat.activityStillRunning' : selected.audit_warning ? 'chat.activityAuditBlocked' : 'chat.activityToolUnknown')}</ChatNotice> : null}<Button type="button" disabled={blocked || controlsDisabled || loading} onClick={() => { stage(selected); close(); }}>{t('chat.continueInComposer')}</Button></section>
        </> : null}
      </section>
    </Modal>
  </>;
}
