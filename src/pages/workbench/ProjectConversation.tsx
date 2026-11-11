import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Input } from '@dgos/ui';
import { ArrowUp } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment, type DevelopmentTask, type Project, type Outcome } from '../../stores/developmentStore';
import { agentSendPreflightErrors, useAgentRunner, type AgentOpenInput, type AgentSession } from '../../stores/agentRunnerStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { developmentDraftKey, useDevelopmentDrafts, useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { capabilitiesChanged, readCliProviderCatalog, readCliProviderRevision, saveCliProvider, useProjectCapabilities, type CliProviderConfiguration } from '../../stores/developmentCapabilities';
import { readDevelopmentWorkspaces, type StoredWorkspace } from '../../stores/developmentDelivery';
import { hostCode } from '../../stores/hostError';
import { errorText } from '../../components/development/shared';
import { ComposerModelPicker } from '../../components/development/ComposerModelPicker';
import { AgentComposerControls } from '../../components/development/AgentComposerControls';
import { acknowledgeConversationReferences, captureConversationReferences, type CapturedConversationReference } from '../../stores/developmentReferences';
import { useComposerReferences } from '../../components/development/ComposerReferences';
import { agentTurnSettings, bindAgentDraftSettings, rememberAgentSessionConfiguration, resolveAgentDefaults } from '../../stores/agentTurnSettings';
import { agentChatPreferences, bindAgentChatPreferences } from '../../stores/agentChatPreferences';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { acknowledgeAgentSkills, captureAgentSkills, type CapturedAgentSkill } from '../../stores/agentSkills';
import { AgentImages, AgentImageDropZone } from '../../components/development/AgentImages';
import { acknowledgeAgentImages, agentImageInputs, agentImagePaste, agentImagesKey, captureAgentImages, transferAgentImages, useAgentImages } from '../../stores/agentImages';
import { useWorkspaceTrust } from '../../stores/developmentTrust';
import { AgentSlashPicker } from '../../components/development/AgentSlashPicker';
import type { CapturedAgentImage } from '../../stores/agentImages';
import { flushDesktopState } from '../../stores/developmentDesktop';

// Only UI intent and reconciliation identities live here. Tasks, bindings, runs and sessions
// remain owned by their existing host repositories and AgentManager.
export interface ConversationDraft {
  prompt: string;
  configuration: CliProviderConfiguration | null;
  taskId: string | null;
  sessionId: string | null;
  requestId: string | null;
  submitted: string | null;
  submittedReferences: CapturedConversationReference[];
  submittedSkills: CapturedAgentSkill[];
  submittedImageIds: string[];
  submittedGeneration: string | null;
  preparedImageGeneration: string | null;
  taskRequestId?: string;
  taskGoal?: string;
  /** Explicit first-send intent survives an uncertain startup, but never a dispatched message. */
  sendIntent?: boolean;
  recoveryAttempts?: number;
  recoveryStopped?: boolean;
  restartAttempts?: number;
  resumePreparation?: boolean;
  phase: 'draft' | 'task_unknown' | 'open_unknown' | 'ready' | 'send_unknown' | 'sent';
  preparing: boolean;
  error: string | null;
}
export const emptyConversationDraft: ConversationDraft = { prompt: '', configuration: null, taskId: null, sessionId: null, requestId: null, submitted: null, submittedReferences: [], submittedSkills: [], submittedImageIds: [], submittedGeneration: null, preparedImageGeneration: null, phase: 'draft', preparing: false, error: null };
const operations = new Set<string>();
const recoveryDelays = [200, 600, 1500, 3000, 5000];
interface StartReconciliation { status: 'not_started' | 'ready' | 'closed' | 'recovering'; session: AgentSession | null }
export const sameConfiguration = (a: CliProviderConfiguration | null, b: CliProviderConfiguration | null) => Boolean(a && b && a.engine === b.engine && a.provider.provider_id === b.provider.provider_id && a.provider.model_id === b.provider.model_id && a.provider.provider_revision === b.provider.provider_revision);
type Open = (input: AgentOpenInput, projectId: string, taskId: string, dispatch: (session: AgentSession) => Promise<Outcome<null>>) => Promise<Outcome<null> & { notStarted?: boolean }>;

export function ProjectConversation({ project, taskId, draftId, onStart, onInspect, setSessionDraft, visible = true, autoPrepare = true, onComposerInput }: { project: Project; taskId: string | null; draftId?: string; onStart: Open; onInspect: (sessionId: string) => Promise<void>; setSessionDraft: (id: string, prompt: string) => void; visible?: boolean; autoPrepare?: boolean; onComposerInput?: () => void }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const saved = useProjectCapabilities(project.id);
  const scope = draftId ? [project.id, draftId] : [project.id];
  const draft = useScopedDevelopmentDraft('conversation', scope, emptyConversationDraft);
  const key = developmentDraftKey('conversation', ...scope);
  const operationKey = `${epoch}:${key}`;
  const value = draft.value;
  const configuration = value.configuration;
  const imageOwner = agentImagesKey(project.id, value.sessionId && value.preparedImageGeneration ? { id: value.sessionId, generation: value.preparedImageGeneration } : draftId ?? project.id);
  const imageDraft = useAgentImages(state => state.drafts[imageOwner]);
  const liveSession = useAgentRunner(state => state.session);
  const draftSession = liveSession?.id === value.sessionId ? liveSession : undefined;
  const readySession = value.phase === 'ready' ? draftSession : undefined;
  const interrupt = useAgentRunner(state => value.sessionId ? state.interrupts[value.sessionId] : undefined);
  const interruptPending = Boolean(draftSession && interrupt?.generation === draftSession.generation && interrupt.runId === draftSession.active_run);
  const [directory, setDirectory] = useState<StoredWorkspace | null>(null);
  const [directoryError, setDirectoryError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const autoAttempt = useRef('');
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const initialPreparation = useRef(false);
  const queuedSend = useRef<{ text: string; references: CapturedConversationReference[]; images: CapturedAgentImage[] } | null>(null);
  const [firstSendQueued, setFirstSendQueued] = useState(false);
  const mounted = useRef(true);
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const surfaceActive = useSurfaceActive();
  const [documentVisible, setDocumentVisible] = useState(!document.hidden);
  useEffect(() => {
    const changed = () => setDocumentVisible(!document.hidden);
    document.addEventListener('visibilitychange', changed);
    return () => document.removeEventListener('visibilitychange', changed);
  }, []);
  const trust = useWorkspaceTrust(project.id, value.taskId ?? taskId, visible && surfaceActive && documentVisible && !pending, { autoAgent: true });
  const visibleOwner = useRef(visible && surfaceActive);
  visibleOwner.current = visible && surfaceActive;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const current = () => visibleOwner.current && !document.hidden && mounted.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration() && useDevelopment.getState().selection.project_id === project.id;
  const readDraft = () => (useDevelopmentDrafts.getState().entries[key]?.value as ConversationDraft | undefined) ?? emptyConversationDraft;
  function update(patch: Partial<ConversationDraft>) {
    if (epoch !== useDevelopment.getState().epoch) return;
    const next = { ...readDraft(), ...patch };
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: next, baseline: emptyConversationDraft } } }));
  }
  function clearCommitted(id: string) {
    if (epoch !== useDevelopment.getState().epoch || useDevelopment.getState().selection.session_id !== id || !['sent', 'ready'].includes(readDraft().phase)) return;
    // References may be added while the acknowledged session is still being selected.
    const remaining = readDraft().prompt;
    if (remaining) setSessionDraft(id, remaining);
    acknowledgeConversationReferences(project.id, draftId ?? project.id, id, []);
    const runner = useAgentRunner.getState();
    const session = runner.snapshots[id] ?? (runner.session?.id === id ? runner.session : undefined);
    if (session) transferAgentImages(imageOwner, agentImagesKey(project.id, session));
    useDevelopmentDrafts.setState(state => { const entries = { ...state.entries }; delete entries[key]; return { entries }; });
  }
  useEffect(() => {
    const preferred = resolveAgentDefaults(project.id).configuration ?? saved.value?.configuration;
    if (current() && !value.configuration && preferred) update({ configuration: preferred });
  }, [project.id, epoch, generation, saved.value, value.configuration, visible, surfaceActive, documentVisible]);
  const targetTask = value.taskId ?? taskId;
  const referenceOwnerId = draftId ?? project.id;
  const inputReferences = useComposerReferences({ projectId: project.id, taskId: targetTask, ownerId: referenceOwnerId, disabled: !visible || value.preparing || project.archived, isCurrent: current });
  const trusted = Boolean(directory && trust.value?.agent && trust.value.path === directory.path && trust.value.directory_identity === directory.directory_identity);
  useEffect(() => {
    let active = true;
    setDirectory(null); setDirectoryError(null);
    if (pending || !visible || !surfaceActive || !documentVisible) return;
    void readDevelopmentWorkspaces({ project_id: project.id, task_id: targetTask }).then(result => {
      if (!active || !current()) return;
      if (!result.ok) { setDirectoryError(result.code); return; }
      const workspace = result.value.workspaces.find(item => result.value.selected_workspace_id ? item.id === result.value.selected_workspace_id : item.kind === 'original');
      if (!workspace) { setDirectoryError('workspace_not_found'); return; }
      setDirectory(workspace);
    });
    return () => { active = false; };
  }, [project.id, project.path, targetTask, epoch, generation, pending, reload, visible, surfaceActive, documentVisible]);
  useEffect(() => {
    if (!autoPrepare || !current() || modelPickerOpen || useDevelopment.getState().selection.session_id || pending || !trusted || !configuration || value.preparing || !['draft', 'ready'].includes(value.phase) || value.error || project.archived) return;
    const resuming = Boolean(value.resumePreparation);
    if (value.phase === 'ready' && !resuming) return;
    const attempt = JSON.stringify([epoch, project.id, draftId, directory?.directory_identity, configuration]);
    if (!resuming && autoAttempt.current === attempt) return;
    const timer = window.setTimeout(() => { autoAttempt.current = attempt; void send(true, resuming); }, 180);
    return () => window.clearTimeout(timer);
  }, [autoPrepare, trusted, configuration, value.preparing, value.phase, value.error, value.resumePreparation, visible, surfaceActive, documentVisible, pending, directory, epoch, generation, project.id, draftId, modelPickerOpen]);
  useEffect(() => {
    if (!current() || pending || value.preparing || value.recoveryStopped || !['task_unknown', 'open_unknown', 'send_unknown', 'sent'].includes(value.phase)) return;
    const attempts = value.recoveryAttempts ?? 0;
    if (attempts >= recoveryDelays.length) { update({ recoveryStopped: true, error: value.error ?? 'unknown' }); return; }
    const timer = window.setTimeout(() => { void reconcile(); }, recoveryDelays[attempts]);
    return () => window.clearTimeout(timer);
  }, [value.phase, value.preparing, value.recoveryAttempts, value.recoveryStopped, visible, surfaceActive, documentVisible, epoch, generation, pending]);
  useLayoutEffect(() => {
    const node = composerInput.current;
    if (!node) return;
    node.rows = 2;
    const metrics = getComputedStyle(node);
    const lineHeight = Number.parseFloat(metrics.lineHeight) || 20;
    const padding = (Number.parseFloat(metrics.paddingTop) || 0) + (Number.parseFloat(metrics.paddingBottom) || 0);
    node.rows = Math.max(2, Math.min(10, Math.ceil((node.scrollHeight - padding) / lineHeight)));
  }, [value.prompt, visible, surfaceActive]);

  async function send(preparationOnly = false, resumeIntent = false) {
    if (!current() || pending || project.archived || !configuration || !directory || draftSession?.persistence_error) return;
    if (value.preparing || operations.has(operationKey)) {
      if (initialPreparation.current && !preparationOnly && !readDraft().sendIntent && (value.prompt.trim() || inputReferences.references.length || imageDraft?.images.length)) {
        queuedSend.current = { text: value.prompt, references: captureConversationReferences(project.id, referenceOwnerId), images: captureAgentImages(imageOwner) };
        update({ sendIntent: true, submitted: queuedSend.current.text, submittedReferences: queuedSend.current.references, submittedImageIds: queuedSend.current.images.map(image => image.id) });
        setFirstSendQueued(true);
      }
      return;
    }
    if (useAgentImages.getState().drafts[imageOwner]?.busy || ['task_unknown', 'open_unknown', 'send_unknown', 'sent'].includes(value.phase)) return;
    const retained = readDraft();
    let text = resumeIntent && retained.sendIntent ? retained.submitted ?? retained.prompt : value.prompt;
    let captured = resumeIntent && retained.sendIntent ? retained.submittedReferences : captureConversationReferences(project.id, referenceOwnerId);
    let images = captureAgentImages(imageOwner);
    if (resumeIntent && retained.sendIntent) {
      images = images.filter(image => retained.submittedImageIds.includes(image.id));
      if (images.length !== retained.submittedImageIds.length) { update({ error: 'agent_image_invalid', resumePreparation: false, sendIntent: false }); return; }
    }
    if (images.length && configuration.engine !== 'codex') { update({ error: 'agent_images_unsupported' }); return; }
    if (!preparationOnly && !text.trim() && !captured.length && !images.length) return;
    if (!trusted) return;
    if (!current()) return;
    const selected = configuration;
    const confirmedDirectory = directory;
    operations.add(operationKey);
    initialPreparation.current = preparationOnly;
    update({ preparing: true, error: null, configuration: selected, resumePreparation: false,
      ...(!resumeIntent ? { recoveryAttempts: 0, recoveryStopped: false } : {}),
      ...(!preparationOnly ? { sendIntent: true, submitted: text, submittedReferences: captured, submittedImageIds: images.map(image => image.id), restartAttempts: 0 } : {}) });
    let activeTask = targetTask;
    try {
      // Revalidate the exact click-time route. Never silently replace a stale revision.
      const [catalog, revision] = await Promise.all([readCliProviderCatalog(project.id, selected.engine), readCliProviderRevision(project.id)]);
      if (!current()) return;
      const provider = catalog.providers.find(item => item.id === selected.provider.provider_id);
      if (catalog.project_id !== project.id || catalog.engine !== selected.engine || !provider?.enabled || provider.availability?.available !== true || provider.availability.reason !== null || provider.config_revision !== selected.provider.provider_revision || !provider.models.some(model => model.id === selected.provider.model_id)) {
        update({ error: provider?.availability?.reason ?? 'provider_revision_changed' }); return;
      }
      if (saved.value && (!revision || revision.revision !== saved.value.revision || revision.permission_epoch !== saved.value.permission_epoch)) { update({ error: 'capability_conflict' }); capabilitiesChanged(); return; }
      const binding = sameConfiguration(revision?.configuration ?? null, selected) ? revision! : await saveCliProvider(project.id, revision?.revision ?? 0, selected);
      if (!current()) return;
      capabilitiesChanged();
      if (!activeTask) {
        const taskRequestId = retained.taskRequestId ?? crypto.randomUUID();
        const taskGoal = retained.taskGoal ?? Array.from(text.trim().split(/\r?\n/, 1)[0] || captured[0]?.reference.path || t('directConversation.defaultTaskGoal')).slice(0, 80).join('');
        // Persist identity and payload before the first host call. Retrying this
        // task reservation cannot create or modify another task.
        update({ phase: 'task_unknown', taskRequestId, taskGoal });
        if (!await flushDesktopState()) { if (current()) update({ recoveryStopped: true, error: 'storage_failed' }); return; }
        if (!current()) return;
        const created = await invoke<DevelopmentTask>('prepare_agent_task', { input: { id: taskRequestId, project_id: project.id, goal: taskGoal } });
        if (epoch !== useDevelopment.getState().epoch) return;
        if (created.id !== taskRequestId || created.project_id !== project.id || created.goal !== taskGoal) { update({ error: 'unknown', recoveryStopped: true }); return; }
        activeTask = created.id;
        update({ taskId: activeTask, phase: 'draft' });
        if (!await useDevelopment.getState().load()) { if (current()) update({ error: useDevelopment.getState().error ?? 'unknown' }); return; }
        if (!current()) return;
      }
      const result = await readDevelopmentWorkspaces({ project_id: project.id, task_id: activeTask });
      if (!current()) return;
      if (!result.ok) { update({ error: result.code }); return; }
      const workspace = result.value.workspaces.find(item => result.value.selected_workspace_id ? item.id === result.value.selected_workspace_id : item.kind === 'original');
      if (!workspace || workspace.path !== confirmedDirectory.path || workspace.directory_identity !== confirmedDirectory.directory_identity) { void trust.refresh(); setReload(n => n + 1); update({ error: 'workspace_changed' }); return; }
      const id = value.sessionId ?? crypto.randomUUID();
      const requestId = crypto.randomUUID();
      update({ taskId: activeTask, sessionId: id, requestId, ...(!readDraft().sendIntent ? { submitted: text, submittedReferences: captured, submittedImageIds: images.map(image => image.id) } : {}), phase: 'open_unknown' });
      const dispatch = async (session: AgentSession): Promise<Outcome<null>> => {
        if (!current()) return { ok: false, code: 'cancelled' };
        if (session.id !== id || session.status !== 'idle') { update({ phase: 'ready', error: 'session_busy' }); return { ok: false, code: 'session_busy' }; }
        if (!sameConfiguration(readDraft().configuration, selected)) {
          const closed = await useAgentRunner.getState().action('close_agent_session', { sessionId: id });
          if (current() && closed) update({ phase: 'draft', sessionId: null, requestId: null, submitted: null, sendIntent: false, error: null });
          return { ok: false, code: closed ? 'cancelled' : 'cleanup_pending' };
        }
        rememberAgentSessionConfiguration(project.id, id, selected);
        bindAgentDraftSettings(project.id, draftId ?? project.id, session);
        bindAgentChatPreferences(project.id, draftId ?? project.id, session);
        const skillSource = developmentDraftKey('conversation', project.id, draftId ?? project.id, 'skills');
        const skillTarget = developmentDraftKey('conversation', project.id, session.id, 'skills');
        if (skillSource !== skillTarget) useDevelopmentDrafts.setState(state => {
          if (!state.entries[skillSource] || state.entries[skillTarget]) return state;
          const entries = { ...state.entries, [skillTarget]: state.entries[skillSource] }; delete entries[skillSource]; return { entries };
        });
        if (preparationOnly && !readDraft().sendIntent) {
          // Commit ownership before onStart binds/selects the session: that transition
          // can unmount this composer before clearCommitted gets control back.
          transferAgentImages(imageOwner, agentImagesKey(project.id, session));
          // Opening discovers controls without dispatching a turn or consuming draft inputs.
          update({ phase: 'ready', submittedGeneration: session.generation, preparedImageGeneration: session.generation, error: null });
          setSessionDraft(id, readDraft().prompt);
          return { ok: true, value: null };
        }
        const intent = readDraft();
        if (intent.sendIntent) {
          text = intent.submitted ?? text; captured = intent.submittedReferences;
          images = captureAgentImages(imageOwner).filter(image => intent.submittedImageIds.includes(image.id));
          if (images.length !== intent.submittedImageIds.length) { update({ phase: 'ready', sendIntent: false, error: 'agent_image_invalid' }); return { ok: false, code: 'agent_image_invalid' }; }
        }
        queuedSend.current = null;
        const skills = captureAgentSkills(project.id, session);
        update({ phase: 'send_unknown', sendIntent: false, recoveryAttempts: 0, recoveryStopped: false, submittedSkills: skills, submittedGeneration: session.generation });
        const { language, brevity, next_step_suggestions } = agentChatPreferences({ projectId: project.id, session });
        const settings = { ...agentTurnSettings(session), response_preferences: { language, brevity, next_step_suggestions } };
        let failure: string | undefined;
        const sent = await useAgentRunner.getState().action('send_agent_message', { input: { session_id: id, generation: session.generation, request_id: requestId, text, settings, ...(captured.length ? { references: captured.map(item => item.reference) } : {}), ...(skills.length ? { skills: skills.map(item => item.skill) } : {}), ...(images.length ? { images: agentImageInputs(images) } : {}) } }, code => { failure = code; });
        if (epoch !== useDevelopment.getState().epoch) return { ok: false, code: 'cancelled' };
        if (!sent) {
          const code = failure ?? 'unknown';
          // Explicit host preflight rejections precede any run receipt or native dispatch.
          const rejected = agentSendPreflightErrors[code];
          update({ error: code, ...(rejected ? { phase: 'ready' as const } : {}) });
          return { ok: false, code };
        }
        const remaining = readDraft().prompt === text ? '' : readDraft().prompt;
        acknowledgeConversationReferences(project.id, referenceOwnerId, id, captured);
        acknowledgeAgentSkills(project.id, session, skills);
        acknowledgeAgentImages(imageOwner, images);
        update({ phase: 'sent', prompt: remaining, error: null });
        setSessionDraft(id, remaining);
        return { ok: true, value: null };
      };
      if (value.phase === 'ready') {
        await useAgentRunner.getState().attach(id);
        if (!current()) return;
        const runner = useAgentRunner.getState();
        if (runner.error || runner.session?.id !== id) { update({ phase: 'open_unknown', error: runner.error ?? 'unknown' }); return; }
        if ((await dispatch(runner.session)).ok && current()) { await onInspect(id); clearCommitted(id); }
      } else {
        const opened = await onStart({ intent: 'create', id, engine: selected.engine, task_id: activeTask, project_revision: binding.revision, provider: selected.provider, options: { workspace: workspace.path, resume_id: null, executable: null, model: null, trust_workspace: false } }, project.id, activeTask, dispatch);
        if (epoch !== useDevelopment.getState().epoch) return;
        if (opened.ok) clearCommitted(id);
        else if (readDraft().phase === 'open_unknown') {
          if (opened.notStarted) {
            update({ phase: 'draft', sessionId: null, requestId: null, submitted: null, submittedReferences: [], submittedSkills: [], submittedImageIds: [], submittedGeneration: null, sendIntent: false, resumePreparation: false, error: opened.code });
            return;
          }
          // Only the host reconciliation contract can fence an absent launch. An absent
          // read alone does not prevent a delayed create from arriving afterwards.
          const rejected = ['provider_incompatible', 'provider_protocol_unsupported', 'provider_auth_unsupported', 'provider_route_invalid', 'provider_model_unsupported', 'provider_model_mismatch', 'provider_auth_conflict', 'provider_binding_unconfirmed', 'provider_required', 'provider_revision_changed', 'credential_unavailable', 'cli_incompatible', 'cli_not_installed', 'cli_not_executable', 'cli_probe_timeout', 'cli_probe_failed', 'cli_start_failed', 'cli_handshake_failed', 'cli_handshake_timeout', 'workspace_untrusted', 'workspace_changed', 'invalid_workspace', 'project_archived', 'project_not_found', 'task_not_found', 'capability_conflict', 'capability_revoked'].includes(opened.code);
          update({ phase: rejected ? 'draft' : 'open_unknown', sessionId: rejected ? null : id, error: opened.code });
        } else if (!(opened.code === 'cancelled' && !sameConfiguration(readDraft().configuration, selected) && readDraft().phase === 'draft')) update({ error: opened.code });
      }
    } catch (error) { update({ error: hostCode(error) }); }
    finally { operations.delete(operationKey); initialPreparation.current = false; queuedSend.current = null; if (mounted.current) setFirstSendQueued(false); if (useDevelopmentDrafts.getState().entries[key]) update({ preparing: false }); }
  }

  async function reconcile() {
    if (operations.has(operationKey) || !current() || pending) return;
    const original = readDraft();
    operations.add(operationKey);
    update({ preparing: true, recoveryAttempts: (original.recoveryAttempts ?? 0) + 1 });
    try {
      if (original.phase === 'task_unknown') {
        if (!original.taskRequestId || original.taskGoal === undefined) {
          // Legacy task uncertainty has no provable request identity.
          await useDevelopment.getState().load();
          if (current()) update({ error: 'unknown' });
          return;
        }
        if (!await flushDesktopState()) { if (current()) update({ recoveryStopped: true, error: 'storage_failed' }); return; }
        if (!current()) return;
        const created = await invoke<DevelopmentTask>('prepare_agent_task', { input: { id: original.taskRequestId, project_id: project.id, goal: original.taskGoal } });
        if (!current()) return;
        if (created.id !== original.taskRequestId || created.project_id !== project.id || created.goal !== original.taskGoal) { update({ error: 'unknown', recoveryStopped: true }); return; }
        if (!await useDevelopment.getState().load()) { if (current()) update({ error: useDevelopment.getState().error ?? 'unknown' }); return; }
        if (current()) update({ taskId: created.id, phase: 'draft', resumePreparation: true, recoveryAttempts: 0, error: null });
        return;
      }
      if (!original.sessionId) return;
      if (original.phase === 'open_unknown') {
        const result = await invoke<StartReconciliation>('reconcile_agent_start', { input: { session_id: original.sessionId, task_id: original.taskId ?? taskId } });
        if (!current()) return;
        if (result.status === 'recovering') { update({ error: result.session?.persistence_error ? 'storage_failed' : original.error ?? 'cleanup_pending' }); return; }
        if (result.status === 'not_started' || result.status === 'closed') {
          if ((original.restartAttempts ?? 0) >= 1) {
            update({ phase: 'draft', sessionId: null, requestId: null, sendIntent: false, recoveryStopped: true, error: original.error ?? 'cli_start_failed' });
          } else {
            update({ phase: 'draft', sessionId: null, requestId: null, restartAttempts: (original.restartAttempts ?? 0) + 1, resumePreparation: true, recoveryAttempts: 0, error: null });
          }
          return;
        }
        if (!result.session || result.session.id !== original.sessionId || result.session.status !== 'idle' || result.session.active_run || result.session.pending.length || result.session.persistence_error || result.session.events.some(event => event.run)) {
          update({ error: result.session?.persistence_error ? 'storage_failed' : 'session_busy' }); return;
        }
        // open_unknown precedes dispatch. Ready is evidence that this original
        // session can be selected, or consume the one retained explicit send.
        update({ phase: 'ready', resumePreparation: true, recoveryAttempts: 0, error: null });
        return;
      }
      await useAgentRunner.getState().attach(original.sessionId);
      if (!current()) return;
      const runner = useAgentRunner.getState();
      if (runner.error || runner.session?.id !== original.sessionId) { update({ error: runner.error ?? 'unknown' }); return; }
      if (original.phase === 'send_unknown') {
        // Admission evidence acknowledges the original request; no recovery path sends it again.
        if (!runner.session.events.some(event => event.run === original.requestId) && !(runner.session.generation === original.submittedGeneration && runner.session.active_run === original.requestId)) { update({ error: 'unknown' }); return; }
        const remaining = readDraft().prompt === original.submitted ? '' : readDraft().prompt;
        acknowledgeConversationReferences(project.id, referenceOwnerId, original.sessionId, original.submittedReferences ?? []);
        if (original.submittedGeneration) acknowledgeAgentSkills(project.id, { id: original.sessionId, generation: original.submittedGeneration }, original.submittedSkills ?? []);
        acknowledgeAgentImages(imageOwner, (original.submittedImageIds ?? []).map(id => ({ id })));
        update({ phase: 'sent', prompt: remaining, error: null });
        setSessionDraft(original.sessionId, remaining);
      } else if (original.phase === 'sent') setSessionDraft(original.sessionId, readDraft().prompt);
      await onInspect(original.sessionId);
      if (epoch === useDevelopment.getState().epoch) clearCommitted(original.sessionId);
    } catch (error) { if (current()) update({ error: hostCode(error) }); }
    finally { operations.delete(operationKey); if (useDevelopmentDrafts.getState().entries[key]) update({ preparing: false }); }
  }
  function retry() {
    if (!current() || value.preparing) return;
    autoAttempt.current = '';
    update({ error: null, recoveryStopped: false, recoveryAttempts: 0, ...(['draft', 'ready'].includes(value.phase) ? { sendIntent: false, restartAttempts: 0, resumePreparation: true } : {}) });
  }
  const blocked = pending || imageDraft?.busy || Boolean(draftSession?.persistence_error) || project.archived || !configuration || !directory || firstSendQueued || (value.preparing && !initialPreparation.current) || (!initialPreparation.current && ['task_unknown', 'open_unknown', 'send_unknown', 'sent'].includes(value.phase)) || !value.prompt.trim() && !inputReferences.references.length && !imageDraft?.images.length;
  const imageDisabled = !visible || !surfaceActive || value.preparing || project.archived || ['task_unknown', 'open_unknown', 'send_unknown', 'sent'].includes(value.phase);
  return <div className="agent-conversation-product" data-testid="dev-project-conversation">
    <div className="agent-dialogue-transcript"><h2>{t('directConversation.title')}</h2><p>{t('directConversation.hint')}</p>
      {draftSession ? <header className="agent-dialogue-header"><span className="agent-dialogue-status" data-state={draftSession.status}>{t(`agent.state.${draftSession.status}`)}</span><div className="agent-dialogue-actions"><Button size="xs" variant="secondary" disabled={interruptPending || !draftSession.active_run || !['running', 'waiting'].includes(draftSession.status)} onClick={() => { if (current()) void useAgentRunner.getState().action('interrupt_agent_session', { sessionId: draftSession.id, generation: draftSession.generation, runId: draftSession.active_run }); }}>{t('agent.interrupt')}</Button><Button size="xs" variant="secondary" disabled={draftSession.status === 'closed'} onClick={() => { if (current()) void useAgentRunner.getState().action('close_agent_session', { sessionId: draftSession.id }); }}>{t('agent.close')}</Button></div></header> : null}
    </div>
    <footer className="agent-dialogue-footer">
      {value.preparing ? <p role="status">{t(firstSendQueued ? 'desktop.queuedFirst' : 'desktop.preparing')}</p> : null}
      {interruptPending ? <p role="status">{t('agent.interruptPending')}</p> : null}
      {project.archived ? <p role="alert" className="text-status-error">{t('dev.error.project_archived')}</p> : null}
      {draftSession?.persistence_error ? <p role="alert" className="text-status-error">{t('agent.storageFailed')}</p> : null}
      {(value.error || directoryError) && (value.error ?? directoryError) !== trust.error ? <p role="alert" className="text-status-error">{errorText(value.error ?? directoryError!)}</p> : null}
      {trust.error ? <p role="alert" className="text-status-error">{errorText(trust.error)} <Button size="xs" variant="ghost" onClick={() => void trust.refresh()}>{t('review.refresh')}</Button></p> : null}
      {['task_unknown', 'open_unknown', 'send_unknown', 'sent'].includes(value.phase) && !value.preparing && !value.recoveryStopped ? <p role="status">{t('directConversation.recovering')}</p> : null}
      {(value.error || directoryError) && !value.preparing && (value.phase === 'draft' || value.phase === 'ready' || value.recoveryStopped) ? <Button size="xs" variant="ghost" onClick={() => { if (directoryError) setReload(n => n + 1); retry(); }}>{t('directConversation.retry')}</Button> : null}
      <AgentImageDropZone owner={imageOwner} enabled={configuration?.engine === 'codex' && !imageDisabled && current()}><div className="agent-composer-shell">{inputReferences.chips}{configuration?.engine === 'codex' ? <AgentImages owner={imageOwner} disabled={imageDisabled} active={visible && surfaceActive} /> : null}<div className="agent-composer-input"><Input multiline rows={2} id="agent-message" ref={composerInput} label={t('agent.prompt')} placeholder={t('agentPreferences.placeholder')} className="type-reading" aria-describedby="agent-send-hint" value={value.prompt} onPaste={agentImagePaste(imageOwner, imageDisabled || configuration?.engine !== 'codex')} onChange={event => { onComposerInput?.(); update({ prompt: inputReferences.promptChange(event.target.value, value.prompt, event.target.selectionStart, (event.nativeEvent as InputEvent).isComposing), configuration }); }} onKeyDown={event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) { event.preventDefault(); if (!event.repeat) void send(); } }} /></div>
        <AgentSlashPicker session={readySession} skillDraftKey={developmentDraftKey('conversation', project.id, draftId ?? project.id, 'skills')} engine={configuration?.engine} projectId={project.id} value={value.prompt} onChange={prompt => update({ prompt })} inputRef={composerInput} active={visible && surfaceActive} disabled={value.preparing} />
        <AgentComposerControls session={readySession} projectId={project.id} engine={configuration?.engine} draftId={draftId} visible={visible} disabled={value.preparing} referencePicker={inputReferences.picker} model={visible && surfaceActive ? <ComposerModelPicker projectId={project.id} value={configuration} preferred={saved.value?.configuration ?? null} onOpenChange={setModelPickerOpen} disabled={Boolean(value.sessionId && value.phase !== 'draft' && !initialPreparation.current)} onChange={next => { if (current()) { if (next.engine !== 'codex' && captureAgentImages(imageOwner).length) { update({ error: 'agent_images_unsupported' }); return; } if (next.engine !== 'codex' && useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', project.id, draftId ?? project.id, 'skills')]) { update({ error: 'agent_skills_unsupported' }); return; } queuedSend.current = null; setFirstSendQueued(false); autoAttempt.current = ''; update({ configuration: next, error: null, sendIntent: false, resumePreparation: false, restartAttempts: 0, recoveryAttempts: 0, recoveryStopped: false }); } }} /> : null} more={<Button size="xs" variant="ghost" disabled={value.preparing} onClick={() => { if (current()) { capabilitiesChanged(); setReload(n => n + 1); } }}>{t('capabilities.reload')}</Button>} sendHint={t('agent.sendShortcut')} send={<Button iconOnly aria-label={t('agent.send')} title={t('agent.sendShortcut')} data-testid="direct-conversation-send" disabled={Boolean(blocked) || !trusted} onClick={() => void send()}><ArrowUp size={18} aria-hidden="true" /></Button>} />
        {trust.value?.agent_revoked && !trust.loading ? <div className="agent-composer-trust"><span>{t('desktop.trust.disabled')}</span><Button size="xs" variant="secondary" data-testid="direct-conversation-authorize" disabled={!directory || value.preparing} onClick={() => void trust.grant('agent')}>{t('desktop.trust.start')}</Button></div> : null}
      </div></AgentImageDropZone>
    </footer>
  </div>;
}
