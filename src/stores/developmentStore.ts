import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { Engine } from './agentRunnerStore';
import { hostCode } from './hostError';

export type TaskStatus = 'todo' | 'in_progress' | 'blocked' | 'review' | 'completed' | 'cancelled';
export const TASK_STATUSES: readonly TaskStatus[] = ['todo', 'in_progress', 'blocked', 'review', 'completed', 'cancelled'];
export interface Project { id: string; owner: string; name: string; path: string; directory_identity: string; archived: boolean; created_at: number; last_opened_at: number | null }
export interface DevelopmentTask { id: string; owner: string; project_id: string; goal: string; requirements: string; status: TaskStatus; created_at: number }
export interface DevelopmentSession { id: string; owner: string; engine: Engine; native_id: string | null; workspace: string; executable: string | null; model: string | null; project_id: string | null; task_id: string | null }
export interface Selection { project_id: string | null; task_id: string | null; session_id: string | null }
export interface ProjectOperation { id: string; owner: string; kind: 'create' | 'clone'; path: string; status: 'pending' | 'succeeded' | 'failed' | 'unknown'; project_id: string | null }
export type PathState = 'available' | 'missing' | 'changed' | 'inaccessible';
export interface ProjectInspection { path_state: PathState; kind: 'directory' | 'git' | 'unknown'; repository_root: string | null; branch: string | null; changes: string[]; truncated: boolean; error: string | null }
export interface CreateProjectInput { kind: 'import' | 'create' | 'clone'; name: string; path: string; source: string | null }
export type ProjectAction = 'rename' | 'archive' | 'restore' | 'unlink' | 'relocate';
export interface SaveTaskInput { id: string | null; project_id: string; goal: string; requirements: string; status: TaskStatus }
export type Outcome<T> = { ok: true; value: T } | { ok: false; code: string };
interface Workspace { projects: Project[]; tasks: DevelopmentTask[]; sessions: DevelopmentSession[]; selection: Selection; operations: ProjectOperation[] }

interface DevelopmentState extends Workspace {
  loaded: boolean; error: string | null;
  /** Bumps on account reset. Every async result is compared against it before it may touch state. */
  epoch: number;
  load(): Promise<boolean>;
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
    ...emptyWorkspace, loaded: false, error: null, epoch: 0,
    async load() {
      const epoch = get().epoch;
      const seq = ++loadSeq;
      const selectionAtStart = selectionSeq;
      const result = await call<Workspace>('read_development_workspace', {});
      // Only the newest load may apply, and only within the epoch it started in.
      if (!result.ok || seq !== loadSeq || epoch !== get().epoch) return false;
      const data = result.value;
      // A selection committed while this load was in flight is newer than the value the load read.
      const selection = selectionSeq === selectionAtStart ? data.selection : get().selection;
      set({ loaded: true, projects: data.projects, tasks: data.tasks, sessions: data.sessions, selection, operations: data.operations });
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
      set({ ...emptyWorkspace, loaded: false, error: null, epoch: get().epoch + 1 });
    },
  };
});
