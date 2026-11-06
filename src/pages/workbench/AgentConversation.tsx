import { useEffect, useMemo, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession, type Engine, type Json, type Pending, type AgentEvent as Event } from '../../stores/agentRunnerStore';
import Markdown from '../../components/chat/Markdown';
import ExecutionDetails, { NativeValue } from '../../components/chat/ExecutionDetails';
import { SessionCapabilitySnapshot } from '../../components/development/CapabilitySnapshotView';
import { HostCapabilityConfirmations, HostConfirmationRequests, type HostConfirmationProjection } from '../../components/development/HostCapabilityConfirmations';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useDevelopment } from '../../stores/developmentStore';
import { AgentOutputReferences } from '../../components/development/AgentOutputReferences';
import { agentEventFailed, type AgentReferenceActions } from '../../components/development/eventOutputReferences';
import './AgentConversation.css';
import { useSurfaceActive } from '../../workspace/SurfaceContext';

function object(value: Json | undefined): { [key: string]: Json } { return value && typeof value === 'object' && !Array.isArray(value) ? value : {}; }
function text(value: Json | undefined) { return typeof value === 'string' ? value : ''; }
function blocks(event: Event): Json[] { const content = object(event.data).content; return Array.isArray(content) ? content : []; }
function prose(event: Event): string {
  const data = object(event.data);
  return event.text || (typeof data.content === 'string' ? data.content : blocks(event).filter(block => object(block).type === 'text').map(block => text(object(block).text)).join('\n\n'));
}
function isUser(event: Event) { return event.kind === 'userMessage' || object(event.data).type === 'user' && (Boolean(prose(event)) || !blocks(event).some(block => object(block).type === 'tool_result')); }
const isFailed = agentEventFailed;
function itemKey(event: Event, id: string) { return `${event.run ?? ''}:${id}`; }

type TranscriptRow = { event: Event; hidden?: boolean; suppressText?: boolean };
// Projection only: every native event remains available in the inspector. Coalesce deltas by
// native item/run; only a matching full snapshot supersedes its stream, never a later turn.
function transcript(events: Event[]) {
  const rows: TranscriptRow[] = [];
  const streams = new Map<string, TranscriptRow>();
  const outputs = new Map<string, TranscriptRow>();
  const completed = new Set(events.filter(event => event.kind !== 'item/started' && text(object(event.data).id)).map(event => itemKey(event, text(object(event.data).id))));
  const toolResults = new Map<string, Json>();
  const toolNames = new Map<string, string>();
  for (const event of events) for (const block of blocks(event)) {
    const value = object(block);
    if (value.type === 'tool_result') toolResults.set(itemKey(event, text(value.tool_use_id)), block);
    if (object(event.data).type === 'assistant' && value.type === 'tool_use') toolNames.set(itemKey(event, text(value.id)), text(value.name));
  }
  let lastAssistant = '';
  let lastRun: string | null | undefined;
  for (const event of events) {
    const data = object(event.data);
    if (isUser(event) || event.run !== lastRun) { lastAssistant = ''; lastRun = event.run; streams.clear(); outputs.clear(); }
    if (['item/agentMessage/delta', 'text_delta'].includes(event.kind)) {
      const key = itemKey(event, text(data.itemId));
      let row = streams.get(key);
      if (!row) { row = { event: { ...event, kind: 'agentMessage', text: '', data: null } }; streams.set(key, row); rows.push(row); }
      row.event.text = (row.event.text ?? '') + (event.text ?? '');
      continue;
    }
    if (event.kind.endsWith('/outputDelta')) {
      const key = itemKey(event, text(data.itemId));
      let row = outputs.get(key);
      if (!row) { row = { event: { ...event, text: '' } }; outputs.set(key, row); rows.push(row); }
      row.event.text = (row.event.text ?? '') + (event.text ?? '');
      continue;
    }
    const source = prose(event);
    const terminal = event.kind === 'result' || event.kind === 'turn/completed';
    if (event.kind === 'agentMessage' || data.type === 'assistant' || terminal) {
      const key = itemKey(event, text(data.id));
      const stream = streams.get(key);
      if (stream && source && source.startsWith(stream.event.text ?? '')) { stream.hidden = true; streams.delete(key); }
      if (source && !terminal) lastAssistant = source.trim();
      // A terminal status often repeats Claude's last assistant answer. Keep its status,
      // errors and raw source, but do not print that answer a second time.
      const repeated = terminal && !isFailed(event) && source.trim() !== '' && source.trim() === lastAssistant;
      rows.push({ event, suppressText: repeated });
      if (terminal || event.kind === 'message') streams.clear();
      continue;
    }
    const output = outputs.get(itemKey(event, text(data.id)));
    if (output && typeof data.aggregatedOutput === 'string' && data.aggregatedOutput.includes(output.event.text ?? '')) output.hidden = true;
    if (event.kind === 'item/started' && completed.has(itemKey(event, text(data.id)))) continue;
    if (event.kind === 'tool_progress' && toolNames.has(itemKey(event, text(data.tool_use_id)))) continue;
    rows.push({ event });
  }
  return { rows: rows.filter(row => !row.hidden), toolResults, toolNames };
}

// Parent-owned drafts are keyed by session and reset with the account/context projection.
export function AgentConversation({ session, drafts, setDraft, busy, error, confirmations, onOpenReference, onSelectRun }: AgentReferenceActions & { session: AgentSession; drafts: { [id: string]: string }; setDraft: (id: string, value: string) => void; busy: boolean; error: boolean; confirmations?: HostConfirmationProjection }) {
  const { t } = useLocale();
  const action = useAgentRunner(state => state.action);
  const epoch = useAgentRunner(state => state.epoch);
  const devEpoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  const prompt = drafts[session.id] ?? '';
  const [sending, setSending] = useState(false);
  const sendingRef = useRef(false);
  const draftRef = useRef({ id: session.id, prompt, epoch, generation });
  draftRef.current = { id: session.id, prompt, epoch, generation };
  const viewport = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const requestFocus = useRef<HTMLElement | null>(null);
  const surfaceActive = useSurfaceActive();
  useEffect(() => {
    const previous = requestFocus.current;
    if (!previous || previous.isConnected || !surfaceActive || !root.current?.getClientRects().length || document.activeElement !== document.body) return;
    const target = root.current.querySelector<HTMLElement>('.agent-dialogue-requests button:not(:disabled)') ?? root.current.querySelector<HTMLElement>('#agent-message');
    requestFocus.current = null;
    target?.focus({ preventScroll: true });
  });
  const follow = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const [inspect, setInspect] = useState(false);
  const { rows, toolResults, toolNames } = useMemo(() => transcript(session.events), [session.events]);
  const reason = busy || sending ? t('agent.sendBusy') : session.pending.length || session.status === 'waiting' ? t('agent.sendWaiting') : session.status === 'running' ? t('agent.sendRunning') : session.status !== 'idle' ? t('agent.sendUnavailable') : !prompt.trim() ? t('agent.sendEmpty') : '';
  function latest() { const node = viewport.current; if (node) node.scrollTop = node.scrollHeight; follow.current = true; setAtBottom(true); }
  useEffect(() => { follow.current = true; setInspect(false); latest(); }, [session.id, epoch, generation]);
  useEffect(() => { if (follow.current) latest(); }, [session.events[session.events.length - 1]?.seq]);
  const navigationOwner = useRef({ sessionId: session.id, epoch, devEpoch, generation, mounted: true });
  navigationOwner.current = { sessionId: session.id, epoch, devEpoch, generation, mounted: true };
  useEffect(() => { navigationOwner.current.mounted = true; return () => { navigationOwner.current.mounted = false; }; }, []);
  const currentOwner = () => navigationOwner.current.mounted && navigationOwner.current.sessionId === session.id && navigationOwner.current.epoch === epoch && navigationOwner.current.devEpoch === devEpoch && navigationOwner.current.generation === generation && epoch === useAgentRunner.getState().epoch && devEpoch === useDevelopment.getState().epoch && generation === contextGeneration() && useAgentRunner.getState().selected === session.id;
  const referenceActions: AgentReferenceActions = {
    onOpenReference: onOpenReference ? (reference, mode) => currentOwner() ? onOpenReference(reference, mode) : undefined : undefined,
    onSelectRun: onSelectRun ? runId => { if (currentOwner() && session.events.some(event => event.run === runId)) onSelectRun(runId); } : undefined,
  };
  async function send() {
    if (sendingRef.current || reason || !prompt.trim() || epoch !== useAgentRunner.getState().epoch || devEpoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    const id = session.id;
    const sentPrompt = prompt;
    sendingRef.current = true; setSending(true);
    try {
      const sent = await action('send_agent_message', { input: { session_id: id, request_id: crypto.randomUUID(), text: sentPrompt } });
      if (sent && epoch === useAgentRunner.getState().epoch && devEpoch === useDevelopment.getState().epoch && generation === contextGeneration() && useAgentRunner.getState().selected === id && draftRef.current.id === id && draftRef.current.prompt === sentPrompt && draftRef.current.epoch === epoch && draftRef.current.generation === generation) setDraft(id, '');
    } finally { sendingRef.current = false; setSending(false); }
  }
  return <div ref={root} className="agent-conversation-product" onFocusCapture={event => { requestFocus.current = (event.target as HTMLElement).closest('.agent-dialogue-requests') ? event.target as HTMLElement : null; }}>
    <header className="agent-dialogue-header">
      <div className="agent-dialogue-identity"><strong>{t(`agent.engine.${session.engine}`)}</strong><span className="agent-dialogue-status" data-state={session.status}>{t(`agent.state.${session.status}`)}</span></div>
      <div className="agent-dialogue-actions"><Button size="xs" variant="secondary" disabled={busy || !['running', 'waiting'].includes(session.status)} onClick={() => void action('interrupt_agent_session', { sessionId: session.id })}>{t('agent.interrupt')}</Button><Button size="xs" variant="secondary" disabled={busy || session.status === 'closed'} onClick={() => void action('close_agent_session', { sessionId: session.id })}>{t('agent.close')}</Button></div>
    </header>
    <div className="agent-dialogue-transcript" ref={viewport} role="region" tabIndex={0} aria-label={t('agent.messages')} onScroll={event => { const node = event.currentTarget; const bottom = node.scrollHeight - node.scrollTop - node.clientHeight < 64; follow.current = bottom; setAtBottom(bottom); }}>
      <details className="agent-dialogue-capabilities"><summary>{t('capabilities.actual')}</summary><SessionCapabilitySnapshot key={session.id} sessionId={session.id} snapshot={session.capabilities} /></details>
      <div className="agent-dialogue-messages">
        {session.history_truncated ? <p className="text-sm text-ink-secondary">{t('agent.truncated')}</p> : null}
        {rows.map(({ event, suppressText }) => <EventContent key={`${session.id}:${event.seq}`} event={event} suppressText={suppressText} toolResults={toolResults} toolNames={toolNames} {...referenceActions} />)}
        {!session.events.length ? <p className="agent-dialogue-empty">{t('agent.empty')}</p> : null}
      </div>
      {session.events.length ? <details className="agent-dialogue-inspector" open={inspect} onToggle={event => setInspect(event.currentTarget.open)}><summary>{t('agent.rawEvents', { count: session.events.length })}</summary>{inspect ? session.events.map(event => <NativeEvent key={event.seq} event={event} />) : null}</details> : null}
    </div>
    <footer className="agent-dialogue-footer">
      {!atBottom ? <Button size="xs" variant="secondary" className="agent-dialogue-latest" onClick={latest}>{t('agent.latest')}</Button> : null}
      <div className="agent-dialogue-requests">
        {session.status !== 'closed' ? session.pending.map(pending => <Request key={`${epoch}:${generation}:${session.id}:${pending.id}`} engine={session.engine} pending={pending} submit={response => epoch === useAgentRunner.getState().epoch && devEpoch === useDevelopment.getState().epoch && generation === contextGeneration() ? action('respond_agent_request', { input: { session_id: session.id, request_id: pending.id, response } }) : Promise.resolve(false)} />) : null}
        {session.status !== 'closed' ? confirmations ? <HostConfirmationRequests confirmations={confirmations} /> : <HostCapabilityConfirmations key={`${epoch}:${generation}:${session.id}`} sessionId={session.id} /> : null}
      </div>
      {session.error && !error ? <p role="alert" className="agent-dialogue-error text-status-error">{t('agent.failure')}<br />{session.error}</p> : null}
      <div className="agent-dialogue-composer"><Input multiline rows={2} id="agent-message" label={t('agent.prompt')} className="type-reading" aria-describedby="agent-send-hint" value={prompt} onChange={event => setDraft(session.id, event.target.value)} onKeyDown={event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) { event.preventDefault(); if (!event.repeat) void send(); } }} />
        <div className="agent-dialogue-send"><p id="agent-send-hint">{reason || t('agent.sendShortcut')}</p><Button disabled={Boolean(reason)} onClick={() => void send()}>{t('agent.send')}</Button></div>
      </div>
    </footer>
  </div>;
}

function NativeEvent({ event }: { event: Event }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  return <details className="agent-dialogue-native" onToggle={event => setOpen(event.currentTarget.open)}><summary>{t('agent.details')} · {event.kind} · #{event.seq}</summary>{open ? <pre className="type-code">{JSON.stringify(event, null, 2)}</pre> : null}</details>;
}

function ToolBlock({ event, block, toolResults, toolNames }: { event: Event; block: Json; toolResults: ReadonlyMap<string, Json>; toolNames: ReadonlyMap<string, string> }) {
  const { t } = useLocale();
  const value = object(block);
  const use = value.type === 'tool_use';
  const result = use ? toolResults.get(itemKey(event, text(value.id))) : block;
  const failed = Boolean(object(result).is_error);
  const name = use ? text(value.name) : toolNames.get(itemKey(event, text(value.tool_use_id))) || text(value.tool_use_id);
  const command = text(object(value.input).command) || text(object(value.input).file_path) || text(object(value.input).pattern);
  return <details className={`agent-dialogue-tool ${failed ? 'text-status-error' : ''}`}><summary><strong>{name || t('agent.activity')}</strong><span>{t(result ? failed ? 'execution.run_failed' : 'execution.completed' : 'execution.in_progress')}</span>{command ? <code>{command}</code> : null}</summary><div className="agent-dialogue-tool-body">{use ? <NativeValue value={value.input} /> : null}{result ? <NativeValue value={object(result).content} /> : null}</div></details>;
}

export function EventContent({ event, suppressText = false, toolResults = new Map<string, Json>(), toolNames = new Map<string, string>(), onOpenReference, onSelectRun }: AgentReferenceActions & { event: Event; suppressText?: boolean; toolResults?: ReadonlyMap<string, Json>; toolNames?: ReadonlyMap<string, string> }) {
  const { t } = useLocale();
  const data = object(event.data);
  const content = blocks(event);
  const failed = isFailed(event) || content.some(block => object(block).type === 'tool_use' && object(toolResults.get(itemKey(event, text(object(block).id)))).is_error === true);
  const user = isUser(event);
  const assistant = event.kind === 'agentMessage' || data.type === 'assistant';
  const terminal = event.kind === 'result' || event.kind === 'turn/completed';
  const dialogue = user || assistant || terminal;
  const source = !suppressText && dialogue ? prose(event) : '';
  const tools = content.filter(block => { const value = object(block); return value.type === 'tool_use' || value.type === 'tool_result' && !toolNames.has(itemKey(event, text(value.tool_use_id))); });
  const command = text(data.command) || text(data.query) || text(object(data.action).query);
  const type = text(data.type) || event.kind;
  const name = text(data.tool) || text(data.tool_name) || (type === 'commandExecution' ? t('direct.command') : type === 'fileChange' ? t('execution.artifacts') : type === 'webSearch' ? t('skills.search') : ['mcpToolCall', 'dynamicToolCall'].includes(type) ? t('capabilities.tool') : event.kind.endsWith('/outputDelta') ? t('direct.output') : t('agent.activity'));
  const execution = ['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch', 'fileChange', 'tool_progress', 'turn/plan/updated', 'turn/diff/updated'].includes(event.kind) || event.kind === 'item/started' && ['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch', 'fileChange'].includes(type) || event.kind.endsWith('/outputDelta');
  const status = terminal ? `execution.run_${['completed', 'failed', 'cancelled'].includes(text(data.run_status)) ? text(data.run_status) : 'unknown'}` : failed ? 'execution.run_failed' : event.kind === 'item/started' || event.kind === 'tool_progress' ? 'execution.in_progress' : ['completed', 'inProgress', 'failed'].includes(text(data.status)) ? `execution.${data.status === 'inProgress' ? 'in_progress' : data.status === 'failed' ? 'run_failed' : 'completed'}` : 'execution.unknown';
  // Reasoning/signatures and initialization metadata are retained in the native inspector,
  // not mistaken for assistant prose. Unknown native payloads remain inspectable there too.
  if (!dialogue && !execution && !failed && !tools.length && !content.some(block => object(block).type === 'tool_result')) return null;
  return <article data-testid={`agent-event-${event.seq}`} data-event-seq={event.seq} data-event-run={event.run ?? ''} data-event-kind={event.kind} className={`agent-dialogue-event ${user ? 'agent-dialogue-user' : ''} ${failed ? 'text-status-error-text' : ''}`}>
    {source ? <div className="agent-dialogue-prose"><strong className="agent-dialogue-speaker">{t(user ? 'agent.you' : 'agent.assistant')}</strong><Markdown source={source} /></div> : null}
    {tools.map((block, index) => <ToolBlock key={index} event={event} block={block} toolResults={toolResults} toolNames={toolNames} />)}
    {execution || terminal ? <details className="agent-dialogue-tool"><summary><strong>{terminal ? t('agent.finished') : event.kind === 'turn/plan/updated' ? t('execution.plan') : event.kind === 'turn/diff/updated' ? t('execution.diff') : name}</strong><span>{t(status)}</span>{command ? <code>{command}</code> : null}</summary><div className="agent-dialogue-tool-body"><ExecutionDetails event={event} />{event.kind.endsWith('/outputDelta') && event.text ? <pre className="type-code">{event.text}</pre> : null}<NativeEvent event={event} /></div></details> : null}
    {failed && !dialogue ? <div className="agent-dialogue-error">{event.text ? <pre>{event.text}</pre> : <NativeValue value={data.error} />}</div> : null}
    {failed && dialogue && data.error ? <NativeValue value={data.error} /> : null}
    <AgentOutputReferences event={event} toolNames={toolNames} onOpenReference={onOpenReference} onSelectRun={onSelectRun} />
  </article>;
}

export function Request({ engine, pending, submit }: { engine: Engine; pending: Pending; submit: (response: Json) => Promise<boolean> }) {
  const { t } = useLocale();
  const [answers, setAnswers] = useState<{ [key: string]: string[] }>({});
  const [busy, setBusy] = useState(false);
  const responding = useRef(false);
  const [failed, setFailed] = useState(false);
  const data = object(pending.data);
  const input = engine === 'claude' ? object(data.input ?? null) : data;
  const questions = (engine === 'claude' ? pending.title === 'AskUserQuestion' : pending.kind === 'tool/requestUserInput') && Array.isArray(input.questions) ? input.questions : [];
  const schema = object(data.requestedSchema ?? null);
  const fields = pending.kind === 'mcpServer/elicitation/request' ? object(schema.properties ?? null) : {};
  const [form, setForm] = useState<{ [key: string]: Json }>({});
  const respond = async (value: Json) => { if (responding.current) return; responding.current = true; setBusy(true); setFailed(false); try { if (await submit(value)) return; } catch { /* Keep the same request and answer available for an explicit retry. */ } responding.current = false; setFailed(true); setBusy(false); };
  const accept = (): Json => {
    if (engine === 'claude') return { behavior: 'allow', ...(questions.length ? { answers: Object.fromEntries(Object.entries(answers).map(([key, value]) => [key, value.join(', ')])) } : {}) };
    if (pending.kind === 'tool/requestUserInput') return { answers: Object.fromEntries(Object.entries(answers).map(([key, value]) => [key, { answers: value }])) };
    if (pending.kind === 'mcpServer/elicitation/request') return { action: 'accept', content: data.mode === 'url' ? null : form };
    return { decision: 'accept' };
  };
  const approval = engine === 'codex' && ['item/commandExecution/requestApproval', 'item/fileChange/requestApproval'].includes(pending.kind);
  const decisions = Array.isArray(data.availableDecisions) ? data.availableDecisions.filter(value => typeof value === 'string' && ['accept', 'acceptForSession', 'decline', 'cancel'].includes(value)) : ['accept', 'decline'];
  return <section className="grid gap-3 rounded-md border border-border-subtle p-3" aria-label={t('agent.approval')}><strong>{engine === 'claude' ? pending.title : t('agent.approval')}</strong>
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
    {failed ? <p role="alert" className="text-sm text-status-error">{t('agent.failure')}</p> : null}
    <div className="flex flex-wrap gap-2">{approval ? decisions.map(decision => <Button key={String(decision)} variant="secondary" disabled={busy} onClick={() => void respond({ decision })}>{decision === 'accept' ? t('agent.allow') : decision === 'decline' ? t('agent.deny') : decision === 'cancel' ? t('common.cancel') : t('agent.allowSession')}</Button>) : <><Button disabled={busy} onClick={() => void respond(accept())}>{t(questions.length ? 'agent.answer' : 'agent.allow')}</Button><Button variant="secondary" disabled={busy} onClick={() => void respond(engine === 'claude' ? { behavior: 'deny' } : pending.kind === 'mcpServer/elicitation/request' ? { action: 'decline', content: null } : pending.kind === 'tool/requestUserInput' ? { answers: {} } : { decision: 'decline' })}>{t('agent.deny')}</Button></>}</div>
  </section>;
}
