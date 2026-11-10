import { useEffect, useRef, useState } from 'react';
import { Button, Modal, Popover } from '@dgos/ui';
import { Layers } from 'lucide-react';
import { formatNumber, t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore, type ChatContextBudget, type ChatSession } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { fetchConnectedTools, toolKey } from './catalog';
import { ChatNotice } from './ChatNotice';
import Markdown from './Markdown';
import FoundationControls from './FoundationControls';

export default function ContextPanel({ session, content, toolKeys, disabled }: { session: ChatSession; content: string; toolKeys: string[]; disabled: boolean }) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const assets = useChatStore(state => state.sessionAssets[session.id]);
  const messages = useChatStore(state => state.sessionViews[session.id]?.messages);
  const skillAccess = useChatStore(state => state.sessionSkillAccess[session.id]);
  const foundation = useChatStore(state => state.sessionFoundation[session.id]);
  const resources = useChatStore(state => state.sessionResources[session.id]);
  const savedSummary = useChatStore(state => state.sessionSummaries[session.id]);
  const inspect = useChatStore(state => state.inspectContext);
  const summarize = useChatStore(state => state.summarize);
  const [open, setOpen] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [budget, setBudget] = useState<ChatContextBudget | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const toolSignature = JSON.stringify(toolKeys);
  useEffect(() => { revision.current++; setBudget(null); setError(null); setLoading(false); setConfirm(false); }, [session, content, assets, messages, toolSignature, account, savedSummary, skillAccess, foundation, resources]);
  useEffect(() => () => { revision.current++; }, []);
  const load = async () => {
    const ticket = ++revision.current;
    setLoading(true); setError(null);
    try {
      const available = toolKeys.length ? await fetchConnectedTools() : [];
      const selected = toolKeys.map(key => available.find(tool => toolKey(tool) === key));
      if (selected.some(tool => !tool)) throw new Error('tools_changed');
      const value = await inspect(session.id, content, selected.filter(tool => !!tool));
      if (ticket === revision.current) setBudget(value);
    } catch (failure) { if (ticket === revision.current) setError(failure instanceof Error && failure.message === 'tools_changed' ? t('chat.toolsChanged') : chatErrorText(failure)); }
    finally { if (ticket === revision.current) setLoading(false); }
  };
  const fields: [string, number | null][] = budget ? [
    ['chat.modelWindow', budget.model_window_tokens], ['chat.estimatedInput', budget.estimated_input_tokens],
    ['chat.outputReserve', budget.reserved_output_tokens], ['chat.imageReserve', budget.image_reserve_tokens],
    ['chat.hostTextLimit', budget.host_text_limit_bytes], ['chat.contextTextBytes', budget.text_bytes],
    ['chat.contextHistoryBytes', budget.history_bytes], ['chat.contextSkillBytes', budget.skill_bytes], ['chat.contextToolBytes', budget.tool_schema_bytes],
  ] : [];
  return <>
    <Popover label={t('chat.contextBudget')} side="top" align="start" open={open} onOpenChange={next => { setOpen(next); if (next) void load(); }} trigger={<Button type="button" size="sm" variant="ghost" iconOnly aria-label={t('chat.contextBudget')} title={t('chat.contextBudget')}><Layers size={16} aria-hidden="true" /></Button>} content={<section className="chat-inspector">
      <h3 className="m-0 text-sm font-semibold">{t('chat.contextBudget')}</h3>
      <p className="m-0 text-xs text-ink-secondary">{t('chat.contextEstimateHelp')}</p>
      <FoundationControls key={`${account}:${session.id}`} sessionId={session.id} disabled={disabled} />
      <Button type="button" variant="secondary" size="sm" loading={loading} disabled={disabled} onClick={() => void load()}>{t('chat.inspectContext')}</Button>
      {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
      {budget ? <>
        <dl className="chat-budget-grid">{fields.map(([label, value]) => <div key={label}><dt>{t(label)}</dt><dd>{value === null ? t('chat.limitUnknown') : formatNumber(value)}</dd></div>)}</dl>
        {budget.model_window_tokens === null ? <p className="text-xs text-status-warning-text">{t('chat.unknownWindowHelp')}</p> : null}
        {budget.dropped_turns > 0 ? <ChatNotice tone="warning">{t('chat.omitted', { count: formatNumber(budget.dropped_turns) })}</ChatNotice> : null}
        <p role="status" className={budget.can_send ? 'text-xs text-ink-secondary' : 'text-xs text-status-error-text'}>{t(budget.can_send ? 'chat.contextReady' : 'chat.contextRejected')}</p>
        {budget.rejection ? <p className="m-0 text-xs wrap-anywhere">{t('chat.contextRejectionCode', { code: budget.rejection })}</p> : null}
        {budget.summary ? <details><summary>{t('chat.summarySource', { first: budget.summary.source_first_seq, last: budget.summary.source_last_seq })}</summary><p className="text-xs wrap-anywhere">{budget.summary.model_id} · {budget.summary.source_digest}</p><Markdown source={budget.summary.content} /></details> : <p className="text-xs text-ink-secondary">{t('chat.noSummary')}</p>}
      </> : <p className="m-0 text-xs text-ink-secondary">{t('chat.contextRefreshHelp')}</p>}
      <Button type="button" size="sm" variant="secondary" disabled={disabled || !session.provider_id || !session.model_id} onClick={() => setConfirm(true)}>{t('chat.summarizeContext')}</Button>
      <p className="m-0 text-xs text-ink-secondary">{t('chat.summaryOriginalsKept')}</p>
    </section>} />
    <Modal open={confirm} title={t('chat.summaryConfirmTitle')} icon="warning" description={t('chat.summaryConfirmHelp', { model: session.model_id ?? '' })} confirmText={t('chat.confirmPaidSummary')} onOpenChange={setConfirm} onConfirm={async () => { if (disabled || useChatStore.getState().activeSessionId !== session.id) return; setConfirm(false); setOpen(false); await summarize(); }} />
  </>;
}
