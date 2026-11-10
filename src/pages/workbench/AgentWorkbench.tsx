import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Button, Modal } from '@dgos/ui';
import { FolderOpen, Files, GitBranch, ListTodo, X } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentOpenInput, type AgentSession } from '../../stores/agentRunnerStore';
import { useDevelopment, type ConversationTab, type DevelopmentSession, type Outcome } from '../../stores/developmentStore';
import { chooseContext, contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceActive, useSurfaceCloseGuard, useSurfaceStatus } from '../../workspace/SurfaceContext';
import { confirmDiscard, prepareAgentClose } from '../../workspace/closeCoordinator';
import { errorText } from '../../components/development/shared';
import { TaskPanel } from '../../components/development/TaskPanel';
import { UnclassifiedPanel } from '../../components/development/UnclassifiedPanel';
import { ProjectSessionTree } from '../../components/development/ProjectSessionTree';
import { ProjectPanel } from '../../components/development/ProjectPanel';
import { ProjectCapabilities } from '../../components/development/ProjectCapabilities';
import { readDevelopmentProjectGit } from '../../stores/developmentExplorer';
import { ProjectReview, Runs } from '../../components/development/ProjectReview';
import { Files as ProjectFiles, type FileMutation, type FileTarget } from '../../components/development/ProjectFiles';
import { AgentConversation, AgentSessionHistory } from './AgentConversation';
import { useDevelopmentEditor, hasDevelopmentDraft, discardDevelopmentDraft, openDevelopmentDocument } from '../../stores/developmentEditor';
import { useDevelopmentRuntime, processActive } from '../../stores/developmentRuntime';
import '../../components/development/development.css';
import { useProjectCapabilities, type CliProviderRevision } from '../../stores/developmentCapabilities';
import './workbench.css';
import { readDevelopmentWorkspaces, readDevelopmentGit } from '../../stores/developmentDelivery';
import { developmentDraftKey, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { PanelResizer, useWorkbenchLayout } from '../../components/development/WorkbenchLayout';
import { DevelopmentRuntime } from '../../components/development/DevelopmentRuntime';
import '../../components/development/workbench-layout.css';
import '../../components/development/workbench-tabs.css';
import { ProjectConversation, emptyConversationDraft, type ConversationDraft } from './ProjectConversation';
import { CliCleanupRecheck } from '../../components/development/CliCleanupRecheck';
import { resolveDevelopmentFileReference, type AgentOutputReference } from '../../components/development/eventOutputReferences';
import { ConversationTabs } from '../../components/development/ConversationTabs';
import { ConversationHandoff } from '../../components/development/ConversationHandoff';
import { useAccount } from '../../accounts/AccountContext';
import { FileWorkspace } from '../../components/development/FileWorkspace';
import { GitPanel } from '../../components/development/GitPanel';
import { GitCommitDetail } from '../../components/development/GitCommitDetail';
import { useCommitMessageGenerator } from '../../components/development/CommitMessageGenerator';
import { addConversationReference, captureConversationReferences, clearConversationReferences, type ConversationReference } from '../../stores/developmentReferences';
import { clearAgentImages, useAgentImages } from '../../stores/agentImages';

const RESTING = ['idle', 'closed', 'error'];

export default function AgentWorkbench() {
  const { t } = useLocale();
  const development = useDevelopment();
  const commitMessage = useCommitMessageGenerator(development.selection.project_id, development.selection.task_id);
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
  const [taskWorkspace, setTaskWorkspace] = useState<{taskId:string|null;path:string;kind:string;branch:string|null;head:string|null}|null>(null);
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
  function setFileTarget(target: FileTarget | null | ((current: FileTarget | null) => FileTarget | null)) { const current = ownedFileTarget?.owner === viewOwner ? ownedFileTarget.target : null; const value = typeof target === 'function' ? target(current) : target; setOwnedFileTarget(value ? { owner: viewOwner, target: value } : null); }
  const [diffSource, setDiffSource] = useState<'task' | 'index' | 'worktree'>('task');
  const [runTarget, setRunTarget] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [revealTarget, setRevealTarget] = useState<{ tabId: string; projectId: string; sessionId: string | null; nonce: number }>();
  const [manager, setManager] = useState<{ projectId: string | null; section: 'project' | 'capabilities' } | null>(null);
  const navigationTrigger = useRef<HTMLButtonElement>(null);
  const navigation = useRef<HTMLElement>(null);
  const agentPanel = useRef<HTMLDivElement>(null);
  const navigationOpen = layout.drawer !== null && (!compact || drawerOpen);
  const tools = [
    { id: 'projects', label: 'workbenchNavigation.projectsAndSessions', icon: FolderOpen },
    { id: 'files', label: 'workbenchNavigation.filesAndGit', icon: Files },
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
      setLayout(value => ({ ...value, drawer, codeView: drawer === 'files' ? value.codeView === 'changes' ? 'changes' : 'files' : drawer === 'tasks' ? 'review' : value.codeView }));
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
  const commitOwner = JSON.stringify([development.epoch, project?.id, task?.id, taskWorkspace?.path ?? project?.path]);
  const [commitSelection, setCommitSelection] = useState<{ owner: string; oid: string; workspacePath: string } | null>(null);
  const selectedCommit = !context.pending && commitSelection?.owner === commitOwner ? commitSelection : null;
  useEffect(() => { setCommitSelection(null); }, [commitOwner, context.pending]);
  const commitTrigger = useRef<HTMLButtonElement | null>(null);
  const [commitOverlay, setCommitOverlay] = useState(true);
  useLayoutEffect(() => {
    const node = root.current;
    if (!node) return;
    function measure() {
      const rem = Number.parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
      const hasDocuments = fileEditor.tabs.some(tab => tab.projectId === project?.id && tab.taskId === (task?.id ?? null) && tab.workspacePath === (taskWorkspace?.path ?? project?.path));
      setCommitOverlay((node!.clientWidth || window.innerWidth) < (3.5 + (navigationOpen ? layout.drawerWidth : 0) + 25 + 28 + (hasDocuments ? layout.documentWidth : 0)) * rem);
    }
    measure();
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(node); window.addEventListener('resize', measure);
    return () => { observer?.disconnect(); window.removeEventListener('resize', measure); };
  }, [root, navigationOpen, layout.drawerWidth, layout.documentWidth, fileEditor.tabs, project?.id, task?.id, taskWorkspace?.path, project?.path]);
  function selectCommit(oid: string, trigger?: HTMLButtonElement, workspacePath = taskWorkspace?.path ?? project?.path ?? '') {
    if (context.pending || !project) return;
    if (trigger) commitTrigger.current = trigger;
    setCommitSelection({ owner: commitOwner, oid, workspacePath });
    window.requestAnimationFrame(() => root.current?.querySelector<HTMLButtonElement>('[data-testid="git-commit-close"]')?.focus());
  }
  function closeCommit() {
    setCommitSelection(null);
    window.requestAnimationFrame(() => {
      const trigger = commitTrigger.current;
      if (trigger?.isConnected && !trigger.closest('[hidden], [inert]')) trigger.focus();
      else navigation.current?.querySelector<HTMLButtonElement>('.git-history-row')?.focus();
    });
  }
  function showCode(codeView: 'files' | 'changes' | 'review' | 'capabilities' | 'delivery') {
    if (codeView === 'capabilities') { setManager({ projectId: project?.id ?? null, section: 'capabilities' }); return; }
    openDrawer(codeView === 'review' ? 'tasks' : 'files');
    setLayout(value => ({ ...value, codeView: codeView === 'delivery' ? 'changes' : codeView }));
  }
  function showRuntime() { setMainView('runtime'); if (compact) setDrawerOpen(false); setLayout(value => ({ ...value, runtimeView: 'processes' })); }
  function manageProjects() { setManager({ projectId: project?.id ?? null, section: 'project' }); }
  async function openFile(target: FileTarget, source: 'task' | 'index' | 'worktree' = 'worktree') {
    if (context.pending || !project) return;
    const workspacePath = taskWorkspace?.path ?? project.path;
    openDevelopmentDocument({ projectId: project.id, taskId: task?.id ?? null, workspacePath, path: target.path, mode: target.mode, line: target.line, source });
    setOwnedFileTarget({ owner: viewOwner, target }); setDiffSource(source); setMainPosition({ owner: viewOwner, view: 'file' });
    if (compact) setDrawerOpen(false);
  }
  function fileMutated(_mutation: FileMutation) {
    setWorkspaceRefresh(value => value + 1);
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
  async function ensureWorkspaceTask(): Promise<string | null> {
    if (task) return task.id;
    if (!project || context.pending || preparingWorkspace.current) return null;
    const preparation = useDevelopmentDrafts.getState().entries[workspaceDraftKey]?.value as { phase: string; taskId?: string } | undefined;
    if (preparation?.phase === 'unknown') return null;
    if (preparation?.taskId) return preparation.taskId;
    const epoch = development.epoch, generation = contextGeneration();
    preparingWorkspace.current = true; setBusy(true);
    const record = (phase: string, taskId?: string) => useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [workspaceDraftKey]: { kind: 'task', value: { phase, taskId }, baseline: {} } } }));
    try {
      record('pending');
      const result = await development.saveTask({ id: null, project_id: project.id, goal: t('workbenchFiles.projectWork', { name: project.name }), requirements: '', status: 'todo' });
      if (epoch !== useDevelopment.getState().epoch) return null;
      if (!result.ok) { record(result.code === 'unknown' ? 'unknown' : 'failed'); setFailure(result.code); return null; }
      record('ready', result.value.id);
      return generation === contextGeneration() ? result.value.id : null;
    } finally { preparingWorkspace.current = false; if (epoch === useDevelopment.getState().epoch) setBusy(false); }
  }
  async function prepareWorkspace() {
    if (!project || !activeTab || task) return;
    const epoch = development.epoch, generation = contextGeneration(), tabId = activeTab.id;
    const id = await ensureWorkspaceTask();
    if (!id || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration() || useDevelopment.getState().activeTabId !== tabId) return;
    const selected = await chooseContext({ project_id: project.id, task_id: id, session_id: null });
    if (selected.ok && epoch === useDevelopment.getState().epoch) {
      useDevelopment.setState(state => ({ tabs: state.tabs.map(tab => tab.id === tabId ? { ...tab, taskId: id } : tab) }));
      const owner = JSON.stringify([epoch, project.id, id, null]);
      setMainPosition({ owner, view: mainView });
      if (fileTarget) setOwnedFileTarget({ owner, target: fileTarget });
    } else if (!selected.ok) setFailure(selected.code);
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
  async function newConversation(configuration?: CliProviderRevision['configuration'], projectId = project?.id) {
    if (!projectId || context.pending) return;
    const epoch = development.epoch;
    const result = await chooseContext({ project_id: projectId, task_id: null, session_id: null });
    if (!result.ok || epoch !== useDevelopment.getState().epoch) return;
    const tab = useDevelopment.getState().newTab(projectId);
    if (configuration) {
      const key = developmentDraftKey('conversation', projectId, tab.id);
      useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { ...emptyConversationDraft, configuration }, baseline: emptyConversationDraft } } }));
    }
    revealAgent();
  }
  async function selectProject(projectId: string) {
    if (context.pending) return;
    const tabs = useDevelopment.getState().tabs;
    const open = tabs.find(tab => tab.projectId === projectId && tab.id === development.activeTabId) ?? tabs.find(tab => tab.projectId === projectId);
    if (open) { await selectTab(open); revealAgent(); return; }
    const epoch = development.epoch;
    const result = await chooseContext({ project_id: projectId, task_id: null, session_id: null });
    if (result.ok && epoch === useDevelopment.getState().epoch) revealAgent();
  }
  async function refreshProject(projectId: string) {
    const epoch = development.epoch;
    const inspected = await useDevelopment.getState().inspect(projectId);
    if (epoch !== useDevelopment.getState().epoch) return;
    if (!inspected.ok) throw inspected.code;
    if (inspected.value.path_state !== 'available') throw 'workspace_changed';
    if (!await useDevelopment.getState().load()) throw useDevelopment.getState().error ?? 'unknown';
    if (epoch === useDevelopment.getState().epoch && useDevelopment.getState().selection.project_id === projectId) setWorkspaceRefresh(value => value + 1);
  }
  async function openProjectFiles(projectId: string) {
    const epoch = development.epoch;
    await selectProject(projectId);
    if (epoch !== useDevelopment.getState().epoch || useDevelopment.getState().selection.project_id !== projectId) return;
    openDrawer('files');
    setLayout(value => ({ ...value, codeView: 'files' }));
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
    const pendingSend = () => tab.sessionId && Object.entries(useDevelopmentDrafts.getState().entries).some(([key, entry]) => {
      const scope = JSON.parse(key);
      return entry.kind === 'conversation' && scope[0] === epoch && scope[2] === tab.projectId && scope[3] === tab.sessionId && ['pending-send', 'pending-steer'].includes(scope[5]);
    });
    if (pendingSend()) { setFailure('unknown'); return; }
    const key = tab.sessionId ? developmentDraftKey('conversation', tab.projectId, tab.sessionId, 'message') : developmentDraftKey('conversation', tab.projectId, tab.id);
    const entry = useDevelopmentDrafts.getState().entries[key];
    const intent = !tab.sessionId ? entry?.value as ConversationDraft | undefined : undefined;
    if (intent?.preparing || intent && ['task_unknown', 'open_unknown', 'send_unknown'].includes(intent.phase)) { setFailure('session_busy'); return; }
    const references = captureConversationReferences(tab.projectId, tab.sessionId ?? tab.id);
    const imageEntries = () => Object.entries(useAgentImages.getState().drafts).filter(([owner]) => {
      const scope = JSON.parse(owner);
      return scope[0] === epoch && scope[2] === tab.projectId && scope[3] === 'images' && (tab.sessionId ? scope[4] === tab.sessionId : scope[4] === 'draft' && scope[5] === tab.id);
    });
    const images = imageEntries();
    const imagesUnchanged = () => { const current = imageEntries(); return images.length === current.length && images.every(([owner, value]) => current.some(([key, draft]) => key === owner && draft === value)); };
    const unsent = references.length > 0 || images.some(([, draft]) => draft.busy || draft.images.length > 0) || (typeof entry?.value === 'string' ? Boolean(entry.value.trim()) : Boolean(intent?.prompt.trim()));
    if (unsent && !await confirmDiscard() || epoch !== useDevelopment.getState().epoch || entry !== useDevelopmentDrafts.getState().entries[key] || !imagesUnchanged()) return;
    if (pendingSend()) { setFailure('unknown'); return; }
    if (JSON.stringify(references) !== JSON.stringify(captureConversationReferences(tab.projectId, tab.sessionId ?? tab.id))) return;
    let nextSessionToAttach: string | null = null;
    if (tab.id === useDevelopment.getState().activeTabId) {
      const remaining = useDevelopment.getState().tabs.filter(item => item.id !== tab.id);
      const next = remaining[Math.min(development.tabs.findIndex(item => item.id === tab.id), remaining.length - 1)];
      const result = await chooseTabContext(next);
      if (!result.ok || epoch !== useDevelopment.getState().epoch || !imagesUnchanged() || pendingSend()) return;
      useDevelopment.getState().closeTab(tab.id);
      if (next?.sessionId && useAgentRunner.getState().live[next.sessionId]) nextSessionToAttach = next.sessionId;
    } else useDevelopment.getState().closeTab(tab.id);
    if (epoch !== useDevelopment.getState().epoch) return;
    useDevelopmentDrafts.setState(state => { const entries = { ...state.entries }; delete entries[key]; return { entries }; });
    clearConversationReferences(tab.projectId, tab.sessionId ?? tab.id, references);
    for (const [owner] of images) clearAgentImages(owner);
    if (nextSessionToAttach) await useAgentRunner.getState().attach(nextSessionToAttach);
    if (tab.id === activeTab?.id && surfaceActive) window.requestAnimationFrame(() => { const id = useDevelopment.getState().activeTabId; if (id) document.getElementById(`conversation-tab-${id}`)?.focus({ preventScroll: true }); });
    // Handoff reservation identities remain recoverable from history even when their work surface closes.
  }
  // External project/task selection and history selection join the same tab owner.
  useEffect(() => {
    if (!development.loaded || context.pending) return;
    const selection = development.selection;
    if (!selection.project_id) return;
    if (compact && activeTab?.projectId !== selection.project_id) setDrawerOpen(false);
    if (activeTab?.projectId === selection.project_id && activeTab.taskId === selection.task_id && activeTab.sessionId === selection.session_id) return;
    if (selection.session_id) {
      const saved = development.sessions.find(item => item.id === selection.session_id);
      if (saved) development.openTab(saved);
    } else {
      const blank = development.tabs.find(tab => tab.projectId === selection.project_id && tab.taskId === selection.task_id && !tab.sessionId);
      if (blank) development.activateTab(blank.id); else development.newTab(selection.project_id, selection.task_id);
    }
  }, [development.loaded, development.selection, context.pending, activeTab?.id, compact]);
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
    if (!project || context.pending) return;
    let active = true; const epoch = development.epoch, generation = contextGeneration();
    void Promise.all([readDevelopmentWorkspaces({ project_id: project.id, task_id: task?.id ?? null }), task ? readDevelopmentGit(task.id) : readDevelopmentProjectGit(project.id)]).then(([result, git]) => {
      if (!active || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      if (!result.ok) { setWorkspaceError(result.code); return; }
      const selected = result.value.workspaces.find(item => result.value.selected_workspace_id ? item.id === result.value.selected_workspace_id : item.kind === 'original');
      if (!selected) { setWorkspaceError('workspace_not_found'); return; }
      setTaskWorkspace({ taskId: task?.id ?? null, path: selected.path, kind: selected.kind, branch: git.ok ? git.value.branch : null, head: git.ok ? git.value.head : null });
      if (!git.ok) setWorkspaceError(git.code);
    });
    return () => { active = false; };
  }, [projectPath, task?.id, development.epoch, context.generation, context.pending, workspaceRefresh]);
  useEffect(() => { setBusy(false); setFailure(null); }, [development.epoch, context.generation]);
  useEffect(() => { setTrusted(false); setHandoffSource(null); }, [development.epoch, context.generation]);
  useEffect(() => { setRunTarget(null); }, [development.epoch, context.generation]);
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
  const dirty = Object.values(drafts).some(value => value.trim()) || Object.keys(editorDrafts).length > 0 || Object.values(hostRuntime.drafts).some(draft => draft.dirty) || hasDevelopmentDraft();
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
  async function openNativeFork(sessionId: string) {
    if (context.pending || !current) return;
    const epoch = development.epoch, generation = contextGeneration(), source = current.id;
    if (!await useDevelopment.getState().load()) return;
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration() || useDevelopment.getState().selection.session_id !== source) return;
    const saved = useDevelopment.getState().sessions.find(item => item.id === sessionId);
    if (!saved || saved.project_id !== project?.id || !saved.task_id) return;
    await selectSession(saved);
    if (epoch === useDevelopment.getState().epoch && useDevelopment.getState().selection.session_id === sessionId) revealAgent();
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
  const codeViews = ['review'] as const;
  const nativeRefreshKey = `${current?.id ?? ''}:${current?.events.reduce((seq, event) => event.kind === 'result' || event.kind === 'turn/completed' ? event.seq : seq, 0) ?? 0}`;
  const documentTabs = fileEditor.tabs.filter(tab => tab.projectId === project?.id && tab.taskId === (task?.id ?? null) && tab.workspacePath === (taskWorkspace?.path ?? project?.path));
  const documentsOpen = documentTabs.length > 0;
  const conversationVisible = mainView !== 'runtime' && (!compact || mainView !== 'file' || !documentsOpen);
  const viewNavigation = <nav className="dev-main-view-nav" aria-label={t('workbenchFiles.views')}>
    <Button size="xs" variant={mainView === 'conversation' ? 'secondary' : 'ghost'} aria-pressed={mainView === 'conversation'} onClick={() => revealAgent()}>{t('workbenchFiles.conversation')}</Button>
    {compact && documentsOpen ? <Button size="xs" variant={mainView === 'file' ? 'secondary' : 'ghost'} aria-pressed={mainView === 'file'} onClick={() => { setMainView('file'); setDrawerOpen(false); }}>{t('fileWorkspace.title')}{hasDevelopmentDraft() ? ' *' : ''}</Button> : null}
    <Button size="xs" variant={mainView === 'runtime' ? 'secondary' : 'ghost'} disabled={!project || context.pending} aria-pressed={mainView === 'runtime'} onClick={showRuntime}>{t('workbenchFiles.runtime')}</Button>
  </nav>;
  return <section ref={root} className="agent-workbench dev-workspace" data-compact={compact} data-drawer-width={layout.drawerWidth} data-document-width={layout.documentWidth}>
    <span className="sr-only" role="status" data-testid="agent-session-announcement">{surfaceActive && current ? `${t(`agent.engine.${current.engine}`)}: ${t(`agent.state.${current.status}`)}. ${t('dev.pendingApprovals', { count: current.pending.length })}` : ''}</span>
    {failureCode ? <p role="alert" className="dev-workspace-error text-sm text-status-error">{errorText(failureCode)}</p> : null}
    {failureCode === 'cleanup_pending' ? <CliCleanupRecheck /> : null}
    {compact ? viewNavigation : null}
    <div className="dev-workspace-body" data-drawer-open={navigationOpen} data-commit-open={!!selectedCommit} data-commit-overlay={commitOverlay}>
      <nav className="dev-tool-rail" aria-label={t('dev.projectTools')} inert={!!selectedCommit && commitOverlay}>
        {tools.map(({ id, label, icon: Icon }) => <Button key={id} ref={node => { if (node && (layout.drawer === id || !navigationTrigger.current)) navigationTrigger.current = node; }} size="sm" variant="ghost" title={t(label)} aria-label={t(label)} aria-expanded={navigationOpen && layout.drawer === id} aria-controls="dev-navigation" data-active={navigationOpen && layout.drawer === id} data-testid={`dev-tool-${id}`} onClick={event => toggleDrawer(id, event.currentTarget)}><Icon size={20} aria-hidden="true" /></Button>)}
      </nav>
      <aside ref={navigation} id="dev-navigation" className="dev-navigation" aria-label={t(drawerTitle)} hidden={!navigationOpen} inert={!!selectedCommit && commitOverlay} onKeyDown={event => {
        if (event.key === 'Escape' && !event.defaultPrevented) { event.preventDefault(); closeNavigation(); }
      }}>
        <header className="dev-drawer-header"><h2>{t(drawerTitle)}</h2><div hidden={layout.drawer !== 'projects'}><ProjectPanel mode="add-only" /></div>{layout.drawer !== 'projects' && layout.drawer !== 'files' ? <Button size="xs" variant="ghost" aria-label={t('common.close')} onClick={closeNavigation}><X size={16} aria-hidden="true" /></Button> : null}</header>
        <div className="dev-drawer-content">
          <div hidden={layout.drawer !== 'projects'}>
            <ProjectSessionTree revealTarget={revealTarget} busy={context.pending} onSelect={selectSession} onNewConversation={id => void newConversation(undefined, id)} onSelectProject={id => void selectProject(id)} onSelectDraft={tab => void selectTab(tab)} onRefreshProject={refreshProject} onProjectFiles={openProjectFiles} />
            <details className="dev-context-details" data-testid="dev-context-details"><summary>{t('layout.contextDetails')}</summary><div className="dev-detail">
              <p>{t('review.cwd')}: {taskWorkspace?.path ?? t('layout.directoryUnresolved')}</p>
              <p>{t('dev.selected')}: {project?.id ?? '—'}</p><p>{t('dev.tasks')}: {task?.id ?? '—'}</p><p>{t('dev.sessions')}: {current?.id ?? selectedSaved?.id ?? '—'}</p><p>{t('layout.head')}: {taskWorkspace?.head ?? '—'}</p>
              <p>{taskWorkspace?.branch ?? t('dev.kindLabel.directory')} / {taskWorkspace?.kind ?? t('layout.directoryUnresolved')}</p>
              {workspaceError ? <p className="text-status-error-text">{t('layout.contextFailed')}: {errorText(workspaceError)}</p> : null}
              <Button size="xs" variant="secondary" onClick={() => setWorkspaceRefresh(value => value + 1)}>{t('dev.inspect')}</Button>
              <Button size="xs" variant="ghost" onClick={manageProjects}>{t('workbenchNavigation.manageProject')}</Button>
            </div></details>
          </div>
          <div hidden={layout.drawer !== 'tasks'}>{project ? <TaskPanel project={project} onConversation={() => { closeNavigation(); revealAgent(); }} onContinue={id => { const saved = development.sessions.find(session => session.id === id); if (saved) void selectSession(saved); }} /> : <p>{t('dev.none')}</p>}
            {task ? <details className="dev-task-brief" data-testid="dev-task-summary"><summary>{t('dev.taskFor', { goal: task.goal })}<span>{t(`dev.taskStatus.${task.status}`)}</span></summary><p>{task.requirements}</p></details> : null}
            <UnclassifiedPanel sessions={unclassified} taskId={task?.id ?? null} busy={context.pending || busy || runner.busy} onClassify={saved => void classify(saved)} />
          </div>
          <div hidden={layout.drawer !== 'files'}>
            {project ? <div className="dev-explorer-location"><span title={taskWorkspace?.path}>{project.name}</span>{taskWorkspace?.branch ? <><GitBranch size={14} aria-hidden="true" /><span>{taskWorkspace.branch}</span></> : null}</div> : null}
            <div className="dev-explorer-tabs" role="group" aria-label={t('workbenchNavigation.filesAndGit')}>
              <Button size="xs" variant={layout.codeView !== 'changes' ? 'secondary' : 'ghost'} aria-pressed={layout.codeView !== 'changes'} onClick={() => showCode('files')}><Files size={16} aria-hidden="true" />{t('workbenchNavigation.files')}</Button>
              <Button size="xs" variant={layout.codeView === 'changes' ? 'secondary' : 'ghost'} aria-pressed={layout.codeView === 'changes'} onClick={() => showCode('changes')}><GitBranch size={16} aria-hidden="true" />{t('workbenchNavigation.git')}</Button>
            </div>
            <div hidden={layout.codeView === 'changes'}>{project && !context.pending ? <ProjectFiles key={`${development.epoch}:${context.generation}:${project.id}:${task?.id ?? ''}:${taskWorkspace?.path ?? project.path}`} projectId={project.id} taskId={task?.id ?? null} workspacePath={taskWorkspace?.path ?? project.path} target={fileTarget} onOpenFile={target => void openFile(target)} onReference={activeTab ? addReference : undefined} onMutation={fileMutated} /> : <p>{t('dev.none')}</p>}</div>
            <div hidden={layout.codeView !== 'changes'}>{project && !context.pending ? <GitPanel key={`${development.epoch}:${project.id}:${task?.id ?? ''}`} projectId={project.id} taskId={task?.id ?? null} ensureTask={ensureWorkspaceTask} sessions={sessions} live={runner.live} workspacePath={taskWorkspace?.path ?? project.path} refreshKey={`${nativeRefreshKey}:${workspaceRefresh}`} selectedCommit={selectedCommit?.oid} onSelectCommit={selectCommit} onOpenFile={path => void openFile({ path, mode: 'file', nonce: Date.now() })} onOpenDiff={(path, source = 'worktree') => void openFile({ path, mode: 'diff', nonce: Date.now() }, source)} onChanged={() => setWorkspaceRefresh(value => value + 1)} onGenerateMessage={commitMessage.generate} /> : <p>{t('dev.none')}</p>}</div>
          </div>
          <div id="dev-panel-code" data-testid="dev-panel-code" className="dev-tools-pane" hidden={layout.drawer !== 'tasks'}>
            {project && !context.pending ? <ProjectReview key={`${development.epoch}:${context.generation}:${project.id}:${task?.id ?? ''}`} projectId={project.id} taskId={task?.id ?? null} sessions={sessions} live={runner.live} capabilityRevision={projectCapabilities.value?.revision ?? 0} view={codeViews[0]} views={codeViews} onViewChange={showCode} onOpenFile={target => void openFile(target)} snapshot={current?.configuration?.format === 'provider_bound' ? current.configuration.snapshot : null} onUseConfiguration={useConfiguration} onOpenRuntime={showRuntime} workspacePath={taskWorkspace?.path ?? ''} onWorkspaceChange={() => { setWorkspaceRefresh(value => value + 1); setTrusted(false); }} onHandoff={sessionId => { revealAgent(); void acceptHandoff(sessionId); }} /> : <p>{t('dev.none')}</p>}
          </div>
        </div>
        <PanelResizer value={layout.drawerWidth} onChange={drawerWidth => setLayout(value => ({ ...value, drawerWidth }))} label={t('workbenchNavigation.resizeDrawer')} testId="dev-drawer-resizer" controls="dev-navigation" />
      </aside>
      {project && selectedCommit ? <aside className="dev-commit-column" aria-label={t('gitPanel.commitDetails')} data-testid="dev-commit-column"><GitCommitDetail projectId={project.id} taskId={task?.id ?? null} workspacePath={selectedCommit.workspacePath} oid={selectedCommit.oid} onSelect={oid => selectCommit(oid, undefined, selectedCommit.workspacePath)} onClose={closeCommit} /></aside> : null}
      <main className="dev-workspace-main" hidden={compact && mainView === 'file' && documentsOpen} inert={compact && navigationOpen || !!selectedCommit && commitOverlay}>
        <ConversationTabs busy={context.pending} onSelect={tab => void selectTab(tab)} onClose={closeTab} onReveal={async tab => { openDrawer('projects'); setRevealTarget({ tabId: tab.id, projectId: tab.projectId, sessionId: tab.sessionId, nonce: Date.now() }); }} />
        {!compact ? viewNavigation : null}
        {project && activeTab && !task && mainView === 'runtime' ? <div className="dev-workspace-preparation"><Button size="xs" variant="secondary" disabled={busy || context.pending || project.archived || workspacePreparation?.phase === 'unknown'} onClick={() => void prepareWorkspace()}>{t('workbenchFiles.prepare')}</Button><p className="text-xs">{t(workspacePreparation?.phase === 'unknown' ? 'workbenchFiles.prepareUnknown' : 'workbenchFiles.prepareHint')}</p></div> : null}
        {pendingCount ? <div className="dev-approval-navigation"><Button size="xs" variant="secondary" data-testid="dev-pending-approvals" onClick={() => revealAgent(true)}>{t('dev.pendingApprovals', { count: pendingCount })}</Button></div> : null}
        <div className="dev-conversation-column" hidden={!conversationVisible}>
        <div ref={agentPanel} id="dev-panel-agent" data-testid="dev-panel-agent" className="dev-conversation-pane" role="tabpanel" aria-labelledby={activeTab ? `conversation-tab-${activeTab.id}` : undefined}>
          {activeTab && (!project || activeTab.sessionId && !selectedSaved) ? <div className="dev-workspace-empty"><p role="alert">{t('workbenchNavigation.unavailable')}</p><Button size="sm" onClick={manageProjects}>{t('dev.projects')}</Button><Button size="sm" onClick={() => void closeTab(activeTab)}>{t('workbenchNavigation.closeTab')}</Button></div>
            : handoffSource && project && selectedSaved?.id === handoffSource ? <ConversationHandoff key={`${development.epoch}:${handoffSource}`} project={project} source={selectedSaved} onCancel={() => setHandoffSource(null)} onReady={async id => { setHandoffSource(null); await acceptHandoff(id); }} />
            : current ? <AgentConversation key={`${development.epoch}:${current.id}:${current.generation}`} visible={conversationVisible && surfaceActive} session={current} drafts={drafts} setDraft={setSessionDraft} busy={busy || runner.busy || Boolean(project?.archived)} error={Boolean(runner.error)} onNewConversation={newConversation} onContinueModel={() => setHandoffSource(current.id)} onOpenSession={sessionId => void openNativeFork(sessionId)} onOpenReference={openReference} onSelectRun={runId => { setRunTarget(runId); setMainView('runtime'); if (compact) setDrawerOpen(false); setLayout(value => ({ ...value, runtimeView: 'runs' })); }} />
            : !context.pending && selectedSaved?.task_id ? <AgentSessionHistory key={`${development.epoch}:${selectedSaved.id}`} visible={conversationVisible && surfaceActive} sessionId={selectedSaved.id} taskId={selectedSaved.task_id}>
              <div data-testid="dev-resume-entry"><p>{t('dev.resumeNotStarted')}</p>
              {project?.archived ? <p role="alert">{t('dev.error.project_archived')}</p> : null}
              {workspaceError ? <div><p role="alert">{errorText(workspaceError)}</p><Button size="xs" variant="secondary" onClick={manageProjects}>{t('dev.projects')}</Button><Button size="xs" variant="ghost" onClick={() => setWorkspaceRefresh(value => value + 1)}>{t('dev.inspect')}</Button></div> : null}
              <label className="dev-checkbox"><input type="checkbox" checked={trusted} onChange={event => setTrusted(event.target.checked)} />{t('dev.trustProject')}</label>
              <Button size="sm" variant="secondary" disabled={busy || runner.busy || !trusted || !taskWorkspace || Boolean(workspaceError) || project?.archived} onClick={() => resume(selectedSaved)}>{t('dev.trustResume')}</Button>
              <Button size="sm" variant="ghost" disabled={Boolean(project?.archived)} onClick={() => setHandoffSource(selectedSaved.id)}>{t('workbenchNavigation.continueModel')}</Button></div></AgentSessionHistory>
            : project && activeTab && !context.pending ? <ProjectConversation key={`${development.epoch}:${activeTab.id}`} visible={conversationVisible && surfaceActive} draftId={activeTab.id} project={project} taskId={task?.id ?? null} setSessionDraft={setSessionDraft} onInspect={inspectSession} onStart={(input, projectId, taskId, dispatch) => connect<Outcome<null>>(() => startConnection(input, projectId, taskId, null, dispatch), { ok: false, code: 'cancelled' })} />
            : <div className="dev-workspace-empty"><h2>{t('dev.welcome')}</h2><p>{t('workbenchNavigation.noTabs')}</p><Button size="sm" variant="secondary" onClick={manageProjects}>{t('dev.projects')}</Button></div>}
        </div>
        {project && taskWorkspace && mainView === 'conversation' ? <div className="dev-composer-location"><FolderOpen size={13} aria-hidden="true" /><span title={taskWorkspace.path}>{taskWorkspace.path}</span>{taskWorkspace.branch ? <><GitBranch size={13} aria-hidden="true" /><span>{taskWorkspace.branch}</span></> : null}</div> : null}
        </div>
        <section id="dev-panel-runtime" data-testid="dev-panel-runtime" className="dev-main-tool-surface" hidden={mainView !== 'runtime'} aria-label={t('workbenchFiles.runtime')}>
          <header className="dev-main-tool-header"><strong>{project?.name}</strong><Button size="xs" variant="ghost" onClick={() => revealAgent()}>{t('workbenchFiles.hideRuntime')}</Button></header>
          <p className="text-xs text-ink-secondary">{t('layout.runtimeCounts', { active: activeProcesses, failed: failedProcesses, unknown: unknownProcesses + unresolvedOperations })}</p>
          {hostRuntime.read.status === 'error' || hostRuntime.read.status === 'ready' && hostRuntime.read.refresh.status !== 'current' ? <p className="text-xs text-status-warning-text">{t('direct.lastKnownHistory')}</p> : null}
          <div className="dev-runtime-tabs" role="group" aria-label={t('layout.runtime')}><Button size="xs" data-testid="dev-runtime-processes" variant={layout.runtimeView === 'processes' ? 'secondary' : 'ghost'} aria-pressed={layout.runtimeView === 'processes'} onClick={() => setLayout(value => ({ ...value, runtimeView: 'processes' }))}>{t('direct.title')}</Button><Button size="xs" data-testid="dev-runtime-runs" variant={layout.runtimeView === 'runs' ? 'secondary' : 'ghost'} aria-pressed={layout.runtimeView === 'runs'} onClick={() => setLayout(value => ({ ...value, runtimeView: 'runs' }))}>{t('review.runs')}</Button></div>
          {project && task && !context.pending ? <><div hidden={layout.runtimeView !== 'processes'}><DevelopmentRuntime key={`${development.epoch}:${project.id}:${task.id}`} projectId={project.id} taskId={task.id} visible={mainView === 'runtime' && layout.runtimeView === 'processes'} /></div><div hidden={layout.runtimeView !== 'runs'}>{mainView === 'runtime' && layout.runtimeView === 'runs' ? <Runs key={`${development.epoch}:${task.id}`} taskId={task.id} targetRunId={runTarget} /> : null}</div></> : <p>{t('dev.noTaskSelected')}</p>}
        </section>
      </main>
        {project && !context.pending ? <aside className="dev-document-column" aria-label={t('fileWorkspace.title')} hidden={!documentsOpen || compact && mainView !== 'file'} inert={compact && navigationOpen || !!selectedCommit && commitOverlay}>
          <PanelResizer reverse value={layout.documentWidth} onChange={documentWidth => setLayout(value => ({ ...value, documentWidth }))} label={t('fileWorkspace.resize')} testId="dev-document-resizer" controls="dev-panel-documents" />
          <FileWorkspace projectId={project.id} taskId={task?.id ?? null} workspacePath={taskWorkspace?.path ?? project.path} target={fileTarget} source={diffSource} ensureTask={ensureWorkspaceTask} visible={surfaceActive && (!compact || mainView === 'file')} onClose={() => { setFileTarget(null); revealAgent(); }} onOpenFile={(target, source) => void openFile(target, source)} onReference={activeTab ? addReference : undefined} onChanged={() => setWorkspaceRefresh(value => value + 1)} />
          {activeTab && !task && fileTarget?.mode === 'file' ? <div className="dev-workspace-preparation"><Button size="xs" variant="secondary" disabled={busy || context.pending || project.archived || workspacePreparation?.phase === 'unknown'} onClick={() => void prepareWorkspace()}>{t('workbenchFiles.prepare')}</Button><p className="text-xs">{t(workspacePreparation?.phase === 'unknown' ? 'workbenchFiles.prepareUnknown' : 'workbenchFiles.prepareHint')}</p></div> : null}
        </aside> : null}
    </div>
    {commitMessage.dialog}
    <Modal open={manager !== null} onOpenChange={open => { if (!open) setManager(null); }} title={t(manager?.section === 'capabilities' ? 'capabilities.title' : 'dev.projects')} footer={<Button variant="secondary" onClick={() => setManager(null)}>{t('common.close')}</Button>}>
      {manager ? <><div className="dev-main-view-nav"><Button size="xs" variant={manager.section === 'project' ? 'secondary' : 'ghost'} onClick={() => setManager({ ...manager, section: 'project' })}>{t('dev.projects')}</Button>{manager.projectId ? <Button size="xs" variant={manager.section === 'capabilities' ? 'secondary' : 'ghost'} onClick={() => setManager({ ...manager, section: 'capabilities' })}>{t('capabilities.title')}</Button> : null}</div><div hidden={manager.section !== 'project'}><ProjectPanel mode="manage" projectId={manager.projectId} /></div>{manager.projectId && manager.section === 'capabilities' ? <ProjectCapabilities projectId={manager.projectId} onUseConfiguration={manager.projectId === project?.id ? useConfiguration : undefined} /> : null}</> : null}
    </Modal>
  </section>;
}
