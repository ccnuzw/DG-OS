import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession, type Engine, type Json, type Pending, type AgentEvent as Event } from '../../stores/agentRunnerStore';
import Markdown from '../../components/chat/Markdown';
import ExecutionDetails, { NativeValue } from '../../components/chat/ExecutionDetails';
import { SessionCapabilitySnapshot } from '../../components/development/CapabilitySnapshotView';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useDevelopment } from '../../stores/developmentStore';
import { AgentOutputReferences } from '../../components/development/AgentOutputReferences';
import { agentEventFailed, agentOutputReferences, type AgentReferenceActions } from '../../components/development/eventOutputReferences';
import './AgentConversation.css';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { listDevelopmentRuns, type DevRun } from '../../stores/developmentReview';
import { errorText } from '../../components/development/shared';
import { ComposerModelPicker } from '../../components/development/ComposerModelPicker';
import type { CliProviderConfiguration } from '../../stores/developmentCapabilities';

import { AgentComposerControls } from '../../components/development/AgentComposerControls';
import { agentTurnSettings } from '../../stores/agentTurnSettings';
import { captureConversationReferences, clearConversationReferences, type CapturedConversationReference } from '../../stores/developmentReferences';
import { useComposerReferences } from '../../components/development/ComposerReferences';
import { developmentDraftKey, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { acknowledgeAgentSkills, captureAgentSkills, type CapturedAgentSkill } from '../../stores/agentSkills';
import { projectCodexTranscript, inspectableAgentEvent, isReasoningEvent, type CodexTranscriptEntry } from '../../components/development/codexTranscript';
import { CodexTranscriptContent } from '../../components/development/CodexTranscriptContent';
import { CodexTurnControls } from '../../components/development/CodexTurnControls';
import { NativeElicitation, elicitationDefaults, elicitationValid } from '../../components/development/NativeElicitation';
import CodexOperations from '../../components/development/CodexOperations';
import { AgentImages } from '../../components/development/AgentImages';
import { acknowledgeAgentImages, agentImageInputs, agentImagePaste, agentImagesKey, captureAgentImages, useAgentImages } from '../../stores/agentImages';

type PendingConversationSend = { requestId: string; generation: string; text: string; references: CapturedConversationReference[]; skills: CapturedAgentSkill[]; imageOwner?: string; images?: { id: string }[] };
function object(value: Json | undefined): { [key: string]: Json } { return value && typeof value === 'object' && !Array.isArray(value) ? value : {}; }
function text(value: Json | undefined) { return typeof value === 'string' ? value : ''; }
function blocks(event: Event): Json[] { const content = object(event.data).content; return Array.isArray(content) ? content : []; }
function prose(event: Event): string {
  const data = object(event.data);
  return event.text || (typeof data.content === 'string' ? data.content : blocks(event).filter(block => object(block).type === 'text').map(block => text(object(block).text)).join('\n\n'));
}
function isUser(event: Event) { return ['userMessage', 'userSteering'].includes(event.kind) || object(event.data).type === 'user' && (Boolean(prose(event)) || !blocks(event).some(block => object(block).type === 'tool_result')); }
const isFailed = agentEventFailed;
function itemKey(event: Event, id: string) { return `${event.run ?? ''}:${id}`; }

type ActivityState = 'processing' | 'waiting' | 'stopped' | 'failed' | 'unknown';
function activityStates(events: Event[], session?: AgentSession) {
  const states = new Map<string, ActivityState>();
  for (const event of events) {
    const data = object(event.data);
    if (!states.has(event.run ?? '')) states.set(event.run ?? '', 'unknown');
    if (event.kind === 'result' || event.kind === 'turn/completed' || ['error', 'failure'].includes(event.kind)) states.set(event.run ?? '', data.run_status === 'cancelled' ? 'stopped' : 'unknown');
  }
  if (session?.active_run) states.set(session.active_run, session.status === 'waiting' ? 'waiting' : session.status === 'running' ? 'processing' : 'unknown');
  return states;
}
function operationStatus(event: Event, runState?: ActivityState) {
  const data = object(event.data);
  if (data.run_status === 'cancelled' || ['cancelled', 'interrupted', 'stopped'].includes(text(data.status))) return 'stopped';
  if (event.kind === 'result' || event.kind === 'turn/completed') return data.run_status === 'completed' ? 'replyCompleted' : data.run_status === 'failed' ? 'failed' : 'unknown';
  if (isFailed(event)) return 'failed';
  if (data.status === 'completed' || data.success === true || typeof data.exitCode === 'number' && data.exitCode === 0) return 'toolCompleted';
  // A start/delta is not completion evidence. A finished run without a matching
  // operation result stays uncertain rather than inheriting the turn's success.
  if (runState) return runState;
  return event.kind === 'item/started' || event.kind === 'tool_progress' || event.kind.endsWith('/outputDelta') || data.status === 'inProgress' ? 'processing' : 'unknown';
}
function OperationRow({ label, status, hint, children }: { label: string; status: string; hint?: string; children: ReactNode }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  return <details className="agent-dialogue-tool" data-operation-state={status} onToggle={event => setOpen(event.currentTarget.open)}><summary><strong>{label}</strong><span>{t(`agentExecution.${status}`)}</span>{hint ? <code>{hint}</code> : null}</summary>{open ? <div className="agent-dialogue-tool-body">{children}</div> : null}</details>;
}

type TranscriptRow = { event: Event; hidden?: boolean; suppressText?: boolean; codex?: CodexTranscriptEntry };
// Projection only: every native event remains available in the inspector. Coalesce deltas by
// native item/run; only a matching full snapshot supersedes its stream, never a later turn.
function transcript(events: Event[]) {
  const rows: TranscriptRow[] = [];
  const codexEvents = projectCodexTranscript(events);
  const streams = new Map<string, TranscriptRow>();
  const outputs = new Map<string, TranscriptRow>();
  const completed = new Set(events.filter(event => ['commandExecution', 'fileChange', 'mcpToolCall', 'dynamicToolCall', 'webSearch'].includes(event.kind) && text(object(event.data).id) && object(event.data).status !== 'inProgress').map(event => itemKey(event, text(object(event.data).id))));
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
    if (codexEvents.has(event)) {
      const codex = codexEvents.get(event);
      if (codex) rows.push({ event, codex });
      continue;
    }
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

// Drafts belong to the existing account-scoped draft store; this is only its visible projection.
export function AgentConversation({ session, drafts, setDraft, busy, error, visible = true, onOpenReference, onSelectRun, onNewConversation, onContinueModel, onOpenSession }: AgentReferenceActions & { session: AgentSession; drafts: { [id: string]: string }; setDraft: (id: string, value: string, expected?: string) => void; busy: boolean; error: boolean; visible?: boolean; onNewConversation?: (configuration?: CliProviderConfiguration) => Promise<void>; onContinueModel?: () => void; onOpenSession?: (sessionId: string) => void }) {
  const { t } = useLocale();
  const action = useAgentRunner(state => state.action);
  const acting = useAgentRunner(state => state.acting);
  const epoch = useAgentRunner(state => state.epoch);
  const interrupt = useAgentRunner(state => state.interrupts[session.id]);
  const interruptPending = interrupt?.generation === session.generation && interrupt.runId === session.active_run;
  const devEpoch = useDevelopment(state => state.epoch);
  const savedSession = useDevelopment(state => state.sessions.find(item => item.id === session.id));
  const projectId = useDevelopment(state => savedSession ? savedSession.project_id : state.selection.project_id);
  const taskId = useDevelopment(state => savedSession ? savedSession.task_id : state.selection.task_id);
  const { generation } = useContextSelection();
  const prompt = drafts[session.id] ?? '';
  const imageOwner = agentImagesKey(projectId, session);
  const imageDraft = useAgentImages(state => state.drafts[imageOwner]);
  const hasImages = session.engine === 'codex' && Boolean(imageDraft?.images.length);
  const pendingSendKey = developmentDraftKey('conversation', projectId, session.id, session.generation, 'pending-send');
  const pendingSend = useDevelopmentDrafts(state => state.entries[pendingSendKey]?.value as PendingConversationSend | undefined);
  const pendingSteerKey = developmentDraftKey('conversation', projectId, session.id, session.generation, 'pending-steer');
  const pendingSteer = useDevelopmentDrafts(state => Boolean(state.entries[pendingSteerKey]));
  const binding = session.configuration?.format === 'provider_bound' ? session.configuration.snapshot : null;
  const boundConfiguration: CliProviderConfiguration | null = binding ? { engine: session.engine, provider: { provider_id: binding.provider_id, model_id: binding.model_id, provider_revision: binding.provider_revision } } : null;
  const emptySession = !session.events.length && !session.active_run && session.status === 'idle';
  const [sending, setSending] = useState(false);
  const sendingRef = useRef(false);
  const viewport = useRef<HTMLDivElement>(null);
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const requestFocus = useRef<HTMLElement | null>(null);
  const surfaceActive = useSurfaceActive();
  const visibleOwner = useRef(visible && surfaceActive);
  visibleOwner.current = visible && surfaceActive;
  useEffect(() => {
    const previous = requestFocus.current;
    if (!previous || previous.isConnected || !visibleOwner.current || !root.current?.getClientRects().length || document.activeElement !== document.body) return;
    const target = root.current.querySelector<HTMLElement>('.agent-dialogue-requests button:not(:disabled)') ?? root.current.querySelector<HTMLElement>('#agent-message');
    requestFocus.current = null;
    target?.focus({ preventScroll: true });
  });
  const follow = useRef(true);
  const [atBottom, setAtBottom] = useState(true);
  const [inspect, setInspect] = useState(false);
  const { rows, toolResults, toolNames } = useMemo(() => transcript(session.events), [session.events]);
  const runStates = useMemo(() => activityStates(session.events, session), [session.events, session.active_run, session.status]);
  const referenceEvent = useMemo(() => {
    for (let index = session.events.length - 1; index >= 0; index -= 1) {
      const event = session.events[index];
      if (agentOutputReferences(event, toolNames).references.length) return event;
    }
    return undefined;
  }, [session.events, toolNames]);
  function latest() { if (!visibleOwner.current) return; const node = viewport.current; if (node) node.scrollTop = node.scrollHeight; follow.current = true; setAtBottom(true); }
  useLayoutEffect(() => {
    if (!visible || !surfaceActive) return;
    const position = useDevelopment.getState().reading[session.id];
    follow.current = position?.follow ?? true; setAtBottom(follow.current);
    const node = viewport.current;
    if (node) node.scrollTop = follow.current ? node.scrollHeight : position?.top ?? 0;
  }, [session.id, epoch, visible, surfaceActive]);
  useEffect(() => { setInspect(false); }, [session.id, epoch]);
  useEffect(() => { if (follow.current) latest(); }, [session.events[session.events.length - 1]?.seq, visible, surfaceActive]);
  useLayoutEffect(() => {
    const node = composerInput.current;
    if (!node) return;
    node.rows = 2;
    const metrics = getComputedStyle(node);
    const lineHeight = Number.parseFloat(metrics.lineHeight) || 20;
    const padding = (Number.parseFloat(metrics.paddingTop) || 0) + (Number.parseFloat(metrics.paddingBottom) || 0);
    node.rows = Math.max(2, Math.min(10, Math.ceil((node.scrollHeight - padding) / lineHeight)));
  }, [prompt, visible, surfaceActive]);
  const navigationOwner = useRef({ sessionId: session.id, epoch, devEpoch, generation, mounted: true });
  navigationOwner.current = { sessionId: session.id, epoch, devEpoch, generation, mounted: true };
  useEffect(() => { navigationOwner.current.mounted = true; return () => { navigationOwner.current.mounted = false; }; }, []);
  const currentOwner = () => visibleOwner.current && navigationOwner.current.mounted && navigationOwner.current.sessionId === session.id && navigationOwner.current.epoch === epoch && navigationOwner.current.devEpoch === devEpoch && navigationOwner.current.generation === generation && epoch === useAgentRunner.getState().epoch && devEpoch === useDevelopment.getState().epoch && generation === contextGeneration() && useAgentRunner.getState().selected === session.id && useAgentRunner.getState().session?.generation === session.generation;
  const inputReferences = useComposerReferences({ projectId, taskId, ownerId: session.id, disabled: !visible || busy || sending || acting || session.status !== 'idle' || Boolean(session.pending.length), isCurrent: currentOwner });
  const reason = session.persistence_error ? t('agent.storageFailed') : busy || sending || acting || imageDraft?.busy ? t('agent.sendBusy') : pendingSend ? t('directConversation.send_unknown') : pendingSteer ? t('codexInteractions.unknown') : session.pending.length || session.status === 'waiting' ? t('agent.sendWaiting') : session.status === 'running' || session.active_run ? t(session.engine === 'codex' && session.controls?.steering ? 'codexInteractions.runningHint' : 'agent.sendRunning') : session.status !== 'idle' ? t('agent.sendUnavailable') : !prompt.trim() && !inputReferences.references.length && !hasImages ? t('agent.sendEmpty') : '';
  const referenceActions: AgentReferenceActions = {
    onOpenReference: onOpenReference ? (reference, mode) => currentOwner() ? onOpenReference(reference, mode) : undefined : undefined,
    onSelectRun: onSelectRun ? runId => { if (currentOwner() && session.events.some(event => event.run === runId)) onSelectRun(runId); } : undefined,
  };
  function clearPendingSend(requestId: string) {
    if (epoch !== useAgentRunner.getState().epoch || devEpoch !== useDevelopment.getState().epoch) return;
    useDevelopmentDrafts.setState(state => {
      if ((state.entries[pendingSendKey]?.value as PendingConversationSend | undefined)?.requestId !== requestId) return state;
      const entries = { ...state.entries }; delete entries[pendingSendKey]; return { entries };
    });
  }
  function acknowledgeSend(capture: PendingConversationSend) {
    if (epoch !== useAgentRunner.getState().epoch || devEpoch !== useDevelopment.getState().epoch) return;
    setDraft(session.id, '', capture.text);
    if (projectId) clearConversationReferences(projectId, session.id, capture.references);
    acknowledgeAgentSkills(projectId, { id: session.id, generation: capture.generation }, capture.skills ?? []);
    if (capture.imageOwner) acknowledgeAgentImages(capture.imageOwner, capture.images ?? []);
    clearPendingSend(capture.requestId);
  }
  async function recheckSend() {
    if (!pendingSend || sendingRef.current || busy || acting || !currentOwner()) return;
    sendingRef.current = true; setSending(true);
    try {
      await useAgentRunner.getState().attach(session.id);
      if (!currentOwner()) return;
      const runner = useAgentRunner.getState();
      const live = runner.session;
      if (!runner.error && live?.generation === pendingSend.generation && (live.active_run === pendingSend.requestId || live.events.some(event => event.run === pendingSend.requestId))) acknowledgeSend(pendingSend);
    } finally { sendingRef.current = false; if (navigationOwner.current.mounted) setSending(false); }
  }
  function selectCommand(command: string) {
    const live = useAgentRunner.getState().session;
    if (!currentOwner() || busy || sendingRef.current || acting || pendingSend || live?.status !== 'idle' || live.active_run || live.pending.length) return;
    setDraft(session.id, `${command} ${prompt}`, prompt);
    composerInput.current?.focus({ preventScroll: true });
  }
  async function send() {
    if (sendingRef.current || reason || !currentOwner()) return;
    const id = session.id;
    const sentPrompt = prompt;
    const captured = projectId ? captureConversationReferences(projectId, id) : [];
    const skills = captureAgentSkills(projectId, session);
    const images = session.engine === 'codex' ? captureAgentImages(imageOwner) : [];
    if (!sentPrompt.trim() && !captured.length && !images.length) return;
    const proposedSettings = agentTurnSettings(session);
    const settings = proposedSettings ? { ...proposedSettings } : undefined;
    const capture: PendingConversationSend = { requestId: crypto.randomUUID(), generation: session.generation, text: sentPrompt, references: captured, skills, imageOwner, images: images.map(image => ({ id: image.id })) };
    sendingRef.current = true; setSending(true);
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [pendingSendKey]: { kind: 'conversation', value: capture, baseline: null } } }));
    try {
      let failure: string | undefined;
      const sent = await action('send_agent_message', { input: { session_id: id, generation: capture.generation, request_id: capture.requestId, text: sentPrompt, ...(settings ? { settings } : {}), ...(captured.length ? { references: captured.map(item => item.reference) } : {}), ...(skills.length ? { skills: skills.map(item => item.skill) } : {}), ...(images.length ? { images: agentImageInputs(images) } : {}) } }, code => { failure = code; });
      if (sent) acknowledgeSend(capture);
      else if (failure && failure !== 'unknown') clearPendingSend(capture.requestId);
    } finally { sendingRef.current = false; if (navigationOwner.current.mounted) setSending(false); }
  }
  return <div ref={root} className="agent-conversation-product" onFocusCapture={event => { requestFocus.current = (event.target as HTMLElement).closest('.agent-dialogue-requests') ? event.target as HTMLElement : null; }}>
    <header className="agent-dialogue-header">
      <div className="agent-dialogue-identity"><strong>{t(`agent.engine.${session.engine}`)}</strong><span className="agent-dialogue-status" data-state={session.status}>{t(`agent.state.${session.status}`)}</span></div>
      <div className="agent-dialogue-actions"><Button size="xs" variant="secondary" disabled={busy || interruptPending || !session.active_run || !['running', 'waiting'].includes(session.status)} onClick={() => { const current = useAgentRunner.getState().session; if (currentOwner() && current && current.active_run === session.active_run && ['running', 'waiting'].includes(current.status)) void action('interrupt_agent_session', { sessionId: session.id, generation: session.generation, runId: session.active_run }); }}>{t('agent.interrupt')}</Button><Button size="xs" variant="secondary" disabled={busy || session.status === 'closed'} onClick={() => { if (currentOwner()) void action('close_agent_session', { sessionId: session.id }); }}>{t('agent.close')}</Button></div>
    </header>
    <div className="agent-dialogue-transcript" ref={viewport} role="region" tabIndex={0} aria-label={t('agent.messages')} onScroll={event => { if (!visibleOwner.current) return; const node = event.currentTarget; const bottom = node.scrollHeight - node.scrollTop - node.clientHeight < 64; follow.current = bottom; setAtBottom(bottom); if (devEpoch === useDevelopment.getState().epoch) useDevelopment.getState().setReading(session.id, { top: node.scrollTop, follow: bottom }); }}>
      <details className="agent-dialogue-capabilities"><summary>{t('capabilities.actual')}</summary><SessionCapabilitySnapshot key={session.id} sessionId={session.id} snapshot={session.configuration} runtime={session} /></details>
      {session.engine === 'codex' ? <CodexOperations session={session} disabled={busy || !visible || !surfaceActive} onOpenSession={onOpenSession} onSessionChanged={() => { if (currentOwner()) void useAgentRunner.getState().attach(session.id); }} /> : null}
      <div className="agent-dialogue-messages">
        {session.history_truncated ? <p className="text-sm text-ink-secondary">{t('agent.truncated')}</p> : null}
        {rows.map(({ event, suppressText, codex }) => <EventContent key={`${session.id}:${event.seq}`} event={event} suppressText={suppressText} codex={codex} toolResults={toolResults} toolNames={toolNames} runState={runStates.get(event.run ?? '')} {...referenceActions} />)}
        {!session.events.length ? <p className="agent-dialogue-empty">{t('agent.empty')}</p> : null}
      </div>
      {session.events.length ? <details className="agent-dialogue-inspector" open={inspect} onToggle={event => setInspect(event.currentTarget.open)}><summary>{t('agent.rawEvents', { count: session.events.length })}</summary>{inspect ? session.events.map(event => <NativeEvent key={event.seq} event={event} />) : null}</details> : null}
    </div>
    <footer className="agent-dialogue-footer">
      {!atBottom ? <Button size="xs" variant="secondary" className="agent-dialogue-latest" onClick={latest}>{t('agent.latest')}</Button> : null}
      {interruptPending ? <p role="status">{t('agent.interruptPending')}</p> : null}
      {session.persistence_error ? <p role="alert" className="text-status-error">{t('agent.storageFailed')}</p> : null}
      {session.engine === 'codex' ? <CodexTurnControls key={`${epoch}:${session.id}:${session.generation}`} session={session} projectId={projectId} prompt={prompt} setDraft={setDraft} currentOwner={currentOwner} disabled={busy || !visible || !surfaceActive} /> : null}
      <div className="agent-dialogue-requests">
        {['idle', 'running', 'waiting'].includes(session.status) ? session.pending.map(pending => <Request key={`${epoch}:${generation}:${session.id}:${session.generation}:${pending.id}:${JSON.stringify(pending.data)}`} engine={session.engine} pending={pending} sessionId={session.id} generation={session.generation} submit={response => currentOwner() && ['idle', 'running', 'waiting'].includes(useAgentRunner.getState().session?.status ?? '') && useAgentRunner.getState().session?.pending.some(item => item.id === pending.id && item.kind === pending.kind && JSON.stringify(item.data) === JSON.stringify(pending.data)) ? action('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: pending.id, response } }) : Promise.resolve(false)} />) : null}
      </div>
      {session.error && !error ? <div role="alert" className="agent-dialogue-error text-status-error"><p>{t('agent.failure')}</p><details className="agent-dialogue-native"><summary>{t('agent.details')}</summary><pre>{session.error}</pre></details></div> : null}
      {pendingSend && !sending && !acting ? <div><p role="alert">{t('directConversation.send_unknown')}</p><Button size="xs" disabled={busy} onClick={() => void recheckSend()}>{t('directConversation.recheck')}</Button></div> : null}
      <div className="agent-dialogue-composer"><div className="agent-composer-shell">{inputReferences.chips}{session.engine === 'codex' ? <AgentImages owner={imageOwner} active={visible && surfaceActive} disabled={busy || sending || acting || Boolean(pendingSend) || !visible || !surfaceActive || session.status !== 'idle'} /> : null}<div className="agent-composer-input"><Input multiline rows={2} id="agent-message" ref={composerInput} label={t('agent.prompt')} className="type-reading" aria-describedby="agent-send-hint" value={prompt} onPaste={session.engine === 'codex' ? agentImagePaste(imageOwner, busy || sending || acting || Boolean(pendingSend) || !visible || !surfaceActive || session.status !== 'idle') : undefined} onChange={event => setDraft(session.id, inputReferences.promptChange(event.target.value, prompt, event.target.selectionStart, (event.nativeEvent as InputEvent).isComposing))} onKeyDown={event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) { event.preventDefault(); if (!event.repeat) void send(); } }} /></div>
        <AgentComposerControls session={session} projectId={projectId} visible={visible} disabled={busy || sending || acting || Boolean(pendingSend)} onCommandSelect={selectCommand} referencePicker={inputReferences.picker} references={referenceEvent ? <AgentOutputReferences key={`${session.id}:${referenceEvent.seq}`} event={referenceEvent} toolNames={toolNames} {...referenceActions} /> : undefined} model={visible && surfaceActive && projectId && onNewConversation && emptySession ? <ComposerModelPicker projectId={projectId} value={boundConfiguration} preferred={boundConfiguration} disabled={busy || sending || acting} onChange={configuration => { if (currentOwner()) void onNewConversation(configuration); }} /> : <div className="dev-review-toolbar"><span className="text-xs">{binding ? `${binding.model_id} · ${t(`agent.engine.${session.engine}`)}` : t(`agent.engine.${session.engine}`)}</span>{onContinueModel ? <Button size="xs" variant="ghost" onClick={() => { if (currentOwner()) onContinueModel(); }}>{t('workbenchNavigation.continueModel')}</Button> : null}</div>} sendHint={reason || t('agent.sendShortcut')} send={<Button disabled={Boolean(reason)} onClick={() => void send()}>{t('agent.send')}</Button>} />
      </div></div>
    </footer>
  </div>;
}

// Persisted run events are read-only evidence, not a fabricated live session or a second runner.
export function AgentSessionHistory({ sessionId, taskId, children, visible = true }: { sessionId: string; taskId: string; children: ReactNode; visible?: boolean }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  const [runs, setRuns] = useState<DevRun[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const surfaceActive = useSurfaceActive();
  const visibleOwner = useRef(visible && surfaceActive);
  visibleOwner.current = visible && surfaceActive;
  useLayoutEffect(() => {
    if (!visible || !surfaceActive || !runs || !viewport.current) return;
    const position = useDevelopment.getState().reading[sessionId];
    viewport.current.scrollTop = position?.follow === false ? position.top : viewport.current.scrollHeight;
  }, [sessionId, runs, visible, surfaceActive]);
  useEffect(() => {
    let active = true;
    setRuns(null); setError(null);
    void listDevelopmentRuns(taskId).then(result => {
      if (!active || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      if (result.ok) setRuns(result.value.filter(run => run.session_id === sessionId).sort((a, b) => a.started_at - b.started_at));
      else setError(result.code);
    });
    return () => { active = false; };
  }, [sessionId, taskId, epoch, generation]);
  return <div className="agent-conversation-product" data-testid="dev-session-history">
    <div className="agent-dialogue-transcript" ref={viewport} role="region" aria-label={t('agent.messages')} tabIndex={0} onScroll={event => { if (!visibleOwner.current) return; const node = event.currentTarget; if (epoch === useDevelopment.getState().epoch) useDevelopment.getState().setReading(sessionId, { top: node.scrollTop, follow: node.scrollHeight - node.scrollTop - node.clientHeight < 64 }); }}>
      <details className="agent-dialogue-capabilities"><summary>{t('capabilities.actual')}</summary><SessionCapabilitySnapshot sessionId={sessionId} snapshot={undefined} /></details>
      {error ? <p role="alert">{errorText(error)}</p> : runs === null ? <p role="status">{t('review.loading')}</p> : !runs.length ? <p>{t('agent.empty')}</p> : runs.map(run => {
        const { rows, toolResults, toolNames } = transcript(run.events);
        const runStates = activityStates(run.events);
        return <div className="agent-dialogue-messages" key={run.id}>{run.truncated ? <p>{t('agent.truncated')}</p> : null}{rows.map(({ event, suppressText, codex }) => <EventContent key={event.seq} event={event} suppressText={suppressText} codex={codex} toolResults={toolResults} toolNames={toolNames} runState={runStates.get(event.run ?? '')} />)}</div>;
      })}
    </div>
    <footer className="agent-dialogue-footer">{children}</footer>
  </div>;
}

function NativeEvent({ event }: { event: Event }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  return <details className="agent-dialogue-native" onToggle={event => setOpen(event.currentTarget.open)}><summary>{t('agent.details')} · {event.kind} · #{event.seq}</summary>{open ? <pre className="type-code">{JSON.stringify(inspectableAgentEvent(event), null, 2)}</pre> : null}</details>;
}

function ToolBlock({ event, block, toolResults, toolNames, runState }: { event: Event; block: Json; toolResults: ReadonlyMap<string, Json>; toolNames: ReadonlyMap<string, string>; runState?: ActivityState }) {
  const { t } = useLocale();
  const value = object(block);
  const use = value.type === 'tool_use';
  const result = use ? toolResults.get(itemKey(event, text(value.id))) : block;
  const failed = object(result).is_error === true;
  const name = use ? text(value.name) : toolNames.get(itemKey(event, text(value.tool_use_id))) || text(value.tool_use_id);
  const input = object(value.input);
  const command = text(input.command) || text(input.file_path) || text(input.notebook_path) || text(input.pattern);
  const label = name === 'Read' ? 'agentExecution.read' : ['Write', 'Edit', 'MultiEdit', 'NotebookEdit'].includes(name) ? 'agentExecution.write' : name === 'Bash' ? 'agentExecution.command' : '';
  return <OperationRow label={label ? `${t(label)} · ${name}` : name || t('agent.activity')} status={result ? failed ? 'failed' : 'toolCompleted' : runState ?? 'unknown'} hint={command}>
    {use ? <div><strong>{t('agentExecution.input')}</strong><NativeValue value={value.input} /></div> : null}
    {result ? <div><strong>{t('agentExecution.output')}</strong><NativeValue value={object(result).content} /></div> : null}
    <small>{t('agentExecution.source', { seq: event.seq })}</small>
  </OperationRow>;
}

export function EventContent({ event, suppressText = false, codex, toolResults = new Map<string, Json>(), toolNames = new Map<string, string>(), runState, onOpenReference, onSelectRun }: AgentReferenceActions & { event: Event; suppressText?: boolean; codex?: CodexTranscriptEntry; toolResults?: ReadonlyMap<string, Json>; toolNames?: ReadonlyMap<string, string>; runState?: ActivityState }) {
  const { t } = useLocale();
  const data = object(event.data);
  const content = blocks(event);
  const terminal = event.kind === 'result' || event.kind === 'turn/completed';
  const failed = terminal ? data.run_status === 'failed' : isFailed(event) || content.some(block => object(block).type === 'tool_use' && object(toolResults.get(itemKey(event, text(object(block).id)))).is_error === true);
  const user = isUser(event);
  const assistant = event.kind === 'agentMessage' || data.type === 'assistant';
  const dialogue = user || assistant || terminal;
  const source = !suppressText && dialogue ? prose(event) : '';
  const imageCount = user ? Math.max(Array.isArray(data.images) ? data.images.length : 0, content.filter(block => object(block).type === 'image').length) : 0;
  const tools = content.filter(block => { const value = object(block); return value.type === 'tool_use' || value.type === 'tool_result' && !toolNames.has(itemKey(event, text(value.tool_use_id))); });
  const command = text(data.command) || text(data.query) || text(object(data.action).query) || (Array.isArray(data.changes) ? data.changes.map(change => text(object(change).path)).filter(Boolean).join(', ') : '');
  const type = text(data.type) || event.kind;
  const name = text(data.tool) || text(data.tool_name) || (type === 'commandExecution' ? t('direct.command') : type === 'fileChange' ? t('execution.artifacts') : type === 'webSearch' ? t('skills.search') : ['mcpToolCall', 'dynamicToolCall'].includes(type) ? t('capabilities.tool') : event.kind.endsWith('/outputDelta') ? t('direct.output') : t('agent.activity'));
  const execution = ['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch', 'fileChange', 'tool_progress', 'turn/plan/updated', 'turn/diff/updated'].includes(event.kind) || event.kind === 'item/started' && ['commandExecution', 'mcpToolCall', 'dynamicToolCall', 'webSearch', 'fileChange'].includes(type) || event.kind.endsWith('/outputDelta');
  const status = operationStatus(event, runState);
  const references = agentOutputReferences(event, toolNames);
  const hasReferences = references.references.length > 0 || references.unresolved || Boolean(failed && event.run);
  const diagnosticReferences = references.references.some(reference => reference.source === 'diagnostic') || failed;
  const nativeProjection = codex ?? projectCodexTranscript([event]).get(event);
  if (nativeProjection) return <article data-testid={`agent-event-${event.seq}`} data-event-seq={event.seq} data-event-run={event.run ?? ''} data-event-kind={event.kind} className="agent-dialogue-event"><CodexTranscriptContent event={event} entry={nativeProjection} inspector={value => <NativeEvent event={value} />} /></article>;
  if (isReasoningEvent(event)) return null;
  // Summary-only Codex projections above never expose raw reasoning. Unknown native
  // payloads remain available in the inspector, with reasoning content redacted.
  if (!dialogue && !execution && !failed && !tools.length && !content.some(block => object(block).type === 'tool_result')) return null;
  return <article data-testid={`agent-event-${event.seq}`} data-event-seq={event.seq} data-event-run={event.run ?? ''} data-event-kind={event.kind} className={`agent-dialogue-event ${user ? 'agent-dialogue-user' : ''} ${failed ? 'text-status-error-text' : ''}`}>
    {source || imageCount ? <div className="agent-dialogue-prose"><strong className="agent-dialogue-speaker">{t(user ? 'agent.you' : 'agent.assistant')}</strong>{source ? <Markdown source={source} /> : null}{imageCount ? <div data-testid="agent-image-history"><p>{t('agentImages.history', { count: imageCount })}</p><p className="text-xs text-ink-secondary">{t('agentImages.historyPrivacy')}</p></div> : null}</div> : null}
    {tools.map((block, index) => <ToolBlock key={index} event={event} block={block} toolResults={toolResults} toolNames={toolNames} runState={runState} />)}
    {execution || terminal || failed && !tools.length ? <OperationRow label={terminal ? t('agent.finished') : event.kind === 'turn/plan/updated' ? t('execution.plan') : event.kind === 'turn/diff/updated' ? t('execution.diff') : name} status={status} hint={command}>
      {terminal ? <p className="agent-dialogue-outcome-note">{t('agentExecution.notAcceptance')}</p> : null}
      <ExecutionDetails event={event} />
      {event.kind.endsWith('/outputDelta') && event.text || failed && !dialogue && event.text ? <pre className="type-code">{event.text}</pre> : null}
      {failed && data.error ? <NativeValue value={data.error} /> : null}
      <NativeEvent event={event} />
    </OperationRow> : null}
    {hasReferences ? diagnosticReferences ? <details className="agent-dialogue-native"><summary>{t('agent.references')}</summary><AgentOutputReferences event={event} toolNames={toolNames} onOpenReference={onOpenReference} onSelectRun={onSelectRun} /></details> : <AgentOutputReferences event={event} toolNames={toolNames} onOpenReference={onOpenReference} onSelectRun={onSelectRun} /> : null}
  </article>;
}

export function Request(props: { engine: Engine; pending: Pending; sessionId?: string; generation?: string; submit: (response: Json) => Promise<boolean> }) {
  return <RequestBody key={`${props.sessionId ?? ''}:${props.generation ?? ''}:${props.pending.id}`} {...props} />;
}

function RequestBody({ engine, pending, sessionId, generation, submit }: { engine: Engine; pending: Pending; sessionId?: string; generation?: string; submit: (response: Json) => Promise<boolean> }) {
  const { t } = useLocale();
  const [answers, setAnswers] = useState<{ [key: string]: string[] }>({});
  const [busy, setBusy] = useState(false);
  const responding = useRef(false);
  const [failed, setFailed] = useState(false);
  const [confirmedRules, setConfirmedRules] = useState<Record<number, boolean>>({});
  const data = object(pending.data);
  const input = engine === 'claude' ? object(data.input ?? null) : data;
  const questions = (engine === 'claude' ? pending.title === 'AskUserQuestion' : pending.kind === 'item/tool/requestUserInput') && Array.isArray(input.questions) ? input.questions : [];
  const schema = object(data.requestedSchema ?? null);
  const elicitation = pending.kind === 'mcpServer/elicitation/request';
  const formMode = elicitation && ['form', 'openai/form', 'openaiForm'].includes(text(data.mode));
  const [form, setForm] = useState<{ [key: string]: Json }>(() => elicitationDefaults(schema));
  const canAccept = !elicitation || data.mode === 'url' || formMode && elicitationValid(schema, form);
  const respond = async (value: Json) => { if (responding.current) return; responding.current = true; setBusy(true); setFailed(false); try { if (await submit(value)) return; } catch { /* Keep the same request and answer available for an explicit retry. */ } responding.current = false; setFailed(true); setBusy(false); };
  const accept = (): Json => {
    if (engine === 'claude') return { behavior: 'allow', ...(questions.length ? { answers: Object.fromEntries(Object.entries(answers).map(([key, value]) => [key, value.join(', ')])) } : {}) };
    if (pending.kind === 'item/tool/requestUserInput') return { answers: Object.fromEntries(Object.entries(answers).map(([key, value]) => [key, { answers: value }])) };
    if (pending.kind === 'mcpServer/elicitation/request') return { action: 'accept', content: data.mode === 'url' ? null : form };
    return { decision: 'accept' };
  };
  const approval = engine === 'codex' && ['item/commandExecution/requestApproval', 'item/fileChange/requestApproval'].includes(pending.kind);
  const permissions = engine === 'codex' && pending.kind === 'item/permissions/requestApproval';
  const decisions: Json[] = Array.isArray(data.availableDecisions) ? data.availableDecisions.filter(value => typeof value === 'string'
    ? ['accept', 'acceptForSession', 'decline', 'cancel'].includes(value)
    : pending.kind === 'item/commandExecution/requestApproval' && value !== null && typeof value === 'object' && !Array.isArray(value)
      && Object.keys(value).length === 1 && ('acceptWithExecpolicyAmendment' in value || 'applyNetworkPolicyAmendment' in value)) : ['accept', 'decline', 'cancel'];
  const supported = engine === 'claude' && pending.kind === 'claude/toolPermission' || ['item/tool/requestUserInput', 'mcpServer/elicitation/request'].includes(pending.kind);
  return <section data-testid="agent-native-request" data-request-id={pending.id} data-request-kind={pending.kind} className="grid gap-3 rounded-md border border-border-subtle p-3" aria-label={t('agent.approval')} aria-busy={busy}><strong>{engine === 'claude' ? pending.title : t('agent.approval')}</strong>
    <details className="agent-dialogue-native"><summary>{t('agent.permissionIdentity')}</summary>
    <dl className="agent-dialogue-request-identity">
      <div><dt>{t('agentExecution.operation')}</dt><dd>{pending.title} · {pending.kind}</dd></div>
      <div><dt>{t('agentExecution.requestId')}</dt><dd>{pending.id}</dd></div>
      {sessionId ? <div><dt>{t('agentExecution.sessionId')}</dt><dd>{sessionId}</dd></div> : null}
      {generation ? <div><dt>{t('agentExecution.generation')}</dt><dd>{generation}</dd></div> : null}
      {text(data.threadId) || text(data.turnId) || text(data.itemId) ? <div><dt>{t('agent.permissionIdentity')}</dt><dd>{text(data.threadId) || '—'} / {text(data.turnId) || '—'} / {text(data.itemId) || '—'}</dd></div> : null}
    </dl>
    </details>
    {!questions.length ? <p className="agent-dialogue-request-scope">{t(permissions ? 'agent.permissionScope' : 'agentExecution.onceScope')}</p> : null}
    {text(input.command) ? <div><strong>{t('agent.requestCommand')}</strong><pre className="overflow-x-auto whitespace-pre-wrap text-sm">{text(input.command)}</pre></div> : null}
    {text(input.cwd) ? <p className="m-0 break-all text-sm">{t('agent.requestDirectory')}: {text(input.cwd)}</p> : null}
    {text(input.file_path) || text(input.path) || text(input.grantRoot) ? <p className="m-0 break-all text-sm">{t('agent.requestPath')}: {text(input.file_path) || text(input.path) || text(input.grantRoot)}</p> : null}
    {engine === 'claude' && !questions.length ? <div><strong>{t('agent.requestInput')}</strong><NativeValue value={data.input} /></div> : null}
    {pending.kind === 'item/fileChange/requestApproval' && Array.isArray(data.changes) ? <NativeValue value={data.changes} /> : null}
    {permissions ? <div className="grid gap-2"><strong>{t('agent.permissionExpansion')}</strong><NativeValue value={data.permissions} /></div> : null}
    {text(data.message) || text(data.reason) ? <p className="m-0 text-sm">{text(data.message) || text(data.reason)}</p> : null}
    {text(data.url) ? <p className="break-all text-sm">{text(data.url)}</p> : null}
    <details><summary className="cursor-pointer text-xs">{t('agent.details')}</summary><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs">{JSON.stringify(pending.data, null, 2)}</pre></details>
    {questions.map((value, i) => {
      const question = object(value);
      const key = engine === 'claude' ? text(question.question) || String(i) : text(question.id) || String(i);
      const choices = Array.isArray(question.options) ? question.options : [];
      const selected = answers[key] ?? [];
      return <fieldset key={key} className="min-w-0 space-y-2" disabled={busy}><legend className="text-sm">{text(question.header) || text(question.question) || key}</legend>
        <p className="m-0 text-sm">{text(question.question) || key}</p>
        {engine === 'claude' || !choices.length || question.isOther === true || question.isSecret === true ? <Input type={question.isSecret ? 'password' : 'text'} label={t('agent.customAnswer')} autoComplete="off" value={selected.join(', ')} onChange={e => setAnswers(current => ({ ...current, [key]: e.target.value ? [e.target.value] : [] }))} /> : null}
        <div className="flex flex-wrap gap-2">{choices.map((choice, index) => { const option = object(choice); const label = text(option.label); return <Button key={index} variant="secondary" title={text(option.description)} aria-pressed={selected.includes(label)} onClick={() => setAnswers(current => ({ ...current, [key]: engine === 'claude' && question.multiSelect ? selected.includes(label) ? selected.filter(item => item !== label) : [...selected, label] : [label] }))}>{label}{text(option.description) ? <span className="text-xs"> — {text(option.description)}</span> : null}</Button>; })}</div>
      </fieldset>;
    })}
    {formMode ? <NativeElicitation schema={schema} values={form} disabled={busy} onChange={setForm} /> : elicitation && data.mode !== 'url' ? <p role="alert">{t('agentExecution.unsupported')}</p> : null}
    {failed ? <p role="alert" className="text-sm text-status-error">{t('agent.failure')}</p> : null}
    {busy ? <p role="status" className="m-0 text-sm">{t('agent.requestSending')}</p> : null}
    {permissions ? <div className="flex flex-wrap gap-2"><Button data-decision="accept-turn" disabled={busy} onClick={() => void respond({ permissions: data.permissions, scope: 'turn' })}>{t('agent.allowTurn')}</Button><Button data-decision="accept-session" variant="secondary" disabled={busy} onClick={() => void respond({ permissions: data.permissions, scope: 'session' })}>{t('agent.allowSession')}</Button><Button data-decision="decline" variant="secondary" disabled={busy} onClick={() => void respond({ permissions: {}, scope: 'turn' })}>{t('agent.deny')}</Button></div> : approval ? <>
      {!decisions.length ? <p className="agent-dialogue-request-scope">{t('agentExecution.unsupported')}</p> : null}
      <div className="grid gap-2">{decisions.map((decision, index) => typeof decision === 'string' ? <Button key={decision} data-decision={decision} variant="secondary" disabled={busy} onClick={() => void respond({ decision })}>{t(decision === 'accept' ? 'agent.allow' : decision === 'acceptForSession' ? 'agent.allowSession' : decision === 'decline' ? 'agent.deny' : 'common.cancel')}</Button> : <fieldset key={index} disabled={busy} className="grid min-w-0 gap-2"><legend>{t('agent.nativePolicy')}</legend><NativeValue value={decision} /><label className="flex items-start gap-2 text-sm"><input type="checkbox" checked={Boolean(confirmedRules[index])} onChange={event => setConfirmedRules(current => ({ ...current, [index]: event.target.checked }))} />{t('agent.confirmPolicy')}</label><Button variant="secondary" data-decision="apply-policy" disabled={busy || !confirmedRules[index]} onClick={() => void respond({ decision })}>{t('agent.applyPolicy')}</Button></fieldset>)}</div>
    </> : supported ? <div className="flex flex-wrap gap-2"><Button disabled={busy || !canAccept} onClick={() => void respond(accept())}>{t(questions.length ? 'agent.answer' : 'agent.allow')}</Button><Button variant="secondary" disabled={busy} onClick={() => void respond(engine === 'claude' ? { behavior: 'deny' } : pending.kind === 'mcpServer/elicitation/request' ? { action: 'decline', content: null } : { answers: {} })}>{t('agent.deny')}</Button>{pending.kind === 'mcpServer/elicitation/request' ? <Button variant="secondary" disabled={busy} onClick={() => void respond({ action: 'cancel', content: null })}>{t('common.cancel')}</Button> : null}</div> : <p className="agent-dialogue-request-scope">{t('agentExecution.unsupported')}</p>}
  </section>;
}
