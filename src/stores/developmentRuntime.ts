import { useCallback, type SetStateAction } from 'react';
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment, type Outcome } from './developmentStore';
import { contextGeneration } from './developmentSelection';
import { hostCode } from './hostError';
import { beginResourceRead, failResourceRead, type ResourceReadState } from '../resourceState';

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
interface RuntimeDraft {
  dirty: boolean; commandId: string | null; name: string; editorApplication: string; program: string; args: string[]; cwd: string;
  env: { target: string; source: string }[]; kind: StartProcessInput['kind']; verification: boolean;
  port: string; probePath: string; timeout: string;
}
const emptyDraft: RuntimeDraft = { dirty: false, commandId: null, name: '', editorApplication: '', program: '', args: [], cwd: '', env: [], kind: 'command', verification: false, port: '', probePath: '/', timeout: '10000' };
// Only user configuration survives panel disposal; trust is intentionally view-local.
export function useRuntimeDraft(projectId: string, taskId: string) {
  const key = JSON.stringify([useDevelopment(state => state.epoch), projectId, taskId]);
  const draft = useDevelopmentRuntime(state => state.drafts[key] ?? emptyDraft);
  const setField = useCallback(<K extends keyof RuntimeDraft>(field: K, value: SetStateAction<RuntimeDraft[K]>) => {
    useDevelopmentRuntime.setState(state => {
      const current = state.drafts[key] ?? emptyDraft;
      return { drafts: { ...state.drafts, [key]: { ...current, dirty: field === 'dirty' ? Boolean(value) : field === 'commandId' || field === 'editorApplication' ? current.dirty : true, [field]: typeof value === 'function' ? (value as (previous: RuntimeDraft[K]) => RuntimeDraft[K])(current[field]) : value } } };
    });
  }, [key]);
  return { draft, setField };
}
export async function runtimeCall<T>(command: string, args: Record<string, unknown>): Promise<Outcome<T>> {
  try { return { ok: true, value: await invoke<T>(command, args) }; }
  catch (error) { return { ok: false, code: hostCode(error) }; }
}
const commandReads = new Map<string, Promise<Outcome<CommandCatalog>>>();
export function readDevelopmentCommands(taskId: string): Promise<Outcome<CommandCatalog>> {
  const key = JSON.stringify([useDevelopment.getState().epoch, taskId]);
  const existing = commandReads.get(key);
  if (existing) return existing;
  const reading = runtimeCall<CommandCatalog>('read_development_commands', { taskId }).finally(() => {
    if (commandReads.get(key) === reading) commandReads.delete(key);
  });
  commandReads.set(key, reading);
  return reading;
}
export const saveDevelopmentCommands = (taskId: string, commands: DevelopmentCommand[]) => runtimeCall<CommandCatalog>('save_development_commands', { input: { task_id: taskId, commands } });
export const startDevelopmentProcess = (input: StartProcessInput) => runtimeCall<DevelopmentProcess>('start_development_process', { input });
// Outstanding reads only: shared across internal panel remounts, never a second snapshot cache.
const detailReads = new Map<string, Promise<Outcome<DevelopmentProcess>>>();
let observation = 0;
export function readDevelopmentProcess(id: string): Promise<Outcome<DevelopmentProcess>> {
  const key = JSON.stringify([useDevelopment.getState().epoch, contextGeneration(), id]);
  const existing = detailReads.get(key);
  if (existing) return existing;
  const epoch = useDevelopment.getState().epoch;
  const generation = contextGeneration();
  const started = ++observation;
  const reading = runtimeCall<DevelopmentProcess>('read_development_process', { id }).then(result => {
    if (result.ok && result.value.id === id && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) reconcileProcesses([result.value], started);
    return result;
  }).finally(() => {
    if (detailReads.get(key) === reading) detailReads.delete(key);
  });
  detailReads.set(key, reading);
  return reading;
}
export const stopDevelopmentProcess = (id: string) => runtimeCall<DevelopmentProcess>('stop_development_process', { id });
export const writeDevelopmentTerminal = (id: string, data: string) => runtimeCall<null>('write_development_terminal', { input: { id, data } });
export const resizeDevelopmentTerminal = (id: string, rows: number, cols: number) => runtimeCall<null>('resize_development_terminal', { input: { id, rows, cols } });
export const openDevelopmentExternal = (projectId: string, taskId: string, target: 'preview' | 'editor', processId: string | null = null, editor: string | null = null) => runtimeCall<null>('open_development_external', { input: { project_id: projectId, task_id: taskId, process_id: processId, target, editor } });
// A timestamp alone cannot establish cleanup or an outcome after host recovery.
export function processActive(process: DevelopmentProcess): boolean { return !(process.finished_at !== null && ['exited', 'failed', 'cancelled'].includes(process.status)); }

export type RuntimeOperationPhase = 'pending' | 'confirming' | 'accepted' | 'succeeded' | 'failed' | 'unknown';
export interface RuntimeOperation {
  id: string; epoch: number; projectId: string; taskId: string; action: 'save' | 'start' | 'stop'; targetId: string;
  phase: RuntimeOperationPhase; error?: string; refreshError?: string; refreshing?: boolean; observedAfter?: number; selectionRevision?: number; generation?: number;
  command?: DevelopmentCommand; commands?: DevelopmentCommand[]; catalog?: CommandCatalog; process?: DevelopmentProcess;
}
type RuntimeOperationInput = { projectId: string; taskId: string } & (
  { action: 'save'; command: DevelopmentCommand; commands: DevelopmentCommand[] } |
  { action: 'start'; input: StartProcessInput } | { action: 'stop'; processId: string }
);
const rejectedBeforeDispatch: Record<string, true> = { invalid_command: true, invalid_readiness: true, invalid_request: true, invalid_request_id: true, secret_argument_denied: true, workspace_untrusted: true, project_archived: true, request_conflict: true, process_capacity: true, workspace_busy: true, maintenance_active: true };
const sameCommand = (left: DevelopmentCommand, right: DevelopmentCommand) => left.id === right.id && left.name === right.name && left.program === right.program && left.cwd === right.cwd && JSON.stringify(left.args) === JSON.stringify(right.args) && Object.keys(left.env_refs).length === Object.keys(right.env_refs).length && Object.entries(left.env_refs).every(([key, value]) => right.env_refs[key] === value);
export function runtimeOperationBlocked(operations: RuntimeOperation[], projectId: string, taskId: string, action: RuntimeOperation['action']): boolean {
  return operations.some(operation => operation.epoch === useDevelopment.getState().epoch && operation.projectId === projectId && (['pending', 'confirming', 'unknown'].includes(operation.phase) || operation.action === 'stop' && operation.phase === 'accepted' || operation.refreshing) && (operation.taskId === taskId || operation.action === 'save' && action === 'save'));
}
function updateOperation(id: string, patch: Partial<RuntimeOperation>) {
  useDevelopmentRuntime.setState(state => ({ operations: state.operations.map(operation => operation.id === id ? { ...operation, ...patch } : operation) }));
}
function processReceipt(operation: RuntimeOperation, process: DevelopmentProcess, acknowledged = false): Partial<RuntimeOperation> {
  const phase = operation.action === 'start' ? process.status === 'unknown' ? 'unknown' : 'accepted' : !processActive(process) ? 'succeeded' : process.status === 'unknown' || !acknowledged && operation.phase !== 'accepted' ? 'unknown' : 'accepted';
  return { process, phase, error: phase === 'unknown' ? process.reason ?? 'unknown' : undefined };
}
function reconcileProcesses(processes: DevelopmentProcess[], started: number) {
  useDevelopmentRuntime.setState(state => ({ operations: state.operations.map(operation => {
    if (operation.epoch !== useDevelopment.getState().epoch || operation.action === 'save' || operation.action === 'start' && operation.phase === 'accepted' || !['unknown', 'accepted', 'confirming'].includes(operation.phase) || started <= (operation.observedAfter ?? Infinity)) return operation;
    const process = processes.find(item => item.id === operation.targetId && item.project_id === operation.projectId && item.task_id === operation.taskId);
    return process ? { ...operation, ...processReceipt(operation, process) } : operation;
  }) }));
}
async function acceptProcess(operation: RuntimeOperation, process: DevelopmentProcess, acknowledged = false) {
  if (operation.epoch !== useDevelopment.getState().epoch) return;
  if (process.id !== operation.targetId || process.project_id !== operation.projectId || process.task_id !== operation.taskId) { updateOperation(operation.id, { phase: 'unknown', error: 'target_mismatch' }); return; }
  updateOperation(operation.id, { ...processReceipt(operation, process, acknowledged), observedAfter: ++observation });
  const current = useDevelopmentRuntime.getState();
  if (current.taskId !== operation.taskId) return;
  const selectStarted = acknowledged && operation.action === 'start' && operation.selectionRevision === selectionRevision && operation.generation === contextGeneration();
  useDevelopmentRuntime.setState({ processes: [process, ...current.processes.filter(item => item.id !== process.id)], selected: selectStarted ? process.id : current.selected ?? process.id });
  await refreshOperation(operation.id);
}
async function refreshOperation(id: string) {
  const operation = useDevelopmentRuntime.getState().operations.find(item => item.id === id);
  if (!operation || operation.epoch !== useDevelopment.getState().epoch || operation.refreshing || useDevelopmentRuntime.getState().taskId !== operation.taskId) return;
  updateOperation(id, { refreshing: true });
  await useDevelopmentRuntime.getState().refresh(operation.taskId, true);
  if (operation.epoch !== useDevelopment.getState().epoch) return;
  const current = useDevelopmentRuntime.getState();
  const read = current.read;
  updateOperation(id, { refreshing: false, refreshError: current.taskId !== operation.taskId ? undefined : read.status === 'error' ? read.error : read.status === 'ready' && read.refresh.status === 'error' ? read.refresh.error : undefined });
}
export async function retryRuntimeOperation(id: string): Promise<void> {
  const operation = useDevelopmentRuntime.getState().operations.find(item => item.id === id);
  if (!operation || operation.epoch !== useDevelopment.getState().epoch || ['pending', 'confirming'].includes(operation.phase)) return;
  if (operation.phase === 'succeeded' || operation.action === 'start' && operation.phase === 'accepted') { await refreshOperation(id); return; }
  if (operation.phase !== 'unknown' && operation.phase !== 'accepted') return;
  updateOperation(id, { phase: 'confirming' });
  if (operation.action === 'save') {
    // Confirmation must not reuse a snapshot begun before the save.
    await commandReads.get(JSON.stringify([operation.epoch, operation.taskId]));
    if (operation.epoch !== useDevelopment.getState().epoch) return;
    const result = await readDevelopmentCommands(operation.taskId);
    if (operation.epoch !== useDevelopment.getState().epoch) return;
    const confirmed = result.ok && operation.commands?.length === result.value.commands.length && operation.commands.every(command => result.value.commands.some(item => sameCommand(item, command)));
    updateOperation(id, { phase: confirmed ? 'succeeded' : 'unknown', catalog: result.ok ? result.value : undefined, error: result.ok ? undefined : result.code });
    if (confirmed) mergeSavedCommandIdentity(operation);
  } else {
    // A stop may finish while an older detail is in flight; only a new read can confirm it.
    await detailReads.get(JSON.stringify([operation.epoch, contextGeneration(), operation.targetId]));
    if (operation.epoch !== useDevelopment.getState().epoch) return;
    const result = await readDevelopmentProcess(operation.targetId);
    if (operation.epoch !== useDevelopment.getState().epoch) return;
    if (result.ok) await acceptProcess(operation, result.value);
    else updateOperation(id, { phase: 'unknown', error: result.code });
  }
}
function mergeSavedCommandIdentity(operation: RuntimeOperation) {
  if (!operation.command || operation.epoch !== useDevelopment.getState().epoch) return;
  const key = JSON.stringify([operation.epoch, operation.projectId, operation.taskId]);
  useDevelopmentRuntime.setState(state => {
    const draft = state.drafts[key];
    if (!draft || draft.commandId !== operation.targetId) return state;
    const unchanged = sameCommand({ id: operation.targetId, name: draft.name, program: draft.program, args: draft.args, cwd: draft.cwd, env_refs: Object.fromEntries(draft.env.map(item => [item.target, item.source])) }, operation.command!);
    return { drafts: { ...state.drafts, [key]: { ...draft, commandId: operation.targetId, dirty: !unchanged } } };
  });
}
export async function submitRuntimeOperation(input: RuntimeOperationInput): Promise<RuntimeOperation | null> {
  const epoch = useDevelopment.getState().epoch;
  const targetId = input.action === 'save' ? input.command.id : input.action === 'start' ? input.input.id : input.processId;
  if (runtimeOperationBlocked(useDevelopmentRuntime.getState().operations, input.projectId, input.taskId, input.action)) return null;
  const operation: RuntimeOperation = { id: crypto.randomUUID(), epoch, projectId: input.projectId, taskId: input.taskId, action: input.action, targetId, phase: 'pending', observedAfter: ++observation, selectionRevision, generation: contextGeneration(), ...(input.action === 'save' ? { command: input.command, commands: input.commands } : {}) };
  useDevelopmentRuntime.setState(state => ({ operations: [...state.operations.filter(item => item.projectId !== input.projectId || item.taskId !== input.taskId || item.action !== input.action || !['succeeded', 'failed', 'accepted'].includes(item.phase)), operation] }));
  const result = input.action === 'save' ? await saveDevelopmentCommands(input.taskId, input.commands) : input.action === 'start' ? await startDevelopmentProcess(input.input) : await stopDevelopmentProcess(input.processId);
  if (epoch !== useDevelopment.getState().epoch) return null;
  if (result.ok) {
    if (input.action === 'save') { updateOperation(operation.id, { phase: 'succeeded', catalog: result.value as CommandCatalog }); mergeSavedCommandIdentity(operation); }
    else await acceptProcess(operation, result.value as DevelopmentProcess, true);
  } else {
    const failed = input.action !== 'stop' && rejectedBeforeDispatch[result.code] === true;
    updateOperation(operation.id, { phase: failed ? 'failed' : 'unknown', error: result.code, observedAfter: ++observation });
    if (!failed) await retryRuntimeOperation(operation.id);
  }
  return useDevelopmentRuntime.getState().operations.find(item => item.id === operation.id) ?? null;
}

interface RuntimeState { taskId: string | null; processes: DevelopmentProcess[]; selected: string | null; operations: RuntimeOperation[]; drafts: Record<string, RuntimeDraft>; read: ResourceReadState; refresh(taskId: string, afterMutation?: boolean): Promise<void>; select(id: string): void; reset(): void }
let request = 0;
let selectionRevision = 0;
const historyReads = new Map<string, Promise<Outcome<DevelopmentProcess[]>>>();
const historyObservations = new WeakMap<Promise<Outcome<DevelopmentProcess[]>>, number>();
export const useDevelopmentRuntime = create<RuntimeState>((set, get) => ({
  taskId: null, processes: [], selected: null, operations: [], drafts: {}, read: { status: 'idle' },
  async refresh(taskId, afterMutation = false) {
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    if (get().taskId !== taskId) { ++request; ++selectionRevision; set({ taskId, processes: [], selected: null, read: { status: 'idle' } }); }
    const ticket = request;
    const key = JSON.stringify([epoch, generation, taskId]);
    let reading = historyReads.get(key);
    // A start/stop must observe a read begun after the mutation, not reuse its predecessor.
    if (afterMutation && reading) {
      await reading;
      if (ticket !== request || get().taskId !== taskId || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      reading = historyReads.get(key);
    }
    if (!reading) {
      reading = runtimeCall<DevelopmentProcess[]>('list_development_processes', { taskId });
      historyReads.set(key, reading);
      historyObservations.set(reading, ++observation);
      void reading.finally(() => { if (historyReads.get(key) === reading) historyReads.delete(key); });
    }
    set({ read: beginResourceRead(get().read) });
    const result = await reading;
    if (ticket !== request || get().taskId !== taskId || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    if (result.ok) {
      const processes = result.value.filter(item => item.task_id === taskId);
      reconcileProcesses(processes, historyObservations.get(reading) ?? 0);
      set({ processes, read: { status: 'ready', refresh: { status: 'current' } }, selected: processes.some(item => item.id === get().selected) ? get().selected : processes[0]?.id ?? null });
    }
    else set({ read: failResourceRead(get().read, result.code) });
  },
  select(selected) { ++selectionRevision; set({ selected }); },
  reset() { ++request; ++selectionRevision; set({ taskId: null, processes: [], selected: null, read: { status: 'idle' } }); },
}));
useDevelopment.subscribe((state, previous) => { if (state.epoch !== previous.epoch) { useDevelopmentRuntime.getState().reset(); useDevelopmentRuntime.setState({ operations: [], drafts: {} }); } });
