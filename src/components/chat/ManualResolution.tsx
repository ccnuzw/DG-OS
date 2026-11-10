import { useEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input } from '@dgos/ui';
import { t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore, utf8Bytes, type ChatRun, type ChatRunStep } from '../../stores/chatStore';
import { ChatNotice } from './ChatNotice';

export default function ManualResolution({ run, step, disabled, onResolved }: { run: ChatRun; step: ChatRunStep; disabled: boolean; onResolved: (run: ChatRun) => void }) {
  useLocale();
  const draftKey = `resolution:${run.task_id}:${step.id}`;
  const note = useChatStore(state => state.runIntentDrafts[draftKey] ?? '');
  const setNote = useChatStore(state => state.setRunIntent);
  const resolve = useChatStore(state => state.resolveRunStep);
  const [outcome, setOutcome] = useState<'completed' | 'not_executed' | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  const tooLong = utf8Bytes(note) > 2048;
  const submit = async () => {
    if (disabled || busy || !confirmed || !outcome || !note.trim() || tooLong) return;
    setBusy(true); setError(null);
    try {
      const updated = await resolve(run.task_id, step.id, outcome, note);
      if (live.current && updated) { setNote(draftKey, ''); setConfirmed(false); onResolved(updated); }
    } catch (failure) { if (live.current) setError(chatErrorText(failure)); }
    finally { if (live.current) setBusy(false); }
  };
  return <details className="space-y-2"><summary>{t('chat.manualResolution', { step: step.ordinal })}</summary>
    <ChatNotice tone="warning">{t('chat.manualResolutionHelp')}</ChatNotice>
    <div className="flex flex-wrap gap-2" role="group" aria-label={t('chat.manualOutcome')}>
      <Button type="button" size="sm" variant="secondary" disabled={disabled || busy} aria-pressed={outcome === 'completed'} onClick={() => { setOutcome('completed'); setConfirmed(false); }}>{t('chat.manualCompleted')}</Button>
      <Button type="button" size="sm" variant="secondary" disabled={disabled || busy} aria-pressed={outcome === 'not_executed'} onClick={() => { setOutcome('not_executed'); setConfirmed(false); }}>{t('chat.manualNotExecuted')}</Button>
    </div>
    <Input multiline rows={2} label={t('chat.manualEvidenceNote')} value={note} error={tooLong ? t('chat.resolutionNoteTooLong') : undefined} disabled={disabled || busy} onChange={event => { setNote(draftKey, event.target.value); setConfirmed(false); }} />
    <Checkbox checked={confirmed} disabled={disabled || busy || !outcome || !note.trim() || tooLong} onChange={setConfirmed} label={t('chat.manualResolutionConfirm')} />
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
    <Button type="button" size="sm" disabled={disabled || !confirmed || !outcome || !note.trim() || tooLong} loading={busy} onClick={() => void submit()}>{t('chat.recordManualResolution')}</Button>
  </details>;
}
