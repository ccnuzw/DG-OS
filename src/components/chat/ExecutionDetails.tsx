import { useLocale } from '../../i18n';
import type { AgentEvent, Json } from '../../stores/agentRunnerStore';
const object = (value: Json | undefined): { [key: string]: Json } => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
const text = (value: Json | undefined) => typeof value === 'string' ? value : '';
export function NativeValue({ value }: { value: Json | undefined }) {
  if (value === undefined || value === null) return null;
  if (Array.isArray(value)) return <div className="space-y-2">{value.map((entry, index) => <NativeValue key={index} value={entry} />)}</div>;
  if (typeof value === 'object') return <dl className="min-w-0 space-y-1">{Object.entries(value).map(([key, entry]) => <div key={key}><dt className="text-xs text-ink-secondary">{key}</dt><dd className="m-0 pl-3"><NativeValue value={entry} /></dd></div>)}</dl>;
  return <pre className="type-code m-0 overflow-x-auto whitespace-pre-wrap break-words text-sm">{String(value)}</pre>;
}
// Only native protocol fields enter these views. Paths are metadata, never filesystem read handles.
export default function ExecutionDetails({ event }: { event: AgentEvent }) {
  const { t } = useLocale();
  const data = object(event.data);
  if (event.kind === 'turn/completed' || event.kind === 'result') {
    const status = ['completed', 'failed', 'cancelled'].includes(text(data.run_status)) ? text(data.run_status) : 'unknown';
    return <section aria-label={t('execution.run')} className={`min-w-0 space-y-1 ${status === 'failed' ? 'text-status-error' : ''}`}><strong data-run-status={status}>{t(`execution.run_${status}`)}</strong><NativeValue value={data.native_status} /><NativeValue value={data.terminal_reason} /><NativeValue value={data.errors} /><small>{t('execution.source', { seq: event.seq })}</small></section>;
  }
  if (event.kind === 'turn/plan/updated') {
    const plan = Array.isArray(data.plan) ? data.plan : [];
    return <section aria-label={t('execution.plan')} className="min-w-0 space-y-2"><strong>{t('execution.plan')}</strong><ol className="list-decimal pl-5">{plan.map((value, index) => { const item = object(value); const status = text(item.status) === 'inProgress' ? 'in_progress' : text(item.status); return <li key={index} className="break-words"><span>{text(item.step)}</span> <span className="text-xs text-ink-secondary">{t(`execution.${['pending', 'in_progress', 'completed'].includes(status) ? status : 'unknown'}`)}</span></li>; })}</ol><small>{t('execution.source', { seq: event.seq })}</small></section>;
  }
  if (event.kind === 'turn/diff/updated') return <details open className="min-w-0"><summary>{t('execution.diff')}</summary><pre className="type-code overflow-x-auto whitespace-pre-wrap break-words">{event.text}</pre><small>{t('execution.source', { seq: event.seq })}</small></details>;
  if (['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch', 'item/started', 'tool_progress'].includes(event.kind) && data.type !== 'fileChange') {
    const started = event.kind === 'item/started';
    const failed = data.status === 'failed' || data.success === false || (typeof data.exitCode === 'number' && data.exitCode !== 0) || Boolean(data.error);
    return <section className={`min-w-0 space-y-2 border-l-2 border-border-subtle pl-3 ${failed ? 'text-status-error' : ''}`}>
      <strong>{text(data.tool) || text(data.tool_name) || text(data.type) || event.kind}</strong>
      <span className="ml-2 text-xs">{started ? t('execution.in_progress') : text(data.status) || (event.kind === 'tool_progress' ? t('execution.in_progress') : t('execution.unknown'))}</span>
      <NativeValue value={data.command} /><NativeValue value={data.cwd} /><NativeValue value={data.arguments} /><NativeValue value={data.query} /><NativeValue value={data.action} />
      <NativeValue value={data.aggregatedOutput} /><NativeValue value={data.result} /><NativeValue value={data.contentItems} />
      {typeof data.exitCode === 'number' ? <p className="m-0 text-sm">{t('execution.exitCode', { code: data.exitCode })}</p> : !started && data.type === 'commandExecution' ? <p className="m-0 text-sm">{t('execution.exitCode', { code: t('execution.unknown') })}</p> : null}
      {data.elapsed_time_seconds !== undefined ? <p className="m-0 text-xs">{t('execution.elapsed', { seconds: String(data.elapsed_time_seconds) })}</p> : null}
      {data.error ? <div role="alert"><NativeValue value={data.error} /></div> : null}
      <small>{t('execution.source', { seq: event.seq })}</small>
    </section>;
  }
  if ((event.kind === 'fileChange' || data.type === 'fileChange') && Array.isArray(data.changes)) return <section aria-label={t('execution.artifacts')} className="min-w-0 space-y-2"><strong>{t('execution.artifacts')}</strong><span className="ml-2 text-xs">{event.kind === 'item/started' ? t('execution.in_progress') : text(data.status) || t('execution.unknown')}</span><p className="text-xs text-ink-secondary">{t('execution.metadata')}</p>{data.changes.map((value, index) => { const change = object(value); const kind = object(change.kind); return <details key={index} open><summary className="break-all">{text(change.path)} · {text(kind.type) || text(change.kind)}</summary>{typeof change.diff === 'string' && <pre className="type-code overflow-x-auto whitespace-pre-wrap break-words">{change.diff}</pre>}</details>; })}<NativeValue value={data.error} /><small>{t('execution.source', { seq: event.seq })}</small></section>;
  if (event.kind === 'message') {
    const content = data.content;
    const blocks = Array.isArray(content) ? content : Array.isArray(object(data.message).content) ? object(data.message).content as Json[] : [];
    const todos = blocks.flatMap(value => { const block = object(value); const input = object(block.input); return block.type === 'tool_use' && block.name === 'TodoWrite' && Array.isArray(input.todos) ? input.todos : []; });
    if (todos.length) return <section aria-label={t('execution.plan')}><strong>{t('execution.plan')}</strong><ol className="list-decimal pl-5">{todos.map((todo, index) => { const item = object(todo); return <li key={index}>{text(item.content)} <span className="text-xs">{t(`execution.${['pending', 'in_progress', 'completed'].includes(text(item.status)) ? text(item.status) : 'unknown'}`)}</span></li>; })}</ol></section>;
  }
  return null;
}
