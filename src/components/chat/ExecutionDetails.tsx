import { useLocale } from '../../i18n';
import type { AgentEvent, Json } from '../../stores/agentRunnerStore';
import { CodeBlock } from './Markdown';
import { AgentDiff, LazyDetails, NativeValue, PlanView } from '../development/AgentExecutionViews';
import { AgentOutputText } from '../development/AgentOutputText';
import { AgentNativeResources } from '../development/AgentReplyResources';
import type { AgentReferenceActions } from '../development/eventOutputReferences';
export { NativeValue } from '../development/AgentExecutionViews';

const object = (value: Json | undefined): Record<string, Json> => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
const text = (value: Json | undefined) => typeof value === 'string' ? value : '';

function Output({ value, lang, ...actions }: AgentReferenceActions & { value: Json | undefined; lang?: string }) {
  if (typeof value === 'string') return value ? <AgentOutputText text={value} lang={lang} /> : null;
  if (value === undefined || value === null) return null;
  const result = object(value);
  if (result.type === 'text' && typeof result.text === 'string') return <Output value={result.text} lang={lang} {...actions} />;
  if (result.type === 'tool_result') return <><Output value={result.content} lang={lang} {...actions} />{result.structuredContent !== undefined ? <AgentNativeResources value={{ structuredContent: result.structuredContent }} {...actions} /> : null}</>;
  if (Array.isArray(value)) return <div className="agent-reply-values">{value.map((entry, index) => <Output key={index} value={entry} lang={lang} {...actions} />)}</div>;
  return <AgentNativeResources value={value} {...actions} />;
}

function fileLanguage(path: string) {
  const extension = path.split('.').pop()?.toLowerCase();
  return extension && ({ tsx: 'tsx', ts: 'typescript', js: 'javascript', jsx: 'jsx', py: 'python', rs: 'rust', sh: 'bash', json: 'json', css: 'css', html: 'html', sql: 'sql', yaml: 'yaml', yml: 'yaml', md: 'markdown', go: 'go' } as Record<string, string>)[extension];
}

// These views show native input/output only. A path never triggers a filesystem read.
export function ToolDetails({ name, input, output, ...actions }: AgentReferenceActions & { name: string; input?: Json; output?: Json }) {
  const { t } = useLocale();
  const fields = object(input);
  const path = text(fields.file_path) || text(fields.path) || text(fields.notebook_path);
  const read = name === 'Read';
  const search = name === 'Grep' || name === 'Glob' || name === 'webSearch';
  const command = name === 'Bash' || name === 'commandExecution';
  const edit = ['Edit', 'MultiEdit', 'NotebookEdit'].includes(name);
  const write = name === 'Write';
  const todos = name === 'TodoWrite' && Array.isArray(fields.todos) ? fields.todos : null;
  const mcp = name.startsWith('mcp__') || ['mcpToolCall', 'dynamicToolCall'].includes(name);
  const edits = Array.isArray(fields.edits) ? fields.edits : [fields];
  const remainingInput = command && input && typeof input === 'object' && !Array.isArray(input) ? Object.fromEntries(Object.entries(fields).filter(([key]) => !['command', 'cwd', 'description'].includes(key))) : input;
  const hasRemainingInput = remainingInput !== undefined && remainingInput !== null && (!command || typeof remainingInput !== 'object' || Array.isArray(remainingInput) || Object.keys(remainingInput).length > 0);
  return <div className="agent-reply-tool-details" data-tool-view={todos ? 'plan' : read ? 'read' : search ? 'search' : command ? 'command' : edit ? 'edit' : write ? 'write' : mcp ? 'mcp' : 'native'}>
    {todos ? <PlanView plan={todos} /> : null}
    {path ? <p className="agent-reply-path type-code">{path}</p> : null}
    {read && (fields.offset !== undefined || fields.limit !== undefined) ? <dl className="agent-reply-fields">{['offset', 'limit'].map(key => fields[key] !== undefined ? <div key={key}><dt>{t(`agentExecution.${key}`)}</dt><dd><NativeValue value={fields[key]} /></dd></div> : null)}</dl> : null}
    {search ? <><dl className="agent-reply-fields">{['pattern', 'glob', 'path', 'query', 'type', 'output_mode'].map(key => fields[key] !== undefined ? <div key={key}><dt>{key}</dt><dd><NativeValue value={fields[key]} /></dd></div> : null)}</dl></> : null}
    {command ? <>{typeof fields.command === 'string' && fields.command ? <CodeBlock text={fields.command} lang="bash" /> : fields.command !== undefined ? <NativeValue value={fields.command} /> : null}{fields.cwd !== undefined && fields.cwd !== null ? typeof fields.cwd === 'string' ? <p className="agent-reply-path type-code">{t('agentExecution.cwd')}: {fields.cwd}</p> : <section><strong>{t('agentExecution.cwd')}</strong><NativeValue value={fields.cwd} /></section> : null}{fields.description !== undefined ? <NativeValue value={fields.description} /> : null}</> : null}
    {edit ? edits.map((entry, index) => { const value = object(entry); return <div className="agent-reply-edit" key={index}>{typeof value.old_string === 'string' ? <section><strong>{t('agentExecution.before')}</strong><CodeBlock text={value.old_string} lang={fileLanguage(path)} /></section> : null}{typeof value.new_string === 'string' ? <section><strong>{t('agentExecution.after')}</strong><CodeBlock text={value.new_string} lang={fileLanguage(path)} /></section> : null}{typeof value.new_source === 'string' ? <CodeBlock text={value.new_source} lang={text(fields.cell_type) === 'markdown' ? 'markdown' : undefined} /> : null}</div>; }) : null}
    {write && typeof fields.content === 'string' ? <CodeBlock text={fields.content} lang={fileLanguage(path)} /> : null}
    {hasRemainingInput ? todos || read || search || command || edit || write ? <LazyDetails summary={t('agentExecution.input')}><NativeValue value={remainingInput} /></LazyDetails> : <section><strong>{t('agentExecution.input')}</strong><NativeValue value={remainingInput} /></section> : null}
    {output !== undefined ? <section><strong>{t('agentExecution.output')}</strong><Output value={output} lang={read ? fileLanguage(path) : undefined} {...actions} /></section> : null}
  </div>;
}

export default function ExecutionDetails({ event, ...actions }: AgentReferenceActions & { event: AgentEvent }) {
  const { t } = useLocale();
  const data = object(event.data);
  if (event.kind === 'turn/completed' || event.kind === 'result') {
    const status = ['completed', 'failed', 'cancelled'].includes(text(data.run_status)) ? text(data.run_status) : 'unknown';
    return <section aria-label={t('execution.run')} className="agent-reply-values"><strong className={status === 'failed' ? 'agent-reply-error-label' : undefined} data-run-status={status}>{t(`execution.run_${status}`)}</strong><NativeValue value={data.native_status} /><NativeValue value={data.terminal_reason} /><NativeValue value={data.errors} /><small>{t('execution.source', { seq: event.seq })}</small></section>;
  }
  if (event.kind === 'turn/plan/updated') return <><PlanView plan={Array.isArray(data.plan) ? data.plan : []} explanation={text(data.explanation)} /><small>{t('execution.source', { seq: event.seq })}</small></>;
  if (event.kind === 'turn/diff/updated') return <AgentDiff diff={text(data.diff) || event.text || ''} seq={event.seq} />;
  if ((event.kind === 'fileChange' || data.type === 'fileChange') && Array.isArray(data.changes)) return <section aria-label={t('execution.artifacts')} className="agent-reply-values"><p className="agent-reply-note">{t('execution.metadata')}</p>{data.changes.map((value, index) => { const change = object(value); const kind = object(change.kind); return <LazyDetails key={index} summary={<><code>{text(change.path)}</code><span>{text(kind.type) || text(change.kind)}</span></>}>{() => <>{typeof change.diff === 'string' ? <AgentDiff diff={change.diff} path={text(change.path)} seq={event.seq} /> : <NativeValue value={change} />}</>}</LazyDetails>; })}<NativeValue value={data.error} /><small>{t('execution.source', { seq: event.seq })}</small></section>;
  if (['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch', 'item/started', 'tool_progress'].includes(event.kind) || event.kind.endsWith('/outputDelta')) {
    const type = text(data.type) || event.kind;
    const name = ['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch'].includes(type) ? type : text(data.tool) || text(data.tool_name) || type;
    const nativeInput = data.arguments ?? data.input;
    const commandFields = { ...(data.command !== undefined ? { command: data.command } : {}), ...(data.cwd !== undefined ? { cwd: data.cwd } : {}), ...(data.description !== undefined ? { description: data.description } : {}) };
    const input = name === 'commandExecution' ? Object.keys(commandFields).length ? { ...(nativeInput !== undefined && nativeInput !== null && (typeof nativeInput !== 'object' || Array.isArray(nativeInput)) ? { input: nativeInput } : object(nativeInput)), ...commandFields } : nativeInput : name === 'webSearch' ? { query: data.query ?? object(data.action).query ?? null, action: data.action ?? null } : nativeInput;
    return <section className="agent-reply-values">{['mcpToolCall', 'dynamicToolCall'].includes(type) ? <dl className="agent-reply-fields">{['server', 'tool'].map(key => data[key] !== undefined ? <div key={key}><dt>{key}</dt><dd><NativeValue value={data[key]} /></dd></div> : null)}</dl> : null}<ToolDetails name={name} input={input} output={data.aggregatedOutput ?? data.result ?? data.contentItems ?? data.output ?? event.text ?? undefined} {...actions} />{typeof data.exitCode === 'number' ? <p>{t('execution.exitCode', { code: data.exitCode })}</p> : null}{data.elapsed_time_seconds !== undefined ? <p className="agent-reply-note">{t('execution.elapsed', { seconds: String(data.elapsed_time_seconds) })}</p> : null}{data.error ? <section className="agent-reply-error"><NativeValue value={data.error} /></section> : null}<small>{t('execution.source', { seq: event.seq })}</small></section>;
  }
  if (event.kind === 'message') {
    const content = data.content ?? object(data.message).content;
    const todos = (Array.isArray(content) ? content : []).flatMap(value => { const block = object(value); const input = object(block.input); return block.type === 'tool_use' && block.name === 'TodoWrite' && Array.isArray(input.todos) ? input.todos : []; });
    if (todos.length) return <PlanView plan={todos} />;
  }
  return null;
}
