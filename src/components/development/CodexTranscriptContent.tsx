import type { ReactNode } from 'react';
import { ChevronRight, CircleHelp, FileDiff, ListChecks, MessageSquareText } from 'lucide-react';
import { useLocale } from '../../i18n';
import type { AgentEvent } from '../../stores/agentRunnerStore';
import Markdown from '../chat/Markdown';
import { AgentDiff, LazyDetails, PlanView } from './AgentExecutionViews';
import type { CodexTranscriptEntry } from './codexTranscript';
import { AgentReplyCitation } from './AgentReplyResources';

export function CodexTranscriptContent({ event, entry, inspector }: { event: AgentEvent; entry: CodexTranscriptEntry; inspector: (event: AgentEvent) => ReactNode }) {
  const { t } = useLocale();
  if (entry.kind === 'malformed') return <LazyDetails className="agent-reply-operation agent-reply-revision" summary={<><ChevronRight size={13} className="agent-reply-chevron" aria-hidden="true" /><CircleHelp size={15} aria-hidden="true" /><strong>{t('codexTranscript.malformed')}</strong></>}><p>{t('codexTranscript.malformedExplanation')}</p>{inspector(event)}</LazyDetails>;
  if (entry.kind === 'summary') return <LazyDetails className="agent-reply-summary agent-reply-revision" data-testid="codex-reasoning-summary" summary={<><ChevronRight size={13} className="agent-reply-chevron" aria-hidden="true" /><MessageSquareText size={15} aria-hidden="true" /><strong>{t('codexTranscript.summary')}</strong><span>{t(entry.complete ? 'codexTranscript.snapshot' : 'codexTranscript.streaming')}</span></>}>
    {() => <>{entry.parts.length ? entry.parts.map(part => <div key={part.index} data-summary-index={part.index}><Markdown source={part.text} citationRenderer={(id, children) => <AgentReplyCitation id={id}>{children}</AgentReplyCitation>} /></div>) : <p>{t('codexTranscript.emptySummary')}</p>}<small className="agent-reply-note">{t('agentExecution.source', { seq: entry.seq })}</small>{inspector(event)}</>}
  </LazyDetails>;
  const latest = entry.revisions[entry.revisions.length - 1] ?? event;
  const previous = entry.revisions.slice(0, -1);
  function revision(value: AgentEvent) {
    const data = value.data && typeof value.data === 'object' && !Array.isArray(value.data) ? value.data : {};
    return <>{value.kind === 'turn/diff/updated' ? <AgentDiff diff={typeof data.diff === 'string' ? data.diff : value.text ?? ''} seq={value.seq} /> : <><PlanView plan={Array.isArray(data.plan) ? data.plan : []} explanation={typeof data.explanation === 'string' ? data.explanation : undefined} /><small className="agent-reply-note">{t('agentExecution.source', { seq: value.seq })}</small></>}{inspector(value)}</>;
  }
  return <LazyDetails className="agent-reply-operation agent-reply-revision" data-testid="codex-turn-revisions" summary={<><ChevronRight size={13} className="agent-reply-chevron" aria-hidden="true" />{event.kind === 'turn/plan/updated' ? <ListChecks size={15} aria-hidden="true" /> : <FileDiff size={15} aria-hidden="true" />}<strong>{t(event.kind === 'turn/plan/updated' ? 'execution.plan' : 'execution.diff')}</strong><span>{t('codexTranscript.latestRevision', { count: entry.revisions.length })}</span></>}>
    {() => <><p className="agent-reply-note">{t('codexTranscript.proposalBoundary')}</p>{revision(latest)}{previous.length ? <LazyDetails summary={t('codexTranscript.previousRevisions', { count: previous.length })}>{() => previous.map(value => <LazyDetails key={value.seq} summary={t('agentExecution.source', { seq: value.seq })}>{() => revision(value)}</LazyDetails>)}</LazyDetails> : null}</>}
  </LazyDetails>;
}
