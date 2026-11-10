import { invoke } from '@tauri-apps/api/core';
import type { AgentSession } from '../../stores/agentRunnerStore';

export interface CodexOperationScope { accountKey: string; owner: string; epoch: number; projectId: string; taskId: string | null; sessionId: string; generation: string }
export interface CodexChildAgent { thread_id: string; parent_thread_id: string; status: string; turn_id?: string | null }
export interface CodexBackgroundTerminal { process_id: string; command: string; status: string }
export interface CodexQueueEntry { id: string; text: string }
export interface CodexHistoryTurn { id: string; status: string; items: { id: string; type: string; text?: string | null }[] }
export interface CodexChildHistory { thread_id: string; turns: CodexHistoryTurn[]; incomplete: boolean }
export interface CodexOperationsSnapshot { session_id: string; generation: string; children: CodexChildAgent[]; terminals: CodexBackgroundTerminal[]; queue: CodexQueueEntry[]; turns: CodexHistoryTurn[]; unsupported: string[]; incomplete: boolean }
export type CodexReviewTarget = { type: 'uncommittedChanges' } | { type: 'baseBranch'; branch: string } | { type: 'commit'; sha: string; title?: string } | { type: 'custom'; instructions: string };
export type CodexOperationAction =
  | { kind: 'review'; target: CodexReviewTarget }
  | { kind: 'compact' }
  | { kind: 'fork'; new_session_id: string; last_turn_id?: string }
  | { kind: 'revert'; before_turn_id: string }
  | { kind: 'interrupt_child'; child_thread_id: string; turn_id: string }
  | { kind: 'terminate_terminal'; process_id: string }
  | { kind: 'queue_add'; text: string }
  | { kind: 'send_child'; child_thread_id: string; text: string }
  | { kind: 'resume_child'; child_thread_id: string }
  | { kind: 'close_child'; child_thread_id: string }
  | { kind: 'queue_update'; submission_id: string; text: string }
  | { kind: 'queue_delete'; submission_id: string }
  | { kind: 'queue_reorder'; submission_ids: string[] }
  | { kind: 'queue_start'; submission_id?: string };
export interface CodexOperationReceipt { session_id: string; generation: string; request_id: string; state: 'applied' | 'unknown'; run_id?: string; fork_session_id?: string }
export interface CodexOperationRequest { session_id: string; generation: string; request_id: string; action: CodexOperationAction }
const identity = (scope: CodexOperationScope) => ({ session_id: scope.sessionId, generation: scope.generation });

/** Only these typed host commands cross the renderer boundary; no native RPC or path forwarding. */
export const codexOperationsService = {
  observe: (scope: CodexOperationScope) => invoke<CodexOperationsSnapshot>('observe_codex_operations', { input: identity(scope) }),
  compression: (scope: CodexOperationScope) => invoke<boolean>('observe_agent_compression', { input: identity(scope) }),
  control: (input: CodexOperationRequest) => invoke<CodexOperationReceipt>('control_codex_operation', { input }),
  inspect: (scope: CodexOperationScope, requestId: string) => invoke<CodexOperationReceipt | null>('inspect_codex_operation', { input: { ...identity(scope), request_id: requestId } }),
  session: (scope: CodexOperationScope) => invoke<AgentSession>('get_agent_session', { sessionId: scope.sessionId }),
  history: (scope: CodexOperationScope, childThreadId: string) => invoke<CodexChildHistory>('read_codex_child_history', { input: { ...identity(scope), child_thread_id: childThreadId } }),
};
