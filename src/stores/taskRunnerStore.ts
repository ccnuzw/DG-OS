import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../i18n';
import { handleError } from '../ui/errors/handleError';
import type { GenerationParameters } from '../components/chat/GenerationControls';

export interface TaskRecord {
  task: { id: string; status: string; provider_id: string; model_id: string; created_at: number; error_code?: string; total_tokens?: number };
  skills: { skill_id: string; version_id: string; content_hash: string }[];
  tools?: { server_id: string; tool_name: string; schema_hash: string }[];
}
export interface TaskOutcome { record: TaskRecord; content?: string; audit_warning?: boolean }
export interface ToolProposal { id: string; server_id: string; server_name: string; tool_name: string; schema_hash: string; arguments: unknown; expires_at: number }
export interface ToolTaskUpdate extends TaskOutcome { phase: string; proposal?: ToolProposal }
export interface TaskInput { provider_id: string; model_id: string; prompt: string; skill_ids: string[]; generation: GenerationParameters; allow_without_skill: boolean }
export interface TaskTool { server_id: string; tool_name: string; schema_hash: string }
export interface TaskRunnerState {
  epoch: number;
  taskId: string | null;
  kind: 'text' | 'tool' | null;
  running: boolean;
  cancelPending: boolean;
  resultUnknown: boolean;
  error: string | null;
  outcome: TaskOutcome | null;
  toolTask: ToolTaskUpdate | null;
  run(input: TaskInput, tools: TaskTool[]): Promise<void>;
  decide(type: 'approve' | 'deny'): Promise<void>;
  cancel(): Promise<void>;
  verify(): Promise<void>;
  reset(): void;
}
export const useTaskRunner = create<TaskRunnerState>((set, get) => {
  let attempt = 0;
  let cancelIntent = false;
  let terminalSeen = false;
  const current = (ticket: number, epoch: number) => ticket === attempt && epoch === get().epoch;
  const remember = (outcome: TaskOutcome) => {
    const terminal = outcome.record.task.status !== 'running';
    if (terminalSeen && !terminal) return;
    if (terminal) terminalSeen = true;
    set({ outcome, toolTask: 'phase' in outcome ? outcome as ToolTaskUpdate : null, taskId: terminal ? null : outcome.record.task.id,
      running: terminal ? false : get().running, resultUnknown: false,
      error: outcome.record.task.status !== 'running' && outcome.record.task.status !== 'succeeded' ? `${t('workbench.taskFailedPrefix', { status: outcome.record.task.status, code: outcome.record.task.error_code || 'unknown_error' })} ${t(outcome.record.task.error_code === 'mcp_unknown_result' ? 'workbench.toolUnknownResult' : outcome.record.task.error_code === 'mcp_audit_warning' ? 'workbench.toolAuditIncomplete' : 'workbench.noSkillRetry')} ${t('workbench.externalCancellationWarning')}` : null });
  };
  return {
    epoch: 0, taskId: null, kind: null, running: false, cancelPending: false, resultUnknown: false, error: null, outcome: null, toolTask: null,
    async run(input, tools) {
      if (get().taskId || get().running) return;
      const ticket = ++attempt;
      const epoch = get().epoch;
      const id = crypto.randomUUID();
      cancelIntent = false;
      terminalSeen = false;
      set({ taskId: id, kind: tools.length ? 'tool' : 'text', running: true, error: null, outcome: null, toolTask: null, resultUnknown: false });
      try {
        if (tools.length) {
          const started = await invoke<ToolTaskUpdate>('start_workbench_tool_task', { input: { task: { ...input, client_task_id: id }, tools } });
          if (!current(ticket, epoch)) return;
          remember(started);
          if (started.record.task.status !== 'running') return;
          if (cancelIntent) { await get().cancel(); return; }
          const result = await invoke<ToolTaskUpdate>('advance_workbench_tool_task', { taskId: started.record.task.id, action: { type: 'run_model' } });
          if (current(ticket, epoch) && get().taskId) remember(result);
        } else {
          const result = await invoke<TaskOutcome>('run_workbench_task', { input: { ...input, client_task_id: id } });
          if (current(ticket, epoch)) remember(result);
        }
      } catch (failure) {
        if (current(ticket, epoch)) { handleError(failure, { where: 'Workbench' }); set({ error: t('workbench.taskNotCompleted', { error: t('workbench.noSkillRetry') }), resultUnknown: true }); await get().verify(); }
      } finally { if (current(ticket, epoch)) set({ running: false }); }
    },
    async decide(type) {
      const { toolTask, running, cancelPending, epoch } = get();
      if (!toolTask?.proposal || running || cancelPending || Date.now() >= toolTask.proposal.expires_at * 1000 || !get().taskId) return;
      const ticket = attempt;
      set({ running: true, error: null });
      try {
        const result = await invoke<ToolTaskUpdate>('advance_workbench_tool_task', { taskId: toolTask.record.task.id, action: { type, proposal_id: toolTask.proposal.id } });
        if (current(ticket, epoch) && get().taskId) remember(result);
      } catch { if (current(ticket, epoch)) set({ error: t('workbench.taskNotRunning'), resultUnknown: true }); }
      finally { if (current(ticket, epoch)) set({ running: false }); }
    },
    async cancel() {
      const { taskId, kind, epoch, cancelPending } = get();
      if (!taskId || cancelPending) return;
      cancelIntent = true;
      const ticket = attempt;
      set({ cancelPending: true, error: null });
      try {
        if (kind === 'tool') {
          const result = await invoke<ToolTaskUpdate>('cancel_workbench_tool_task', { taskId });
          if (current(ticket, epoch) && get().taskId === taskId) remember(result);
        } else {
          await invoke<boolean>('cancel_workbench_task', { taskId });
          if (current(ticket, epoch)) await get().verify();
        }
      } catch (failure) { if (current(ticket, epoch)) { handleError(failure, { where: 'Workbench' }); set({ error: t('workbench.cancellationUnconfirmed', { error: t('workbench.taskNotRunning') }), resultUnknown: true }); } }
      finally { if (current(ticket, epoch)) set({ cancelPending: false }); }
    },
    async verify() {
      const { taskId, epoch } = get();
      if (!taskId) return;
      const ticket = attempt;
      try {
        const records = await invoke<TaskRecord[]>('list_workbench_tasks', {});
        if (!current(ticket, epoch) || get().taskId !== taskId) return;
        const record = records.find(row => row.task.id === taskId);
        if (record && record.task.status !== 'running') remember({ record });
        else set({ resultUnknown: true });
      } catch { if (current(ticket, epoch)) set({ resultUnknown: true }); }
    },
    reset() { ++attempt; terminalSeen = false; cancelIntent = false; set({ epoch: get().epoch + 1, taskId: null, kind: null, running: false, cancelPending: false, resultUnknown: false, error: null, outcome: null, toolTask: null }); },
  };
});
