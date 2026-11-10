import { useEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input, Popover } from '@dgos/ui';
import { History } from 'lucide-react';
import { formatNumber, t, useLocale } from '../../i18n';
import { MAX_INTENT_BYTES, chatErrorText, useChatStore, utf8Bytes, type ChatRun } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { fetchConnectedTools, toolKey } from './catalog';
import RunSteps, { chatRunStatus } from './RunSteps';
import { ChatNotice } from './ChatNotice';
import Markdown from './Markdown';
import ManualResolution from './ManualResolution';

export default function RunInspector({ sessionId, toolKeys, disabled }: { sessionId: string; toolKeys: string[]; disabled: boolean }) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const list = useChatStore(state => state.listRuns);
  const inspect = useChatStore(state => state.inspectRun);
  const proceed = useChatStore(state => state.continueRun);
  const setIntent = useChatStore(state => state.setRunIntent);
  const [open, setOpen] = useState(false);
  const [runs, setRuns] = useState<ChatRun[]>([]);
  const [selected, setSelected] = useState<ChatRun | null>(null);
  const intent = useChatStore(state => selected ? state.runIntentDrafts[selected.task_id] ?? '' : '');
  const [confirmed, setConfirmed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  useEffect(() => { revision.current++; setOpen(false); setRuns([]); setSelected(null); setError(null); setLoading(false); setConfirmed(false); }, [sessionId, account]);
  useEffect(() => () => { revision.current++; }, []);
  const load = async () => {
    const ticket = ++revision.current;
    setLoading(true); setError(null); setConfirmed(false);
    try { const result = await list(sessionId); if (ticket === revision.current && result) { setRuns(result); setSelected(null); } }
    catch (failure) { if (ticket === revision.current) setError(chatErrorText(failure)); }
    finally { if (ticket === revision.current) setLoading(false); }
  };
  const choose = async (taskId: string) => {
    const ticket = ++revision.current;
    setLoading(true); setSelected(null); setError(null); setConfirmed(false);
    try { const result = await inspect(taskId); if (ticket === revision.current && result?.session_id === sessionId) setSelected(result); }
    catch (failure) { if (ticket === revision.current) setError(chatErrorText(failure)); }
    finally { if (ticket === revision.current) setLoading(false); }
  };
  const blocked = !selected || !!selected.audit_warning || ['running', 'generating', 'pending'].includes(selected.status) || selected.steps.some(step => step.kind === 'tool' && (step.result_unknown || step.status === 'dispatched' || (step.dispatched && !step.receipt)));
  const tooLong = utf8Bytes(intent) > MAX_INTENT_BYTES;
  const continueSelected = async () => {
    if (!selected || blocked || disabled || loading || !confirmed || !intent.trim() || tooLong) return;
    const ticket = ++revision.current;
    const source = selected.task_id;
    setLoading(true); setError(null);
    try {
      const available = toolKeys.length ? await fetchConnectedTools() : [];
      const tools = toolKeys.map(key => available.find(tool => toolKey(tool) === key));
      if (tools.some(tool => !tool)) { if (ticket === revision.current) setError(t('chat.toolsChanged')); return; }
      if (ticket !== revision.current || useChatStore.getState().activeSessionId !== sessionId) return;
      const outcome = await proceed(source, intent, tools.filter(tool => !!tool));
      if (ticket === revision.current && outcome === 'started') { setIntent(source, ''); setOpen(false); }
    } catch (failure) { if (ticket === revision.current) setError(chatErrorText(failure)); }
    finally { if (ticket === revision.current) setLoading(false); }
  };
  return <Popover label={t('chat.runHistory')} side="top" align="start" open={open} onOpenChange={next => { setOpen(next); if (next) void load(); }} trigger={<Button type="button" size="sm" variant="ghost" iconOnly aria-label={t('chat.runHistory')} title={t('chat.runHistory')}><History size={16} aria-hidden="true" /></Button>} content={<section className="chat-inspector chat-run-inspector" aria-busy={loading}>
    <h3 className="m-0 text-sm font-semibold">{t('chat.runHistory')}</h3>
    <p className="m-0 text-xs text-ink-secondary">{t('chat.runReadOnlyHelp')}</p>
    <Button type="button" size="sm" variant="secondary" loading={loading} onClick={() => void load()}>{t('chat.refreshRuns')}</Button>
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
    <div className="chat-inspector-list" role="group" aria-label={t('chat.runHistory')}>{runs.map(run => <Button type="button" key={run.task_id} variant="ghost" size="sm" className="chat-run-choice" aria-pressed={selected?.task_id === run.task_id} onClick={() => void choose(run.task_id)}><span className="truncate">{run.task_id}</span><span>{run.result_unknown ? t('chat.runUnknown') : chatRunStatus(run.status)}</span></Button>)}</div>
    {!loading && !runs.length ? <p className="text-xs text-ink-secondary">{t('chat.noRuns')}</p> : null}
    {selected ? <>
      <dl className="chat-budget-grid"><div><dt>{t('chat.dispatchState')}</dt><dd>{t(selected.dispatched ? 'chat.dispatched' : 'chat.notDispatched')}</dd></div><div><dt>{t('chat.runStatus')}</dt><dd>{chatRunStatus(selected.status)}</dd></div><div><dt>{t('chat.runUsage')}</dt><dd>{selected.total_tokens === null ? t('chat.usageUnknown') : formatNumber(selected.total_tokens)}</dd></div></dl>
      {selected.source_run_id ? <p className="text-xs wrap-anywhere">{t('chat.sourceRun', { id: selected.source_run_id })}</p> : null}
      {selected.audit_warning ? <ChatNotice tone="warning">{t('chat.auditWarning')}</ChatNotice> : null}
      {selected.result_unknown ? <ChatNotice tone="warning">{t('chat.historicalUnknownContinuation')}</ChatNotice> : null}
      {selected.partial_content ? <details><summary>{t('chat.persistedPartial')}</summary><Markdown source={selected.partial_content} /></details> : null}
      <RunSteps steps={selected.steps} />
      {selected.status !== 'running' ? selected.steps.filter(step => step.kind === 'tool' && (step.result_unknown || step.status === 'dispatched')).map(step => <ManualResolution key={`${selected.task_id}:${step.id}`} run={selected} step={step} disabled={disabled || loading || !!selected.audit_warning} onResolved={updated => { setSelected(updated); setRuns(current => current.map(run => run.task_id === updated.task_id ? updated : run)); setConfirmed(false); }} />) : null}
      <p className="m-0 text-xs text-ink-secondary">{t('chat.runLimits')}</p>
      {blocked ? <ChatNotice tone="warning">{t('chat.continueBlocked')}</ChatNotice> : null}
      <Input multiline rows={2} label={t('chat.nextIntent')} value={intent} error={tooLong ? t('chat.intentTooLong') : undefined} onChange={event => { setIntent(selected.task_id, event.target.value); setConfirmed(false); }} />
      <Checkbox checked={confirmed} disabled={!!blocked || disabled} onChange={setConfirmed} label={t('chat.continueConfirm')} />
      <p className="m-0 text-xs text-ink-secondary">{t('chat.continueHelp')}</p>
      <Button type="button" disabled={!!blocked || disabled || !confirmed || !intent.trim() || tooLong} loading={loading} onClick={() => void continueSelected()}>{t('chat.startContinuation')}</Button>
    </> : null}
  </section>} />;
}
