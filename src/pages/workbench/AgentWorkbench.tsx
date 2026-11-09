import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { FolderOpen, Files, GitBranch, ListTodo, X } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentOpenInput, type AgentSession } from '../../stores/agentRunnerStore';
import { useDevelopment, type ConversationTab, type DevelopmentSession, type Outcome } from '../../stores/developmentStore';
import { chooseContext, contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceActive, useSurfaceCloseGuard, useSurfaceStatus } from '../../workspace/SurfaceContext';
import { confirmDiscard, prepareAgentClose } from '../../workspace/closeCoordinator';
import { errorText } from '../../components/development/shared';
import { ProjectPanel } from '../../components/development/ProjectPanel';
import { TaskPanel } from '../../components/development/TaskPanel';
import { SessionPanel } from '../../components/development/SessionPanel';
import { UnclassifiedPanel } from '../../components/development/UnclassifiedPanel';
import { ProjectReview, Files as ProjectFiles, ChangeDiff, Runs, type FileTarget } from '../../components/development/ProjectReview';
import { AgentConversation, AgentSessionHistory } from './AgentConversation';
import { useDevelopmentEditor, hasDevelopmentDraft, discardDevelopmentDraft } from '../../stores/developmentEditor';
import { useDevelopmentRuntime, processActive } from '../../stores/developmentRuntime';
import '../../components/development/development.css';
import { useProjectCapabilities, type CliProviderRevision } from '../../stores/developmentCapabilities';
import './workbench.css';
import { readDevelopmentWorkspaces, readDevelopmentGit } from '../../stores/developmentDelivery';
import { developmentDraftKey, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { PanelResizer, useWorkbenchLayout } from '../../components/development/WorkbenchLayout';
import { DevelopmentRuntime } from '../../components/development/DevelopmentRuntime';
import '../../components/development/workbench-layout.css';
import { ProjectConversation, emptyConversationDraft, type ConversationDraft } from './ProjectConversation';
import { CliCleanupRecheck } from '../../components/development/CliCleanupRecheck';
import { resolveDevelopmentFileReference, type AgentOutputReference } from '../../components/development/eventOutputReferences';
import { RuntimeNavigation } from '../../components/chat/RuntimeNavigation';
import { ConversationTabs } from '../../components/development/ConversationTabs';
import { ConversationHandoff } from '../../components/development/ConversationHandoff';
import { useAccount } from '../../accounts/AccountContext';
import { FileEditor } from '../../components/development/FileEditor';
import { DeliveryPanel } from '../../components/development/DeliveryPanel';
import { addConversationReference, captureConversationReferences, clearConversationReferences, type ConversationReference } from '../../stores/developmentReferences';

const RESTING = ['idle', 'closed', 'error'];

export default function AgentWorkbench() {
  const { t } = useLocale();
  const development = useDevelopment();
  const runner = useAgentRunner();
  const accountName = useAccount()?.state.current?.username;
  const context = useContextSelection();
  const surfaceActive = useSurfaceActive();
  const fileEditor = useDevelopmentEditor();
  const hostRuntime = useDevelopmentRuntime();
  const editorDrafts = useDevelopmentDrafts(state => state.entries);
  const [trusted, setTrusted] = useState(false);
  const activeTab = development.tabs.find(tab => tab.id === development.activeTabId) ?? null;
  const drafts = Object.fromEntries(development.sessions.map(session => [session.id, editorDrafts[developmentDraftKey('conversation', session.project_id, session.id, 'message')]?.value as string ?? '']));
  const [handoffSource, setHandoffSource] = useState<string | null>(null);
  const [taskWorkspace, setTaskWorkspace] = useState<{taskId:string;path:string;kind:string;branch:string|null;head:string|null}|null>(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [workspaceRefresh, setWorkspaceRefresh] = useState(0);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const connection = useRef<{ epoch: number; generation: number } | null>(null);
  const { root, layout, setLayout, compact } = useWorkbenchLayout();
  const viewOwner = JSON.stringify([development.epoch, development.selection.project_id, development.selection.task_id, development.selection.session_id]);
  const [mainPosition, setMainPosition] = useState<{ owner: string; view: 'conversation' | 'file' | 'runtime' }>({ owner: viewOwner, view: 'conversation' });
  const mainView = mainPosition.owner === viewOwner ? mainPosition.view : 'conversation';
  function setMainView(view: 'conversation' | 'file' | 'runtime') { setMainPosition({ owner: viewOwner, view }); }
  const [ownedFileTarget, setOwnedFileTarget] = useState<{ owner: string; target: FileTarget } | null>(null);
  const fileTarget = ownedFileTarget?.owner === viewOwner ? ownedFileTarget.target : null;
  function setFileTarget(target: FileTarget | null) { setOwnedFileTarget(target ? { owner: viewOwner, target } : null); }
  const [diffSource, setDiffSource] = useState<'task' | 'index' | 'worktree'>('task');
  const [runTarget, setRunTarget] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const navigationTrigger = useRef<HTMLButtonElement>(null);
  const navigation = useRef<HTMLElement>(null);
  const agentPanel = useRef<HTMLDivElement>(null);
  const navigationOpen = layout.drawer !== null && (!compact || drawerOpen);
  const tools = [
    { id: 'projects', label: 'workbenchNavigation.projectsAndSessions', icon: FolderOpen },
    { id: 'files', label: 'workbenchNavigation.files', icon: Files },
    { id: 'git', label: 'workbenchNavigation.git', icon: GitBranch },
    { id: 'tasks', label: 'workbenchNavigation.tasks', icon: ListTodo },
  ] as const;
  useLayoutEffect(() => {
    if (!surfaceActive || !root.current || root.current.closest('[hidden], [inert]')) return;
    if (!navigationOpen && navigation.current?.contains(document.activeElement)) navigationTrigger.current?.focus();
  }, [navigationOpen, surfaceActive, root]);
  function openDrawer(drawer: NonNullable<typeof layout.drawer>) {
    setLayout(value => ({ ...value, drawer }));
    setDrawerOpen(true);
  }
  function toggleDrawer(drawer: NonNullable<typeof layout.drawer>, trigger: HTMLButtonElement) {
    navigationTrigger.current = trigger;
    if (navigationOpen && layout.drawer === drawer) closeNavigation();
    else {
      setLayout(value => ({ ...value, drawer, codeView: drawer === 'files' ? 'files' : drawer === 'git' ? 'changes' : drawer === 'tasks' ? 'review' : 'capabilities' }));
      setDrawerOpen(true);
    }
  }
  function revealAgent(approval = false) {
    setMainView('conversation');
    if (compact) setDrawerOpen(false);
    window.requestAnimationFrame(() => {
      const target = approval ? agentPanel.current?.querySelector<HTMLElement>('.agent-dialogue-requests section') : agentPanel.current?.querySelector<HTMLElement>('textarea');
      if (target) { target.tabIndex = target.tabIndex < 0 ? -1 : target.tabIndex; target.focus(); target.scrollIntoView?.({ block: 'nearest' }); }
    });
  }
  const wasDrawerOpen = useRef(false);
  function closeNavigation() { setDrawerOpen(false); if (!compact) setLayout(value => ({ ...value, drawer: null })); navigationTrigger.current?.focus(); }
  useEffect(() => {
    if (compact && navigationOpen) navigation.current?.querySelector<HTMLElement>('button, input, summary')?.focus();
    else if (wasDrawerOpen.current && (navigation.current?.contains(document.activeElement) || document.activeElement === document.body)) navigationTrigger.current?.focus();
    wasDrawerOpen.current = compact && navigationOpen;
  }, [compact, navigationOpen]);
  const project = development.projects.find(item => item.id === development.selection.project_id) ?? null;
  const task = development.tasks.find(item => item.id === development.selection.task_id) ?? null;
  const sessions = task ? development.sessions.filter(item => item.task_id === task.id) : [];
  const unclassified = development.sessions.filter(item => item.task_id === null);
  const selectedSaved = development.sessions.find(item => item.id === development.selection.session_id) ?? null;
  const projectPath = project?.path ?? null;
  const projectCapabilities = useProjectCapabilities(project?.id ?? null);
  function showCode(codeView: 'files' | 'changes' | 'review' | 'capabilities' | 'delivery') {
    openDrawer(codeView === 'files' ? 'files' : codeView === 'capabilities' ? 'projects' : codeView === 'review' ? 'tasks' : 'git');
    setLayout(value => ({ ...value, codeView }));
  }
  function showRuntime() { setMainView('runtime'); if (compact) setDrawerOpen(false); setLayout(value => ({ ...value, runtimeView: 'processes' })); }
  function manageProjects() { openDrawer('projects'); window.requestAnimationFrame(() => navigation.current?.querySelector<HTMLElement>('[data-testid="dev-project-create-open"]')?.focus()); }
  async function openFile(target: FileTarget, source: 'task' | 'index' | 'worktree' = 'task') {
    const owner = viewOwner, epoch = development.epoch, generation = contextGeneration();
    if (context.pending || !project) return;
    if (fileTarget?.path !== target.path || fileTarget.mode !== target.mode) {
      if (!await discardDevelopmentDraft()) return;
    }
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    setOwnedFileTarget({ owner, target }); setDiffSource(source); setMainPosition({ owner, view: 'file' });
    if (compact) setDrawerOpen(false);
  }
  function addReference(reference: ConversationReference) {
    if (!activeTab || !project || context.pending || reference.project_id !== project.id || reference.task_id !== (task?.id ?? null)) return;
    if (useDevelopment.getState().activeTabId !== activeTab.id || useDevelopment.getState().epoch !== development.epoch) return;
    addConversationReference(project.id, activeTab.sessionId ?? activeTab.id, reference);
    revealAgent();
  }
  const preparingWorkspace = useRef(false);
  const workspaceDraftKey = developmentDraftKey('task', project?.id ?? '', activeTab?.id ?? '', 'workspace-preparation');
  const workspacePreparation = editorDrafts[workspaceDraftKey]?.value as { phase: string; taskId?: string } | undefined;
  async function prepareWorkspace() {
    if (!project || !activeTab || task || context.pending || preparingWorkspace.current || workspacePreparation?.phase === 'unknown') return;
    const epoch = development.epoch, generation = contextGeneration(), tabId = activeTab.id;
    preparingWorkspace.current = true; setBusy(true);
    const setPreparation = (phase: string, taskId?: string) => useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [workspaceDraftKey]: { kind: 'task', value: { phase, taskId }, baseline: {} } } }));
    try {
      setPreparation('pending');
      const result = workspacePreparation?.taskId ? { ok: true as const, value: { id: workspacePreparation.taskId } } : await development.saveTask({ id: null, project_id: project.id, goal: t('workbenchFiles.projectWork', { name: project.name }), requirements: '', status: 'todo' });
      if (epoch !== useDevelopment.getState().epoch) return;
      if (!result.ok) { setPreparation(result.code === 'unknown' ? 'unknown' : 'failed'); setFailure(result.code); return; }
      setPreparation('ready', result.value.id);
      if (generation !== contextGeneration() || useDevelopment.getState().activeTabId !== tabId) return;
      const selecting = chooseContext({ project_id: project.id, task_id: result.value.id, session_id: null });
      useDevelopment.setState(state => ({ tabs: state.tabs.map(tab => tab.id === tabId ? { ...tab, taskId: result.value.id } : tab) }));
      const selected = await selecting;
      if (epoch !== useDevelopment.getState().epoch) return;
      if (!selected.ok) { useDevelopment.setState(state => ({ tabs: state.tabs.map(tab => tab.id === tabId ? { ...tab, taskId: null } : tab) })); setFailure(selected.code); }
    } finally { preparingWorkspace.current = false; if (epoch === useDevelopment.getState().epoch) setBusy(false); }
  }
  function setSessionDraft(id: string, value: string, expected?: string) {
    if (useDevelopment.getState().epoch !== development.epoch) return;
    const projectId = useDevelopment.getState().sessions.find(session => session.id === id)?.project_id ?? project?.id;
    if (!projectId) return;
    const key = developmentDraftKey('conversation', projectId, id, 'message');
    useDevelopmentDrafts.setState(state => {
      if (expected !== undefined && state.entries[key]?.value !== expected) return state;
      const entries = { ...state.entries };
      if (value) entries[key] = { kind: 'conversation', value, baseline: '' }; else delete entries[key];
      return { entries };
    });
  }
  async function newConversation(configuration?: CliProviderRevision['configuration']) {
    if (!project || context.pending) return;
    const epoch = development.epoch;
    const result = await chooseContext({ project_id: project.id, task_id: null, session_id: null });
    if (!result.ok || epoch !== useDevelopment.getState().epoch) return;
    const tab = useDevelopment.getState().newTab(project.id);
    if (configuration) {
      const key = developmentDraftKey('conversation', project.id, tab.id);
      useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { ...emptyConversationDraft, configuration }, baseline: emptyConversationDraft } } }));
    }
    revealAgent();
  }
  async function chooseTabContext(tab: ConversationTab | undefined): Promise<Outcome<null>> {
    const selection = { project_id: tab?.projectId ?? null, task_id: tab?.taskId ?? null, session_id: tab?.sessionId ?? null };
    const unavailable = tab && (!development.projects.some(item => item.id === tab.projectId) || tab.sessionId && !development.sessions.some(item => item.id === tab.sessionId));
    if (!unavailable) return chooseContext(selection);
    const epoch = development.epoch;
    const result = await chooseContext({ project_id: null, task_id: null, session_id: null });
    if (result.ok && epoch === useDevelopment.getState().epoch) useDevelopment.setState({ selection });
    return result;
  }
  async function selectTab(tab: ConversationTab) {
    if (context.pending || tab.id === development.activeTabId && development.selection.session_id === tab.sessionId && development.selection.project_id === tab.projectId) return;
    const epoch = development.epoch;
    const result = await chooseTabContext(tab);
    if (epoch !== useDevelopment.getState().epoch) return;
    if (!result.ok) { setFailure(result.code); return; }
    useDevelopment.getState().activateTab(tab.id);
    if (tab.sessionId && useAgentRunner.getState().live[tab.sessionId]) await useAgentRunner.getState().attach(tab.sessionId);
  }
  async function closeTab(tab: ConversationTab) {
    const epoch = development.epoch;
    if (tab.sessionId && Object.entries(useDevelopmentDrafts.getState().entries).some(([key, entry]) => {
      const scope = JSON.parse(key);
      return entry.kind === 'conversation' && scope[0] === epoch && scope[2] === tab.projectId && scope[3] === tab.sessionId && scope[5] === 'pending-send';
    })) { setFailure('unknown'); return; }
    const key = tab.sessionId ? developmentDraftKey('conversation', tab.projectId, tab.sessionId, 'message') : developmentDraftKey('conversation', tab.projectId, tab.id);
    const entry = useDevelopmentDrafts.getState().entries[key];
    const intent = !tab.sessionId ? entry?.value as ConversationDraft | undefined : undefined;
    if (intent?.preparing || intent && ['task_unknown', 'open_unknown', 'send_unknown'].includes(intent.phase)) { setFailure('session_busy'); return; }
    const references = captureConversationReferences(tab.projectId, tab.sessionId ?? tab.id);
    const unsent = references.length > 0 || (typeof entry?.value === 'string' ? Boolean(entry.value.trim()) : Boolean(intent?.prompt.trim()));
    if (unsent && !await confirmDiscard() || epoch !== useDevelopment.getState().epoch || entry !== useDevelopmentDrafts.getState().entries[key]) return;
    if (JSON.stringify(references) !== JSON.stringify(captureConversationReferences(tab.projectId, tab.sessionId ?? tab.id))) return;
    if (tab.id === useDevelopment.getState().activeTabId) {
      const remaining = useDevelopment.getState().tabs.filter(item => item.id !== tab.id);
      const next = remaining[Math.min(development.tabs.findIndex(item => item.id === tab.id), remaining.length - 1)];
      const result = await chooseTabContext(next);
      if (!result.ok || epoch !== useDevelopment.getState().epoch) return;
      useDevelopment.getState().closeTab(tab.id);
      if (next?.sessionId && useAgentRunner.getState().live[next.sessionId]) await useAgentRunner.getState().attach(next.sessionId);
    } else useDevelopment.getState().closeTab(tab.id);
    if (epoch !== useDevelopment.getState().epoch) return;
    useDevelopmentDrafts.setState(state => { const entries = { ...state.entries }; delete entries[key]; return { entries }; });
    clearConversationReferences(tab.projectId, tab.sessionId ?? tab.id, references);
    if (tab.id === activeTab?.id && surfaceActive) window.requestAnimationFrame(() => { const id = useDevelopment.getState().activeTabId; if (id) document.getElementById(`conversation-tab-${id}`)?.focus({ preventScroll: true }); });
    // Handoff reservation identities remain recoverable from history even when their work surface closes.
  }
  // External project/task selection and history selection join the same tab owner.
  useEffect(() => {
    if (!development.loaded || context.pending) return;
    const selection = development.selection;
    if (!selection.project_id) return;
    if (activeTab?.projectId === selection.project_id && activeTab.taskId === selection.task_id && activeTab.sessionId === selection.session_id) return;
    if (selection.session_id) {
      const saved = development.sessions.find(item => item.id === selection.session_id);
      if (saved) development.openTab(saved);
    } else {
      const blank = development.tabs.find(tab => tab.projectId === selection.project_id && tab.taskId === selection.task_id && !tab.sessionId);
      if (blank) development.activateTab(blank.id); else development.newTab(selection.project_id, selection.task_id);
    }
  }, [development.loaded, development.selection, context.pending, activeTab?.id]);
  function useConfiguration(revision: CliProviderRevision) { void newConversation(revision.configuration); }
  async function openReference(reference: AgentOutputReference, mode: 'file' | 'diff') {
    if (!project || !task || !runner.session || context.pending) throw Error('invalid_selection');
    const epoch = development.epoch, generation = contextGeneration(), sessionId = runner.session.id;
    const result = await resolveDevelopmentFileReference({ project_id: project.id, task_id: task.id, session_id: sessionId, path: reference.path, mode, line: reference.line });
    const valid = () => epoch === useDevelopment.getState().epoch && generation === contextGeneration() && useAgentRunner.getState().session?.id === sessionId;
    if (!valid()) return;
    if (!result.ok) { setFailure(result.code); throw Error(result.code); }
    await openFile({ path: result.value.path, line: result.value.line ?? undefined, mode, nonce: Date.now() });
  }
  // Trust is per directory: changing the project or its path requires a new explicit confirmation.
  useEffect(() => { setTrusted(false); }, [projectPath, task?.id, development.epoch]);
  useEffect(() => {
    setTaskWorkspace(null); setWorkspaceError(null);
    if (!project || !task || context.pending) return;
    let active = true; const epoch = development.epoch, generation = contextGeneration();
    void Promise.all([readDevelopmentWorkspaces({ project_id: project.id, task_id: task.id }), readDevelopmentGit(task.id)]).then(([result, git]) => {
      if (!active || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      if (!result.ok) { setWorkspaceError(result.code); return; }
      const selected = result.value.workspaces.find(item => result.value.selected_workspace_id ? item.id === result.value.selected_workspace_id : item.kind === 'original');
      if (!selected) { setWorkspaceError('workspace_not_found'); return; }
      setTaskWorkspace({ taskId: task.id, path: selected.path, kind: selected.kind, branch: git.ok ? git.value.branch : null, head: git.ok ? git.value.head : null });
      if (!git.ok) setWorkspaceError(git.code);
    });
    return () => { active = false; };
  }, [projectPath, task?.id, development.epoch, context.generation, context.pending, workspaceRefresh]);
  useEffect(() => { setBusy(false); setFailure(null); }, [development.epoch, context.generation]);
  useEffect(() => { setTrusted(false); setHandoffSource(null); }, [development.epoch, context.generation]);
  useEffect(() => { setFileTarget(null); setRunTarget(null); }, [development.epoch, context.generation]);
  useEffect(() => { if (task && !context.pending) void useDevelopmentRuntime.getState().refresh(task.id); }, [task?.id, development.epoch, context.generation, context.pending]);
  const liveActive = Object.values(runner.live).some(status => !RESTING.includes(status));
  const hostActive = hostRuntime.processes.some(processActive);
  // The running marker outlives the internal development tab and the surface's activity.
  useEffect(() => {
    if (!hostActive || !hostRuntime.taskId || context.pending) return;
    const taskId = hostRuntime.taskId;
    const timer = window.setInterval(() => { void useDevelopmentRuntime.getState().refresh(taskId); }, 1000);
    return () => window.clearInterval(timer);
  }, [hostActive, hostRuntime.taskId, development.epoch, context.generation, context.pending]);
  const dirty = Object.values(drafts).some(value => value.trim()) || Object.keys(editorDrafts).length > 0 || Object.values(hostRuntime.drafts).some(draft => draft.dirty) || (fileEditor.file !== null && hasDevelopmentDraft());
  const preparingConversation = Object.values(editorDrafts).some(entry => entry.kind === 'conversation' && (entry.value as ConversationDraft).preparing);
  const hostMarker = hostRuntime.read.status !== 'ready' || hostRuntime.read.refresh.status !== 'current' ? 'direct.marker.lastKnown' : hostRuntime.processes.some(process => process.status === 'unknown') ? 'direct.status.unknown' : hostRuntime.processes.some(process => process.status === 'cleanup_pending') ? 'direct.status.cleanup_pending' : hostRuntime.processes.some(process => process.status === 'starting') ? 'direct.status.starting' : 'shell.running';
  useSurfaceStatus({ dirty, running: preparingConversation || liveActive || runner.busy || runner.acting || hostActive, runningLabel: preparingConversation ? t('directConversation.preparing') : hostActive && !liveActive && !runner.busy && !runner.acting ? t(hostMarker) : undefined });
  useEffect(() => { void useDevelopment.getState().load(accountName); }, [accountName, development.epoch]);
  useEffect(() => {
    if (!runner.session || context.pending) return;
    const timer = window.setTimeout(() => { void useDevelopment.getState().load(); }, 500);
    return () => window.clearTimeout(timer);
  }, [runner.session?.id, runner.session?.status, context.pending]);
  useSurfaceCloseGuard(async () => {
    if (!await discardDevelopmentDraft()) return false;
    // Conversation/command drafts remain in their existing stores when the work surface is hidden.
    return prepareAgentClose();
  });

  async function connect<T>(operation: () => Promise<T>, cancelled: T): Promise<T> {
    const epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    if (development.epoch !== epoch || context.generation !== generation || context.pending || useAgentRunner.getState().busy || connection.current?.epoch === epoch && connection.current.generation === generation) return cancelled;
    const intent = { epoch, generation };
    connection.current = intent; setBusy(true); setFailure(null);
    try {
      if (!await discardDevelopmentDraft() || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return cancelled;
      return await operation();
    } finally {
      if (connection.current === intent) connection.current = null;
      if (epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setBusy(false);
    }
  }
  // Starts a new runtime or resumes a saved one. Each call is one explicit user action and never retries. Both the
  // development and runtime account epochs are captured first; a response that crosses an account change is dropped.
  async function startConnection(input: AgentOpenInput, projectId: string, taskId: string, sessionId: string | null, dispatch?: (session: AgentSession) => Promise<Outcome<null>>): Promise<Outcome<null>> {
    const devEpoch = useDevelopment.getState().epoch;
    const runEpoch = useAgentRunner.getState().epoch;
    const generation = contextGeneration();
    const sourceTabId = useDevelopment.getState().activeTabId;
    setBusy(true); setFailure(null);
    await runner.open(input);
    if (useDevelopment.getState().epoch !== devEpoch || useAgentRunner.getState().epoch !== runEpoch || contextGeneration() !== generation) return { ok: false, code: 'cancelled' };
    const opened = useAgentRunner.getState();
    if (opened.error || opened.session?.id !== ('id' in input ? input.id : input.session_id)) { const code = opened.error ?? 'unknown'; setFailure(code); setBusy(false); return { ok: false, code }; }
    if (dispatch) {
      const sent = await dispatch(opened.session);
      if (!sent.ok) { if (useDevelopment.getState().epoch === devEpoch && contextGeneration() === generation) { setFailure(sent.code); setBusy(false); } return sent; }
      if (useDevelopment.getState().epoch !== devEpoch || useAgentRunner.getState().epoch !== runEpoch || contextGeneration() !== generation) return { ok: false, code: 'cancelled' };
    }
    await useDevelopment.getState().load();
    if (useDevelopment.getState().epoch !== devEpoch || useAgentRunner.getState().epoch !== runEpoch || contextGeneration() !== generation) return { ok: false, code: 'cancelled' };
    const savedSession = useDevelopment.getState().sessions.find(item => item.id === opened.session!.id);
    if (sourceTabId && savedSession && dispatch) useDevelopment.getState().bindTab(sourceTabId, savedSession);
    const selecting = chooseContext({ project_id: projectId, task_id: taskId, session_id: sessionId ?? opened.session.id });
    const selectionGeneration = contextGeneration();
    // chooseContext publishes its settlement once; any further generation belongs to another selection intent.
    const selected = await selecting;
    if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== selectionGeneration + 1) return { ok: false, code: 'cancelled' };
    if (!selected.ok) setFailure(selected.code);
    setBusy(false);
    if (selected.ok) revealAgent();
    return selected;
  }
  async function selectSession(saved: DevelopmentSession) {
    if (context.pending) return;
    const epoch = development.epoch;
    const result = await chooseContext({ project_id: saved.project_id, task_id: saved.task_id, session_id: saved.id });
    if (epoch !== useDevelopment.getState().epoch) return;
    if (!result.ok) { setFailure(result.code); return; }
    useDevelopment.getState().openTab(saved);
    if (useAgentRunner.getState().live[saved.id]) await useAgentRunner.getState().attach(saved.id);
    if (compact) setDrawerOpen(false);
  }
  function inspectSession(sessionId: string) {
    return connect(async () => {
    const epoch = development.epoch, generation = contextGeneration();
    setBusy(true);
    await runner.attach(sessionId);
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    const attached = useAgentRunner.getState();
    if (attached.error || attached.session?.id !== sessionId) { setFailure(attached.error ?? 'unknown'); setBusy(false); return; }
    await useDevelopment.getState().load();
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    const saved = useDevelopment.getState().sessions.find(item => item.id === sessionId);
    if (saved?.task_id) {
      const blank = useDevelopment.getState().tabs.find(tab => tab.id === useDevelopment.getState().activeTabId && !tab.sessionId);
      const intent = blank ? useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', blank.projectId, blank.id)]?.value as ConversationDraft | undefined : undefined;
      if (blank && intent?.sessionId === sessionId) useDevelopment.getState().bindTab(blank.id, saved);
      const selected = await chooseContext({ project_id: saved.project_id, task_id: saved.task_id, session_id: sessionId });
      if (selected.ok && epoch === useDevelopment.getState().epoch && useDevelopment.getState().selection.session_id === sessionId) revealAgent();
    }
    setBusy(false);
    }, undefined);
  }
  function resume(session: DevelopmentSession, trust = trusted) {
    return connect(async () => {
    if (!project || !session.task_id) return;
    if (runner.live[session.id] && runner.live[session.id] !== 'closed') {
      const devEpoch = useDevelopment.getState().epoch;
      const generation = contextGeneration();
      setBusy(true);
      await runner.attach(session.id);
      if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== generation) return;
      const attached = useAgentRunner.getState();
      if (attached.error || attached.session?.id !== session.id) { setFailure(attached.error ?? 'unknown'); setBusy(false); return; }
      const selecting = chooseContext({ project_id: session.project_id, task_id: session.task_id, session_id: session.id });
      const selectionGeneration = contextGeneration();
      const selected = await selecting;
      if (useDevelopment.getState().epoch !== devEpoch || contextGeneration() !== selectionGeneration + 1) return;
      if (!selected.ok) setFailure(selected.code);
      setBusy(false);
      if (selected.ok) revealAgent();
      return;
    }
    await startConnection({ intent: 'resume', session_id: session.id, trust_workspace: trust }, session.project_id ?? project.id, session.task_id, session.id);
    }, undefined);
  }
  async function classify(session: DevelopmentSession) {
    if (!task) return;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    const result = await development.classify(session.id, task.id);
    if (epoch === useDevelopment.getState().epoch && generation === contextGeneration()) setFailure(result.ok ? null : result.code);
  }
  function acceptHandoff(sessionId: string) {
    return connect(async () => {
    if (!project || !task) return;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    setBusy(true);
    await useDevelopment.getState().load();
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    await useAgentRunner.getState().attach(sessionId);
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    const attached = useAgentRunner.getState();
    if (attached.error || attached.session?.id !== sessionId) { setFailure(attached.error ?? 'unknown'); setBusy(false); return; }
    const result = await chooseContext({ project_id: project.id, task_id: task.id, session_id: sessionId });
    if (epoch === useDevelopment.getState().epoch) { if (!result.ok) setFailure(result.code); setBusy(false); }
    }, undefined);
  }
  const current = context.pending || development.selection.session_id !== runner.session?.id || runner.session?.status === 'closed' ? null : runner.session;
  useEffect(() => { if (surfaceActive && mainView === 'conversation' && !(compact && navigationOpen) && !handoffSource && current && runner.unread[current.id]) runner.markSeen(current.id); }, [surfaceActive, mainView, compact, navigationOpen, handoffSource, current, runner.unread]);
  useEffect(() => {
    const id = development.selection.session_id;
    if (surfaceActive && !context.pending && id && runner.live[id] && runner.selected !== id && !runner.busy) void runner.attach(id);
  }, [surfaceActive, development.selection.session_id, context.pending]);
  const pendingCount = current && current.status !== 'closed' ? current.pending.length : 0;
  const failureCode = failure ?? development.error ?? runner.error;
  const processes = hostRuntime.taskId === task?.id ? hostRuntime.processes : [];
  const activeProcesses = processes.filter(processActive).length;
  const failedProcesses = processes.filter(item => item.status === 'failed').length;
  const unknownProcesses = processes.filter(item => ['unknown', 'cleanup_pending'].includes(item.status)).length;
  const unresolvedOperations = hostRuntime.operations.filter(item => item.taskId === task?.id && ['unknown', 'pending', 'confirming'].includes(item.phase)).length;
  const drawerTitle = tools.find(tool => tool.id === layout.drawer)?.label ?? 'workbenchNavigation.projectsAndSessions';
  const codeViews = layout.drawer === 'tasks' ? ['review'] as const : ['capabilities'] as const;
  const nativeRefreshKey = `${current?.id ?? ''}:${current?.events.reduce((seq, event) => event.kind === 'result' || event.kind === 'turn/completed' ? event.seq : seq, 0) ?? 0}`;
  return <section ref={root} className="agent-workbench dev-workspace" data-compact={compact} data-drawer-width={layout.drawerWidth}>
    <span className="sr-only" role="status" data-testid="agent-session-announcement">{surfaceActive && current ? `${t(`agent.engine.${current.engine}`)}: ${t(`agent.state.${current.status}`)}. ${t('dev.pendingApprovals', { count: current.pending.length })}` : ''}</span>
    <header className="dev-workspace-header" inert={compact && navigationOpen}>
      <RuntimeNavigation runtime="native" />
      <div className="dev-workspace-context"><h1>{project?.name ?? t('dev.title')}</h1></div>
      <details className="dev-context-details" data-testid="dev-context-details"><summary>{t('layout.contextDetails')}</summary><div className="dev-detail">
        <p>{t('review.cwd')}: {taskWorkspace?.path ?? t('layout.directoryUnresolved')}</p>
        <p>{t('dev.selected')}: {project?.id ?? '—'}</p><p>{t('dev.tasks')}: {task?.id ?? '—'}</p><p>{t('dev.sessions')}: {current?.id ?? selectedSaved?.id ?? '—'}</p><p>{t('layout.head')}: {taskWorkspace?.head ?? '—'}</p>
        <p>{taskWorkspace?.branch ?? t('dev.kindLabel.directory')} / {taskWorkspace?.kind ?? t('layout.directoryUnresolved')}</p>
        {workspaceError ? <p className="text-status-error-text">{t('layout.contextFailed')}: {errorText(workspaceError)}</p> : null}
        <Button size="xs" variant="secondary" onClick={() => setWorkspaceRefresh(value => value + 1)}>{t('dev.inspect')}</Button>
      </div></details>
      <Button size="sm" variant="secondary" data-testid="dev-new-conversation" disabled={!project || context.pending} onClick={() => void newConversation()}>{t('directConversation.new')}</Button>
    </header>
    {failureCode ? <p role="alert" className="dev-workspace-error text-sm text-status-error">{errorText(failureCode)}</p> : null}
    {failureCode === 'cleanup_pending' ? <CliCleanupRecheck /> : null}
    <div className="dev-workspace-body" data-drawer-open={navigationOpen}>
      <nav className="dev-tool-rail" aria-label={t('dev.projectTools')}>
        {tools.map(({ id, label, icon: Icon }) => <Button key={id} ref={node => { if (node && (layout.drawer === id || !navigationTrigger.current)) navigationTrigger.current = node; }} size="sm" variant="ghost" title={t(label)} aria-label={t(label)} aria-expanded={navigationOpen && layout.drawer === id} aria-controls="dev-navigation" data-active={navigationOpen && layout.drawer === id} data-testid={`dev-tool-${id}`} onClick={event => toggleDrawer(id, event.currentTarget)}><Icon size={20} aria-hidden="true" /></Button>)}
      </nav>
      <aside ref={navigation} id="dev-navigation" className="dev-navigation" aria-label={t(drawerTitle)} hidden={!navigationOpen} onKeyDown={event => {
        if (event.key === 'Escape' && !event.defaultPrevented) { event.preventDefault(); closeNavigation(); }
      }}>
        <header className="dev-drawer-header"><h2>{t(drawerTitle)}</h2><Button size="xs" variant="ghost" aria-label={t('common.close')} onClick={closeNavigation}><X size={16} aria-hidden="true" /></Button></header>
        <div className="dev-drawer-content">
          <div hidden={layout.drawer !== 'projects'}>
            <ProjectPanel />
            <SessionPanel busy={context.pending} onSelect={selectSession} />
            <UnclassifiedPanel sessions={unclassified} taskId={task?.id ?? null} busy={context.pending || busy || runner.busy} onClassify={saved => void classify(saved)} />
            {project ? <Button size="xs" variant="secondary" onClick={() => showCode('capabilities')}>{t('capabilities.title')}</Button> : null}
          </div>
          <div hidden={layout.drawer !== 'tasks'}>{project ? <TaskPanel project={project} onContinue={id => { const saved = development.sessions.find(session => session.id === id); if (saved) void selectSession(saved); }} /> : <p>{t('dev.none')}</p>}
            {task ? <details className="dev-task-brief" data-testid="dev-task-summary"><summary>{t('dev.taskFor', { goal: task.goal })}<span>{t(`dev.taskStatus.${task.status}`)}</span></summary><p>{task.requirements}</p></details> : null}
          </div>
          <div hidden={layout.drawer !== 'files'}>{project && !context.pending ? <ProjectFiles key={`${development.epoch}:${context.generation}:${project.id}:${task?.id ?? ''}`} projectId={project.id} taskId={task?.id ?? null} target={fileTarget} onOpenFile={target => void openFile(target)} onReference={activeTab ? addReference : undefined} /> : <p>{t('dev.none')}</p>}</div>
          <div hidden={layout.drawer !== 'git'}>{project && task && !context.pending ? <DeliveryPanel key={`${development.epoch}:${project.id}:${task.id}`} mode="git" projectId={project.id} taskId={task.id} sessions={sessions} live={runner.live} capabilityRevision={projectCapabilities.value?.revision ?? 0} workspacePath={taskWorkspace?.path ?? ''} refreshKey={nativeRefreshKey} onOpenDiff={path => void openFile({ path, mode: 'diff', nonce: Date.now() }, 'worktree')} /> : <p>{t('dev.noTaskSelected')}</p>}</div>
          <div id="dev-panel-code" data-testid="dev-panel-code" className="dev-tools-pane" hidden={!['projects', 'tasks'].includes(layout.drawer ?? '') || !codeViews.some(view => view === layout.codeView)}>
            {project && !context.pending ? <ProjectReview key={`${development.epoch}:${context.generation}:${project.id}:${task?.id ?? ''}`} projectId={project.id} taskId={task?.id ?? null} sessions={sessions} live={runner.live} capabilityRevision={projectCapabilities.value?.revision ?? 0} view={codeViews[0]} views={codeViews} onViewChange={showCode} onOpenFile={target => void openFile(target)} snapshot={current?.configuration?.format === 'provider_bound' ? current.configuration.snapshot : null} onUseConfiguration={useConfiguration} onOpenRuntime={showRuntime} workspacePath={taskWorkspace?.path ?? ''} onWorkspaceChange={() => { setWorkspaceRefresh(value => value + 1); setTrusted(false); }} onHandoff={sessionId => { revealAgent(); void acceptHandoff(sessionId); }} /> : <p>{t('dev.none')}</p>}
          </div>
        </div>
        <PanelResizer value={layout.drawerWidth} onChange={drawerWidth => setLayout(value => ({ ...value, drawerWidth }))} label={t('workbenchNavigation.resizeDrawer')} testId="dev-drawer-resizer" controls="dev-navigation" />
      </aside>
      <main className="dev-workspace-main" inert={compact && navigationOpen}>
        <ConversationTabs busy={context.pending} onSelect={tab => void selectTab(tab)} onClose={tab => void closeTab(tab)} />
        <nav className="dev-main-view-nav" aria-label={t('workbenchFiles.views')}>
          <Button size="xs" variant={mainView === 'conversation' ? 'secondary' : 'ghost'} aria-pressed={mainView === 'conversation'} onClick={() => revealAgent()}>{t('workbenchFiles.conversation')}</Button>
          {fileTarget ? <Button size="xs" variant={mainView === 'file' ? 'secondary' : 'ghost'} aria-pressed={mainView === 'file'} onClick={() => setMainView('file')}>{fileTarget.path}{hasDevelopmentDraft() ? ' *' : ''}</Button> : null}
          <Button size="xs" variant={mainView === 'runtime' ? 'secondary' : 'ghost'} disabled={!project || context.pending} aria-pressed={mainView === 'runtime'} onClick={showRuntime}>{t('workbenchFiles.runtime')}</Button>
        </nav>
        {project && activeTab && !task ? <div className="dev-workspace-preparation"><Button size="xs" variant="secondary" disabled={busy || context.pending || project.archived || workspacePreparation?.phase === 'unknown'} onClick={() => void prepareWorkspace()}>{t('workbenchFiles.prepare')}</Button><p className="text-xs">{t(workspacePreparation?.phase === 'unknown' ? 'workbenchFiles.prepareUnknown' : 'workbenchFiles.prepareHint')}</p></div> : null}
        {pendingCount ? <div className="dev-approval-navigation"><Button size="xs" variant="secondary" data-testid="dev-pending-approvals" onClick={() => revealAgent(true)}>{t('dev.pendingApprovals', { count: pendingCount })}</Button></div> : null}
        <div ref={agentPanel} id="dev-panel-agent" data-testid="dev-panel-agent" className="dev-conversation-pane" hidden={mainView !== 'conversation'} role="tabpanel" aria-labelledby={activeTab ? `conversation-tab-${activeTab.id}` : undefined}>
          {activeTab && (!project || activeTab.sessionId && !selectedSaved) ? <div className="dev-workspace-empty"><p role="alert">{t('workbenchNavigation.unavailable')}</p><Button size="sm" onClick={manageProjects}>{t('dev.projects')}</Button><Button size="sm" onClick={() => void closeTab(activeTab)}>{t('workbenchNavigation.closeTab')}</Button></div>
            : handoffSource && project && selectedSaved?.id === handoffSource ? <ConversationHandoff key={`${development.epoch}:${handoffSource}`} project={project} source={selectedSaved} onCancel={() => setHandoffSource(null)} onReady={async id => { setHandoffSource(null); await acceptHandoff(id); }} />
            : current ? <AgentConversation key={`${development.epoch}:${current.id}:${current.generation}`} visible={mainView === 'conversation' && surfaceActive} session={current} drafts={drafts} setDraft={setSessionDraft} busy={busy || runner.busy || Boolean(project?.archived)} error={Boolean(runner.error)} onNewConversation={newConversation} onContinueModel={() => setHandoffSource(current.id)} onOpenReference={openReference} onSelectRun={runId => { setRunTarget(runId); setMainView('runtime'); if (compact) setDrawerOpen(false); setLayout(value => ({ ...value, runtimeView: 'runs' })); }} />
            : !context.pending && selectedSaved?.task_id ? <AgentSessionHistory key={`${development.epoch}:${selectedSaved.id}`} visible={mainView === 'conversation' && surfaceActive} sessionId={selectedSaved.id} taskId={selectedSaved.task_id}>
              <div data-testid="dev-resume-entry"><p>{t('dev.resumeNotStarted')}</p>
              {project?.archived ? <p role="alert">{t('dev.error.project_archived')}</p> : null}
              {workspaceError ? <div><p role="alert">{errorText(workspaceError)}</p><Button size="xs" variant="secondary" onClick={manageProjects}>{t('dev.projects')}</Button><Button size="xs" variant="ghost" onClick={() => setWorkspaceRefresh(value => value + 1)}>{t('dev.inspect')}</Button></div> : null}
              <label className="dev-checkbox"><input type="checkbox" checked={trusted} onChange={event => setTrusted(event.target.checked)} />{t('dev.trustProject')}</label>
              <Button size="sm" variant="secondary" disabled={busy || runner.busy || !trusted || !taskWorkspace || Boolean(workspaceError) || project?.archived} onClick={() => resume(selectedSaved)}>{t('dev.trustResume')}</Button>
              <Button size="sm" variant="ghost" disabled={Boolean(project?.archived)} onClick={() => setHandoffSource(selectedSaved.id)}>{t('workbenchNavigation.continueModel')}</Button></div></AgentSessionHistory>
            : project && activeTab && !context.pending ? <ProjectConversation key={`${development.epoch}:${activeTab.id}`} visible={mainView === 'conversation' && surfaceActive} draftId={activeTab.id} project={project} taskId={task?.id ?? null} setSessionDraft={setSessionDraft} onInspect={inspectSession} onStart={(input, projectId, taskId, dispatch) => connect<Outcome<null>>(() => startConnection(input, projectId, taskId, null, dispatch), { ok: false, code: 'cancelled' })} />
            : <div className="dev-workspace-empty"><h2>{t('dev.welcome')}</h2><p>{t('workbenchNavigation.noTabs')}</p><Button size="sm" variant="secondary" onClick={manageProjects}>{t('dev.projects')}</Button></div>}
        </div>
        <section className="dev-main-tool-surface" hidden={mainView !== 'file'} aria-label={t('workbenchFiles.fileView')}>
          {fileTarget && project && !context.pending ? <><header className="dev-main-tool-header"><strong>{project.name}</strong><Button size="xs" variant="ghost" onClick={() => revealAgent()}>{t('workbenchFiles.back')}</Button><Button size="xs" variant="ghost" onClick={async () => { const generation = contextGeneration(); if (await discardDevelopmentDraft() && generation === contextGeneration()) { setFileTarget(null); revealAgent(); } }}>{t('workbenchFiles.closeFile')}</Button></header>
            {fileTarget.mode === 'diff' && task ? <><div className="dev-main-view-nav" role="group" aria-label={t('workbenchFiles.diffSource')}>{(['worktree', 'index', 'task'] as const).map(source => <Button key={source} size="xs" variant={diffSource === source ? 'secondary' : 'ghost'} aria-pressed={diffSource === source} onClick={() => setDiffSource(source)}>{t(`workbenchFiles.${source}Diff`)}</Button>)}<Button size="xs" variant="ghost" onClick={() => void openFile({ ...fileTarget, mode: 'file', nonce: Date.now() })}>{t('workbenchFiles.openFile')}</Button></div><ChangeDiff key={`${viewOwner}:${fileTarget.path}:${diffSource}`} projectId={project.id} taskId={task.id} path={fileTarget.path} source={diffSource} onReference={activeTab ? addReference : undefined} /></> : <FileEditor key={`${viewOwner}:${fileTarget.path}`} projectId={project.id} taskId={task?.id ?? null} path={fileTarget.path} line={fileTarget.line} visible={mainView === 'file' && surfaceActive} onReference={activeTab ? addReference : undefined} />}
          </> : null}
        </section>
        <section id="dev-panel-runtime" data-testid="dev-panel-runtime" className="dev-main-tool-surface" hidden={mainView !== 'runtime'} aria-label={t('workbenchFiles.runtime')}>
          <header className="dev-main-tool-header"><strong>{project?.name}</strong><Button size="xs" variant="ghost" onClick={() => revealAgent()}>{t('workbenchFiles.hideRuntime')}</Button></header>
          <p className="text-xs text-ink-secondary">{t('layout.runtimeCounts', { active: activeProcesses, failed: failedProcesses, unknown: unknownProcesses + unresolvedOperations })}</p>
          {hostRuntime.read.status === 'error' || hostRuntime.read.status === 'ready' && hostRuntime.read.refresh.status !== 'current' ? <p className="text-xs text-status-warning-text">{t('direct.lastKnownHistory')}</p> : null}
          <div className="dev-runtime-tabs" role="group" aria-label={t('layout.runtime')}><Button size="xs" data-testid="dev-runtime-processes" variant={layout.runtimeView === 'processes' ? 'secondary' : 'ghost'} aria-pressed={layout.runtimeView === 'processes'} onClick={() => setLayout(value => ({ ...value, runtimeView: 'processes' }))}>{t('direct.title')}</Button><Button size="xs" data-testid="dev-runtime-runs" variant={layout.runtimeView === 'runs' ? 'secondary' : 'ghost'} aria-pressed={layout.runtimeView === 'runs'} onClick={() => setLayout(value => ({ ...value, runtimeView: 'runs' }))}>{t('review.runs')}</Button></div>
          {project && task && !context.pending ? <><div hidden={layout.runtimeView !== 'processes'}><DevelopmentRuntime key={`${development.epoch}:${project.id}:${task.id}`} projectId={project.id} taskId={task.id} visible={mainView === 'runtime' && layout.runtimeView === 'processes'} /></div><div hidden={layout.runtimeView !== 'runs'}>{mainView === 'runtime' && layout.runtimeView === 'runs' ? <Runs key={`${development.epoch}:${task.id}`} taskId={task.id} targetRunId={runTarget} /> : null}</div></> : <p>{t('dev.noTaskSelected')}</p>}
        </section>
      </main>
    </div>
  </section>;
}
