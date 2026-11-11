import { useEffect, useMemo, useRef, useState, type ComponentProps } from 'react';
import { Button, Modal, Popover, Tooltip } from '@dgos/ui';
import { ArrowLeft, ChevronRight, CircleGauge, TriangleAlert } from 'lucide-react';
import { formatDateTime, formatNumber, t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore, type ChatContextBudget, type ChatSession, type ChatUsageSummary } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { fetchConnectedTools, toolKey } from './catalog';
import { ChatNotice } from './ChatNotice';
import { compactTokens, compareContextBudgets, contextUsage, usageFieldPartial } from './contextUsage';
import Markdown from './Markdown';
import RunInspector from './RunInspector';
import './contextUsage.css';

interface ContextPanelProps {
  session: ChatSession;
  content: string;
  toolKeys: string[];
  disabled: boolean;
  onOpenMaterials?: () => void;
  onOpenSkills?: () => void;
}
type BudgetSnapshot = { scope: string; key: string; value: ChatContextBudget; checkedAt: number };
type UsageSnapshot = { scope: string; key: string; value: ChatUsageSummary };
type ReadState = { key: string; status: 'loading' | 'ready' | 'error'; error?: string };
type SummaryAttempt = { scope: string; inputs: string; before: ChatContextBudget | null; summaryId: string | null; revision: number };
const summaryReasons: Record<string, string> = {
  no_history: 'chat.usageSummaryNoHistory', busy: 'chat.usageSummaryBusy', unresolved: 'chat.usageViewUnknownResult',
  model_unavailable: 'chat.usageViewChooseModel', context_too_long: 'chat.usageSummaryTooLong',
};

// Forward the Popover trigger props through the tooltip to the actual button.
function UsageTrigger(props: ComponentProps<typeof Button>) {
  return <Tooltip content={props['aria-label'] ?? ''}><Button {...props} /></Tooltip>;
}

export default function ContextPanel({ session, content, toolKeys, disabled, onOpenMaterials, onOpenSkills }: ContextPanelProps) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const surfaceActive = useSurfaceActive();
  const active = useChatStore(state => state.activeSessionId === session.id);
  const assets = useChatStore(state => state.sessionAssets[session.id]);
  const stale = useChatStore(state => state.sessionViews[session.id]?.stale);
  const history = useChatStore(state => state.sessionViews[session.id]?.messages);
  const skillAccess = useChatStore(state => state.sessionSkillAccess[session.id]);
  const foundation = useChatStore(state => state.sessionFoundation[session.id]);
  const resources = useChatStore(state => state.sessionResources[session.id]);
  const webAccess = useChatStore(state => state.sessionWebAccess[session.id]);
  const savedSummary = useChatStore(state => state.sessionSummaries[session.id]);
  const usageRevision = useChatStore(state => state.sessionUsageRevisions[session.id] ?? 0);
  const generation = useChatStore(state => state.generations[session.id]);
  const busy = useChatStore(state => !!state.preparingSessionId || Object.keys(state.generations).length > 0);
  const unknown = useChatStore(state => state.sessionUnknown[session.id]);
  const destinationPending = useChatStore(state => state.sessionPendingDestinations[session.id]?.kind === 'summary');
  const inspect = useChatStore(state => state.inspectContext);
  const inspectUsage = useChatStore(state => state.inspectUsage);
  const summarize = useChatStore(state => state.summarize);
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState(false);
  const [confirm, setConfirm] = useState<{ account: number; scope: string; sessionId: string } | null>(null);
  const [snapshot, setSnapshot] = useState<BudgetSnapshot | null>(null);
  const [usageSnapshot, setUsageSnapshot] = useState<UsageSnapshot | null>(null);
  const [budgetRead, setBudgetRead] = useState<ReadState | null>(null);
  const [usageRead, setUsageRead] = useState<ReadState | null>(null);
  const [catalogRevision, setCatalogRevision] = useState(0);
  const [budgetRefresh, setBudgetRefresh] = useState(0);
  const [usageRefresh, setUsageRefresh] = useState(0);
  const [attempt, setAttempt] = useState<SummaryAttempt | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [summaryRejected, setSummaryRejected] = useState(false);
  const budgetTicket = useRef(0);
  const usageTicket = useRef(0);
  const backButton = useRef<HTMLButtonElement>(null);
  const detailsButton = useRef<HTMLButtonElement>(null);
  const focusNavigation = useRef(false);
  const toolSignature = JSON.stringify(toolKeys);
  const scope = JSON.stringify([account, session.id, session.provider_id, session.model_id]);
  const usageScope = JSON.stringify([account, session.id]);
  // Deliberately excludes history and summary: only those may change in a valid before/after comparison.
  const inputs = JSON.stringify([scope, content, assets, session.skills, session.generation, toolKeys, skillAccess, foundation, resources, webAccess, catalogRevision]);
  const historyKey = useMemo(() => JSON.stringify(history?.map(message => [message.id, message.seq, message.status, message.content, message.updated_at, message.result_unknown, message.audit_warning])), [history]);
  const budgetKey = JSON.stringify([inputs, usageRevision, savedSummary?.id, !!unknown, historyKey, budgetRefresh]);
  const usageKey = JSON.stringify([scope, usageRevision, savedSummary?.id, usageRefresh]);
  const readable = surfaceActive && active && !stale;
  const eligible = readable && !disabled && !busy && !!session.provider_id && !!session.model_id;
  const budget = snapshot?.scope === scope ? snapshot.value : null;
  const actual = usageSnapshot?.scope === usageScope ? usageSnapshot.value : null;
  const fresh = snapshot?.scope === scope && snapshot.key === budgetKey;
  const loading = eligible && (budgetRead?.key !== budgetKey || budgetRead.status === 'loading');
  const error = budgetRead?.key === budgetKey && budgetRead.status === 'error' ? budgetRead.error : null;
  const usageError = usageRead?.key === usageKey && usageRead.status === 'error';
  const usageLoading = readable && (usageRead?.key !== usageKey || usageRead.status === 'loading');

  useEffect(() => {
    const invalidate = () => setCatalogRevision(value => value + 1);
    window.addEventListener('dgos:catalog-invalidated', invalidate);
    return () => window.removeEventListener('dgos:catalog-invalidated', invalidate);
  }, []);
  useEffect(() => { setOpen(false); setDetails(false); setConfirm(null); }, [scope, active, surfaceActive]);
  useEffect(() => { setAttempt(null); setSubmitting(false); setSummaryRejected(false); }, [scope]);
  useEffect(() => {
    if (!focusNavigation.current) return;
    focusNavigation.current = false;
    (details ? backButton : detailsButton).current?.focus({ preventScroll: true });
  }, [details]);
  useEffect(() => {
    const ticket = ++budgetTicket.current;
    if (!eligible) return;
    setBudgetRead({ key: budgetKey, status: 'loading' });
    const current = () => ticket === budgetTicket.current && useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === session.id;
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
          if (!current()) return;
          if (!value || value.model_id !== session.model_id) throw new Error('stale_context');
          setSnapshot({ scope, key: budgetKey, value, checkedAt: Date.now() });
          setBudgetRead({ key: budgetKey, status: 'ready' });
        } catch (failure) {
          if (current()) setBudgetRead({ key: budgetKey, status: 'error', error: failure instanceof Error && failure.message === 'tools_changed' ? t('chat.toolsChanged') : chatErrorText(failure) });
        }
      })();
    }, 350);
    return () => { window.clearTimeout(timer); budgetTicket.current++; };
  }, [eligible, account, session.id, session.model_id, scope, budgetKey, toolSignature, content, assets, inspect]);
  useEffect(() => {
    const ticket = ++usageTicket.current;
    if (!readable) return;
    setUsageRead({ key: usageKey, status: 'loading' });
    const current = () => ticket === usageTicket.current && useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === session.id;
    void (async () => {
      try {
        const value = await inspectUsage(session.id);
        if (!current()) return;
        if (!value) throw new Error('stale_usage');
        setUsageSnapshot({ scope: usageScope, key: usageKey, value });
        setUsageRead({ key: usageKey, status: 'ready' });
      } catch {
        if (current()) setUsageRead({ key: usageKey, status: 'error' });
      }
    })();
    return () => { usageTicket.current++; };
  }, [readable, account, session.id, usageScope, usageKey, inspectUsage]);

  const usage = budget ? contextUsage(budget) : null;
  const emptyDraft = !content.trim() && !assets?.length && !foundation?.context_refs.length;
  const needsContent = budget?.rejection === 'invalid_message_content' && emptyDraft;
  const rejected = !!budget && !budget.can_send && !needsContent;
  const warning = rejected || !!budget?.dropped_turns || !!usage?.nearLimit;
  const gaugeValue = usage?.percent == null ? t('chat.usageCapacityUnknown') : t('chat.usageViewPercent', { count: formatNumber(Math.round(usage.percent)) });
  const status = generation ? 'chat.usageLastCheck' : loading ? 'chat.usageViewChecking' : error ? 'chat.usageViewUnavailable' : !session.model_id || !session.provider_id ? 'chat.usageViewChooseModel' : rejected ? 'chat.usageViewBlocked' : warning ? 'chat.usageViewAttention' : 'chat.usageViewEstimate';
  const gaugeLabel = t('chat.usageViewGaugeLabel', { value: gaugeValue, status: t(status) });
  const summaryRunning = generation?.kind === 'summary';
  const summaryReason = unknown ? 'chat.usageViewUnknownResult' : busy || disabled ? 'chat.usageSummaryBusy' : !session.provider_id || !session.model_id ? 'chat.usageViewChooseModel' : loading ? 'chat.usageViewChecking' : !eligible || !fresh || error ? 'chat.usageSummaryUnavailable' : !budget?.can_summarize ? summaryReasons[budget?.summary_unavailable_reason ?? ''] ?? 'chat.usageSummaryUnavailable' : null;
  const summaryDisabled = submitting || destinationPending || !!summaryReason;
  const currentAttempt = attempt?.scope === scope ? attempt : null;
  const installed = !!currentAttempt && !!savedSummary && savedSummary.id !== currentAttempt.summaryId;
  const summaryFailed = !!currentAttempt && !installed && !summaryRunning && !submitting && !destinationPending && (summaryRejected || usageRevision > currentAttempt.revision);
  const refreshed = installed && fresh && usageSnapshot?.key === usageKey && !usageError;
  const comparison = installed && fresh && budget && currentAttempt?.before ? compareContextBudgets(currentAttempt.before, budget, currentAttempt.inputs === inputs) : null;
  const comparisonValue = (value: number) => comparison?.unit === 'percent' ? t('chat.usageViewPercent', { count: formatNumber(Math.round(value)) }) : t('chat.usageViewTokens', { count: formatNumber(value) });
  const tokens = (value: number | null | undefined) => value == null ? '—' : formatNumber(value);
  const metric = (value: number | null | undefined, field: keyof ChatUsageSummary['coverage'], unavailable = '—') => <>{value == null ? actual && actual.coverage[field] > 0 ? '—' : unavailable : formatNumber(value)}{actual && usageFieldPartial(actual, field) ? <small>{t('chat.usagePartial')}</small> : null}</>;
  const duration = actual?.duration_ms == null ? '—' : actual.duration_ms < 60_000 ? t('chat.usageSeconds', { count: formatNumber(Math.round(actual.duration_ms / 100) / 10) }) : t('chat.usageMinutesSeconds', { minutes: formatNumber(Math.floor(Math.round(actual.duration_ms / 1000) / 60)), seconds: formatNumber(Math.round(actual.duration_ms / 1000) % 60) });
  const summary = savedSummary ?? budget?.summary;
  const openRelated = (callback: () => void) => { setOpen(false); callback(); };
  const navigate = (next: boolean) => { focusNavigation.current = true; setDetails(next); };
  const summaryFeedback = summaryRunning || submitting ? t('chat.usageCompressing') : installed ? !refreshed ? t('chat.usageSummaryPending') : comparison ? comparison.lowered ? t('chat.usageSummaryReduced', { before: comparisonValue(comparison.before), after: comparisonValue(comparison.after) }) : t('chat.usageSummaryNotReduced') : t('chat.usageSummaryComplete') : summaryFailed ? t(unknown ? 'chat.usageSummaryUnknown' : 'chat.usageSummaryFailed') : null;

  return <>
    <Popover label={t('chat.usageViewTitle')} side="top" align="end" collisionPadding={16} open={open} onOpenChange={next => { setOpen(next); if (!next) setDetails(false); }} trigger={
      <UsageTrigger type="button" size="sm" variant="ghost" iconOnly className={`chat-usage-trigger${warning ? ' chat-usage-trigger-warning' : ''}${rejected ? ' chat-usage-trigger-error' : ''}`} aria-label={gaugeLabel}>
        <svg className="chat-usage-ring" width="21" height="21" viewBox="0 0 24 24" aria-hidden="true">
          <circle className="chat-usage-ring-track" cx="12" cy="12" r="9" />
          {usage?.percent != null ? <circle className="chat-usage-ring-value" cx="12" cy="12" r="9" pathLength="100" strokeDasharray={`${Math.min(100, Math.max(0, usage.percent))} 100`} /> : null}
        </svg>
        {rejected ? <span className="chat-usage-ring-blocked" aria-hidden="true">!</span> : null}
      </UsageTrigger>
    } content={<section className={`chat-usage-panel${rejected ? ' chat-usage-panel-error' : warning ? ' chat-usage-panel-warning' : ''}`} aria-busy={loading || usageLoading}>
      <header className="chat-usage-heading">
        {details ? <Button ref={backButton} type="button" size="sm" variant="ghost" iconOnly aria-label={t('chat.usageBack')} onClick={() => navigate(false)}><ArrowLeft size={16} aria-hidden="true" /></Button> : <CircleGauge size={17} aria-hidden="true" />}
        <h3>{t(details ? 'chat.usageDetails' : 'chat.usageViewTitle')}</h3>
      </header>
      {!details ? <>
        <section className="chat-usage-context" aria-label={t('chat.usageContextWindow')}>
          <div className="chat-usage-context-heading"><h4>{t('chat.usageContextWindow')}</h4><span className="chat-usage-estimate-badge">{t('chat.usageEstimated')}</span><strong>{usage?.percent == null ? '—' : gaugeValue}</strong></div>
          <div className="chat-usage-meter-track">{usage?.percent != null ? <progress className="chat-usage-meter" value={Math.min(usage.percent, 100)} max={100} aria-label={t('chat.usageViewEstimate')} aria-valuetext={gaugeValue} /> : <span className="chat-usage-meter-unknown" />}</div>
          <div className="chat-usage-capacity"><span>{usage ? compactTokens(usage.requested) : '—'} / {usage?.window ? t('chat.usageViewTokens', { count: compactTokens(usage.window) }) : t('chat.usageCapacityUnknown')}</span><Button type="button" size="sm" variant="secondary" disabled={summaryDisabled} aria-describedby={summaryReason ? 'chat-summary-reason' : undefined} onClick={() => setConfirm({ account, scope, sessionId: session.id })}>{t(summaryRunning || submitting ? 'chat.usageCompressing' : 'chat.usageViewSummarize')}</Button></div>
          {loading || generation ? <p className="chat-usage-snapshot-note" role="status">{t(generation ? 'chat.usageLastCheck' : 'chat.usageUpdating')}</p> : null}
          {rejected || budget?.dropped_turns || usage?.nearLimit ? <div className="chat-usage-context-notice"><TriangleAlert size={14} aria-hidden="true" /><span>{t(rejected ? budget?.rejection === 'fixed_context_exceeds_limit' ? 'chat.usageContentBlocked' : 'chat.usageViewBlocked' : budget?.dropped_turns ? 'chat.usageHistoryOmitted' : 'chat.usageNearLimit')}</span>{(rejected || usage?.nearLimit) && onOpenMaterials ? <Button type="button" size="sm" variant="ghost" onClick={() => openRelated(onOpenMaterials)}>{t('chat.usageViewMaterials')}</Button> : null}</div> : null}
          {summaryReason && !summaryRunning && !loading ? <p id="chat-summary-reason" className="chat-usage-snapshot-note">{t(summaryReason)}</p> : null}
          {error ? <ChatNotice tone="error">{t('chat.usageViewUnavailable')}<Button type="button" size="sm" variant="ghost" disabled={!eligible} onClick={() => setBudgetRefresh(value => value + 1)}>{t('chat.usageViewRetry')}</Button></ChatNotice> : null}
        </section>
        <section className="chat-usage-session" aria-label={t('chat.usageSession')}>
          <div className="chat-usage-scope"><h4>{t('chat.usageSession')}</h4>{actual?.partial || actual?.legacy ? <span>{t('chat.usagePartiallyReported')}</span> : actual?.call_count === 0 ? <span>{t('chat.usageNoCalls')}</span> : null}</div>
          <dl className="chat-usage-values chat-usage-stats">
            <div><dt>{t('chat.usageTotal')}</dt><dd>{metric(actual?.total_tokens, 'total')}</dd></div>
            <div><dt>{t('chat.usageInput')}</dt><dd>{metric(actual?.input_tokens, 'input')}</dd></div>
            <div className="chat-usage-cache-row"><dt>{t('chat.usageCacheRead')}</dt><dd>{metric(actual?.cache_read_tokens, 'cache_read', actual ? t('chat.usageNotProvided') : '—')}</dd></div>
            <div><dt>{t('chat.usageOutput')}</dt><dd>{metric(actual?.output_tokens, 'output')}</dd></div>
            <div><dt>{t('chat.usageCacheHit')}</dt><dd>{actual?.cache_hit_rate == null ? '—' : t('chat.usageViewPercent', { count: formatNumber(Math.round(actual.cache_hit_rate * 1000) / 10) })}</dd></div>
            <div><dt>{t('chat.usageDuration')}</dt><dd>{duration}{actual?.duration_ms != null && usageFieldPartial(actual, 'duration') ? <small>{t('chat.usagePartial')}</small> : null}</dd></div>
          </dl>
          {usageError ? <p className="chat-usage-read-error" role="status">{t('chat.usageUnavailable')} <Button type="button" size="sm" variant="ghost" onClick={() => setUsageRefresh(value => value + 1)}>{t('chat.retry')}</Button></p> : null}
        </section>
        {summaryFeedback ? <div className="chat-usage-summary-feedback" role="status">{summaryFeedback}{summaryFailed ? <RunInspector sessionId={session.id} /> : null}</div> : null}
        <Button ref={detailsButton} type="button" size="sm" variant="ghost" className="chat-usage-details-link" onClick={() => navigate(true)}>{t('chat.usageViewDetails')}<ChevronRight size={15} aria-hidden="true" /></Button>
      </> : <>
        <section className="chat-usage-section"><h4>{t('chat.usageViewEstimate')}</h4><dl className="chat-usage-values">
          <div><dt>{t('chat.usageViewInput')}</dt><dd>{tokens(budget?.estimated_input_tokens)}</dd></div>
          <div><dt>{t('chat.usageViewOutput')}</dt><dd>{tokens(budget?.reserved_output_tokens)}</dd></div>
          <div><dt>{t('chat.modelWindow')}</dt><dd>{tokens(usage?.window)}</dd></div>
          <div><dt>{t('chat.usageModelSource')}</dt><dd>{budget?.model_id ?? '—'}</dd></div>
          <div><dt>{t('chat.usageCheckedAt')}</dt><dd>{snapshot?.scope === scope ? formatDateTime(snapshot.checkedAt) : '—'}</dd></div>
        </dl><p>{t('chat.usageEstimateHelp')}</p>{loading || generation || !fresh ? <p>{t(generation ? 'chat.usageLastCheck' : 'chat.usageUpdating')}</p> : null}{error ? <ChatNotice tone="error">{error}</ChatNotice> : null}</section>
        <section className="chat-usage-section"><h4>{t('chat.usageSession')}</h4><p>{t('chat.usageSessionHelp')}</p>{actual ? <>
          <p>{t('chat.usageCoverage', { reported: formatNumber(actual.reported_count), calls: formatNumber(actual.call_count), pending: formatNumber(actual.pending_count) })}</p>
          {actual.legacy ? <p>{t('chat.usageLegacy')}</p> : null}
          <dl className="chat-usage-values">{(['total', 'input', 'output', 'cache_read', 'duration'] as const).map(field => <div key={field}><dt>{t(`chat.usageCoverage_${field}`)}</dt><dd>{t('chat.usageCoverageCount', { count: formatNumber(actual.coverage[field]), calls: formatNumber(actual.call_count) })}</dd></div>)}
            {actual.cache_write_tokens !== null ? <div><dt>{t('chat.usageCacheWrite')}</dt><dd>{metric(actual.cache_write_tokens, 'cache_write')}</dd></div> : null}
            {actual.reasoning_tokens !== null ? <div><dt>{t('chat.usageReasoning')}</dt><dd>{metric(actual.reasoning_tokens, 'reasoning')}</dd></div> : null}
          </dl><p>{t('chat.usageCacheHelp')}</p>
          {actual.latest ? <div className="chat-usage-latest"><h4>{t('chat.usageLatest')}</h4><p>{actual.latest.model_id}{actual.latest.created_at !== null ? ` · ${formatDateTime(actual.latest.created_at)}` : ''}{actual.latest.partial ? ` · ${t('chat.usagePartiallyReported')}` : ''}</p><dl className="chat-usage-values">
            <div><dt>{t('chat.usageTotal')}</dt><dd>{tokens(actual.latest.total_tokens)}</dd></div>
            <div><dt>{t('chat.usageInput')}</dt><dd>{tokens(actual.latest.input_tokens)}</dd></div>
            <div><dt>{t('chat.usageOutput')}</dt><dd>{tokens(actual.latest.output_tokens)}</dd></div>
            <div><dt>{t('chat.usageCacheRead')}</dt><dd>{tokens(actual.latest.cache_read_tokens)}</dd></div>
            <div><dt>{t('chat.usageDuration')}</dt><dd>{actual.latest.duration_ms === null ? '—' : t('chat.usageSeconds', { count: formatNumber(Math.round(actual.latest.duration_ms / 100) / 10) })}</dd></div>
          </dl></div> : null}
        </> : <p>{t(usageError ? 'chat.usageUnavailable' : 'chat.usageViewUnknown')}</p>}</section>
        <section className="chat-usage-section"><h4>{t('chat.usageSummaryRecord')}</h4>{summary ? <><p>{formatDateTime(summary.created_at * 1000)}</p><details><summary>{t('chat.summarySource', { first: summary.source_first_seq, last: summary.source_last_seq })}</summary><Markdown source={summary.content} /></details><p>{t('chat.summaryOriginalsKept')}</p></> : <p>{t('chat.noSummary')}</p>}{summaryFeedback ? <p role="status">{summaryFeedback}</p> : null}{summaryFailed || unknown ? <RunInspector sessionId={session.id} /> : null}</section>
        <details className="chat-usage-debug"><summary>{t('chat.usageViewTechnical')}</summary>{budget ? <><dl className="chat-usage-values">{([
          ['chat.hostTextLimit', budget.host_text_limit_bytes], ['chat.contextTextBytes', budget.text_bytes], ['chat.contextHistoryBytes', budget.history_bytes], ['chat.contextSkillBytes', budget.skill_bytes], ['chat.contextToolBytes', budget.tool_schema_bytes], ['chat.usageViewImageIncluded', budget.image_reserve_tokens],
        ] as const).map(([label, value]) => <div key={label}><dt>{t(label)}</dt><dd>{formatNumber(value)}</dd></div>)}</dl><p>{t('chat.usageViewEstimateMethod')}</p><p>{t('chat.usageViewByteHelp')}</p>{budget.rejection ? <p>{t('chat.contextRejectionCode', { code: budget.rejection })}</p> : null}{onOpenSkills ? <Button type="button" size="sm" variant="ghost" onClick={() => openRelated(onOpenSkills)}>{t('chat.usageViewSkills')}</Button> : null}</> : <p>{t('chat.usageViewUnavailable')}</p>}</details>
      </>}
    </section>} />
    <Modal open={confirm !== null} title={t('chat.usageViewSummaryConfirm')} icon="warning" description={t('chat.usageViewSummaryConfirmHelp', { model: session.model_id ?? '' })} confirmText={t('chat.usageConfirmCompress')} onOpenChange={next => { if (!next) setConfirm(null); }} onConfirm={async () => {
      if (!confirm || summaryDisabled || confirm.scope !== scope || useDevelopment.getState().epoch !== confirm.account || useChatStore.getState().activeSessionId !== confirm.sessionId || confirm.sessionId !== session.id) return;
      setConfirm(null);
      setAttempt({ scope, inputs, before: fresh ? budget : null, summaryId: savedSummary?.id ?? budget?.summary?.id ?? null, revision: usageRevision });
      setSummaryRejected(false);
      setSubmitting(true);
      const current = () => useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === session.id && useChatStore.getState().activeSession?.provider_id === session.provider_id && useChatStore.getState().activeSession?.model_id === session.model_id;
      try { const outcome = await summarize(); if (current() && outcome === 'rejected') setSummaryRejected(true); }
      finally { if (current()) setSubmitting(false); }
    }} />
  </>;
}
