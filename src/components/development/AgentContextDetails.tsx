import { useEffect } from 'react';
import { Minimize2, RefreshCw } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { useAgentRunner, type AgentSession } from '../../stores/agentRunnerStore';
import { codexOperationsKey, emptyCodexOperations, useCodexOperations } from '../../stores/codexOperations';
import type { CodexOperationScope } from './codexOperationsService';
import type { AgentContextUsage } from './agentContextUsage';
import './agent-context-details.css';

export function AgentContextDetails({ usage, session, disabled = false }: { usage: AgentContextUsage; session?: AgentSession; disabled?: boolean }) {
  const { t, locale } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const accountKey = useDevelopment(state => state.workspaceOwner);
  const metadata = useDevelopment(state => state.sessions.find(item => item.id === session?.id));
  const project = useDevelopment(state => state.projects.find(item => item.id === metadata?.project_id));
  const scope: CodexOperationScope | null = session && accountKey && metadata?.owner && project?.owner === metadata.owner && !project.archived
    ? { epoch, accountKey, owner: metadata.owner, projectId: project.id, taskId: metadata.task_id, sessionId: session.id, generation: session.generation } : null;
  const number = new Intl.NumberFormat(locale);
  const percent = usage.used !== null && usage.limit !== null ? usage.used / usage.limit : null;
  const totals = usage.cumulative;
  const rows = [
    ['total', totals?.total], ['input', totals?.input], ['output', totals?.output], ['cacheRead', totals?.cached],
  ] as const;
  return <section className="agent-usage" aria-label={t('agentUsage.title')} data-testid="agent-context-details">
    <header className="agent-usage-heading"><h3>{t('agentUsage.title')}</h3><span>{percent === null ? '—' : new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: 1 }).format(percent)}</span></header>
    <div className="agent-usage-progress"><progress aria-label={t('agentUsage.context')} aria-valuetext={percent === null ? '—' : undefined} max={usage.limit ?? 1} value={percent === null ? undefined : Math.min(usage.used!, usage.limit!)} /><span>{usage.used === null ? '—' : number.format(usage.used)} / {usage.limit === null ? '—' : number.format(usage.limit)}</span></div>
    {session && scope ? <Compression key={codexOperationsKey(scope)} session={session} scope={scope} disabled={disabled} /> : <button type="button" disabled title={t('agentUsage.unavailable')}><Minimize2 size={14} aria-hidden="true" />{t('agentUsage.compress')}</button>}
    <dl className="agent-usage-totals">
      {rows.map(([key, value]) => <div key={key}><dt>{t(`agentUsage.${key}`)}</dt><dd>{value == null ? '—' : number.format(value)}</dd></div>)}
      <div><dt>{t('agentUsage.cacheHit')}</dt><dd>{usage.cacheHit === null ? '—' : new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: 1 }).format(usage.cacheHit)}</dd></div>
      <div><dt title={usage.costEstimated ? t('agentUsage.costEstimateHint') : undefined}>{t(usage.costEstimated ? 'agentUsage.estimatedCost' : 'agentUsage.cost')}</dt><dd>{usage.costUsd === null ? '—' : new Intl.NumberFormat(locale, { style: 'currency', currency: 'USD', maximumFractionDigits: 6 }).format(usage.costUsd)}</dd></div>
      <div><dt>{t('agentUsage.duration')}</dt><dd>{usage.durationMs === null ? '—' : t('agentUsage.seconds', { count: number.format(usage.durationMs / 1000) })}</dd></div>
    </dl>
  </section>;
}

function Compression({ session, scope, disabled }: { session: AgentSession; scope: CodexOperationScope; disabled: boolean }) {
  const { t } = useLocale();
  const state = useCodexOperations(store => store.entries[codexOperationsKey(scope)] ?? emptyCodexOperations);
  const store = useCodexOperations.getState();
  useEffect(() => {
    if (!state.reading && !state.busy && state.compressionSupported === null && !state.error) void useCodexOperations.getState().observeCompression(scope);
  }, [scope.epoch, scope.accountKey, scope.owner, scope.projectId, scope.taskId, scope.sessionId, scope.generation, state.reading, state.busy, state.compressionSupported, state.error]);
  const idle = session.status === 'idle' && !session.active_run && !session.pending.length && !session.persistence_error;
  const blocked = disabled || !idle || state.busy || state.reading || !!state.pending || state.compressionSupported !== true;
  const refresh = async () => {
    await store.observeCompression(scope);
    const development = useDevelopment.getState(), runner = useAgentRunner.getState();
    if (development.epoch === scope.epoch && development.workspaceOwner === scope.accountKey && runner.selected === scope.sessionId && runner.session?.generation === scope.generation) await runner.attach(scope.sessionId);
  };
  const compress = async () => {
    if (blocked) return;
    const receipt = await store.act(scope, { kind: 'compact' });
    if (receipt?.state === 'applied') await refresh();
  };
  const inspect = async () => {
    const receipt = await store.inspect(scope);
    if (receipt?.state === 'applied') await refresh();
  };
  const error = state.error === 'codex_operation_unsupported' ? 'unsupported' : state.error ? 'failed' : null;
  return <div className="agent-usage-compression">
    <button type="button" data-testid="agent-context-compress" disabled={blocked} title={t(!idle ? 'agentUsage.idleOnly' : state.compressionSupported === false ? 'agentUsage.unsupported' : 'agentUsage.compress')} onClick={() => void compress()}><Minimize2 size={14} aria-hidden="true" />{t(state.busy ? 'agentUsage.compressing' : 'agentUsage.compress')}</button>
    {state.pending ? <button type="button" disabled={state.busy} onClick={() => void inspect()}>{t('agentUsage.check')}</button> : error ? <button type="button" disabled={state.busy || state.reading} onClick={() => void refresh()}><RefreshCw size={14} aria-hidden="true" />{t('agentUsage.retry')}</button> : null}
    {state.pending || error ? <p role={state.pending ? 'status' : 'alert'}>{t(state.pending ? 'agentUsage.unknown' : `agentUsage.${error}`)}</p> : null}
  </div>;
}
