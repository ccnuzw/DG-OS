import { t, formatDateTime, formatNumber, useLocale } from '../../i18n';
import { webErrorText, useChatStore, type ChatRunStep } from '../../stores/chatStore';
import { SourcePreview } from './SourcePreview';
import { executionSteps, operationFailed, webActivity } from './sources';

const phases: Record<string, string> = { model: 'chat.phaseModel', awaiting_confirmation: 'chat.awaitingTool', tool: 'chat.toolRunning', processing_result: 'chat.phaseResult', final_answer: 'chat.phaseAnswer', generating_answer: 'chat.phaseAnswer' };
const statuses: Record<string, string> = { pending: 'chat.runPending', running: 'chat.running', generating: 'chat.generating', completed: 'chat.runCompleted', settled: 'chat.runSettled', dispatched: 'chat.dispatched', supplied: 'chat.activitySupplied', manually_resolved: 'chat.activityUserReported', interrupted: 'chat.interrupted', cancelled: 'chat.stopped', failed: 'chat.failed', unknown: 'chat.runUnknown' };
export const chatPhaseText = (phase: string) => phases[phase] ? t(phases[phase]) : t('chat.phaseOther', { phase });
export const chatRunStatus = (status: string) => statuses[status] ? t(statuses[status]) : t('chat.phaseOther', { phase: status });
const record = (value: unknown): Record<string, unknown> | null => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
const scalar = (value: unknown): value is string | number | boolean => ['string', 'number', 'boolean'].includes(typeof value);
const bounded = (value: string) => value.length > 2400 ? `${value.slice(0, 2400)}…` : value;

function ResultValue({ value }: { value: unknown }) {
  if (typeof value === 'string') return <p className="chat-result-text">{bounded(value)}</p>;
  const object = record(value);
  const content = object?.content;
  if (typeof content === 'string') return <p className="chat-result-text">{bounded(content)}</p>;
  if (Array.isArray(content)) {
    const text = content.filter(item => record(item)?.type === 'text' && typeof record(item)?.text === 'string').map(item => String(record(item)?.text)).join('\n');
    if (text) return <p className="chat-result-text">{bounded(text)}</p>;
  }
  const rows = Array.isArray(value) ? value : Array.isArray(object?.rows) ? object.rows : null;
  if (rows?.length && rows.every(item => !!record(item) && Object.values(record(item)!).every(scalar))) {
    const columns = Object.keys(record(rows[0])!).slice(0, 6);
    return <div className="chat-result-table"><table><thead><tr>{columns.map(column => <th key={column}>{column}</th>)}</tr></thead><tbody>{rows.slice(0, 10).map((row, index) => <tr key={index}>{columns.map(column => <td key={column}>{bounded(String(record(row)?.[column] ?? ''))}</td>)}</tr>)}</tbody></table>{rows.length > 10 ? <p>{t('chat.resultExcerpt')}</p> : null}</div>;
  }
  const fields = object ? Object.entries(object).filter(([, item]) => scalar(item)).slice(0, 12) : [];
  if (fields.length) return <dl className="chat-result-fields">{fields.map(([key, item]) => <div key={key}><dt>{key}</dt><dd>{bounded(String(item))}</dd></div>)}</dl>;
  return <p className="text-xs text-ink-secondary">{t('chat.resultStructured')}</p>;
}

export default function RunSteps({ steps, diagnostic = false, active = false, taskId }: { steps: ChatRunStep[]; diagnostic?: boolean; active?: boolean; taskId?: string }) {
  useLocale();
  const session = useChatStore(state => state.activeSessionId);
  const resources = useChatStore(state => session ? state.sessionResources[session] : undefined);
  const expanded = useChatStore(state => state.replyExpanded);
  const setExpanded = useChatStore(state => state.setReplyExpanded);
  return <ol className="chat-run-steps">{(diagnostic ? steps : executionSteps(steps)).map(step => {
    const receipt = record(step.receipt?.original_receipt) ?? step.receipt;
    const web = webActivity(step);
    const pin = record(receipt?.pin);
    const context = Array.isArray(receipt?.context_receipts) ? receipt.context_receipts : [];
    const title = web ? t(web.operation === 'search' ? 'chat.webSearchAction' : 'chat.webFetchAction') : step.kind === 'skill' ? t('chat.skillReadAction') : step.kind === 'context' ? t('chat.contextReadAction') : typeof receipt?.tool_name === 'string' ? `${typeof receipt.server_name === 'string' ? receipt.server_name : typeof receipt.server_id === 'string' ? receipt.server_id : t('chat.stepTool')} / ${receipt.tool_name}` : chatPhaseText(step.phase);
    const target = web?.query || web?.url || (step.kind === 'skill' ? [pin?.skill_id, receipt?.path].filter(value => typeof value === 'string').join(' / ') : '');
    const inFlight = active && ['running', 'dispatched', 'pending'].includes(step.status);
    const status = inFlight ? t('chat.operationRunning') : step.result_unknown ? t('chat.runUnknown') : operationFailed(step) ? t('chat.operationFailedState') : web ? t(web.status === 'running' ? 'chat.operationRunning' : 'chat.runCompleted') : chatRunStatus(step.status);
    const disclosureKey = taskId ? `${taskId}:step:${web?.id ?? step.id}` : null;
    return <li key={web?.id ?? step.id}>
      <details open={disclosureKey ? expanded[disclosureKey] ?? false : undefined} onToggle={event => { if (disclosureKey) setExpanded(disclosureKey, event.currentTarget.open); }}><summary><span><strong>{title}</strong>{target ? <span className="chat-step-target">{target}</span> : null}</span><span className={(!inFlight && step.result_unknown) || operationFailed(step) ? 'text-status-warning-text' : 'text-ink-secondary'}>{status}</span></summary>
        <div className="chat-step-result">
          <time className="text-xs text-ink-secondary" dateTime={new Date(step.updated_at * 1000).toISOString()}>{formatDateTime(new Date(step.updated_at * 1000))} · {t('chat.elapsedSeconds', { seconds: formatNumber(Math.max(0, step.updated_at - step.created_at)) })}</time>
          {step.result_unknown && !inFlight ? <p className="text-sm text-status-warning-text">{t('chat.activityToolUnknown')}</p> : null}
          {step.status === 'manually_resolved' ? <p className="text-sm text-ink-secondary">{t('chat.activityUserReportedHelp')}</p> : null}
          {web ? <>
            {web.error ? <p className="text-sm text-status-warning-text">{webErrorText(web.error)}</p> : null}
            {web.status === 'completed' && !web.sources.length ? <p>{t('chat.webNoSources')}</p> : null}
            {web.sources.map(source => <div className="chat-step-source" key={source.id}><SourcePreview source={source} /><small>{t(source.kind === 'web_page' ? 'chat.sourcePage' : 'chat.sourceSearch')}</small>{source.excerpt ? <p>{bounded(source.excerpt)}</p> : null}</div>)}
            {web.truncated ? <p className="text-xs text-ink-secondary">{t('chat.resultExcerpt')}</p> : null}
          </> : step.kind === 'context' ? context.map((value, index) => {
            const item = record(value);
            const name = resources?.find(resource => resource.id === item?.resource_id)?.name ?? item?.resource_id;
            return <p key={index} className="text-sm wrap-anywhere">{String(name ?? t('chat.contextReadAction'))} · {t('chat.contextBytesRead', { count: formatNumber(typeof item?.bytes === 'number' ? item.bytes : 0) })}</p>;
          }) : step.kind === 'skill' ? <p className="text-sm">{t('chat.contextBytesRead', { count: formatNumber(typeof receipt?.bytes === 'number' ? receipt.bytes : 0) })}</p> : receipt?.result !== undefined ? <ResultValue value={receipt.result} /> : <p className="text-xs text-ink-secondary">{t(step.status === 'dispatched' ? 'chat.operationAwaitingReceipt' : 'chat.noReceipt')}</p>}
          {diagnostic && step.receipt ? <details><summary>{t('chat.activityRecordedDetails')}</summary><pre tabIndex={0} className="chat-run-receipt type-code whitespace-pre-wrap wrap-anywhere">{JSON.stringify(step.receipt, null, 2).slice(0, 16384)}</pre></details> : null}
        </div>
      </details>
    </li>;
  })}</ol>;
}
