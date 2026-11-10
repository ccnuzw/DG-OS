import { t, formatDateTime, formatNumber, useLocale } from '../../i18n';
import type { ChatRunStep } from '../../stores/chatStore';

const phases: Record<string, string> = { model: 'chat.phaseModel', awaiting_confirmation: 'chat.awaitingTool', tool: 'chat.toolRunning', processing_result: 'chat.phaseResult', final_answer: 'chat.phaseAnswer', generating_answer: 'chat.phaseAnswer' };
const statuses: Record<string, string> = { pending: 'chat.runPending', running: 'chat.running', generating: 'chat.generating', completed: 'chat.runCompleted', settled: 'chat.runSettled', dispatched: 'chat.dispatched', supplied: 'chat.activitySupplied', manually_resolved: 'chat.activityUserReported', interrupted: 'chat.interrupted', cancelled: 'chat.stopped', failed: 'chat.failed', unknown: 'chat.runUnknown' };
export const chatPhaseText = (phase: string) => phases[phase] ? t(phases[phase]) : t('chat.phaseOther', { phase });
export const chatRunStatus = (status: string) => statuses[status] ? t(statuses[status]) : t('chat.phaseOther', { phase: status });
export default function RunSteps({ steps }: { steps: ChatRunStep[] }) {
  useLocale();
  return <ol className="chat-run-steps">{steps.map(step => {
    const original = step.receipt?.original_receipt;
    const receipt = original && typeof original === 'object' && !Array.isArray(original) ? original as Record<string, unknown> : step.receipt;
    const title = typeof receipt?.tool_name === 'string' ? receipt.tool_name : chatPhaseText(step.phase);
    return <li key={step.id}>
    <details><summary><span>{title}</span><span className={step.result_unknown ? 'text-status-warning-text' : 'text-ink-secondary'}>{step.result_unknown ? t('chat.runUnknown') : chatRunStatus(step.status)}</span></summary>
      <dl className="chat-budget-grid"><div><dt>{t('chat.stepKind')}</dt><dd className="wrap-anywhere">{step.kind === 'tool' ? t('chat.stepTool') : step.kind === 'model' ? t('chat.stepModel') : step.kind === 'skill' ? t('chat.stepSkill') : step.kind}</dd></div><div><dt>{t('chat.dispatchState')}</dt><dd>{t(step.dispatched ? 'chat.dispatched' : 'chat.notDispatched')}</dd></div><div><dt>{t('chat.stepElapsed')}</dt><dd>{t('chat.elapsedSeconds', { seconds: formatNumber(Math.max(0, step.updated_at - step.created_at)) })}</dd></div></dl>
      <time className="text-xs text-ink-secondary" dateTime={new Date(step.updated_at * 1000).toISOString()}>{formatDateTime(new Date(step.updated_at * 1000))}</time>
      {step.result_unknown ? <p className="text-xs text-status-warning-text">{t('chat.unknownResult')}</p> : null}
      {step.receipt ? <div className="mt-2"><strong className="text-xs">{t(step.status === 'manually_resolved' ? 'chat.activityUserReported' : step.kind === 'tool' && !step.result_unknown && step.status === 'settled' ? 'chat.confirmedReceipt' : 'chat.activityRecordedDetails')}</strong>{step.status === 'manually_resolved' ? <p className="text-sm text-ink-secondary">{t('chat.activityUserReportedHelp')}</p> : null}<pre tabIndex={0} aria-label={t('chat.activityRecordedDetails')} className="chat-run-receipt type-code whitespace-pre-wrap wrap-anywhere">{JSON.stringify(step.receipt, null, 2)}</pre></div> : <p className="text-xs text-ink-secondary">{t('chat.noReceipt')}</p>}
    </details>
  </li>; })}</ol>;
}
