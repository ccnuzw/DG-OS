import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input, Modal } from '@dgos/ui';
import { History } from 'lucide-react';
import { formatDateTime, formatNumber, t, useLocale } from '../../i18n';
import { MAX_INTENT_BYTES, chatErrorText, useChatStore, utf8Bytes, type ChatRun } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { fetchConnectedTools, toolKey } from './catalog';
import RunSteps, { chatRunStatus } from './RunSteps';
import { ChatNotice } from './ChatNotice';
import Markdown from './Markdown';
import ManualResolution from './ManualResolution';
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
  const proceed = useChatStore(state => state.continueRun);
  const setIntent = useChatStore(state => state.setRunIntent);
  const messages = useChatStore(state => state.messages);
  const unavailable = useChatStore(state => state.activeStale || !!state.preparingSessionId || Object.keys(state.generations).length > 0);
  const [open, setOpen] = useState(false);
  const [runs, setRuns] = useState<ChatRun[]>([]);
  const [selected, setSelected] = useState<ChatRun | null>(null);
  const intent = useChatStore(state => selected ? state.runIntentDrafts[selected.task_id] ?? '' : '');
  const [confirmed, setConfirmed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const current = (ticket: number) => ticket === revision.current && useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === sessionId;
  useLayoutEffect(() => { revision.current++; setOpen(false); setRuns([]); setSelected(null); setError(null); setLoading(false); setConfirmed(false); }, [sessionId, account, taskId, messageId]);
  useEffect(() => () => { revision.current++; }, []);
  const choose = async (id: string) => {
    const ticket = ++revision.current;
    setLoading(true); setSelected(null); setError(null); setConfirmed(false);
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
    setLoading(true); setError(null); setConfirmed(false);
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
  const tooLong = utf8Bytes(intent) > MAX_INTENT_BYTES;
  const continueSelected = async () => {
    if (!selected || blocked || controlsDisabled || loading || !confirmed || !intent.trim() || tooLong) return;
    const ticket = ++revision.current;
    const source = selected.task_id;
    setLoading(true); setError(null);
    try {
      const keys = useChatStore.getState().sessionTools[sessionId] ?? [];
      const available = keys.length ? await fetchConnectedTools() : [];
      if (!current(ticket)) return;
      const tools = keys.map(key => available.find(tool => toolKey(tool) === key));
      if (tools.some(tool => !tool)) { setError(t('chat.toolsChanged')); return; }
      const outcome = await proceed(source, intent, tools.filter(tool => !!tool));
      if (current(ticket)) {
        if (outcome === 'started') { setIntent(source, ''); setOpen(false); }
        else if (outcome === 'rejected') setError(useChatStore.getState().actionError || t('chat.activityNotStarted'));
      }
    } catch (failure) { if (current(ticket)) setError(chatErrorText(failure)); }
    finally { if (current(ticket)) setLoading(false); }
  };
  const close = () => { revision.current++; setOpen(false); setLoading(false); setConfirmed(false); };
  const label = t(taskId ? 'chat.activityDetails' : 'chat.activityHistory');
  return <>
    <Button type="button" size="sm" variant="ghost" onClick={() => { setOpen(true); void load(); }}><History size={15} aria-hidden="true" />{label}</Button>
    <Modal open={open} onOpenChange={next => { if (!next) close(); }} kind="alert" title={label} description={t('chat.activityHistoryHelp')} footer={<Button type="button" variant="secondary" onClick={close}>{t('chat.activityClose')}</Button>}>
      <section className="chat-activity-dialog" aria-busy={loading}>
        <div className="chat-activity-toolbar"><Button type="button" size="sm" variant="secondary" loading={loading} onClick={() => void (selected ? choose(selected.task_id) : load())}>{t('chat.refreshRuns')}</Button>{!taskId && selected ? <Button type="button" size="sm" variant="ghost" onClick={() => { revision.current++; setSelected(null); setLoading(false); setConfirmed(false); }}>{t('chat.activityAll')}</Button> : null}</div>
        {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
        {!selected && !taskId ? <div className="chat-activity-list" role="group" aria-label={label}>{runs.map(run => <Button type="button" key={run.task_id} variant="ghost" className="chat-activity-choice" onClick={() => void choose(run.task_id)}><span className="chat-activity-choice-copy"><strong>{runTitle(run)}</strong><time dateTime={new Date(run.created_at * 1000).toISOString()}>{formatDateTime(new Date(run.created_at * 1000))}</time></span><span>{run.result_unknown ? t('chat.runUnknown') : chatRunStatus(run.status)}</span></Button>)}</div> : null}
        {!loading && !selected && !taskId && !runs.length ? <p className="text-sm text-ink-secondary">{t('chat.noRuns')}</p> : null}
        {selected ? <>
          <header className="chat-activity-heading"><h3>{runTitle(selected)}</h3><span>{chatRunStatus(selected.status)}</span><time dateTime={new Date(selected.created_at * 1000).toISOString()}>{formatDateTime(new Date(selected.created_at * 1000))}</time></header>
          {selected.audit_warning ? <ChatNotice tone="warning">{t('chat.activityAuditBlocked')}</ChatNotice> : null}
          {unresolvedTool ? <ChatNotice tone="warning">{t('chat.activityToolUnknown')}</ChatNotice> : selected.result_unknown ? <ChatNotice tone="warning">{t('chat.historicalUnknownContinuation')}</ChatNotice> : null}
          {selected.partial_content ? <details className="chat-activity-section"><summary>{t('chat.activityPartial')}</summary><p className="text-sm text-ink-secondary">{t('chat.activityPartialHelp')}</p><Markdown source={selected.partial_content} /></details> : null}
          <section className="chat-activity-section"><h4>{t('chat.activitySteps')}</h4>{selected.steps.length ? <RunSteps steps={selected.steps} /> : <p className="text-sm text-ink-secondary">{t('chat.activityNoSteps')}</p>}</section>
          {!activeRun ? selected.steps.filter(step => step.kind === 'tool' && step.result_unknown).map(step => <ManualResolution key={`${account}:${selected.task_id}:${step.id}`} run={selected} step={step} disabled={controlsDisabled || loading || !!selected.audit_warning} onResolved={updated => { if (updated.session_id !== sessionId || updated.task_id !== selected.task_id || useDevelopment.getState().epoch !== account || useChatStore.getState().activeSessionId !== sessionId) return; setSelected(updated); setRuns(currentRuns => currentRuns.map(run => run.task_id === updated.task_id ? updated : run)); setConfirmed(false); }} />) : null}
          <details className="chat-activity-section"><summary>{t('chat.activityTechnical')}</summary><dl className="chat-activity-metadata"><div><dt>{t('chat.activityTaskId')}</dt><dd>{selected.task_id}</dd></div><div><dt>{t('chat.activityMessageId')}</dt><dd>{selected.assistant_message_id}</dd></div>{selected.source_run_id ? <div><dt>{t('chat.activitySourceId')}</dt><dd>{selected.source_run_id}</dd></div> : null}<div><dt>{t('chat.dispatchState')}</dt><dd>{t(selected.dispatched ? 'chat.dispatched' : 'chat.notDispatched')}</dd></div><div><dt>{t('chat.runUsage')}</dt><dd>{selected.total_tokens === null ? t('chat.usageUnknown') : formatNumber(selected.total_tokens)}</dd></div></dl></details>
          <section className="chat-activity-continue"><h4>{t('chat.activityContinue')}</h4><p>{t('chat.activityContinueHelp')}</p>{blocked ? <ChatNotice tone="warning">{t(activeRun ? 'chat.activityStillRunning' : selected.audit_warning ? 'chat.activityAuditBlocked' : 'chat.activityToolUnknown')}</ChatNotice> : null}<Input multiline rows={3} label={t('chat.activityIntent')} value={intent} disabled={controlsDisabled || loading || blocked} error={tooLong ? t('chat.intentTooLong') : undefined} onChange={event => { setIntent(selected.task_id, event.target.value); setConfirmed(false); }} /><Checkbox checked={confirmed} disabled={blocked || controlsDisabled || loading} onChange={setConfirmed} label={t('chat.activityContinueConfirm')} /><Button type="button" disabled={blocked || controlsDisabled || !confirmed || !intent.trim() || tooLong} loading={loading} onClick={() => void continueSelected()}>{t('chat.activityStartRequest')}</Button></section>
        </> : null}
      </section>
    </Modal>
  </>;
}
