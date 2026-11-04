import { useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession, type Engine, type Json, type Pending, type AgentEvent as Event } from '../../stores/agentRunnerStore';
import Markdown from '../../components/chat/Markdown';
import ExecutionDetails, { NativeValue } from '../../components/chat/ExecutionDetails';
import { SessionCapabilitySnapshot } from '../../components/development/CapabilitySnapshotView';
import { HostCapabilityConfirmations } from '../../components/development/HostCapabilityConfirmations';

function object(value: Json): { [key: string]: Json } { return value && typeof value === 'object' && !Array.isArray(value) ? value : {}; }
function text(value: Json | undefined) { return typeof value === 'string' ? value : ''; }

// Live conversation for the selected runtime. `drafts` is owned by the parent and keyed by session id, so a draft
// survives switching sessions and is never shown under another session.
export function AgentConversation({ session, drafts, setDraft, busy, error }: { session: AgentSession; drafts: { [id: string]: string }; setDraft: (id: string, value: string) => void; busy: boolean; error: boolean }) {
  const { t } = useLocale();
  const action = useAgentRunner(state => state.action);
  const active = ['running', 'waiting'].includes(session.status);
  const prompt = drafts[session.id] ?? '';
  const lastComplete = session.events.reduce((index, event, current) => ['agentMessage', 'message', 'result', 'turn/completed'].includes(event.kind) ? current : index, -1);
  const liveText = session.events.slice(lastComplete + 1).filter(event => ['item/agentMessage/delta', 'text_delta'].includes(event.kind)).map(event => event.text ?? '').join('');
  const [sending, setSending] = useState(false);
  async function send() {
    if (sending || busy || session.status !== 'idle') return;
    setSending(true);
    const sent = await action('send_agent_message', { input: { session_id: session.id, request_id: crypto.randomUUID(), text: prompt } });
    if (sent) setDraft(session.id, '');
    setSending(false);
  }
  const toolResults = new Map<string, Json>();
  const toolNames = new Map<string, string>();
  const completedItems = new Set<string>();
  for (const event of session.events) {
    const data = object(event.data);
    if (event.kind !== 'item/started' && typeof data.id === 'string') completedItems.add(`${event.run ?? ''}:${data.id}`);
    if (Array.isArray(data.content)) for (const block of data.content) {
      const value = object(block);
      if (value.type === 'tool_result') toolResults.set(`${event.run ?? ''}:${text(value.tool_use_id)}`, block);
      if (value.type === 'tool_use') toolNames.set(`${event.run ?? ''}:${text(value.id)}`, text(value.name));
    }
  }
  return <>
    <div className="flex flex-wrap items-center gap-2"><strong>{session.engine === 'codex' ? 'Codex' : 'Claude'}</strong><span role="status" className="text-sm">{t(`agent.state.${session.status}`)}</span><Button variant="secondary" disabled={!active} onClick={() => void action('interrupt_agent_session', { sessionId: session.id })}>{t('agent.interrupt')}</Button><Button variant="secondary" disabled={session.status === 'closed'} onClick={() => void action('close_agent_session', { sessionId: session.id })}>{t('agent.close')}</Button></div>
    <SessionCapabilitySnapshot sessionId={session.id} snapshot={session.capabilities} />
    <div className="agent-messages" aria-label={t('agent.messages')}>
      {session.history_truncated ? <p className="text-sm text-muted-foreground">{t('agent.truncated')}</p> : null}
      {session.events.filter(event => !['item/agentMessage/delta', 'text_delta'].includes(event.kind) && !(event.kind === 'item/started' && completedItems.has(`${event.run ?? ''}:${text(object(event.data).id)}`))).map(event => <EventContent key={`${session.id}:${event.seq}`} event={event} toolResults={toolResults} toolNames={toolNames} />)}
      {liveText ? <Markdown source={liveText} /> : null}
      {!session.events.length ? <p className="text-sm text-muted-foreground">{t('agent.empty')}</p> : null}
    </div>
    {session.pending.map(pending => <Request key={`${session.id}:${pending.id}`} engine={session.engine} pending={pending} submit={response => action('respond_agent_request', { input: { session_id: session.id, request_id: pending.id, response } })} />)}
    <HostCapabilityConfirmations key={session.id} sessionId={session.id} />
    <div className="agent-composer"><Input multiline rows={3} id="agent-message" label={t('agent.prompt')} value={prompt} onChange={e => setDraft(session.id, e.target.value)} />
      <Button disabled={busy || sending || session.status !== 'idle' || !prompt.trim()} onClick={() => void send()}>{t('agent.send')}</Button></div>
    {session.error && !error ? <p role="alert" className="text-sm text-destructive">{t('agent.failure')}<br />{session.error}</p> : null}
  </>;
}

export function EventContent({ event, toolResults = new Map<string, Json>(), toolNames = new Map<string, string>() }: { event: Event; toolResults?: ReadonlyMap<string, Json>; toolNames?: ReadonlyMap<string, string> }) {
  const { t } = useLocale();
  const data = object(event.data);
  const content = data.content;
  const blocks = Array.isArray(content) ? content : [];
  const tools = blocks.some(block => ['tool_use', 'tool_result'].includes(text(object(block).type)));
  const failed = ['error', 'failure'].includes(event.kind) || Boolean(data.error) || data.run_status === 'failed';
  const label = event.kind === 'userMessage' || (data.type === 'user' && !tools) ? t('agent.you') : event.kind === 'agentMessage' || data.type === 'assistant' ? t('agent.assistant') : event.kind === 'turn/completed' || event.kind === 'result' ? t('agent.finished') : t('agent.activity');
  return <article className={`min-w-0 space-y-2 ${failed ? 'text-destructive' : ''}`} role={failed ? 'alert' : undefined}><strong className="text-xs text-muted-foreground">{label}</strong>
    <ExecutionDetails event={event} />
    {event.text ? event.kind.includes('outputDelta') || failed ? <pre className="type-code overflow-x-auto whitespace-pre-wrap break-words text-sm">{event.text}</pre> : <Markdown source={event.text} /> : null}
    {typeof content === 'string' ? <Markdown source={content} /> : null}
    {blocks.map((block, index) => {
      const value = object(block);
      if (value.type === 'tool_use') {
        const result = toolResults.get(`${event.run ?? ''}:${text(value.id)}`);
        return <section key={index} className="min-w-0 space-y-2 border-l-2 border-border pl-3"><strong>{text(value.name)}</strong><span className="ml-2 text-xs">{t(result ? object(result).is_error ? 'execution.run_failed' : 'execution.completed' : 'execution.in_progress')}</span><NativeValue value={value.input} /></section>;
      }
      if (value.type === 'tool_result') return <section key={index} role={value.is_error ? 'alert' : undefined} className={`min-w-0 space-y-2 border-l-2 border-border pl-3 ${value.is_error ? 'text-destructive' : ''}`}><strong>{toolNames.get(`${event.run ?? ''}:${text(value.tool_use_id)}`) || text(value.tool_use_id)}</strong><span className="ml-2 text-xs">{t(value.is_error ? 'execution.run_failed' : 'execution.completed')}</span><NativeValue value={value.content} /></section>;
      if (value.type === 'text' || typeof value.text === 'string') return <Markdown key={index} source={text(value.text)} />;
      return <NativeValue key={index} value={block} />;
    })}
    <NativeValue value={data.tool_use_result} />
    {data.error ? <div role="alert"><NativeValue value={data.error} /></div> : null}
    {event.data !== null ? <details><summary className="cursor-pointer text-xs text-muted-foreground">{t('agent.details')}</summary><pre className="overflow-x-auto whitespace-pre-wrap break-words text-xs">{JSON.stringify(event.data, null, 2)}</pre></details> : null}
  </article>;
}

export function Request({ engine, pending, submit }: { engine: Engine; pending: Pending; submit: (response: Json) => Promise<boolean> }) {
  const { t } = useLocale();
  const [answers, setAnswers] = useState<{ [key: string]: string[] }>({});
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const data = object(pending.data);
  const input = engine === 'claude' ? object(data.input ?? null) : data;
  const questions = (engine === 'claude' ? pending.title === 'AskUserQuestion' : pending.kind === 'tool/requestUserInput') && Array.isArray(input.questions) ? input.questions : [];
  const schema = object(data.requestedSchema ?? null);
  const fields = pending.kind === 'mcpServer/elicitation/request' ? object(schema.properties ?? null) : {};
  const [form, setForm] = useState<{ [key: string]: Json }>({});
  const respond = async (value: Json) => { if (busy) return; setBusy(true); setFailed(false); try { if (!(await submit(value))) { setFailed(true); setBusy(false); } } catch { setFailed(true); setBusy(false); } };
  const accept = (): Json => {
    if (engine === 'claude') return { behavior: 'allow', ...(questions.length ? { answers: Object.fromEntries(Object.entries(answers).map(([key, value]) => [key, value.join(', ')])) } : {}) };
    if (pending.kind === 'tool/requestUserInput') return { answers: Object.fromEntries(Object.entries(answers).map(([key, value]) => [key, { answers: value }])) };
    if (pending.kind === 'mcpServer/elicitation/request') return { action: 'accept', content: data.mode === 'url' ? null : form };
    return { decision: 'accept' };
  };
  const approval = engine === 'codex' && ['item/commandExecution/requestApproval', 'item/fileChange/requestApproval'].includes(pending.kind);
  const decisions = Array.isArray(data.availableDecisions) ? data.availableDecisions.filter(value => typeof value === 'string' && ['accept', 'acceptForSession', 'decline', 'cancel'].includes(value)) : ['accept', 'decline'];
  return <section className="grid gap-3 rounded-md border border-border p-3" aria-label={t('agent.approval')}><strong>{engine === 'claude' ? pending.title : t('agent.approval')}</strong>
    {text(input.command) ? <pre className="overflow-x-auto whitespace-pre-wrap text-sm">{text(input.command)}</pre> : null}
    {text(data.message) || text(data.reason) ? <p className="m-0 text-sm">{text(data.message) || text(data.reason)}</p> : null}
    {text(data.url) ? <p className="break-all text-sm">{text(data.url)}</p> : null}
    <details><summary className="cursor-pointer text-xs">{t('agent.details')}</summary><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs">{JSON.stringify(pending.data, null, 2)}</pre></details>
    {questions.map((value, i) => {
      const question = object(value);
      const key = engine === 'claude' ? text(question.question) || String(i) : text(question.id) || String(i);
      const choices = Array.isArray(question.options) ? question.options : [];
      const selected = answers[key] ?? [];
      return <fieldset key={key} className="min-w-0 space-y-2" disabled={busy}><legend className="text-sm">{text(question.header) || text(question.question) || key}</legend>
        <Input type={question.isSecret ? 'password' : 'text'} label={text(question.question) || key} value={selected.join(', ')} onChange={e => setAnswers(current => ({ ...current, [key]: e.target.value ? [e.target.value] : [] }))} />
        <div className="flex flex-wrap gap-2">{choices.map((choice, index) => { const option = object(choice); const label = text(option.label); return <Button key={index} variant="secondary" title={text(option.description)} aria-pressed={selected.includes(label)} onClick={() => setAnswers(current => ({ ...current, [key]: question.multiSelect ? selected.includes(label) ? selected.filter(item => item !== label) : [...selected, label] : [label] }))}>{label}</Button>; })}</div>
      </fieldset>;
    })}
    {Object.entries(fields).map(([key, value]) => { const field = object(value); const label = text(field.title) || key; return field.type === 'boolean' ? <label key={key} className="flex gap-2 text-sm"><input type="checkbox" checked={form[key] === true} onChange={e => setForm(current => ({ ...current, [key]: e.target.checked }))} />{label}</label> : <Input key={key} label={label} type={field.type === 'number' || field.type === 'integer' ? 'number' : 'text'} value={String(form[key] ?? '')} onChange={e => setForm(current => ({ ...current, [key]: field.type === 'number' || field.type === 'integer' ? Number(e.target.value) : field.type === 'array' ? e.target.value.split(',').map(s => s.trim()) : e.target.value }))} />; })}
    {failed ? <p role="alert" className="text-sm text-destructive">{t('agent.failure')}</p> : null}
    <div className="flex flex-wrap gap-2">{approval ? decisions.map(decision => <Button key={String(decision)} variant="secondary" disabled={busy} onClick={() => void respond({ decision })}>{decision === 'accept' ? t('agent.allow') : decision === 'decline' ? t('agent.deny') : decision === 'cancel' ? t('common.cancel') : `${t('agent.allow')} (${decision})`}</Button>) : <><Button disabled={busy} onClick={() => void respond(accept())}>{t(questions.length ? 'agent.answer' : 'agent.allow')}</Button><Button variant="secondary" disabled={busy} onClick={() => void respond(engine === 'claude' ? { behavior: 'deny' } : pending.kind === 'mcpServer/elicitation/request' ? { action: 'decline', content: null } : pending.kind === 'tool/requestUserInput' ? { answers: {} } : { decision: 'decline' })}>{t('agent.deny')}</Button></>}</div>
  </section>;
}
