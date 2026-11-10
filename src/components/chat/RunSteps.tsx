import { t, formatDateTime, formatNumber, useLocale } from '../../i18n';
import type { ChatRunStep } from '../../stores/chatStore';

const phases: Record<string, string> = { model: 'chat.phaseModel', awaiting_confirmation: 'chat.awaitingTool', tool: 'chat.toolRunning', processing_result: 'chat.phaseResult', final_answer: 'chat.phaseAnswer', generating_answer: 'chat.phaseAnswer' };
const statuses: Record<string, string> = { pending: 'chat.runPending', running: 'chat.running', generating: 'chat.generating', completed: 'chat.runCompleted', settled: 'chat.runSettled', dispatched: 'chat.dispatched', manually_resolved: 'chat.runManuallyResolved', interrupted: 'chat.interrupted', cancelled: 'chat.stopped', failed: 'chat.failed', unknown: 'chat.runUnknown' };
export const chatPhaseText = (phase: string) => phases[phase] ? t(phases[phase]) : t('chat.phaseOther', { phase });
export const chatRunStatus = (status: string) => statuses[status] ? t(statuses[status]) : t('chat.phaseOther', { phase: status });
export default function RunSteps({ steps }: { steps: ChatRunStep[] }) {
  useLocale();
  return <ol className="chat-run-steps">{steps.map(step => <li key={step.id}>
    <details><summary><span>{chatPhaseText(step.phase)}</span><span className={step.result_unknown ? 'text-status-warning-text' : 'text-ink-secondary'}>{step.result_unknown ? t('chat.runUnknown') : chatRunStatus(step.status)}</span></summary>
      <dl className="chat-budget-grid"><div><dt>{t('chat.stepKind')}</dt><dd className="wrap-anywhere">{step.kind === 'tool' ? t('chat.stepTool') : step.kind === 'model' ? t('chat.stepModel') : step.kind === 'skill' ? t('chat.stepSkill') : step.kind}</dd></div><div><dt>{t('chat.dispatchState')}</dt><dd>{t(step.dispatched ? 'chat.dispatched' : 'chat.notDispatched')}</dd></div><div><dt>{t('chat.stepElapsed')}</dt><dd>{t('chat.elapsedSeconds', { seconds: formatNumber(Math.max(0, step.updated_at - step.created_at)) })}</dd></div></dl>
      <time className="text-xs text-ink-secondary" dateTime={new Date(step.updated_at * 1000).toISOString()}>{formatDateTime(new Date(step.updated_at * 1000))}</time>
      {step.result_unknown ? <p className="text-xs text-status-warning-text">{t('chat.unknownResult')}</p> : null}
      {step.receipt ? <div className="mt-2"><strong className="text-xs">{t(step.status === 'manually_resolved' ? 'chat.runManuallyResolved' : 'chat.confirmedReceipt')}</strong><pre tabIndex={0} className="chat-run-receipt type-code whitespace-pre-wrap wrap-anywhere">{JSON.stringify(step.receipt, null, 2)}</pre></div> : <p className="text-xs text-ink-secondary">{t('chat.noReceipt')}</p>}
    </details>
  </li>)}</ol>;
}
