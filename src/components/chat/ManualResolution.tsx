import { useLayoutEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input } from '@dgos/ui';
import { t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore, utf8Bytes, type ChatRun, type ChatRunStep } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { ChatNotice } from './ChatNotice';

export default function ManualResolution({ run, step, disabled, onResolved }: { run: ChatRun; step: ChatRunStep; disabled: boolean; onResolved: (run: ChatRun) => void }) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const draftKey = `resolution:${run.task_id}:${step.id}`;
  const note = useChatStore(state => state.runIntentDrafts[draftKey] ?? '');
  const setNote = useChatStore(state => state.setRunIntent);
  const resolve = useChatStore(state => state.resolveRunStep);
  const [outcome, setOutcome] = useState<'completed' | 'not_executed' | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  useLayoutEffect(() => {
    revision.current++; setOutcome(null); setConfirmed(false); setBusy(false); setError(null);
    return () => { revision.current++; };
  }, [account, run.session_id, run.task_id, step.id]);
  const tooLong = utf8Bytes(note) > 2048;
  const submit = async () => {
    if (disabled || busy || !confirmed || !outcome || !note.trim() || tooLong || run.audit_warning || !step.result_unknown || run.status === 'running' || useChatStore.getState().activeSessionId !== run.session_id || useDevelopment.getState().epoch !== account) return;
    const ticket = ++revision.current;
    setBusy(true); setError(null);
    try {
      const updated = await resolve(run.task_id, step.id, outcome, note);
      if (ticket !== revision.current || useDevelopment.getState().epoch !== account || useChatStore.getState().activeSessionId !== run.session_id) return;
      if (updated?.task_id === run.task_id && updated.session_id === run.session_id) { setNote(draftKey, ''); setConfirmed(false); onResolved(updated); }
      else setError(t('chat.activityResolutionNotSaved'));
    } catch (failure) { if (ticket === revision.current && useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === run.session_id) setError(chatErrorText(failure)); }
    finally { if (ticket === revision.current && useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === run.session_id) setBusy(false); }
  };
  return <details className="chat-activity-resolution"><summary>{t('chat.activityResolve', { step: step.ordinal + 1 })}</summary>
    <ChatNotice tone="warning">{t('chat.activityResolveHelp')}</ChatNotice>
    <div className="flex flex-wrap gap-2" role="group" aria-label={t('chat.activityOutcome')}>
      <Button type="button" size="sm" variant="secondary" disabled={disabled || busy} aria-pressed={outcome === 'completed'} onClick={() => { setOutcome('completed'); setConfirmed(false); }}>{t('chat.activityCompletedReport')}</Button>
      <Button type="button" size="sm" variant="secondary" disabled={disabled || busy} aria-pressed={outcome === 'not_executed'} onClick={() => { setOutcome('not_executed'); setConfirmed(false); }}>{t('chat.activityNotExecutedReport')}</Button>
    </div>
    <Input multiline rows={3} label={t('chat.activityEvidence')} value={note} error={tooLong ? t('chat.resolutionNoteTooLong') : undefined} disabled={disabled || busy} onChange={event => { setNote(draftKey, event.target.value); setConfirmed(false); }} />
    <Checkbox checked={confirmed} disabled={disabled || busy || !outcome || !note.trim() || tooLong} onChange={setConfirmed} label={t('chat.activityReportConfirm')} />
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
    <Button type="button" size="sm" disabled={disabled || !confirmed || !outcome || !note.trim() || tooLong} loading={busy} onClick={() => void submit()}>{t('chat.activitySaveReport')}</Button>
  </details>;
}
