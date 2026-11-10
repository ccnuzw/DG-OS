import { useEffect, useMemo, useRef, useState } from 'react';
import { Button, Modal, Popover } from '@dgos/ui';
import { CircleGauge, TriangleAlert } from 'lucide-react';
import { formatNumber, t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore, type ChatContextBudget, type ChatSession } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { fetchConnectedTools, toolKey } from './catalog';
import { ChatNotice } from './ChatNotice';
import { contextUsage, reportedUsage } from './contextUsage';
import Markdown from './Markdown';
import './contextUsage.css';

interface ContextPanelProps {
  session: ChatSession;
  content: string;
  toolKeys: string[];
  disabled: boolean;
  onOpenMaterials?: () => void;
  onOpenSkills?: () => void;
}

export default function ContextPanel({ session, content, toolKeys, disabled, onOpenMaterials, onOpenSkills }: ContextPanelProps) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const surfaceActive = useSurfaceActive();
  const active = useChatStore(state => state.activeSessionId === session.id);
  const assets = useChatStore(state => state.sessionAssets[session.id]);
  const view = useChatStore(state => state.sessionViews[session.id]);
  const skillAccess = useChatStore(state => state.sessionSkillAccess[session.id]);
  const foundation = useChatStore(state => state.sessionFoundation[session.id]);
  const resources = useChatStore(state => state.sessionResources[session.id]);
  const savedSummary = useChatStore(state => state.sessionSummaries[session.id]);
  const unknown = useChatStore(state => state.sessionUnknown[session.id]);
  const inspect = useChatStore(state => state.inspectContext);
  const summarize = useChatStore(state => state.summarize);
  const [open, setOpen] = useState(false);
  const [confirm, setConfirm] = useState<{ account: number; sessionId: string } | null>(null);
  const [budget, setBudget] = useState<ChatContextBudget | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [catalogRevision, setCatalogRevision] = useState(0);
  const revision = useRef(0);
  const toolSignature = JSON.stringify(toolKeys);
  const sessionSignature = JSON.stringify([session.provider_id, session.model_id, session.skills, session.generation]);
  const messages = view?.messages;
  const eligible = !disabled && surfaceActive && active && !view?.stale && !!session.provider_id && !!session.model_id;
  const actual = useMemo(() => reportedUsage(messages), [messages]);

  useEffect(() => {
    const invalidate = () => setCatalogRevision(value => value + 1);
    window.addEventListener('dgos:catalog-invalidated', invalidate);
    return () => window.removeEventListener('dgos:catalog-invalidated', invalidate);
  }, []);
  useEffect(() => { setOpen(false); setConfirm(null); }, [account, session.id, sessionSignature, active, surfaceActive, disabled]);
  useEffect(() => {
    const ticket = ++revision.current;
    setBudget(null);
    setError(null);
    setLoading(eligible);
    if (!eligible) return;
    const current = () => ticket === revision.current && useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === session.id;
    // Inspection resolves host-owned context only. It never dispatches a model request.
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const keys: string[] = JSON.parse(toolSignature);
          const available = keys.length ? await fetchConnectedTools() : [];
          if (!current()) return;
          const selected = keys.map(key => available.find(tool => toolKey(tool) === key));
          if (selected.some(tool => !tool)) throw new Error('tools_changed');
          const value = await inspect(session.id, content, selected.filter(tool => !!tool), assets?.map(asset => asset.id));
          if (current()) setBudget(value);
        } catch (failure) {
          if (current()) setError(failure instanceof Error && failure.message === 'tools_changed' ? t('chat.toolsChanged') : chatErrorText(failure));
        } finally {
          if (current()) setLoading(false);
        }
      })();
    }, 350);
    return () => { window.clearTimeout(timer); revision.current++; };
  }, [eligible, disabled, session.id, sessionSignature, content, assets, messages, toolSignature, account, savedSummary, skillAccess, foundation, resources, catalogRevision, inspect]);

  const usage = budget ? contextUsage(budget) : null;
  const emptyDraft = !content.trim() && !assets?.length && !foundation?.context_refs.length;
  const needsContent = budget?.rejection === 'invalid_message_content' && emptyDraft;
  const rejected = !!budget && !budget.can_send && !needsContent;
  const warning = rejected || !!budget?.dropped_turns || !!usage?.nearLimit;
  const status = disabled ? 'chat.usageViewPaused' : loading ? 'chat.usageViewChecking' : error ? 'chat.usageViewUnavailable' : !session.model_id || !session.provider_id ? 'chat.usageViewChooseModel' : needsContent ? 'chat.usageViewNeedsContent' : rejected ? 'chat.usageViewBlocked' : warning ? 'chat.usageViewAttention' : budget ? 'chat.usageViewReady' : 'chat.usageViewUnknown';
  const gaugeValue = usage?.percent === null || !usage ? t('chat.usageViewUnknownShort') : t('chat.usageViewPercent', { count: formatNumber(Math.round(usage.percent)) });
  const unknownValue = t('chat.usageViewUnknown');
  const tokens = (value: number | null) => value === null ? unknownValue : t('chat.usageViewTokens', { count: formatNumber(value) });
  const summaryDisabled = disabled || !eligible || !!unknown;
  const openRelated = (callback: () => void) => { setOpen(false); callback(); };

  return <>
    <Popover label={t('chat.usageViewTitle')} side="top" align="end" open={open} onOpenChange={setOpen} trigger={
      <Button type="button" size="sm" variant="ghost" className={`chat-usage-trigger${warning ? ' chat-usage-trigger-warning' : ''}${rejected ? ' chat-usage-trigger-error' : ''}`} aria-label={t('chat.usageViewGaugeLabel', { value: gaugeValue, status: t(status) })} title={t('chat.usageViewGaugeLabel', { value: gaugeValue, status: t(status) })}>
        {warning ? <TriangleAlert size={15} aria-hidden="true" /> : <CircleGauge size={15} aria-hidden="true" />}
        <span>{t('chat.usageViewGauge')}</span><span className="chat-usage-trigger-value">{loading ? '…' : gaugeValue}</span>
      </Button>
    } content={<section className="chat-usage-panel" aria-busy={loading}>
      <header className="chat-usage-heading"><h3>{t('chat.usageViewTitle')}</h3><span role="status">{t(status)}</span></header>
      <section className="chat-usage-section">
        <h4>{t('chat.usageViewEstimate')}</h4>
        <p>{t('chat.usageViewEstimateHelp')}</p>
        <div className="chat-usage-estimate"><strong>{tokens(usage?.requested ?? null)}</strong><span>{usage?.window ? t('chat.usageViewWindow', { count: formatNumber(usage.window) }) : t('chat.usageViewWindowUnknown')}</span></div>
        {usage?.percent !== null && usage ? <progress className="chat-usage-meter" value={Math.min(usage.percent, 100)} max={100} aria-label={t('chat.usageViewEstimate')} aria-valuetext={gaugeValue} /> : null}
        <dl className="chat-usage-values">
          <div><dt>{t('chat.usageViewInput')}</dt><dd>{tokens(budget?.estimated_input_tokens ?? null)}</dd></div>
          <div><dt>{t('chat.usageViewOutput')}</dt><dd>{tokens(budget?.reserved_output_tokens ?? null)}</dd></div>
          <div><dt>{t('chat.usageViewImageIncluded')}</dt><dd>{tokens(budget?.image_reserve_tokens ?? null)}</dd></div>
        </dl>
        <p>{t('chat.usageViewEstimateMethod')}</p>
        {budget ? <p className="chat-usage-host-cap">{t('chat.usageViewHostCap', { used: formatNumber(budget.text_bytes), limit: formatNumber(budget.host_text_limit_bytes) })}</p> : null}
        {needsContent ? <p>{t('chat.usageViewAddContent')}</p> : null}
        {error ? <ChatNotice tone="error">{error} {t('chat.usageViewInspectError')}<span className="chat-usage-actions"><Button type="button" size="sm" variant="secondary" disabled={!eligible || loading} onClick={() => setCatalogRevision(value => value + 1)}>{t('chat.usageViewRetry')}</Button></span></ChatNotice> : null}
        {warning ? <ChatNotice tone={rejected ? 'error' : 'warning'}>
          {rejected ? t('chat.usageViewRejectedHelp') : usage?.nearLimit ? t('chat.usageViewNearLimitHelp') : null}
          {budget && budget.dropped_turns > 0 ? <> {t('chat.usageViewDropped', { count: formatNumber(budget.dropped_turns) })}</> : null}
          <span className="chat-usage-actions">
            {onOpenMaterials ? <Button type="button" size="sm" variant="ghost" disabled={disabled} onClick={() => openRelated(onOpenMaterials)}>{t('chat.usageViewMaterials')}</Button> : null}
            {onOpenSkills ? <Button type="button" size="sm" variant="ghost" disabled={disabled} onClick={() => openRelated(onOpenSkills)}>{t('chat.usageViewSkills')}</Button> : null}
            <Button type="button" size="sm" variant="ghost" disabled={summaryDisabled} onClick={() => setConfirm({ account, sessionId: session.id })}>{t('chat.usageViewSummarize')}</Button>
          </span>
        </ChatNotice> : null}
      </section>
      <section className="chat-usage-section">
        <h4>{t('chat.usageViewActual')}</h4>
        <dl className="chat-usage-values">
          <div><dt>{t('chat.usageViewLatest')}</dt><dd>{tokens(actual.latest)}</dd></div>
          <div><dt>{t('chat.usageViewLoadedSubtotal')}</dt><dd>{tokens(actual.total)}</dd></div>
        </dl>
        <p>{t('chat.usageViewLoadedScope', { reported: formatNumber(actual.reported), count: formatNumber(actual.count), missing: formatNumber(actual.missing) })}</p>
        <p>{t('chat.usageViewActualHelp')}</p>
      </section>
      <section className="chat-usage-section">
        <div className="chat-usage-heading"><h4>{t('chat.usageViewSummary')}</h4><Button type="button" size="sm" variant="secondary" disabled={summaryDisabled} onClick={() => setConfirm({ account, sessionId: session.id })}>{t('chat.usageViewSummarize')}</Button></div>
        <p>{t('chat.usageViewSummaryHelp')}</p>
        {unknown ? <p>{t('chat.usageViewUnknownResult')}</p> : null}
        {budget?.summary ? <details><summary>{t('chat.summarySource', { first: budget.summary.source_first_seq, last: budget.summary.source_last_seq })}</summary><Markdown source={budget.summary.content} /></details> : <p>{t('chat.noSummary')}</p>}
      </section>
      {budget ? <details className="chat-usage-debug"><summary>{t('chat.usageViewTechnical')}</summary>
        <dl className="chat-usage-values">{([
          ['chat.contextTextBytes', budget.text_bytes], ['chat.contextHistoryBytes', budget.history_bytes], ['chat.contextSkillBytes', budget.skill_bytes], ['chat.contextToolBytes', budget.tool_schema_bytes],
        ] as const).map(([label, value]) => <div key={label}><dt>{t(label)}</dt><dd>{formatNumber(value)}</dd></div>)}</dl>
        <p>{t('chat.usageViewByteHelp')}</p>
        {budget.rejection ? <p>{t('chat.contextRejectionCode', { code: budget.rejection })}</p> : null}
        {budget.summary ? <p>{budget.summary.model_id} · {budget.summary.source_digest}</p> : null}
      </details> : null}
    </section>} />
    <Modal open={confirm !== null} title={t('chat.usageViewSummaryConfirm')} icon="warning" description={t('chat.usageViewSummaryConfirmHelp', { model: session.model_id ?? '' })} confirmText={t('chat.confirmPaidSummary')} onOpenChange={next => { if (!next) setConfirm(null); }} onConfirm={async () => {
      if (!confirm || summaryDisabled || useDevelopment.getState().epoch !== confirm.account || useChatStore.getState().activeSessionId !== confirm.sessionId || confirm.sessionId !== session.id) return;
      setConfirm(null);
      setOpen(false);
      await summarize();
    }} />
  </>;
}
