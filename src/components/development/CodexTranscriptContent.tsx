import { useState, type ReactNode } from 'react';
import { useLocale } from '../../i18n';
import type { AgentEvent } from '../../stores/agentRunnerStore';
import Markdown from '../chat/Markdown';
import type { CodexTranscriptEntry } from './codexTranscript';

export function CodexTranscriptContent({ event, entry, inspector }: { event: AgentEvent; entry: CodexTranscriptEntry; inspector: (event: AgentEvent) => ReactNode }) {
  const { t } = useLocale();
  const [summaryOpen, setSummaryOpen] = useState(false);
  if (entry.kind === 'malformed') return <details className="agent-dialogue-tool"><summary>{t('codexTranscript.malformed')}</summary><p>{t('codexTranscript.malformedExplanation')}</p>{inspector(event)}</details>;
  if (entry.kind === 'summary') return <details className="agent-dialogue-tool" data-testid="codex-reasoning-summary" onToggle={event => setSummaryOpen(event.currentTarget.open)}><summary><strong>{t('codexTranscript.summary')}</strong><span>{t(entry.complete ? 'codexTranscript.snapshot' : 'codexTranscript.streaming')}</span></summary>{summaryOpen ? <div className="agent-dialogue-tool-body">{entry.parts.length ? entry.parts.map(part => <div key={part.index} data-summary-index={part.index}><Markdown source={part.text} /></div>) : <p>{t('codexTranscript.emptySummary')}</p>}<small>{t('agentExecution.source', { seq: entry.seq })}</small></div> : null}</details>;
  const previous = entry.revisions.slice(0, -1);
  function revision(value: AgentEvent) {
    const data = value.data && typeof value.data === 'object' && !Array.isArray(value.data) ? value.data : {};
    if (value.kind === 'turn/diff/updated') return <><pre className="type-code overflow-x-auto whitespace-pre-wrap break-words">{typeof data.diff === 'string' ? data.diff : value.text}</pre><small>{t('agentExecution.source', { seq: value.seq })}</small>{inspector(value)}</>;
    return <>{typeof data.explanation === 'string' && data.explanation ? <p>{data.explanation}</p> : null}{Array.isArray(data.plan) ? <ol className="list-decimal pl-5">{data.plan.map((step, index) => {
      const item = step && typeof step === 'object' && !Array.isArray(step) ? step : {};
      const status = item.status;
      return <li key={index}><span>{typeof item.step === 'string' ? item.step : t('codexTranscript.malformed')}</span> <span className="text-xs text-ink-secondary">{t(status === 'inProgress' ? 'execution.in_progress' : status === 'pending' ? 'execution.pending' : status === 'completed' ? 'execution.completed' : 'execution.unknown')}{typeof status === 'string' && !['pending', 'inProgress', 'completed'].includes(status) ? ` (${status})` : ''}</span></li>;
    })}</ol> : <p>{t('codexTranscript.malformedExplanation')}</p>}<small>{t('agentExecution.source', { seq: value.seq })}</small>{inspector(value)}</>;
  }
  return <details className="agent-dialogue-tool" data-testid="codex-turn-revisions"><summary><strong>{t(event.kind === 'turn/plan/updated' ? 'execution.plan' : 'execution.diff')}</strong><span>{t('codexTranscript.latestRevision', { count: entry.revisions.length })}</span></summary><div className="agent-dialogue-tool-body"><p>{t('codexTranscript.proposalBoundary')}</p>{revision(event)}{previous.length ? <details className="agent-dialogue-native"><summary>{t('codexTranscript.previousRevisions', { count: previous.length })}</summary>{previous.map(value => <details key={value.seq}><summary>{t('agentExecution.source', { seq: value.seq })}</summary>{revision(value)}</details>)}</details> : null}</div></details>;
}
