import type { ModelCapabilities } from '../stores/providerStore';
// D136 wire contract. No caller identity, credentials, lease or approval token is supplied by a module.
export interface AiSkillRef { skill_id: string; version: string; content_hash: string }
export interface AiToolSelection { server_id: string; tool_name: string; schema_hash: string }
export type AiSkillMode = 'manual' | 'on_demand';
export type AiContextMode = 'inline' | 'on_demand';
export interface AiContextRef { resource_id: string; sha256: string; start: number; max_bytes: number; mode: AiContextMode }
export interface AiResourceImport { name: string; mime: string; base64: string }
export interface AiResource { id: string; name: string; mime: string; bytes_len: number; sha256: string; expires_at: number }
/** Validated after generation by the host; not a Provider-native guarantee. */
export type AiOutputContract = { kind: 'text' } | { kind: 'json_schema'; schema: unknown };
export type AiOperation = 'generate' | 'summarize';
export interface AiFoundationOptions {
  skill_mode: AiSkillMode; context_refs: AiContextRef[]; output: AiOutputContract;
  retain_result: boolean; source_run_id: string | null; operation: AiOperation; confirmed: boolean;
}
export function defaultAiFoundation(): AiFoundationOptions {
  return { skill_mode: 'manual', context_refs: [], output: { kind: 'text' }, retain_result: false, source_run_id: null, operation: 'generate', confirmed: false };
}
export interface AiContextReceipt { resource_id: string; sha256: string; start: number; bytes: number; total_bytes: number }
export interface AiRunContent {
  run_id: string; content: string | null; status: AiRunStatus | 'expired'; output_valid: boolean;
  expires_at: number; acknowledged_at: number | null; source_run_id: string | null;
  context_receipts: AiContextReceipt[]; tool_receipts: unknown; skill_receipts: unknown;
}
export interface AiContextBudget {
  text_bytes: number; image_reserve_tokens: number; tool_schema_bytes: number; host_text_limit_bytes: number;
  model_window_tokens: number | null; reserved_output_tokens: number; estimated_input_tokens: number;
  can_send: boolean; rejection: string | null;
}
export interface AiRunInput {
  provider_id: string;
  model_id: string;
  provider_revision: number;
  prompt: string;
  skill_refs: AiSkillRef[];
  tools: AiToolSelection[];
  generation: { temperature?: number | null; reasoning?: { effort?: string | null; budget_tokens?: number | null } | null; max_tokens?: number | null; instruction?: string };
  foundation?: AiFoundationOptions;
}
export type AiRunStatus = 'running' | 'succeeded' | 'failed' | 'cancelled' | 'interrupted';
export interface AiFailure { domain: string; code: number; reason: string; result_unknown: boolean }
export interface AiRunSummary {
  run_id: string; request_id: string; provider_id: string; model_id: string;
  created_at: number; finished_at: number | null; status: AiRunStatus;
  error: AiFailure | null; result_unknown: boolean;
}
export interface AiRunListInput { before?: string | null; limit?: number | null }
export interface AiRunPage { items: AiRunSummary[]; next_before: string | null }
export interface AiTerminal {
  status: AiRunStatus; content: string | null; error: AiFailure | null;
  result_unknown: boolean; audit_warning: boolean; total_tokens: number | null;
}
export type AiEvent = { run_id: string; request_id: string; seq: number } & (
  | { kind: 'started'; provider_revision: number; skill_refs: AiSkillRef[]; tools: AiToolSelection[] }
  | { kind: 'delta' | 'reasoning_delta'; text: string }
  | { kind: 'model_segment'; round: number }
  | { kind: 'phase'; phase: 'model' | 'awaiting_confirmation' | 'tool' | 'processing_result' | 'final_answer' }
  | { kind: 'terminal'; terminal: AiTerminal }
);
export interface AiModelEntry { model_id: string; display_name: string; kind: string; capabilities: ModelCapabilities; context_window: number | null }
export interface AiProviderEntry { provider_id: string; display_name: string; config_revision: number; models: AiModelEntry[] }
export interface AiSkillEntry extends AiSkillRef { name: string }
export interface AiToolEntry extends AiToolSelection { server_name: string; description: string | null; input_schema: unknown }
