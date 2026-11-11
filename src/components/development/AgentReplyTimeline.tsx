import { memo, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ChevronRight, ListChecks } from 'lucide-react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { Engine, Json } from '../../stores/agentRunnerStore';
import { EventContent, operationStatus, toolOperationStatus } from './AgentReplyContent';
import { AgentReplySourcesProvider } from './AgentReplyResources';
import { blocks, eventScope, isUser, itemKey, object, prose, text, type AgentTranscriptRow as Row } from './agentTranscript';
import { agentEventFailed, type AgentReferenceActions } from './eventOutputReferences';
import './agent-reply-timeline.css';

type ActivityState = 'processing' | 'waiting' | 'stopped' | 'failed' | 'unknown';
type Props = AgentReferenceActions & {
  rows: readonly Row[];
  engine?: Engine;
  toolResults: ReadonlyMap<string, Json>;
  toolNames: ReadonlyMap<string, string>;
  runStates: ReadonlyMap<string, ActivityState>;
  bufferingRun?: string | null;
};
type Section = 'process' | 'changes' | 'answer';
function section(row: Row, bufferText = false): Section | undefined {
  const data = object(row.event.data);
  if (isUser(row.event) || bufferText && row.codex?.kind === 'summary') return undefined;
  if (row.parentToolUseId || text(data.parent_tool_use_id)) return 'process';
  if (row.event.kind === 'fileChange' || row.event.kind === 'turn/diff/updated' || data.type === 'fileChange') return 'changes';
  if (agentEventFailed(row.event)) return 'process';
  if (['result', 'turn/completed'].includes(row.event.kind) && !bufferText && !row.suppressText && !agentEventFailed(row.event) && prose(row.event).trim()) return 'answer';
  if (!row.codex && (row.event.kind === 'agentMessage' || data.type === 'assistant') && blocks(row.event).every(block => ['text', 'thinking', 'redacted_thinking', 'signature', 'thinking_delta', 'signature_delta', 'reasoning'].includes(text(object(block).type)))) {
    if (bufferText || row.suppressText || !prose(row.event).trim()) return undefined;
    return data.phase === 'commentary' ? 'process' : 'answer';
  }
  return 'process';
}
type ReadCounts = { files: number; searches: number; directories: number };
function completedRead(row: Row, results: ReadonlyMap<string, Json>): ReadCounts | undefined {
  const event = row.event, data = object(event.data);
  if (agentEventFailed(event) || row.codex || row.parentToolUseId || text(data.parent_tool_use_id) || (text(data.status) && data.status !== 'completed') || (text(data.run_status) && data.run_status !== 'completed')) return;
  const counts: ReadCounts = { files: 0, searches: 0, directories: 0 };
  const content = blocks(event);
  if (content.length === 1) {
    const tool = object(content[0]);
    const result = results.get(itemKey(event, text(tool.id)));
    if (!['message', 'agentMessage'].includes(event.kind) || tool.type !== 'tool_use' || prose(event).trim() || result === undefined || toolOperationStatus(event, result) !== 'toolCompleted') return;
    if (tool.name === 'Read') counts.files = 1;
    else if (tool.name === 'Grep' || tool.name === 'Glob') counts.searches = 1;
    else return;
    return counts;
  }
  const actions = data.commandActions;
  if (event.kind !== 'commandExecution' || data.exitCode !== 0 || !Array.isArray(actions) || !actions.length) return;
  for (const action of actions) {
    const type = text(object(action).type);
    if (type === 'read') counts.files++;
    else if (type === 'search') counts.searches++;
    else if (type === 'listFiles') counts.directories++;
    else return;
  }
  return counts;
}
const ReplyRow = memo(function ReplyRow({ row, engine, toolResults, toolNames, runState, bufferText, onOpenReference, onSelectRun, onQuoteReply, onOpenAgentActivity, onPreviewResource }: Omit<Props, 'rows' | 'runStates' | 'bufferingRun'> & { row: Row; runState?: ActivityState; bufferText: boolean }) {
  const replySection = section(row, bufferText);
  const terminal = ['result', 'turn/completed'].includes(row.event.kind);
  return <div className="agent-reply-timeline-row" data-reply-key={row.key} data-reply-section={replySection} data-reply-terminal={terminal ? operationStatus(row.event, runState) : undefined} data-reply-final={object(row.event.data).phase === 'final_answer' && replySection === 'answer' || undefined}>
    <EventContent event={row.event} suppressText={row.suppressText} codex={row.codex} sources={row.sources} parentToolUseId={row.parentToolUseId} identityUncertain={row.identityUncertain} engine={engine} toolResults={toolResults} toolNames={toolNames} runState={runState} bufferText={bufferText} onOpenReference={onOpenReference} onSelectRun={onSelectRun} onQuoteReply={onQuoteReply} onOpenAgentActivity={onOpenAgentActivity} onPreviewResource={onPreviewResource} />
  </div>;
});
function ReadGroup({ rows, counts, children }: { rows: readonly Row[]; counts: ReadCounts; children: ReactNode }) {
  const { t } = useLocale();
  const container = useRef<HTMLDetailsElement>(null);
  const single = rows.length === 1;
  const [open, setOpen] = useState(single);
  const wasSingle = useRef(single);
  useLayoutEffect(() => {
    const node = container.current;
    if (!node) return;
    if (single) {
      node.open = true;
      if (document.activeElement === node.querySelector(':scope > summary')) node.querySelector<HTMLElement>('.agent-reply-operation > summary')?.focus({ preventScroll: true });
    } else if (wasSingle.current) node.open = Boolean(node.querySelector('.agent-reply-operation[open]')) || node.contains(document.activeElement);
    wasSingle.current = single;
    setOpen(node.open);
  }, [single]);
  // The transparent singleton keeps operation DOM, lazy bodies and search anchors
  // stable when a later read turns it into a visible group.
  const summary = [counts.files ? t('agentReply.readSummaryFiles', { count: counts.files }) : '', counts.searches ? t('agentReply.readSummarySearches', { count: counts.searches }) : '', counts.directories ? t('agentReply.readSummaryDirectories', { count: counts.directories }) : ''].filter(Boolean).join(' · ');
  return <details ref={container} className={single ? 'agent-reply-single-read' : 'agent-reply-read-group'} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary hidden={single} tabIndex={single ? -1 : undefined}><ChevronRight size={14} aria-hidden="true" className={open ? 'agent-reply-chevron-open' : ''} /><ListChecks size={15} aria-hidden="true" /><strong>{summary}</strong><span>{t('agentExecution.operationCount', { count: rows.length })}</span></summary>
    <div className="agent-reply-group-items">{children}</div>
  </details>;
}
function ReplyTurn({ rows, engine, toolResults, toolNames, runStates, bufferingRun, onOpenReference, onSelectRun, onQuoteReply, onOpenAgentActivity, onPreviewResource }: Props) {
  const { t } = useLocale();
  const root = useRef<HTMLDivElement>(null);
  const sourceEvents = useMemo(() => rows.flatMap(row => row.sources), [rows]);
  const segments = useMemo(() => {
    const result: { key: string; scope: string; rows: Row[]; counts?: ReadCounts }[] = [];
    for (const row of rows) {
      const previous = result[result.length - 1];
      const counts = completedRead(row, toolResults);
      const scope = JSON.stringify(eventScope(row.event));
      if (counts && previous?.counts && previous.scope === scope) {
        previous.rows.push(row);
        previous.counts.files += counts.files;
        previous.counts.searches += counts.searches;
        previous.counts.directories += counts.directories;
      } else result.push({ key: row.key, scope, rows: [row], counts });
    }
    return result;
  }, [rows, toolResults]);
  const sections = new Set(rows.map(row => section(row, Boolean(bufferingRun && row.event.run === bufferingRun))).filter((value): value is Section => value !== undefined));
  const navigate = (target: Section) => {
    const matches = root.current?.querySelectorAll<HTMLElement>(`[data-reply-section="${target}"]`);
    const finals = target === 'answer' ? root.current?.querySelectorAll<HTMLElement>('[data-reply-final="true"]') : undefined;
    const node = target === 'answer' ? finals?.[finals.length - 1] ?? matches?.[matches.length - 1] : matches?.[0];
    if (!node) return;
    let ancestor = node.parentElement;
    while (ancestor && ancestor !== root.current) { if (ancestor instanceof HTMLDetailsElement) ancestor.open = true; ancestor = ancestor.parentElement; }
    node.tabIndex = -1; node.focus({ preventScroll: true }); node.scrollIntoView({ block: 'start', behavior: 'auto' });
  };
  const renderRow = (row: Row) => <ReplyRow key={row.key} row={row} engine={engine} toolResults={toolResults} toolNames={toolNames} runState={runStates.get(row.event.run ?? '')} bufferText={Boolean(bufferingRun && row.event.run === bufferingRun)} onOpenReference={onOpenReference} onSelectRun={onSelectRun} onQuoteReply={onQuoteReply} onOpenAgentActivity={onOpenAgentActivity} onPreviewResource={onPreviewResource} />;
  return <AgentReplySourcesProvider events={sourceEvents}><div ref={root} className="agent-reply-timeline">
    {rows.length >= 8 && sections.size >= 2 ? <nav className="agent-reply-navigation" aria-label={t('agentExecution.replyNavigation')}>{(['process', 'changes', 'answer'] as const).filter(name => sections.has(name)).map(name => <Button key={name} size="xs" variant="ghost" onClick={() => navigate(name)}>{t(`agentExecution.${name === 'process' ? 'jumpProcess' : name === 'changes' ? 'jumpChanges' : 'jumpAnswer'}`)}</Button>)}</nav> : null}
    {segments.map(segment => segment.counts ? <ReadGroup key={segment.key} rows={segment.rows} counts={segment.counts}>{segment.rows.map(renderRow)}</ReadGroup> : segment.rows.map(renderRow))}
  </div></AgentReplySourcesProvider>;
}

export function AgentReplyTimeline(props: Props) {
  const turns = useMemo(() => {
    const groups: { key: string; scope: string; rows: Row[] }[] = [];
    for (const row of props.rows) {
      const previous = groups[groups.length - 1];
      const scope = JSON.stringify(eventScope(row.event));
      if (previous && previous.scope === scope && !isUser(row.event)) previous.rows.push(row);
      else groups.push({ key: row.key, scope, rows: [row] });
    }
    return groups;
  }, [props.rows]);
  return <>{turns.map(turn => <section className="agent-reply-turn" key={turn.key} data-reply-run={turn.rows[0].event.run ?? ''}><ReplyTurn {...props} rows={turn.rows} /></section>)}</>;
}
