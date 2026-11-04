import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment, type Outcome } from './developmentStore';
import { contextGeneration } from './developmentSelection';
import { hostCode } from './hostError';

export interface DevelopmentCommand { id: string; name: string; program: string; args: string[]; cwd: string; env_refs: Record<string, string> }
export interface CommandCatalog { commands: DevelopmentCommand[]; discovered: DevelopmentCommand[] }
export interface ReadinessInput { port: number; path: string; timeout_ms: number }
export interface StartProcessInput { id: string; task_id: string; kind: 'terminal' | 'command' | 'service'; program: string; args: string[]; cwd: string; env_refs: Record<string, string>; trust_workspace: boolean; verification: boolean; readiness: ReadinessInput | null; native_run_id?: string | null }
export interface DevelopmentProcess {
  id: string; project_id: string; task_id: string; kind: string; source: string; status: string; output: string; truncated: boolean;
  exit_code: number | null; reason: string | null; started_at: number; finished_at: number | null;
  program: string; args: string[]; cwd: string; fingerprint: string; start_fingerprint: string;
  verification_status: string; verification_complete: boolean; mutation_observed: boolean; preview_url: string | null;
  native_run_id?: string | null;
}
export async function runtimeCall<T>(command: string, args: Record<string, unknown>): Promise<Outcome<T>> {
  try { return { ok: true, value: await invoke<T>(command, args) }; }
  catch (error) { return { ok: false, code: hostCode(error) }; }
}
export const readDevelopmentCommands = (projectId: string) => runtimeCall<CommandCatalog>('read_development_commands', { projectId });
export const saveDevelopmentCommands = (projectId: string, commands: DevelopmentCommand[]) => runtimeCall<CommandCatalog>('save_development_commands', { input: { project_id: projectId, commands } });
export const startDevelopmentProcess = (input: StartProcessInput) => runtimeCall<DevelopmentProcess>('start_development_process', { input });
export const readDevelopmentProcess = (id: string) => runtimeCall<DevelopmentProcess>('read_development_process', { id });
export const stopDevelopmentProcess = (id: string) => runtimeCall<DevelopmentProcess>('stop_development_process', { id });
export const writeDevelopmentTerminal = (id: string, data: string) => runtimeCall<null>('write_development_terminal', { input: { id, data } });
export const resizeDevelopmentTerminal = (id: string, rows: number, cols: number) => runtimeCall<null>('resize_development_terminal', { input: { id, rows, cols } });
export const openDevelopmentExternal = (projectId: string, target: 'preview' | 'editor', processId: string | null = null, editor: string | null = null) => runtimeCall<null>('open_development_external', { input: { project_id: projectId, process_id: processId, target, editor } });
export function processActive(process: DevelopmentProcess): boolean { return process.finished_at === null || ['cleanup_pending', 'starting', 'running', 'ready'].includes(process.status); }

interface RuntimeState { taskId: string | null; processes: DevelopmentProcess[]; selected: string | null; error: string | null; refresh(taskId: string): Promise<void>; select(id: string): void; reset(): void }
let request = 0;
export const useDevelopmentRuntime = create<RuntimeState>((set, get) => ({
  taskId: null, processes: [], selected: null, error: null,
  async refresh(taskId) {
    const ticket = ++request;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    if (get().taskId !== taskId) set({ taskId, processes: [], selected: null, error: null });
    const result = await runtimeCall<DevelopmentProcess[]>('list_development_processes', { taskId });
    if (ticket !== request || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    if (result.ok) set({ processes: result.value, error: null, selected: result.value.some(item => item.id === get().selected) ? get().selected : result.value[0]?.id ?? null });
    else set({ error: result.code });
  },
  select(selected) { set({ selected }); },
  reset() { ++request; set({ taskId: null, processes: [], selected: null, error: null }); },
}));
useDevelopment.subscribe((state, previous) => { if (state.epoch !== previous.epoch) useDevelopmentRuntime.getState().reset(); });
