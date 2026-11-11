import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Input } from '@dgos/ui';
import { ArrowUp, Square } from 'lucide-react';
import { useLocale } from '../../i18n';
import { agentSendPreflightErrors, useAgentRunner, type AgentSession, type AgentEvent as Event } from '../../stores/agentRunnerStore';
import { MarkdownHighlightContext } from '../../components/chat/Markdown';
import { Request } from '../../components/development/AgentNativeRequest';
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
import { blocks, itemKey, createAgentTranscriptProjector, projectAgentTranscript, findAgentTranscriptRow, object, text, isUser } from '../../components/development/agentTranscript';
import { NativeEvent } from '../../components/development/AgentReplyContent';
import { AgentReplyTimeline } from '../../components/development/AgentReplyTimeline';
import { CodexTurnControls } from '../../components/development/CodexTurnControls';
import CodexOperations from '../../components/development/CodexOperations';
import { AgentImages, AgentImageDropZone } from '../../components/development/AgentImages';
import { acknowledgeAgentImages, agentImageInputs, agentImagePaste, agentImagesKey, captureAgentImages, useAgentImages } from '../../stores/agentImages';
import { AgentSlashPicker } from '../../components/development/AgentSlashPicker';
import { useCodexOperations, codexOperationsKey, emptyCodexOperations } from '../../stores/codexOperations';
import type { CodexOperationScope } from '../../components/development/codexOperationsService';
import { pendingAgentDraftKeys } from '../../components/development/agentPendingRequests';
import { buffersAgentText, useAgentChatPreferences } from '../../stores/agentChatPreferences';

export type AgentRuntimeAction = 'test' | 'preview' | 'build' | 'terminal';
export type AgentMessageFocus = { runId: string; seq: number; nonce: number };

type PendingConversationSend = { requestId: string; generation: string; text: string; references: CapturedConversationReference[]; skills: CapturedAgentSkill[]; images?: { id: string }[]; titleEligible?: boolean; recoveryAttempts?: number; recoveryStopped?: boolean; recoveryError?: string };
const sendRecoveryDelays = [200, 600, 1500, 3000, 5000];
const sendRecoveryFlights = new Set<string>();

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

function revealMessage(target: HTMLElement, viewport: HTMLElement) {
  let ancestor = target.parentElement;
  while (ancestor && ancestor !== viewport) {
    if (ancestor instanceof HTMLDetailsElement) ancestor.open = true;
    ancestor = ancestor.parentElement;
  }
  const child = target.querySelector<HTMLDetailsElement>('.agent-reply-subtask-message');
  if (child) child.open = true;
}

// Drafts belong to the existing account-scoped draft store; this is only its visible projection.
export function AgentConversation({ session, drafts, setDraft, busy, error, visible = true, onOpenReference, onSelectRun, onNewConversation, onContinueModel, onOpenSession, onOpenRuntime, onOpenChanges, focusMessage }: AgentReferenceActions & { session: AgentSession; drafts: { [id: string]: string }; setDraft: (id: string, value: string, expected?: string) => void; busy: boolean; error: boolean; visible?: boolean; onNewConversation?: (configuration?: CliProviderConfiguration) => Promise<void>; onContinueModel?: () => void; onOpenSession?: (sessionId: string) => void; onOpenRuntime?: (action?: AgentRuntimeAction) => void; onOpenChanges?: () => void; focusMessage?: AgentMessageFocus }) {
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
  const preferences = useAgentChatPreferences({ projectId, session });
  const { generation } = useContextSelection();
  const prompt = drafts[session.id] ?? '';
  const imageOwner = agentImagesKey(projectId, session);
  const imageDraft = useAgentImages(state => state.drafts[imageOwner]);
  const hasImages = session.engine === 'codex' && Boolean(imageDraft?.images.length);
  const draftEntries = useDevelopmentDrafts(state => state.entries);
  const pendingSendKeys = pendingAgentDraftKeys(draftEntries, devEpoch, projectId, session.id, 'pending-send');
  const pendingSendKey = pendingSendKeys[0] ?? developmentDraftKey('conversation', projectId, session.id, session.generation, 'pending-send');
  const pendingSend = draftEntries[pendingSendKey]?.value as PendingConversationSend | undefined;
  const pendingSteer = pendingAgentDraftKeys(draftEntries, devEpoch, projectId, session.id, 'pending-steer').length > 0;
  const binding = session.configuration?.format === 'provider_bound' ? session.configuration.snapshot : null;
  const boundConfiguration: CliProviderConfiguration | null = binding ? { engine: session.engine, provider: { provider_id: binding.provider_id, model_id: binding.model_id, provider_revision: binding.provider_revision } } : null;
  const emptySession = !session.events.some(event => event.run || isUser(event)) && !session.active_run && session.status === 'idle' && !pendingSend && !pendingSteer;
  const [sending, setSending] = useState(false);
  const sendingRef = useRef(false);
  const viewport = useRef<HTMLDivElement>(null);
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const requestFocus = useRef<HTMLElement | null>(null);
  const surfaceActive = useSurfaceActive();
  const [documentVisible, setDocumentVisible] = useState(!document.hidden);
  useEffect(() => {
    const changed = () => setDocumentVisible(!document.hidden);
    document.addEventListener('visibilitychange', changed);
    return () => document.removeEventListener('visibilitychange', changed);
  }, []);
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
  const [expandedInput, setExpandedInput] = useState(false);
  const [hasNewContent, setHasNewContent] = useState(false);
  const [shownRows, setShownRows] = useState(100);
  const olderAnchor = useRef<{ top: number; height: number } | null>(null);
  const focusedMessage = useRef<string | null>(null);
  const projector = useMemo(() => createAgentTranscriptProjector(), [session.id, session.generation, epoch]);
  const { rows, toolResults, toolNames } = useMemo(() => projector.project(session.events), [projector, session.events]);
  const runStates = useMemo(() => activityStates(session.events, session), [session.events, session.active_run, session.status]);
  const bufferingText = useMemo(() => buffersAgentText(session, session.active_run, preferences.stream_response), [session.events, session.active_run, session.status, preferences.stream_response]);
  const referenceEvent = useMemo(() => {
    for (let index = session.events.length - 1; index >= 0; index -= 1) {
      const event = session.events[index];
      if (agentOutputReferences(event, toolNames).references.length) return event;
    }
    return undefined;
  }, [session.events, toolNames]);
  function latest() { if (!visibleOwner.current) return; const node = viewport.current; if (node) node.scrollTop = node.scrollHeight; follow.current = true; setAtBottom(true); setHasNewContent(false); }
  useLayoutEffect(() => {
    if (!visible || !surfaceActive) return;
    const position = useDevelopment.getState().reading[session.id];
    follow.current = position?.follow ?? true; setAtBottom(follow.current);
    const node = viewport.current;
    if (node) node.scrollTop = follow.current ? node.scrollHeight : position?.top ?? 0;
  }, [session.id, epoch, visible, surfaceActive]);
  useEffect(() => { setInspect(false); setExpandedInput(false); setHasNewContent(false); setShownRows(100); }, [session.id, epoch]);
  const readingAnchor = useRef<{ node: HTMLElement; offset: number } | null>(null);
  useLayoutEffect(() => {
    const node = viewport.current;
    if (!node || !visibleOwner.current) return;
    const anchor = readingAnchor.current;
    const selection = document.getSelection();
    const selecting = Boolean(selection && !selection.isCollapsed && selection.anchorNode && node.contains(selection.anchorNode));
    if (selecting) { follow.current = false; setAtBottom(false); }
    if (follow.current) latest();
    else {
      if (anchor?.node.isConnected) node.scrollTop += anchor.node.getBoundingClientRect().top - node.getBoundingClientRect().top - anchor.offset;
      if (session.events.length) setHasNewContent(true);
    }
    return () => {
      if (follow.current) { readingAnchor.current = null; return; }
      const top = node.getBoundingClientRect().top;
      const first = Array.from(node.querySelectorAll<HTMLElement>('[data-event-seq]')).find(item => item.getClientRects().length > 0 && item.getBoundingClientRect().bottom > top);
      readingAnchor.current = first ? { node: first, offset: first.getBoundingClientRect().top - top } : null;
    };
  }, [session.events, bufferingText, visible, surfaceActive]);
  useLayoutEffect(() => {
    const node = viewport.current, anchor = olderAnchor.current;
    if (!node || !anchor) return;
    node.scrollTop = anchor.top + node.scrollHeight - anchor.height;
    olderAnchor.current = null;
    follow.current = false; setAtBottom(false);
  }, [shownRows]);
  useLayoutEffect(() => {
    if (!focusMessage || !visible || !surfaceActive || !viewport.current || focusedMessage.current === `${session.id}:${focusMessage.nonce}`) return;
    const row = findAgentTranscriptRow(rows, { run: focusMessage.runId, seq: focusMessage.seq });
    const position = row ? rows.indexOf(row) : -1;
    if (position >= 0 && rows.length - position > shownRows) { setShownRows(rows.length - position); return; }
    const wrapper = Array.from(viewport.current.querySelectorAll<HTMLElement>('[data-reply-key]')).find(node => node.dataset.replyKey === row?.key);
    const target = wrapper?.querySelector<HTMLElement>('[data-event-seq]') ?? wrapper;
    if (!target) return;
    focusedMessage.current = `${session.id}:${focusMessage.nonce}`;
    follow.current = false; setAtBottom(false); target.tabIndex = -1;
    revealMessage(target, viewport.current);
    target.scrollIntoView({ block: 'center' }); target.focus({ preventScroll: true });
    if (devEpoch === useDevelopment.getState().epoch) useDevelopment.getState().setReading(session.id, { top: viewport.current.scrollTop, follow: false });
  }, [focusMessage?.nonce, shownRows, session.id, visible, surfaceActive]);
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
  const currentOwner = () => visibleOwner.current && !document.hidden && navigationOwner.current.mounted && navigationOwner.current.sessionId === session.id && navigationOwner.current.epoch === epoch && navigationOwner.current.devEpoch === devEpoch && navigationOwner.current.generation === generation && epoch === useAgentRunner.getState().epoch && devEpoch === useDevelopment.getState().epoch && generation === contextGeneration() && useAgentRunner.getState().selected === session.id && useAgentRunner.getState().session?.generation === session.generation;
  const manuallyBrowsing = useRef(false);
  const followedEditSeq = useRef(session.events[session.events.length - 1]?.seq ?? 0);
  const [followError, setFollowError] = useState<string | null>(null);
  const followPreferences = useRef(preferences.follow_edits);
  followPreferences.current = preferences.follow_edits;
  useEffect(() => {
    const browse = (event: globalThis.Event) => {
      if (visibleOwner.current && event.target instanceof Element && event.target.closest('.dev-document-column, .dev-navigation, .dev-commit-column, .dev-main-view-nav, .agent-output-references')) manuallyBrowsing.current = true;
    };
    document.addEventListener('pointerdown', browse, true);
    document.addEventListener('keydown', browse, true);
    document.addEventListener('wheel', browse, true);
    return () => { document.removeEventListener('pointerdown', browse, true); document.removeEventListener('keydown', browse, true); document.removeEventListener('wheel', browse, true); };
  }, []);
  useEffect(() => {
    const after = followedEditSeq.current;
    followedEditSeq.current = session.events[session.events.length - 1]?.seq ?? after;
    if (!preferences.follow_edits || !currentOwner() || !follow.current || manuallyBrowsing.current || !onOpenReference || !session.active_run) return;
    for (let index = session.events.length - 1; index >= 0; index -= 1) {
      const edit = session.events[index];
      if (edit.seq <= after) break;
      if (edit.run !== session.active_run || agentEventFailed(edit)) continue;
      if (edit.kind === 'item/started' || object(edit.data).status === 'inProgress') continue;
      if (edit.kind === 'message' && blocks(edit).some(block => object(block).type === 'tool_use' && !toolResults.has(itemKey(edit, text(object(block).id))))) continue;
      const reference = agentOutputReferences(edit, toolNames).references.find(reference => reference.diff);
      if (!reference) continue;
      void Promise.resolve().then(() => { const shouldOpen = () => currentOwner() && !manuallyBrowsing.current && follow.current && followPreferences.current; if (shouldOpen()) return onOpenReference(reference, 'diff', shouldOpen); }).catch(error => { if (currentOwner() && followPreferences.current && !manuallyBrowsing.current) setFollowError(error instanceof Error ? error.message : 'unavailable'); });
      break;
    }
  }, [session.events, preferences.follow_edits]);
  const inputReferences = useComposerReferences({ projectId, taskId, ownerId: session.id, disabled: !visible || busy || sending || acting || session.status !== 'idle' || Boolean(session.pending.length), isCurrent: currentOwner });
  const reason = session.persistence_error ? t('agent.storageFailed') : busy || sending || acting || imageDraft?.busy ? t('agent.sendBusy') : pendingSend ? t('directConversation.send_unknown') : pendingSteer ? t('codexInteractions.unknown') : session.pending.length || session.status === 'waiting' ? t('agent.sendWaiting') : session.status === 'running' || session.active_run ? t(session.engine === 'codex' && session.controls?.steering ? 'codexInteractions.runningHint' : 'agent.sendRunning') : session.status !== 'idle' ? t('agent.sendUnavailable') : !prompt.trim() && !inputReferences.references.length && !hasImages ? t('agent.sendEmpty') : '';
  const referenceActions: AgentReferenceActions = {
    onOpenReference: onOpenReference ? (reference, mode, shouldOpen) => { const canOpen = () => currentOwner() && (!shouldOpen || shouldOpen()); if (!canOpen()) return; manuallyBrowsing.current = true; return onOpenReference(reference, mode, canOpen); } : undefined,
    onSelectRun: onSelectRun ? runId => { if (currentOwner() && session.events.some(event => event.run === runId)) onSelectRun(runId); } : undefined,
    onQuoteReply: value => { if (!currentOwner()) return; setDraft(session.id, `${prompt}${prompt ? '\n\n' : ''}> ${value.replace(/\n/g, '\n> ')}\n\n`, prompt); composerInput.current?.focus({ preventScroll: true }); },
    onOpenAgentActivity: session.engine === 'codex' ? () => { if (!currentOwner()) return; const panel = root.current?.querySelector<HTMLDetailsElement>('[data-testid="codex-operations"]'); if (panel) { panel.open = true; panel.scrollIntoView({ block: 'start' }); panel.querySelector<HTMLElement>('summary')?.focus(); } } : undefined,
    onPreviewResource: async previewId => {
      if (!currentOwner()) throw new Error('image_preview_expired');
      const preview = await invoke<{ base64: string; mime: string }>('preview_agent_output_image', { sessionId: session.id, generation: session.generation, previewId });
      if (!currentOwner()) throw new Error('image_preview_expired');
      return preview;
    },
  };
  function clearPendingSend(requestId: string, requestGeneration: string) {
    if (epoch !== useAgentRunner.getState().epoch || devEpoch !== useDevelopment.getState().epoch) return;
    useDevelopmentDrafts.setState(state => {
      const keys = pendingAgentDraftKeys(state.entries, devEpoch, projectId, session.id, 'pending-send').filter(key => {
        const pending = state.entries[key].value as PendingConversationSend;
        return pending.requestId === requestId && pending.generation === requestGeneration;
      });
      if (!keys.length) return state;
      const entries = { ...state.entries };
      for (const key of keys) delete entries[key];
      return { entries };
    });
  }
  function acknowledgeSend(capture: PendingConversationSend) {
    if (epoch !== useAgentRunner.getState().epoch || devEpoch !== useDevelopment.getState().epoch) return;
    if (capture.titleEligible) {
      const workspace = useDevelopment.getState();
      const tab = workspace.tabs.find(item => item.sessionId === session.id);
      const title = Array.from(capture.text.split(/\r?\n/, 1)[0].trim()).slice(0, 80).join('');
      if (title && tab && !tab.title.trim() && !workspace.sessionNames[session.id]?.trim()) workspace.renameTab(tab.id, title);
    }
    setDraft(session.id, '', capture.text);
    if (projectId) clearConversationReferences(projectId, session.id, capture.references);
    acknowledgeAgentSkills(projectId, { id: session.id, generation: capture.generation }, capture.skills ?? []);
    if (capture.images?.length) acknowledgeAgentImages(agentImagesKey(projectId, { id: session.id, generation: capture.generation }), capture.images);
    clearPendingSend(capture.requestId, capture.generation);
  }
  async function recheckSend() {
    if (!pendingSend || sendingRef.current || busy || acting || !currentOwner() || sendRecoveryFlights.has(pendingSendKey)) return;
    sendRecoveryFlights.add(pendingSendKey);
    sendingRef.current = true; setSending(true);
    try {
      await useAgentRunner.getState().attach(session.id);
      if (!currentOwner()) return;
      const runner = useAgentRunner.getState();
      const live = runner.session;
      if (!runner.error && live?.id === session.id && (live.events.some(event => event.run === pendingSend.requestId) || live.generation === pendingSend.generation && live.active_run === pendingSend.requestId)) acknowledgeSend(pendingSend);
      else useDevelopmentDrafts.setState(state => {
        const entry = state.entries[pendingSendKey];
        if (!entry || (entry.value as PendingConversationSend).requestId !== pendingSend.requestId) return state;
        const recoveryAttempts = (pendingSend.recoveryAttempts ?? 0) + 1;
        return { entries: { ...state.entries, [pendingSendKey]: { ...entry, value: { ...pendingSend, recoveryAttempts, recoveryStopped: recoveryAttempts >= sendRecoveryDelays.length, recoveryError: runner.error ?? 'unknown' } } } };
      });
    } finally { sendRecoveryFlights.delete(pendingSendKey); sendingRef.current = false; if (navigationOwner.current.mounted) setSending(false); }
  }
  useEffect(() => {
    if (!pendingSend || sending || acting || busy || !currentOwner()) return;
    // A retained matching event is admission evidence even after a runtime generation changes.
    if (session.events.some(event => event.run === pendingSend.requestId) || session.generation === pendingSend.generation && session.active_run === pendingSend.requestId) { acknowledgeSend(pendingSend); return; }
    if (pendingSend.recoveryStopped) return;
    const timer = window.setTimeout(() => { void recheckSend(); }, sendRecoveryDelays[Math.min(pendingSend.recoveryAttempts ?? 0, sendRecoveryDelays.length - 1)]);
    return () => window.clearTimeout(timer);
  }, [pendingSend, sending, acting, busy, session.events, session.active_run, session.generation, visible, surfaceActive, documentVisible, epoch, devEpoch, generation]);
  async function send() {
    if (sendingRef.current || reason || !currentOwner()) return;
    const entries = useDevelopmentDrafts.getState().entries;
    if (pendingAgentDraftKeys(entries, devEpoch, projectId, session.id, 'pending-send').length || pendingAgentDraftKeys(entries, devEpoch, projectId, session.id, 'pending-steer').length) return;
    manuallyBrowsing.current = false; setFollowError(null);
    const id = session.id;
    const sentPrompt = prompt;
    const captured = projectId ? captureConversationReferences(projectId, id) : [];
    const skills = captureAgentSkills(projectId, session);
    const images = session.engine === 'codex' ? captureAgentImages(imageOwner) : [];
    if (!sentPrompt.trim() && !captured.length && !images.length) return;
    const proposedSettings = agentTurnSettings(session);
    const settings = { ...proposedSettings, response_preferences: { language: preferences.language, brevity: preferences.brevity, next_step_suggestions: preferences.next_step_suggestions } };
    const capture: PendingConversationSend = { requestId: crypto.randomUUID(), generation: session.generation, text: sentPrompt, references: captured, skills, images: images.map(image => ({ id: image.id })), titleEligible: emptySession };
    sendingRef.current = true; setSending(true);
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [pendingSendKey]: { kind: 'conversation', value: capture, baseline: null } } }));
    try {
      let failure: string | undefined;
      const sent = await action('send_agent_message', { input: { session_id: id, generation: capture.generation, request_id: capture.requestId, text: sentPrompt, ...(settings ? { settings } : {}), ...(captured.length ? { references: captured.map(item => item.reference) } : {}), ...(skills.length ? { skills: skills.map(item => item.skill) } : {}), ...(images.length ? { images: agentImageInputs(images) } : {}) } }, code => { failure = code; });
      if (sent) acknowledgeSend(capture);
      else if (failure && agentSendPreflightErrors[failure]) clearPendingSend(capture.requestId, capture.generation);
    } finally { sendingRef.current = false; if (navigationOwner.current.mounted) setSending(false); }
  }
  return <div ref={root} className="agent-conversation-product" onFocusCapture={event => { requestFocus.current = (event.target as HTMLElement).closest('.agent-dialogue-requests') ? event.target as HTMLElement : null; }}>
    <MarkdownHighlightContext.Provider value={preferences.code_highlighting}>
    <header className="agent-dialogue-header">
      <div className="agent-dialogue-identity"><strong>{t(`agent.engine.${session.engine}`)}</strong><span className="agent-dialogue-status" data-state={session.status}>{t(`agent.state.${session.status}`)}</span></div>
      <div className="agent-dialogue-actions"><Button size="xs" variant="secondary" disabled={busy || session.status === 'closed'} onClick={() => { if (currentOwner()) void action('close_agent_session', { sessionId: session.id }); }}>{t('agent.close')}</Button></div>
    </header>
    <div className="agent-dialogue-transcript" ref={viewport} role="region" tabIndex={0} aria-label={t('agent.messages')} onScroll={event => { if (!visibleOwner.current) return; const node = event.currentTarget; const bottom = node.scrollHeight - node.scrollTop - node.clientHeight < 64; follow.current = bottom; setAtBottom(bottom); if (devEpoch === useDevelopment.getState().epoch) useDevelopment.getState().setReading(session.id, { top: node.scrollTop, follow: bottom }); }}>
      <details className="agent-dialogue-capabilities"><summary>{t('capabilities.actual')}</summary><SessionCapabilitySnapshot key={session.id} sessionId={session.id} snapshot={session.configuration} runtime={session} /></details>
      {session.engine === 'codex' ? <CodexOperations session={session} disabled={busy || !visible || !surfaceActive} onOpenSession={onOpenSession} onSessionChanged={() => { if (currentOwner()) void useAgentRunner.getState().attach(session.id); }} /> : null}
      <div className="agent-dialogue-messages">
        {session.history_truncated ? <p className="text-sm text-ink-secondary">{t('agent.truncated')}</p> : null}
        {rows.length > shownRows ? <Button size="xs" variant="secondary" onClick={() => { const node = viewport.current; if (node) olderAnchor.current = { top: node.scrollTop, height: node.scrollHeight }; setShownRows(count => count + 100); }}>{t('agent.loadOlder')}</Button> : null}
        <AgentReplyTimeline rows={rows.slice(-shownRows)} engine={session.engine} toolResults={toolResults} toolNames={toolNames} runStates={runStates} bufferingRun={bufferingText ? session.active_run : undefined} {...referenceActions} />
        {!session.events.length ? <p className="agent-dialogue-empty">{t('agent.empty')}</p> : null}
        {preferences.next_step_suggestions && session.status === 'idle' && session.events.some(event => ['result', 'turn/completed'].includes(event.kind)) && (onOpenRuntime || onOpenChanges) ? <section className="agent-dialogue-result-actions" aria-label={t('agent.resultActions')}>
          <p>{t('agent.resultBoundary')}</p>
          {onOpenChanges ? <Button size="xs" variant="secondary" onClick={() => { if (currentOwner()) onOpenChanges(); }}>{t('agent.viewChanges')}</Button> : null}
          {onOpenRuntime ? <><Button size="xs" variant="secondary" onClick={() => { if (currentOwner()) onOpenRuntime('test'); }}>{t('agent.runTests')}</Button><Button size="xs" variant="secondary" onClick={() => { if (currentOwner()) onOpenRuntime('preview'); }}>{t('agent.openPreview')}</Button></> : null}
        </section> : null}
      </div>
      {session.events.length ? <details className="agent-dialogue-inspector" open={inspect} onToggle={event => setInspect(event.currentTarget.open)}><summary>{t('agent.rawEvents', { count: session.events.length })}</summary>{inspect ? session.events.map(event => <NativeEvent key={event.seq} event={event} />) : null}</details> : null}
    </div>
    <footer className={`agent-dialogue-footer${expandedInput ? ' agent-dialogue-footer-expanded' : ''}`}>
      {!atBottom ? <Button size="xs" variant="secondary" className="agent-dialogue-latest" onClick={latest}>{t(hasNewContent ? 'agent.newContent' : 'agent.latest')}</Button> : null}
      {interruptPending ? <p role="status">{t('agent.interruptPending')}</p> : null}
      {followError ? <p role="alert" className="text-status-error">{errorText(followError)}</p> : null}
      {session.persistence_error ? <p role="alert" className="text-status-error">{t('agent.storageFailed')}</p> : null}
      {session.engine === 'codex' ? <CodexTurnControls hideMode key={`${epoch}:${session.id}:${session.generation}`} session={session} projectId={projectId} prompt={prompt} setDraft={setDraft} currentOwner={currentOwner} disabled={busy || !visible || !surfaceActive || Boolean(pendingSend)} /> : null}
      <div className="agent-dialogue-requests">
        {['idle', 'running', 'waiting'].includes(session.status) ? session.pending.map(pending => <Request key={`${epoch}:${generation}:${session.id}:${session.generation}:${pending.id}:${JSON.stringify(pending.data)}`} engine={session.engine} pending={pending} sessionId={session.id} generation={session.generation} events={session.events} activeRun={session.active_run} submit={response => currentOwner() && ['idle', 'running', 'waiting'].includes(useAgentRunner.getState().session?.status ?? '') && useAgentRunner.getState().session?.pending.some(item => item.id === pending.id && item.kind === pending.kind && JSON.stringify(item.data) === JSON.stringify(pending.data)) ? action('respond_agent_request', { input: { session_id: session.id, generation: session.generation, request_id: pending.id, response } }) : Promise.resolve(false)} />) : null}
      </div>
      {session.error && !error ? <div role="alert" className="agent-dialogue-error text-status-error"><p>{t('agent.failure')}</p><details className="agent-dialogue-native"><summary>{t('agent.details')}</summary><pre>{session.error}</pre></details></div> : null}
      {pendingSend && !sending && !acting ? <div><p role={pendingSend.recoveryStopped ? 'alert' : 'status'}>{pendingSend.recoveryStopped ? errorText(pendingSend.recoveryError ?? 'unknown') : t('directConversation.send_unknown')}</p>{pendingSend.recoveryStopped ? <Button size="xs" disabled={busy} onClick={() => {
        if (!currentOwner()) return;
        useDevelopmentDrafts.setState(state => {
          const entry = state.entries[pendingSendKey];
          return entry ? { entries: { ...state.entries, [pendingSendKey]: { ...entry, value: { ...pendingSend, recoveryAttempts: 0, recoveryStopped: false, recoveryError: undefined } } } } : state;
        });
      }}>{t('directConversation.retry')}</Button> : null}</div> : null}
      {session.status === 'running' || session.active_run ? <p className="agent-next-draft-note">{t('agent.editWhileRunning')}</p> : null}
      {session.engine === 'codex' ? <ConversationQueue session={session} prompt={prompt} setDraft={setDraft} currentOwner={currentOwner} active={visible && surfaceActive} disabled={busy || sending || acting || Boolean(pendingSend) || Boolean(pendingSteer) || hasImages || Boolean(inputReferences.references.length) || Boolean(captureAgentSkills(projectId, session).length)} /> : session.status === 'running' ? <p className="agent-next-draft-note">{t('agent.queueUnavailable')}</p> : null}
      <AgentImageDropZone owner={imageOwner} enabled={session.engine === 'codex' && currentOwner() && !busy && !sending && !acting && !pendingSend && session.status === 'idle'}><div className={`agent-composer-shell${expandedInput ? ' agent-composer-shell-expanded' : ''}`}>
        <AgentSlashPicker session={session} projectId={projectId} value={prompt} onChange={(value, expected) => { if (currentOwner()) setDraft(session.id, value, expected); }} inputRef={composerInput} active={visible && surfaceActive} disabled={busy || sending || acting || Boolean(pendingSend)} />
        {inputReferences.chips}{session.engine === 'codex' ? <AgentImages owner={imageOwner} active={visible && surfaceActive} disabled={busy || sending || acting || Boolean(pendingSend) || !visible || !surfaceActive || session.status !== 'idle'} /> : null}
        <div className="agent-composer-input"><Input multiline rows={2} id="agent-message" ref={composerInput} label={t('agent.prompt')} placeholder={t('agentPreferences.placeholder')} className="type-reading" aria-describedby="agent-send-hint" value={prompt} onPaste={session.engine === 'codex' ? agentImagePaste(imageOwner, busy || sending || acting || Boolean(pendingSend) || !visible || !surfaceActive || session.status !== 'idle') : undefined} onChange={event => setDraft(session.id, inputReferences.promptChange(event.target.value, prompt, event.target.selectionStart, (event.nativeEvent as InputEvent).isComposing))} onKeyDown={event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) { event.preventDefault(); if (!event.repeat) void send(); } }} /></div>
        <div className="agent-composer-edit-actions"><small>{t('agentSkills.slash.hint')}</small><Button size="xs" variant="ghost" aria-expanded={expandedInput} onClick={() => { setExpandedInput(value => !value); composerInput.current?.focus({ preventScroll: true }); }}>{t(expandedInput ? 'agent.collapseInput' : 'agent.expandInput')}</Button></div>
        <AgentComposerControls session={session} projectId={projectId} visible={visible} disabled={busy || sending || acting || Boolean(pendingSend)} referencePicker={inputReferences.picker} references={referenceEvent ? <AgentOutputReferences key={`${session.id}:${referenceEvent.seq}`} event={referenceEvent} toolNames={toolNames} {...referenceActions} /> : undefined} model={visible && surfaceActive && projectId && onNewConversation && emptySession ? <ComposerModelPicker projectId={projectId} value={boundConfiguration} preferred={boundConfiguration} disabled={busy || sending || acting} onChange={configuration => { if (currentOwner()) void onNewConversation(configuration); }} /> : <div className="dev-review-toolbar"><span className="text-xs">{binding ? `${binding.model_id} · ${t(`agent.engine.${session.engine}`)}` : t(`agent.engine.${session.engine}`)}</span>{onContinueModel ? <Button size="xs" variant="ghost" onClick={() => { if (currentOwner()) onContinueModel(); }}>{t('workbenchNavigation.continueModel')}</Button> : null}</div>} sendHint={reason || t('agent.sendShortcut')} send={session.active_run && ['running', 'waiting'].includes(session.status) ? <Button iconOnly aria-label={t('agent.interrupt')} title={t('agent.interrupt')} disabled={busy || interruptPending} onClick={() => { const current = useAgentRunner.getState().session; if (currentOwner() && current && current.active_run === session.active_run && ['running', 'waiting'].includes(current.status)) void action('interrupt_agent_session', { sessionId: session.id, generation: session.generation, runId: session.active_run }); }}><Square size={16} aria-hidden="true" /></Button> : <Button iconOnly aria-label={t('agent.send')} title={t('agent.send')} disabled={Boolean(reason)} onClick={() => void send()}><ArrowUp size={18} aria-hidden="true" /></Button>} />
      </div></AgentImageDropZone>
    </footer>
    </MarkdownHighlightContext.Provider>
  </div>;
}

function ConversationQueue({ session, prompt, setDraft, currentOwner, active, disabled }: { session: AgentSession; prompt: string; setDraft: (id: string, value: string, expected?: string) => void; currentOwner: () => boolean; active: boolean; disabled: boolean }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const accountKey = useDevelopment(state => state.workspaceOwner);
  const metadata = useDevelopment(state => state.sessions.find(item => item.id === session.id));
  const project = useDevelopment(state => state.projects.find(item => item.id === metadata?.project_id));
  const scope: CodexOperationScope | null = accountKey && metadata?.owner && project?.owner === metadata.owner && !project.archived && metadata.engine === 'codex' ? { epoch, accountKey, owner: metadata.owner, projectId: project.id, taskId: metadata.task_id, sessionId: session.id, generation: session.generation } : null;
  const key = scope ? codexOperationsKey(scope) : '';
  const state = useCodexOperations(store => store.entries[key] ?? emptyCodexOperations);
  const [confirmed, setConfirmed] = useState(false);
  useEffect(() => { setConfirmed(false); }, [key, prompt]);
  // Reading capabilities is local IPC, not a turn or a paid model probe.
  useEffect(() => { if (active && scope && !state.snapshot && !state.reading && !state.pending && !state.busy && session.status === 'running') void useCodexOperations.getState().observe(scope); }, [key, active, session.status]);
  const running = Boolean(session.active_run) && ['running', 'waiting'].includes(session.status);
  const supported = Boolean(state.snapshot && !state.stale && !state.snapshot.unsupported.includes('queue_add'));
  async function add() {
    if (!scope || !currentOwner() || !active || disabled || !running || !prompt.trim() || !supported || state.busy || state.pending) return;
    const captured = prompt;
    const receipt = await useCodexOperations.getState().act(scope, { kind: 'queue_add', text: captured });
    if (epoch !== useDevelopment.getState().epoch || accountKey !== useDevelopment.getState().workspaceOwner) return;
    if (receipt?.state === 'applied') { setDraft(session.id, '', captured); if (currentOwner()) setConfirmed(true); await useCodexOperations.getState().observe(scope); }
  }
  async function inspectQueue() {
    if (!scope || !currentOwner() || !active || state.busy) return;
    const submitted = state.pending?.action;
    const receipt = await useCodexOperations.getState().inspect(scope);
    if (epoch !== useDevelopment.getState().epoch || accountKey !== useDevelopment.getState().workspaceOwner) return;
    if (receipt?.state === 'applied') {
      if (submitted?.kind === 'queue_add') { setDraft(session.id, '', submitted.text); if (currentOwner()) setConfirmed(true); }
      await useCodexOperations.getState().observe(scope);
    }
  }
  if (!running && !state.pending && !state.snapshot?.queue.length && !confirmed) return null;
  return <section className="agent-conversation-queue" aria-label={t('agent.queueTitle')}>
    {running ? <><Button size="xs" variant="secondary" disabled={disabled || !supported || state.busy || Boolean(state.pending) || !prompt.trim()} onClick={() => void add()}>{t('agent.queueNext')}</Button><small>{t('agent.queueBoundary')}</small></> : null}
    {running && !supported ? <p>{t(state.reading ? 'codexOperations.loading' : 'agent.queueUnverified')}{scope ? <Button size="xs" variant="ghost" disabled={state.busy || state.reading || Boolean(state.pending)} onClick={() => { if (currentOwner() && active) void useCodexOperations.getState().observe(scope); }}>{t('codexOperations.refresh')}</Button> : null}</p> : null}
    {confirmed ? <p role="status">{t('agent.queued')}</p> : null}
    {state.error && !state.pending ? <p role="alert">{t('agent.queueFailed')}</p> : null}
    {state.pending ? <div role="status"><p>{t('codexOperations.unknown')}</p><Button size="xs" variant="secondary" disabled={state.busy || !active} onClick={() => void inspectQueue()}>{t('codexOperations.inspect')}</Button></div> : null}
    {state.snapshot?.queue.length ? <details><summary>{t('agent.queuedCount', { count: state.snapshot.queue.length })}</summary><ol>{state.snapshot.queue.map(item => <li key={item.id}>{item.text}</li>)}</ol><p>{t('agent.queueManage')}</p></details> : null}
  </section>;
}

// Persisted run events are read-only evidence, not a fabricated live session or a second runner.
export function AgentSessionHistory({ sessionId, taskId, children, visible = true, focusMessage, onOpenReference, onSelectRun, onQuoteReply }: AgentReferenceActions & { sessionId: string; taskId: string; children: ReactNode; visible?: boolean; focusMessage?: AgentMessageFocus }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const projectId = useDevelopment(state => state.sessions.find(item => item.id === sessionId)?.project_id ?? state.selection.project_id);
  const preferences = useAgentChatPreferences({ projectId, sessionId });
  const { generation } = useContextSelection();
  const [runs, setRuns] = useState<DevRun[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const [shownRuns, setShownRuns] = useState(10);
  const historyAnchor = useRef<{ top: number; height: number } | null>(null);
  const focusedMessage = useRef<string | null>(null);
  const surfaceActive = useSurfaceActive();
  const visibleOwner = useRef(visible && surfaceActive);
  visibleOwner.current = visible && surfaceActive;
  useLayoutEffect(() => {
    if (!visible || !surfaceActive || !runs || !viewport.current) return;
    const position = useDevelopment.getState().reading[sessionId];
    viewport.current.scrollTop = position?.follow === false ? position.top : viewport.current.scrollHeight;
  }, [sessionId, runs, visible, surfaceActive]);
  useLayoutEffect(() => {
    const node = viewport.current, anchor = historyAnchor.current;
    if (!node || !anchor) return;
    node.scrollTop = anchor.top + node.scrollHeight - anchor.height;
    historyAnchor.current = null;
  }, [shownRuns]);
  useLayoutEffect(() => {
    if (!focusMessage || !visible || !surfaceActive || !runs || !viewport.current || focusedMessage.current === `${sessionId}:${focusMessage.nonce}`) return;
    const index = runs.findIndex(run => run.events.some(event => event.run === focusMessage.runId && event.seq === focusMessage.seq));
    if (index >= 0 && runs.length - index > shownRuns) { setShownRuns(runs.length - index); return; }
    const run = runs.find(value => value.events.some(event => event.run === focusMessage.runId));
    const row = run && findAgentTranscriptRow(projectAgentTranscript(run.events).rows, { run: focusMessage.runId, seq: focusMessage.seq });
    const wrapper = Array.from(viewport.current.querySelectorAll<HTMLElement>('[data-reply-key]')).find(node => node.dataset.replyKey === row?.key);
    const target = wrapper?.querySelector<HTMLElement>('[data-event-seq]') ?? wrapper;
    if (!target) return;
    focusedMessage.current = `${sessionId}:${focusMessage.nonce}`;
    revealMessage(target, viewport.current);
    target.tabIndex = -1; target.scrollIntoView({ block: 'center' }); target.focus({ preventScroll: true });
    if (epoch === useDevelopment.getState().epoch) useDevelopment.getState().setReading(sessionId, { top: viewport.current.scrollTop, follow: false });
  }, [focusMessage?.nonce, runs, shownRuns, visible, surfaceActive]);
  useEffect(() => {
    let active = true;
    setRuns(null); setError(null);
    setShownRuns(10);
    void listDevelopmentRuns(taskId).then(result => {
      if (!active || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      if (result.ok) setRuns(result.value.filter(run => run.session_id === sessionId).sort((a, b) => a.started_at - b.started_at));
      else setError(result.code);
    });
    return () => { active = false; };
  }, [sessionId, taskId, epoch, generation]);
  return <div className="agent-conversation-product" data-testid="dev-session-history">
    <MarkdownHighlightContext.Provider value={preferences.code_highlighting}>
    <div className="agent-dialogue-transcript" ref={viewport} role="region" aria-label={t('agent.messages')} tabIndex={0} onScroll={event => { if (!visibleOwner.current) return; const node = event.currentTarget; if (epoch === useDevelopment.getState().epoch) useDevelopment.getState().setReading(sessionId, { top: node.scrollTop, follow: node.scrollHeight - node.scrollTop - node.clientHeight < 64 }); }}>
      <details className="agent-dialogue-capabilities"><summary>{t('capabilities.actual')}</summary><SessionCapabilitySnapshot sessionId={sessionId} snapshot={undefined} /></details>
      {runs && runs.length > shownRuns ? <Button size="xs" variant="secondary" onClick={() => { const node = viewport.current; if (node) historyAnchor.current = { top: node.scrollTop, height: node.scrollHeight }; setShownRuns(count => count + 10); }}>{t('agent.loadOlder')}</Button> : null}
      {error ? <p role="alert">{errorText(error)}</p> : runs === null ? <p role="status">{t('review.loading')}</p> : !runs.length ? <p>{t('agent.empty')}</p> : runs.slice(-shownRuns).map(run => {
        const { rows, toolResults, toolNames } = projectAgentTranscript(run.events);
        const runStates = activityStates(run.events);
        return <div className="agent-dialogue-messages" key={run.id}>{run.truncated ? <p>{t('agent.truncated')}</p> : null}<AgentReplyTimeline rows={rows} engine={run.engine === 'host' ? undefined : run.engine} toolResults={toolResults} toolNames={toolNames} runStates={runStates} onOpenReference={visibleOwner.current ? onOpenReference : undefined} onSelectRun={visibleOwner.current ? onSelectRun : undefined} onQuoteReply={visibleOwner.current ? onQuoteReply : undefined} /></div>;
      })}
    </div>
    <footer className="agent-dialogue-footer">{children}</footer>
    </MarkdownHighlightContext.Provider>
  </div>;
}
