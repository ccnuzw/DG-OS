import { create } from 'zustand';
import { useDevelopment } from './developmentStore';
import { useAgentRunner } from './agentRunnerStore';
import { codexOperationsService as service, type CodexChildHistory, type CodexOperationAction, type CodexOperationReceipt, type CodexOperationRequest, type CodexOperationScope, type CodexOperationsSnapshot } from '../components/development/codexOperationsService';

export const codexOperationsKey = (scope: CodexOperationScope) => JSON.stringify([scope.epoch, scope.accountKey, scope.owner, scope.projectId, scope.taskId, scope.sessionId, scope.generation]);
export interface CodexOperationsState {
  snapshot: CodexOperationsSnapshot | null;
  compressionSupported: boolean | null;
  histories: Record<string, CodexChildHistory>;
  busy: boolean;
  reading: boolean;
  stale: boolean;
  pending: CodexOperationRequest | null;
  receipt: CodexOperationReceipt | null;
  error: string | null;
  drafts: Record<string, string>;
}
interface Store {
  entries: Record<string, CodexOperationsState>;
  draft(scope: CodexOperationScope, id: string, value: string): void;
  observe(scope: CodexOperationScope): Promise<void>;
  observeCompression(scope: CodexOperationScope): Promise<void>;
  history(scope: CodexOperationScope, threadId: string): Promise<void>;
  act(scope: CodexOperationScope, action: CodexOperationAction): Promise<CodexOperationReceipt | null>;
  inspect(scope: CodexOperationScope): Promise<CodexOperationReceipt | null>;
  reset(): void;
}
export const emptyCodexOperations: CodexOperationsState = { snapshot: null, compressionSupported: null, histories: {}, busy: false, reading: false, stale: true, pending: null, receipt: null, error: null, drafts: {} };
const safeRefusals: Record<string, true> = { invalid_codex_operation: true, invalid_request_id: true, session_not_found: true, stale_generation: true, session_closed: true, project_archived: true, file_access_denied: true, busy: true, session_busy: true, task_not_found: true, codex_operation_unsupported: true, unsupported_operation: true, engine_rejected: true, method_unsupported: true, codex_child_unobserved: true, not_running: true, codex_terminal_unobserved: true, codex_queue_unobserved: true, codex_queue_empty: true, codex_queue_membership_changed: true, codex_turn_unobserved: true, codex_observation_incomplete: true };
const knownErrors: Record<string, true> = { ...safeRefusals, session_invalid: true, workspace_changed: true, provider_revision_changed: true, provider_binding_unconfirmed: true, capability_revoked: true, storage_failed: true, request_conflict: true };
export const codexOperationError = (error: unknown) => typeof error === 'string' && Object.prototype.hasOwnProperty.call(knownErrors, error) ? error : 'unavailable';
function owned(scope: CodexOperationScope): boolean {
  const state = useDevelopment.getState();
  if (state.epoch !== scope.epoch || state.workspaceOwner !== scope.accountKey) return false;
  const metadata = state.sessions.find(session => session.id === scope.sessionId);
  const project = state.projects.find(item => item.id === scope.projectId && item.owner === scope.owner && !item.archived);
  if (!project || !metadata || metadata.owner !== scope.owner || metadata.project_id !== scope.projectId || metadata.task_id !== scope.taskId) return false;
  const runner = useAgentRunner.getState();
  const runtime = runner.session?.id === scope.sessionId ? runner.session : runner.snapshots[scope.sessionId];
  return !runtime || runtime.generation === scope.generation && runtime.engine === metadata.engine;
}
function validReceipt(scope: CodexOperationScope, requestId: string, receipt: CodexOperationReceipt | null): receipt is CodexOperationReceipt {
  return !!receipt && receipt.session_id === scope.sessionId && receipt.generation === scope.generation && receipt.request_id === requestId && ['applied', 'unknown'].includes(receipt.state);
}
export const useCodexOperations = create<Store>((set, get) => {
  const reads = new Map<string, number>();
  const entry = (scope: CodexOperationScope) => get().entries[codexOperationsKey(scope)] ?? emptyCodexOperations;
  const patch = (scope: CodexOperationScope, value: Partial<CodexOperationsState>) => {
    if (!owned(scope)) return;
    const key = codexOperationsKey(scope);
    set({ entries: { ...get().entries, [key]: { ...entry(scope), ...value } } });
  };
  const applyReceipt = (scope: CodexOperationScope, request: CodexOperationRequest, value: CodexOperationReceipt | null) => {
    if (!owned(scope) || entry(scope).pending !== request || !validReceipt(scope, request.request_id, value)) return null;
    const action = request.action;
    const draftId = action.kind === 'queue_add' ? 'queue:new' : action.kind === 'queue_update' ? `queue:${action.submission_id}` : action.kind === 'send_child' ? `child:${action.child_thread_id}` : null;
    if (value.state === 'applied' && draftId && 'text' in action && entry(scope).drafts[draftId] === action.text) {
      const drafts = { ...entry(scope).drafts };
      delete drafts[draftId];
      patch(scope, { drafts });
    }
    patch(scope, { pending: value.state === 'unknown' ? request : null, receipt: value, stale: true, snapshot: null, histories: {}, error: value.state === 'unknown' ? 'unknown' : null });
    return value;
  };
  return {
    entries: {},
    draft(scope, id, value) { patch(scope, { drafts: { ...entry(scope).drafts, [id]: value } }); },
    async observeCompression(scope) {
      if (!owned(scope) || entry(scope).reading || entry(scope).busy) return;
      const key = codexOperationsKey(scope), sequence = (reads.get(key) ?? 0) + 1;
      reads.set(key, sequence); patch(scope, { reading: true, error: null });
      try {
        const supported = await service.compression(scope);
        if (reads.get(key) !== sequence) return;
        if (typeof supported !== 'boolean') throw new Error('Invalid capability');
        patch(scope, { compressionSupported: supported });
      } catch (error) { if (reads.get(key) === sequence) patch(scope, { compressionSupported: null, error: codexOperationError(error) }); }
      finally { if (reads.get(key) === sequence) patch(scope, { reading: false }); }
    },
    async observe(scope) {
      if (!owned(scope) || entry(scope).reading) return;
      const key = codexOperationsKey(scope), sequence = (reads.get(key) ?? 0) + 1;
      reads.set(key, sequence); patch(scope, { reading: true, stale: true, error: null });
      try {
        const value = await service.observe(scope);
        if (!owned(scope) || reads.get(key) !== sequence) return;
        if (value.session_id !== scope.sessionId || value.generation !== scope.generation || !Array.isArray(value.children) || !Array.isArray(value.terminals) || !Array.isArray(value.queue) || !Array.isArray(value.turns) || !Array.isArray(value.unsupported)) throw new Error('Invalid snapshot');
        patch(scope, { snapshot: value, stale: false, error: null });
        const compressionSupported = await service.compression(scope).catch(() => null);
        if (reads.get(key) === sequence && typeof compressionSupported === 'boolean') patch(scope, { compressionSupported });
      } catch (error) { if (reads.get(key) === sequence) patch(scope, { snapshot: null, stale: true, error: codexOperationError(error) }); }
      finally { if (reads.get(key) === sequence) patch(scope, { reading: false }); }
    },
    async history(scope, threadId) {
      const state = entry(scope);
      if (!owned(scope) || state.reading || state.stale || !state.snapshot?.children.some(child => child.thread_id === threadId)) return;
      const key = codexOperationsKey(scope), sequence = (reads.get(key) ?? 0) + 1;
      reads.set(key, sequence); patch(scope, { reading: true, error: null });
      try {
        const value = await service.history(scope, threadId);
        if (!owned(scope) || reads.get(key) !== sequence) return;
        if (value.thread_id !== threadId || !Array.isArray(value.turns)) throw new Error('Invalid history');
        patch(scope, { histories: { ...entry(scope).histories, [threadId]: value } });
      } catch (error) { if (reads.get(key) === sequence) patch(scope, { error: codexOperationError(error) }); }
      finally { if (reads.get(key) === sequence) patch(scope, { reading: false }); }
    },
    async act(scope, action) {
      if (!owned(scope) || entry(scope).busy || entry(scope).pending) return null;
      const runner = useAgentRunner.getState();
      const runtime = runner.session?.id === scope.sessionId ? runner.session : runner.snapshots[scope.sessionId];
      if (runtime && ['closed', 'starting', 'unknown'].includes(runtime.status)) return null;
      const snapshot = entry(scope).snapshot;
      if (action.kind === 'compact') {
        if (entry(scope).compressionSupported !== true) { patch(scope, { error: 'codex_operation_unsupported' }); return null; }
        if (!runtime || runner.selected !== scope.sessionId || runner.busy || runner.acting || runtime.status !== 'idle' || runtime.active_run || runtime.pending.length || runtime.persistence_error) return null;
      } else {
        if (runtime?.engine !== 'codex') return null;
        if (!snapshot || entry(scope).stale) { patch(scope, { error: 'invalid_codex_operation' }); return null; }
        if (snapshot.unsupported.includes(action.kind)) { patch(scope, { error: 'codex_operation_unsupported' }); return null; }
      }
      const child = 'child_thread_id' in action ? snapshot?.children.find(item => item.thread_id === action.child_thread_id) : null;
      const scopedAction = 'child_thread_id' in action || 'submission_id' in action || action.kind === 'queue_reorder' || action.kind === 'revert' || action.kind === 'terminate_terminal' || action.kind === 'fork' && !!action.last_turn_id;
      if (scopedAction && snapshot?.incomplete
        || 'child_thread_id' in action && !child
        || action.kind === 'interrupt_child' && child?.turn_id !== action.turn_id
        || 'submission_id' in action && action.submission_id !== undefined && !snapshot?.queue.some(item => item.id === action.submission_id)
        || action.kind === 'terminate_terminal' && !snapshot?.terminals.some(item => item.process_id === action.process_id)
        || action.kind === 'revert' && !snapshot?.turns.some(item => item.id === action.before_turn_id)
        || action.kind === 'fork' && action.last_turn_id !== undefined && !snapshot?.turns.some(item => item.id === action.last_turn_id)
        || action.kind === 'queue_reorder' && (!snapshot || action.submission_ids.length !== snapshot.queue.length || new Set(action.submission_ids).size !== snapshot.queue.length || action.submission_ids.some(id => !snapshot.queue.some(item => item.id === id)))) {
        patch(scope, { error: 'invalid_codex_operation' }); return null;
      }
      const request: CodexOperationRequest = { session_id: scope.sessionId, generation: scope.generation, request_id: crypto.randomUUID(), action };
      // Store before crossing IPC. Lost responses and remounts never replay a mutation.
      patch(scope, { busy: true, pending: request, error: null, receipt: null, stale: true, snapshot: null, histories: {} });
      const key = codexOperationsKey(scope); reads.set(key, (reads.get(key) ?? 0) + 1); patch(scope, { reading: false });
      try {
        const value = applyReceipt(scope, request, await service.control(request));
        if (!value && owned(scope)) patch(scope, { error: 'unknown' });
        return value;
      } catch (error) {
        const code = codexOperationError(error);
        patch(scope, { error: Object.prototype.hasOwnProperty.call(safeRefusals, code) ? code : 'unknown', ...(Object.prototype.hasOwnProperty.call(safeRefusals, code) ? { pending: null } : {}) });
        return null;
      }
      finally { patch(scope, { busy: false }); }
    },
    async inspect(scope) {
      const request = entry(scope).pending;
      if (!owned(scope) || !request || entry(scope).busy) return null;
      patch(scope, { busy: true });
      try {
        const value = applyReceipt(scope, request, await service.inspect(scope, request.request_id));
        if (!value) patch(scope, { error: 'unknown' });
        return value;
      } catch (error) {
        const code = codexOperationError(error);
        patch(scope, { error: Object.prototype.hasOwnProperty.call(safeRefusals, code) ? code : 'unknown', ...(Object.prototype.hasOwnProperty.call(safeRefusals, code) ? { pending: null } : {}) });
        return null;
      }
      finally { patch(scope, { busy: false }); }
    },
    reset() { reads.clear(); set({ entries: {} }); },
  };
});
useDevelopment.subscribe((state, previous) => { if (state.epoch !== previous.epoch || state.workspaceOwner !== previous.workspaceOwner) useCodexOperations.getState().reset(); });
