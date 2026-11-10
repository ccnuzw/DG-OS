import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useAgentRunner, type Engine } from './agentRunnerStore';
import { hostCode } from './hostError';
import { initializeDesktopState, onDesktopStateRestored, resetDesktopState, setDesktopSlot, useDesktopPersistence } from './developmentDesktop';

export type TaskStatus = 'todo' | 'in_progress' | 'blocked' | 'review' | 'completed' | 'cancelled';
export const TASK_STATUSES: readonly TaskStatus[] = ['todo', 'in_progress', 'blocked', 'review', 'completed', 'cancelled'];
export interface Project { id: string; owner: string; name: string; path: string; directory_identity: string; archived: boolean; created_at: number; last_opened_at: number | null }
export interface DevelopmentTask { id: string; owner: string; project_id: string; goal: string; requirements: string; status: TaskStatus; created_at: number }
export interface DevelopmentSession { id: string; owner: string; engine: Engine; native_id: string | null; workspace: string; executable: string | null; model: string | null; project_id: string | null; task_id: string | null }
export interface Selection { project_id: string | null; task_id: string | null; session_id: string | null }
export interface ProjectOperation { id: string; owner: string; kind: string; path: string; status: 'pending' | 'succeeded' | 'failed' | 'unknown'; project_id: string | null }
export type PathState = 'available' | 'missing' | 'changed' | 'inaccessible';
export interface ProjectInspection { path_state: PathState; kind: 'directory' | 'git' | 'unknown'; repository_root: string | null; branch: string | null; changes: string[]; truncated: boolean; error: string | null }
export interface CreateProjectInput { kind: 'import' | 'create' | 'clone'; name: string; path: string; source: string | null }
export type ProjectAction = 'rename' | 'archive' | 'restore' | 'unlink' | 'relocate';
export interface SaveTaskInput { id: string | null; project_id: string; goal: string; requirements: string; status: TaskStatus }
export type Outcome<T> = { ok: true; value: T } | { ok: false; code: string };
interface Workspace { projects: Project[]; tasks: DevelopmentTask[]; sessions: DevelopmentSession[]; selection: Selection; operations: ProjectOperation[] }

export interface ConversationTab { id: string; projectId: string; taskId: string | null; sessionId: string | null; title: string; hidden?: boolean }
export interface ReadingPosition { top: number; follow: boolean; messageId?: string; offset?: number }
interface ConversationWorkspace { tabs: ConversationTab[]; activeTabId: string | null; sessionNames: Record<string, string>; reading: Record<string, ReadingPosition>; pinnedProjectIds: string[]; archivedSessionIds: string[] }
const emptyConversations = (): ConversationWorkspace => ({ tabs: [], activeTabId: null, sessionNames: {}, reading: {}, pinnedProjectIds: [], archivedSessionIds: [] });
function stringIds(value: unknown): string[] {
  return Array.isArray(value) ? [...new Set(value.filter((id): id is string => typeof id === 'string' && id.trim().length > 0))] : [];
}
function workspaceKey(owner: string) { return `dgos.development.conversations:${owner}`; }
const migratedConversationKeys = new Map<string, string>();
function restoreConversations(owner: string, legacyOwner?: string): ConversationWorkspace | null {
  try {
    // Only legacy navigation metadata is migrated from browser storage. No body
    // or attachment is ever written there by this store.
    let value = (useDesktopPersistence.getState().value.conversations as ConversationWorkspace | undefined) ?? null;
    if (!value) {
      const key = localStorage.getItem(workspaceKey(owner)) !== null ? workspaceKey(owner) : legacyOwner ? workspaceKey(legacyOwner) : workspaceKey(owner);
      const raw = localStorage.getItem(key);
      if (raw === null) return null;
      value = JSON.parse(raw);
      if (!value || !Array.isArray(value.tabs)) throw new Error('desktop_state_invalid');
      migratedConversationKeys.set(owner, key);
    }
    if (!Array.isArray(value.tabs)) throw new Error('desktop_state_invalid');
    const ids = new Set<string>(), sessions = new Set<string>();
    const tabs = value.tabs.filter((tab: ConversationTab) => {
      if (!tab || typeof tab.id !== 'string' || typeof tab.projectId !== 'string' || typeof tab.title !== 'string' || !(tab.taskId === null || typeof tab.taskId === 'string') || !(tab.sessionId === null || typeof tab.sessionId === 'string') || ids.has(tab.id) || tab.sessionId && sessions.has(tab.sessionId)) return false;
      ids.add(tab.id); if (tab.sessionId) sessions.add(tab.sessionId); return true;
    });
    const sessionNames = Object.fromEntries(Object.entries(value.sessionNames ?? {}).filter(([, name]) => typeof name === 'string')) as Record<string, string>;
    const reading = Object.fromEntries(Object.entries(value.reading ?? {}).filter(([, position]) => position && typeof position === 'object' && 'top' in position && Number.isFinite(position.top) && Number(position.top) >= 0 && 'follow' in position && typeof position.follow === 'boolean')) as Record<string, ReadingPosition>;
    const active = value.activeTabId;
    return { tabs, activeTabId: tabs.some((tab: ConversationTab) => tab.id === active && !tab.hidden) ? active : tabs.find((tab: ConversationTab) => !tab.hidden)?.id ?? null, sessionNames, reading, pinnedProjectIds: stringIds(value.pinnedProjectIds), archivedSessionIds: stringIds(value.archivedSessionIds) };
  } catch { useDesktopPersistence.setState({ error: 'desktop_state_invalid' }); return null; }
}

interface DevelopmentState extends Workspace, ConversationWorkspace {
  loaded: boolean; error: string | null;
  /** Bumps on account reset. Every async result is compared against it before it may touch state. */
  epoch: number;
  workspaceOwner: string | null;
  newTab(projectId: string, taskId?: string | null): ConversationTab;
  openTab(session: DevelopmentSession): ConversationTab | null;
  activateTab(id: string): void;
  bindTab(id: string, session: DevelopmentSession): void;
  closeTab(id: string): void;
  renameTab(id: string, title: string): void;
  setReading(id: string, position: ReadingPosition): void;
  setProjectPinned(projectId: string, pinned: boolean): void;
  archiveSessions(sessionIds: string[]): void;
  restoreSessions(sessionIds: string[]): void;
  load(accountName?: string): Promise<boolean>;
  createProject(input: CreateProjectInput): Promise<Outcome<Project>>;
  updateProject(projectId: string, action: ProjectAction, value: string | null): Promise<Outcome<null>>;
  inspect(projectId: string): Promise<Outcome<ProjectInspection>>;
  saveTask(input: SaveTaskInput): Promise<Outcome<DevelopmentTask>>;
  select(selection: Selection): Promise<Outcome<null>>;
  classify(sessionId: string, taskId: string): Promise<Outcome<null>>;
  reset(): void;
}

const emptySelection: Selection = { project_id: null, task_id: null, session_id: null };
const emptyWorkspace: Workspace = { projects: [], tasks: [], sessions: [], selection: emptySelection, operations: [] };

// Module-level sequencing: `loadSeq` makes only the newest load applicable, and `selectionSeq` counts committed
// selections so a load that started before a selection change keeps the newer selection.
let loadSeq = 0;
let selectionSeq = 0;

export const useDevelopment = create<DevelopmentState>((set, get) => {
  // Runs one host command. A response for a superseded account epoch is reported as `unknown` and never applied.
  async function call<T>(command: string, args: { [key: string]: unknown }): Promise<Outcome<T>> {
    const epoch = get().epoch;
    try {
      const value = await invoke<T>(command, args);
      if (epoch !== get().epoch) return { ok: false, code: 'unknown' };
      set({ error: null });
      return { ok: true, value };
    } catch (error) {
      const code = hostCode(error);
      if (epoch === get().epoch) set({ error: code });
      return { ok: false, code };
    }
  }

  // Re-reads the workspace after a mutation. A mutation counts as applied only if the account did not change.
  async function mutated<T>(epoch: number, value: T): Promise<Outcome<T>> {
    await get().load();
    return epoch === get().epoch ? { ok: true, value } : { ok: false, code: 'unknown' };
  }

  return {
    ...emptyWorkspace, ...emptyConversations(), workspaceOwner: null, loaded: false, error: null, epoch: 0,
    newTab(projectId, taskId = null) {
      const tab = { id: crypto.randomUUID(), projectId, taskId, sessionId: null, title: '' };
      set({ tabs: [...get().tabs, tab], activeTabId: tab.id }); return tab;
    },
    openTab(session) {
      if (!session.project_id) return null;
      const existing = get().tabs.find(tab => tab.sessionId === session.id);
      if (existing) { get().activateTab(existing.id); return get().tabs.find(tab => tab.id === existing.id)!; }
      const tab = { id: `session:${session.id}`, projectId: session.project_id, taskId: session.task_id, sessionId: session.id, title: get().sessionNames[session.id] ?? '' };
      set({ tabs: [...get().tabs, tab], activeTabId: tab.id }); return tab;
    },
    activateTab(id) { if (get().tabs.some(tab => tab.id === id)) set({ tabs: get().tabs.map(tab => tab.id === id && tab.hidden ? { ...tab, hidden: false } : tab), activeTabId: id }); },
    bindTab(id, session) {
      if (!session.project_id) return;
      const duplicate = get().tabs.find(tab => tab.sessionId === session.id && tab.id !== id);
      const title = get().tabs.find(tab => tab.id === id)?.title;
      set({ tabs: get().tabs.filter(tab => tab.id !== duplicate?.id).map(tab => tab.id === id ? { ...tab, projectId: session.project_id!, taskId: session.task_id, sessionId: session.id } : tab), sessionNames: title ? { ...get().sessionNames, [session.id]: title } : get().sessionNames });
    },
    closeTab(id) {
      const before = get().tabs, index = before.findIndex(tab => tab.id === id), closing = before[index];
      if (!closing) return;
      const tabs = closing.sessionId ? before.filter(tab => tab.id !== id) : before.map(tab => tab.id === id ? { ...tab, hidden: true } : tab);
      const visible = tabs.filter(tab => !tab.hidden);
      const next = visible.find(tab => before.indexOf(tab) >= index) ?? visible[visible.length - 1];
      set({ tabs, activeTabId: get().activeTabId === id ? next?.id ?? null : get().activeTabId });
    },
    renameTab(id, title) {
      const tab = get().tabs.find(item => item.id === id); if (!tab || !title.trim()) return;
      set({ tabs: get().tabs.map(item => item.id === id ? { ...item, title: title.trim() } : item), sessionNames: tab.sessionId ? { ...get().sessionNames, [tab.sessionId]: title.trim() } : get().sessionNames });
    },
    setReading(id, position) { set({ reading: { ...get().reading, [id]: position } }); },
    setProjectPinned(projectId, pinned) {
      const state = get();
      if (!stringIds([projectId]).length || pinned && !state.projects.some(project => project.id === projectId)) return;
      const pinnedProjectIds = pinned ? [...new Set([...state.pinnedProjectIds, projectId])] : state.pinnedProjectIds.filter(id => id !== projectId);
      set({ pinnedProjectIds });
    },
    archiveSessions(sessionIds) {
      const state = get();
      const runner = useAgentRunner.getState();
      const open = new Set(state.tabs.map(tab => tab.sessionId));
      const existing = new Set(state.sessions.map(session => session.id));
      const eligible = stringIds(sessionIds).filter(id => {
        const snapshot = runner.session?.id === id ? runner.session : runner.snapshots[id];
        return existing.has(id) && id !== state.selection.session_id && !open.has(id)
          && !(runner.live[id] && runner.live[id] !== 'closed')
          && !['starting', 'running'].includes(snapshot?.status ?? '');
      });
      if (eligible.length) set({ archivedSessionIds: [...new Set([...state.archivedSessionIds, ...eligible])] });
    },
    restoreSessions(sessionIds) {
      const restored = new Set(stringIds(sessionIds));
      if (restored.size) set({ archivedSessionIds: get().archivedSessionIds.filter(id => !restored.has(id)) });
    },
    async load(accountName) {
      let epoch = get().epoch;
      let seq = ++loadSeq;
      let selectionAtStart = selectionSeq;
      const result = await call<Workspace>('read_development_workspace', {});
      // Only the newest load may apply, and only within the epoch it started in.
      if (!result.ok || seq !== loadSeq || epoch !== get().epoch) return false;
      const data = result.value;
      // A selection committed while this load was in flight is newer than the value the load read.
      let selection = get().loaded || selectionSeq !== selectionAtStart ? get().selection : data.selection;
      const dataOwner = data.projects[0]?.owner ?? data.sessions[0]?.owner ?? data.tasks[0]?.owner;
      // The account name remains stable when an empty workspace gains its first row.
      const owner = accountName ? `account:${accountName}` : get().workspaceOwner?.startsWith('account:') ? get().workspaceOwner : dataOwner ?? get().workspaceOwner;
      if (get().workspaceOwner && owner !== get().workspaceOwner) {
        get().reset(); epoch = get().epoch; seq = ++loadSeq; selectionAtStart = selectionSeq; selection = data.selection;
      }
      if (owner) await initializeDesktopState(owner);
      if (seq !== loadSeq || epoch !== get().epoch) return false;
      if (selectionSeq !== selectionAtStart) selection = get().selection;
      const ownerChanged = get().workspaceOwner !== owner;
      const restored = owner && ownerChanged && useDesktopPersistence.getState().loaded ? restoreConversations(owner, dataOwner && dataOwner !== owner ? dataOwner : undefined) : null;
      if (restored) {
        const tab = restored.tabs.find(item => item.id === restored.activeTabId);
        selection = tab ? { project_id: tab.projectId, task_id: tab.taskId, session_id: tab.sessionId } : emptySelection;
        ++selectionSeq;
      }
      set({ ...(ownerChanged ? restored ?? emptyConversations() : {}), workspaceOwner: owner, loaded: true, projects: data.projects, tasks: data.tasks, sessions: data.sessions, selection, operations: data.operations });
      return true;
    },
    async createProject(input) {
      const epoch = get().epoch;
      const result = await call<Project>('create_development_project', { input });
      return result.ok ? mutated(epoch, result.value) : result;
    },
    async updateProject(projectId, action, value) {
      const epoch = get().epoch;
      const result = await call<null>('update_development_project', { input: { project_id: projectId, action, value } });
      return result.ok ? mutated(epoch, null) : result;
    },
    async inspect(projectId) {
      return call<ProjectInspection>('inspect_development_project', { projectId });
    },
    async saveTask(input) {
      const epoch = get().epoch;
      const result = await call<DevelopmentTask>('save_development_task', { input });
      return result.ok ? mutated(epoch, result.value) : result;
    },
    async select(selection) {
      const epoch = get().epoch;
      const seq = ++selectionSeq;
      const result = await call<null>('select_development_context', { input: selection });
      // A later select superseded this one, or the account changed: keep whatever is committed now.
      if (!result.ok || epoch !== get().epoch || seq !== selectionSeq) return result.ok ? { ok: false, code: 'unknown' } : result;
      const now = Math.floor(Date.now() / 1000);
      set({ selection, projects: get().projects.map(project => project.id === selection.project_id ? { ...project, last_opened_at: now } : project) });
      return result;
    },
    async classify(sessionId, taskId) {
      const epoch = get().epoch;
      const result = await call<null>('classify_agent_session', { input: { session_id: sessionId, task_id: taskId } });
      return result.ok ? mutated(epoch, null) : result;
    },
    reset() {
      ++loadSeq; ++selectionSeq;
      set({ ...emptyWorkspace, ...emptyConversations(), workspaceOwner: null, loaded: false, error: null, epoch: get().epoch + 1 });
      resetDesktopState();
    },
  };
});

// Presentation metadata only: CLI runtime state, approvals and trust stay host-owned.
useDevelopment.subscribe((state, previous) => {
  if (!previous.loaded && state.loaded && !useDesktopPersistence.getState().loaded) return;
  if (!state.workspaceOwner || state.epoch !== previous.epoch || state.tabs === previous.tabs && state.activeTabId === previous.activeTabId && state.sessionNames === previous.sessionNames && state.reading === previous.reading && state.pinnedProjectIds === previous.pinnedProjectIds && state.archivedSessionIds === previous.archivedSessionIds) return;
  setDesktopSlot('conversations', { tabs: state.tabs, activeTabId: state.activeTabId, sessionNames: state.sessionNames, reading: state.reading, pinnedProjectIds: state.pinnedProjectIds, archivedSessionIds: state.archivedSessionIds });
});
onDesktopStateRestored(() => {
  const owner = useDevelopment.getState().workspaceOwner;
  if (owner) { const restored = restoreConversations(owner); if (restored) useDevelopment.setState(restored); }
});
// Delete the migration source only once the private host copy is confirmed saved.
useDesktopPersistence.subscribe((state, previous) => {
  if (previous.dirty && !state.dirty && state.loaded && !state.error && state.owner) {
    const key = migratedConversationKeys.get(state.owner);
    if (key) try { localStorage.removeItem(key); migratedConversationKeys.delete(state.owner); } catch { /* Legacy copy remains recoverable. */ }
  }
});
