import type { ReactNode } from 'react';
import { Button } from '@dgos/ui';
import { Check, ChevronRight, CircleHelp, FileDiff, FilePenLine, FileSearch, FileText, GitBranch, ListChecks, LoaderCircle, Search, Terminal, TriangleAlert, Wrench, XCircle, UserRound, Bot } from 'lucide-react';
import { useLocale } from '../../i18n';
import type { AgentEvent as Event, Engine, Json } from '../../stores/agentRunnerStore';
import Markdown from '../chat/Markdown';
import ExecutionDetails, { ToolDetails } from '../chat/ExecutionDetails';
import { LazyDetails, NativeValue } from './AgentExecutionViews';
import { AgentOutputReferences } from './AgentOutputReferences';
import { agentEventFailed, agentOutputReferences, type AgentReferenceActions } from './eventOutputReferences';
import { blocks, isUser, itemKey, nativeDiagnosticSeverity, object, prose, text } from './agentTranscript';
import { inspectableAgentEvent, isReasoningEvent, projectCodexTranscript, type CodexTranscriptEntry } from './codexTranscript';
import { CodexTranscriptContent } from './CodexTranscriptContent';
import { AgentReplyActions } from './AgentReplyActions';
import { AgentNativeResources, AgentReplyCitation, AgentReplySourceList } from './AgentReplyResources';
import './agent-reply.css';

export type ActivityState = 'processing' | 'waiting' | 'stopped' | 'failed' | 'unknown';
const emptyResults: ReadonlyMap<string, Json> = new Map();
const emptyNames: ReadonlyMap<string, string> = new Map();

export function operationStatus(event: Event, runState?: ActivityState) {
  const data = object(event.data);
  if (data.run_status === 'cancelled' || ['cancelled', 'interrupted', 'stopped'].includes(text(data.status))) return 'stopped';
  if (event.kind === 'result' || event.kind === 'turn/completed') return data.run_status === 'completed' ? 'replyCompleted' : data.run_status === 'failed' ? 'failed' : 'unknown';
  if (data.status === 'declined') return 'declined';
  if (agentEventFailed(event)) return 'failed';
  if (['waiting', 'pending', 'queued'].includes(text(data.status))) return 'waiting';
  if (text(data.status) && !['completed', 'inProgress', 'in_progress', 'running'].includes(text(data.status))) return 'unknown';
  if (['inProgress', 'in_progress', 'running'].includes(text(data.status)) || ['task_started', 'task_progress'].includes(event.kind) && !text(data.status)) return runState ?? 'processing';
  if (data.status === 'completed' || data.success === true || typeof data.exitCode === 'number' && data.exitCode === 0) return 'toolCompleted';
  // Turn success is never evidence that a missing tool result succeeded.
  if (runState) return runState;
  return event.kind === 'item/started' || ['tool_progress', 'task_started', 'task_progress'].includes(event.kind) || event.kind.endsWith('/outputDelta') || ['inProgress', 'in_progress', 'running'].includes(text(data.status)) ? 'processing' : 'unknown';
}

export function toolOperationStatus(event: Event, result: Json | undefined, runState?: ActivityState) {
  if (result === undefined) {
    const status = operationStatus(event, runState);
    return status === 'toolCompleted' ? runState ?? 'unknown' : status;
  }
  const status = operationStatus({ ...event, kind: 'tool_result', data: result });
  const data = object(result);
  return status === 'unknown' && !text(data.status) && !text(data.run_status) ? 'toolCompleted' : status;
}

function OperationIcon({ name }: { name?: string }) {
  const Icon = name === 'Read' ? FileText : name === 'Grep' ? FileSearch : ['Glob', 'webSearch'].includes(name ?? '') ? Search : ['Bash', 'commandExecution'].includes(name ?? '') ? Terminal : ['Edit', 'Write', 'MultiEdit', 'NotebookEdit', 'fileChange'].includes(name ?? '') ? FilePenLine : name === 'TodoWrite' || name === 'turn/plan/updated' ? ListChecks : name === 'turn/diff/updated' ? FileDiff : ['Agent', 'Task', 'subtask'].includes(name ?? '') ? GitBranch : Wrench;
  return <Icon size={15} aria-hidden="true" />;
}

export function OperationRow({ label, status, hint, children, name, failure }: { label: string; status: string; hint?: string; children: ReactNode | (() => ReactNode); name?: string; failure?: string }) {
  const { t } = useLocale();
  const StateIcon = status === 'failed' || status === 'declined' ? XCircle : status === 'toolCompleted' ? Check : status === 'processing' ? LoaderCircle : CircleHelp;
  return <div className="agent-reply-operation-wrap">
    <LazyDetails className="agent-reply-operation" data-operation-state={status} summary={<><ChevronRight className="agent-reply-chevron" size={13} aria-hidden="true" /><OperationIcon name={name} /><strong>{label}</strong>{hint ? <code className="agent-reply-target" title={hint}>{hint}</code> : null}<span className="agent-reply-operation-status"><StateIcon size={13} aria-hidden="true" /><span>{t(`agentExecution.${status}`)}</span></span></>}>{children}</LazyDetails>
    {failure ? <p className="agent-reply-failure" role="alert"><XCircle size={14} aria-hidden="true" /><span>{failure}</span></p> : null}
  </div>;
}

function NativeEventEvidence({ event, sources }: { event: Event; sources?: readonly Event[] }) {
  return (sources?.length ? sources : [event]).map((source, index) => <pre className="type-code agent-reply-native-value" data-event-seq={source.seq} data-event-run={source.run ?? ''} key={`${source.run ?? ''}:${source.seq}:${index}`}>{JSON.stringify(inspectableAgentEvent(source), null, 2)}</pre>);
}

export function NativeEvent({ event, sources }: { event: Event; sources?: readonly Event[] }) {
  const { t } = useLocale();
  return <LazyDetails className="agent-reply-native" summary={<>{t('agent.details')} · {event.kind} · #{event.seq}{sources && sources.length > 1 ? <span>{t('agentExecution.sourceCount', { count: sources.length })}</span> : null}</>}>
    {() => <NativeEventEvidence event={event} sources={sources} />}
  </LazyDetails>;
}

function failureText(value: Json | undefined): string {
  if (value === undefined || value === null) return '';
  if (typeof value === 'string') return value.trim().slice(0, 320);
  if (Array.isArray(value)) return value.map(failureText).filter(Boolean).join('\n').slice(0, 320);
  const data = object(value);
  if (['thinking', 'redacted_thinking', 'signature', 'thinking_delta', 'signature_delta', 'reasoning'].includes(text(data.type))) return '';
  return failureText(data.message ?? data.text ?? data.error ?? data.content);
}

function NativeDiagnosticNotice({ event, sources, engine }: { event: Event; sources?: readonly Event[]; engine?: Engine }) {
  const { t } = useLocale();
  const severity = nativeDiagnosticSeverity(event);
  if (!severity) return null;
  const safe = inspectableAgentEvent(event);
  const data = object(safe.data);
  const message = failureText(data.error) || failureText(data.errors) || failureText(data.message ?? data.description ?? data.summary ?? data.terminal_reason) || failureText(safe.text);
  const Icon = severity === 'failed' ? XCircle : TriangleAlert;
  return <article data-testid={`agent-event-${event.seq}`} data-event-seq={event.seq} data-event-run={event.run ?? ''} data-event-kind={event.kind} data-engine={engine} data-reply-role="activity" className="agent-dialogue-event agent-reply-event">
    {severity === 'warning' && !message ? <div className="agent-reply-native-notice agent-reply-native-notice-compact" data-native-severity={severity}>
      <LazyDetails className="agent-reply-native-warning" summary={<><Icon size={14} aria-hidden="true" /><span className="agent-reply-native-warning-summary" role="alert"><strong>{t('agentReply.nativeWarning')}</strong><span>{' '}{t('agentReply.nativeWarningNoMessage')}</span></span><span className="agent-reply-native-warning-action">{t('agent.details')}<ChevronRight className="agent-reply-chevron" size={13} aria-hidden="true" /></span></>}>
        {() => <><p className="agent-reply-note">{t('agentReply.nativeWarningHelp')}</p><NativeEventEvidence event={event} sources={sources} /></>}
      </LazyDetails>
    </div> : <div className="agent-reply-native-notice" data-native-severity={severity}>
      <p role="alert"><Icon size={15} aria-hidden="true" /><span><strong>{t(severity === 'failed' ? 'agentReply.nativeFailure' : 'agentReply.nativeWarning')}</strong>{' '}{message || t(severity === 'failed' ? 'agentReply.nativeFailureHelp' : 'agentReply.nativeWarningHelp')}</span></p>
      <NativeEvent event={event} sources={sources} />
    </div>}
  </article>;
}

function EventReferences({ event, toolNames, onOpenReference, onSelectRun }: AgentReferenceActions & { event: Event; toolNames: ReadonlyMap<string, string> }) {
  const { t } = useLocale();
  const { references, unresolved } = agentOutputReferences(event, toolNames);
  const failed = agentEventFailed(event);
  if (!references.length && !unresolved && !(failed && event.run)) return null;
  const content = () => <AgentOutputReferences event={event} toolNames={toolNames} onOpenReference={onOpenReference} onSelectRun={onSelectRun} />;
  return failed || references.some(reference => reference.source === 'diagnostic')
    ? <LazyDetails className="agent-reply-references" summary={t('agent.references')}>{content}</LazyDetails>
    : content();
}

function ToolBlock({ event, block, toolResults, toolNames, runState, sources, ...actions }: AgentReferenceActions & { event: Event; block: Json; toolResults: ReadonlyMap<string, Json>; toolNames: ReadonlyMap<string, string>; runState?: ActivityState; sources?: readonly Event[] }) {
  const { t } = useLocale();
  const value = object(block);
  const use = value.type === 'tool_use';
  const id = text(use ? value.id : value.tool_use_id);
  const result = use ? id ? toolResults.get(itemKey(event, id)) : undefined : block;
  let resultSource = event;
  if (use && result !== undefined && sources) for (let index = sources.length - 1; index >= 0; index--) {
    const source = sources[index];
    if (itemKey(source, id) === itemKey(event, id) && blocks(source).some(block => object(block).type === 'tool_result' && object(block).tool_use_id === id)) { resultSource = source; break; }
  }
  // The result row is folded into its tool owner; retain its native diagnostic
  // fields and run identity without treating assistant input/prose as output.
  const failed = result !== undefined && agentEventFailed({ ...event, kind: 'tool_result', data: result });
  const resultEvent: Event | undefined = use && result !== undefined ? { ...resultSource, kind: 'message', text: null, data: { ...object(resultSource.data), type: 'user', content: [result], ...(failed ? { is_error: true } : {}) } } : undefined;
  const name = use ? text(value.name) : toolNames.get(itemKey(event, id)) || id;
  const input = object(value.input);
  const target = text(input.command) || text(input.file_path) || text(input.notebook_path) || text(input.pattern) || text(input.path) || text(input.description);
  const label = name === 'Read' ? t('agentExecution.read') : name === 'Grep' ? t('agentExecution.searchContent') : name === 'Glob' ? t('agentExecution.findFiles') : ['Write', 'Edit', 'MultiEdit', 'NotebookEdit'].includes(name) ? t('agentExecution.write') : name === 'Bash' ? t('agentExecution.command') : name === 'TodoWrite' ? t('execution.plan') : name || t('agent.activity');
  const status = toolOperationStatus(event, result, runState);
  const elapsed = object(object(object(event.data).tool_progress)[id]).elapsed_time_seconds ?? object(event.data).elapsed_time_seconds;
  return <><OperationRow label={label} name={name} status={status} hint={target} failure={failed ? failureText(object(result).content ?? object(result).error ?? object(result).errors ?? object(result).result) || t('agentExecution.operationFailed') : undefined}>
    {() => <><ToolDetails name={name} input={use ? value.input : undefined} output={result} {...actions} />{elapsed !== undefined ? <p className="agent-reply-note">{t('execution.elapsed', { seconds: String(elapsed) })}</p> : null}<AgentReplySourceList events={sources ?? [event]} /><small className="agent-reply-note">{t('agentExecution.source', { seq: event.seq })}</small><NativeEvent event={event} sources={sources} />{result ? <LazyDetails summary={t('agentExecution.nativeResult')}><NativeValue value={result} /></LazyDetails> : null}</>}
  </OperationRow>{resultEvent ? <EventReferences event={resultEvent} toolNames={toolNames} {...actions} /> : null}</>;
}

export type EventContentProps = AgentReferenceActions & { event: Event; suppressText?: boolean; bufferText?: boolean; codex?: CodexTranscriptEntry; toolResults?: ReadonlyMap<string, Json>; toolNames?: ReadonlyMap<string, string>; runState?: ActivityState; engine?: Engine; sources?: readonly Event[]; parentToolUseId?: string; identityUncertain?: boolean };

export function EventContent({ event, suppressText = false, bufferText = false, codex, toolResults = emptyResults, toolNames = emptyNames, runState, engine, sources, parentToolUseId, identityUncertain, onOpenReference, onSelectRun, onQuoteReply, onOpenAgentActivity, onPreviewResource }: EventContentProps) {
  const { t } = useLocale();
  if (event.kind === 'native/unsupported') return <NativeDiagnosticNotice event={event} sources={sources} engine={engine} />;
  const data = object(event.data);
  const content = blocks(event);
  const terminal = event.kind === 'result' || event.kind === 'turn/completed';
  const failed = terminal ? data.run_status === 'failed' : agentEventFailed(event);
  const user = isUser(event);
  const assistant = event.kind === 'agentMessage' || data.type === 'assistant';
  const dialogue = user || assistant || terminal;
  const hasPrivateBlocks = content.some(block => ['thinking', 'redacted_thinking', 'signature', 'thinking_delta', 'signature_delta', 'reasoning'].includes(text(object(block).type)));
  const source = !suppressText && !(bufferText && !user) && dialogue ? prose(hasPrivateBlocks ? { ...event, text: null } : event) : '';
  const imageCount = user ? Math.max(Array.isArray(data.images) ? data.images.length : 0, content.filter(block => object(block).type === 'image').length) : 0;
  const tools = content.filter(block => { const value = object(block); return value.type === 'tool_use' || value.type === 'tool_result' && !toolNames.has(itemKey(event, text(value.tool_use_id))); });
  const unknownBlocks = content.filter(block => !['text', 'image', 'tool_use', 'tool_result', 'thinking', 'redacted_thinking', 'signature', 'thinking_delta', 'signature_delta', 'reasoning'].includes(text(object(block).type)));
  const media = !user ? content.filter(block => object(block).type === 'image') : [];
  const activity = ['contextCompaction', 'collabAgentToolCall', 'subAgentActivity', 'imageGeneration', 'imageView'].includes(event.kind) || event.kind === 'item/started' && ['contextCompaction', 'collabAgentToolCall', 'imageGeneration', 'imageView'].includes(text(data.type));
  const actions: AgentReferenceActions = { onOpenReference, onSelectRun, onQuoteReply, onOpenAgentActivity, onPreviewResource };
  const type = text(data.type) || event.kind;
  const command = text(data.command) || text(data.query) || text(object(data.action).query) || (Array.isArray(data.changes) ? data.changes.map(change => text(object(change).path)).filter(Boolean).join(', ') : '');
  const name = text(data.tool) || text(data.tool_name) || (type === 'commandExecution' ? t('direct.command') : type === 'fileChange' ? t('execution.artifacts') : type === 'webSearch' ? t('skills.search') : ['mcpToolCall', 'dynamicToolCall'].includes(type) ? t('capabilities.tool') : event.kind.endsWith('/outputDelta') ? t('direct.output') : t('agent.activity'));
  const execution = ['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch', 'fileChange', 'tool_progress', 'turn/plan/updated', 'turn/diff/updated'].includes(event.kind) || event.kind === 'item/started' && ['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch', 'fileChange'].includes(type) || event.kind.endsWith('/outputDelta');
  const task = ['task_started', 'task_progress', 'task_notification'].includes(event.kind);
  const status = operationStatus(event, runState);
  const nativeProjection = codex ?? projectCodexTranscript([event]).get(event);
  if (bufferText && nativeProjection?.kind === 'summary') return null;
  if (isReasoningEvent(event) && !nativeProjection) return null;
  if (!nativeProjection && !dialogue && !execution && !task && !activity && !failed && !tools.length && !unknownBlocks.length && !content.some(block => object(block).type === 'tool_result')) return null;
  const parent = parentToolUseId || text(data.parent_tool_use_id);
  const childStatuses = parent ? tools.map(block => {
    const value = object(block);
    const result = value.type === 'tool_use' ? toolResults.get(itemKey(event, text(value.id))) : block;
    return toolOperationStatus(event, result, runState);
  }) : [];
  const childStatus = failed || childStatuses.includes('failed') ? 'failed' : status === 'waiting' || childStatuses.includes('waiting') ? 'waiting' : status;
  const phase = !user && (engine === 'codex' || data.phase !== undefined) && ['commentary', 'final_answer'].includes(text(data.phase)) ? text(data.phase) : '';
  function body() {
    if (nativeProjection) return <CodexTranscriptContent event={event} entry={nativeProjection} inspector={value => <NativeEvent event={value} sources={value === event ? sources : undefined} />} />;
    return <>
      {identityUncertain ? <p className="agent-reply-note">{t('agentReply.identityUncertain')}</p> : null}
      {source || imageCount ? <div className={`agent-dialogue-prose agent-reply-prose ${phase === 'commentary' ? 'agent-reply-commentary' : ''}`} data-message-phase={phase || undefined}>
        <div className="agent-reply-prose-heading"><span className="agent-reply-avatar" aria-hidden="true">{user ? <UserRound size={16} /> : engine === 'codex' ? <Terminal size={16} /> : <Bot size={16} />}</span><strong className="agent-dialogue-speaker">{user ? t('agent.you') : engine === 'claude' ? 'Claude' : engine === 'codex' ? 'Codex' : t('agent.assistant')}</strong>{source && (user || phase === 'commentary') ? <AgentReplyActions source={source} onQuote={onQuoteReply} compact /> : null}</div>
        {source ? <><Markdown source={source} citationRenderer={(id, children) => <AgentReplyCitation id={id}>{children}</AgentReplyCitation>} />{!user && phase !== 'commentary' ? <AgentReplyActions source={source} onQuote={onQuoteReply} /> : null}</> : null}
        {imageCount ? <div data-testid="agent-image-history"><p>{t('agentImages.history', { count: imageCount })}</p><p className="agent-reply-note">{t('agentImages.historyPrivacy')}</p></div> : null}
      </div> : null}
      {tools.map((block, index) => { const value = object(block); return <ToolBlock key={text(value.id) || text(value.tool_use_id) || index} event={event} block={block} toolResults={toolResults} toolNames={toolNames} runState={runState} sources={sources} {...actions} />; })}
      {media.length ? <AgentNativeResources value={media} {...actions} /> : null}
      {unknownBlocks.length ? <AgentNativeResources value={unknownBlocks} {...actions} /> : null}
      {activity ? <OperationRow label={t(type === 'contextCompaction' ? 'agentReply.compaction' : ['imageGeneration', 'imageView'].includes(type) ? 'agentReply.imageOutput' : 'agentReply.activity')} status={status} name={type} hint={text(data.description) || text(data.tool)} failure={failed ? failureText(data.error ?? data.errors ?? data.message) || t('agentExecution.operationFailed') : undefined}>{() => <><AgentNativeResources value={data} {...actions} />{onOpenAgentActivity && ['collabAgentToolCall', 'subAgentActivity'].includes(type) ? <Button size="xs" variant="secondary" onClick={onOpenAgentActivity}>{t('agentReply.openActivity')}</Button> : null}<NativeEvent event={event} sources={sources} /></>}</OperationRow> : null}
      {task ? <OperationRow label={t('agentExecution.subtask')} name="subtask" status={status} hint={text(data.description) || text(data.task_id) || text(data.agent_id)} failure={status === 'failed' ? failureText(data.summary) || t('agentExecution.operationFailed') : undefined}>{() => <><dl className="agent-reply-fields">{['task_id', 'tool_use_id', 'parent_tool_use_id', 'agent_id', 'status'].map(key => data[key] !== undefined ? <div key={key}><dt>{t(`agentExecution.${key}`)}</dt><dd><NativeValue value={data[key]} /></dd></div> : null)}</dl>{text(data.description) ? <p>{text(data.description)}</p> : null}{text(data.summary) ? <Markdown source={text(data.summary)} /> : null}{Array.isArray(data.resource_links) && data.resource_links.length ? <AgentNativeResources value={{ ...data, type: 'task_notification' }} {...actions} /> : null}<NativeEvent event={event} sources={sources} /></>}</OperationRow> : null}
      {terminal && status === 'replyCompleted' ? <LazyDetails className="agent-reply-terminal" data-operation-state={status} summary={<><Check size={13} aria-hidden="true" /><span>{t('agentExecution.replyCompleted')}</span><ChevronRight className="agent-reply-chevron" size={12} aria-hidden="true" /></>}>
        {() => <><p className="agent-reply-note">{t('agentExecution.notAcceptance')}</p><ExecutionDetails event={event} {...actions} /><AgentReplySourceList events={sources ?? [event]} /><NativeEvent event={event} sources={sources} /></>}
      </LazyDetails> : execution || terminal || failed && !tools.length && !task && !activity ? <OperationRow label={terminal ? t('agent.finished') : event.kind === 'turn/plan/updated' ? t('execution.plan') : event.kind === 'turn/diff/updated' ? t('execution.diff') : name} name={type} status={status} hint={command} failure={failed ? failureText(data.error ?? data.errors ?? data.terminal_reason) || failureText(data.aggregatedOutput ?? data.result) || failureText(event.text) || t('agentExecution.operationFailed') : undefined}>
        {() => <>{terminal ? <p className="agent-reply-note">{t('agentExecution.notAcceptance')}</p> : null}<ExecutionDetails event={event} {...actions} /><AgentReplySourceList events={sources ?? [event]} />{failed && !dialogue && event.text ? <pre className="type-code agent-reply-native-value">{event.text}</pre> : null}{failed && data.error ? <NativeValue value={data.error} /> : null}<NativeEvent event={event} sources={sources} /></>}
      </OperationRow> : null}
      <EventReferences event={event} toolNames={toolNames} onOpenReference={onOpenReference} onSelectRun={onSelectRun} />
    </>;
  }
  return <article data-testid={`agent-event-${event.seq}`} data-event-seq={event.seq} data-event-run={event.run ?? ''} data-event-kind={event.kind} data-engine={engine} data-reply-role={user ? 'user' : source ? 'assistant' : 'activity'} data-message-phase={phase || undefined} data-parent-tool-use-id={parent || undefined} className={`agent-dialogue-event agent-reply-event ${user && !parent ? 'agent-dialogue-user' : ''}`}>
    {parent ? <LazyDetails className="agent-reply-subtask-message" data-operation-state={childStatus} summary={<><ChevronRight className="agent-reply-chevron" size={13} aria-hidden="true" /><GitBranch size={14} aria-hidden="true" /><strong>{t('agentExecution.childMessage')}</strong><span>{t('agentExecution.parentTool', { id: parent })}</span>{childStatus === 'failed' ? <span className="agent-reply-operation-status"><XCircle size={13} aria-hidden="true" />{t('agentExecution.failed')}</span> : null}{childStatus === 'waiting' ? <span className="agent-reply-operation-status"><CircleHelp size={13} aria-hidden="true" />{t('agentExecution.waiting')}</span> : null}</>}>{body}</LazyDetails> : body()}
  </article>;
}
