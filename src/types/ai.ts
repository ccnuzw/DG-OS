// D136 wire contract. No caller identity, credentials, lease or approval token is supplied by a module.
export interface AiSkillRef { skill_id: string; version: string; content_hash: string }
export interface AiToolSelection { server_id: string; tool_name: string; schema_hash: string }
export interface AiRunInput {
  provider_id: string;
  model_id: string;
  provider_revision: number;
  prompt: string;
  skill_refs: AiSkillRef[];
  tools: AiToolSelection[];
  generation: { temperature?: number | null; reasoning?: { effort?: string | null; budget_tokens?: number | null } | null; max_tokens?: number | null; instruction?: string };
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
  | { kind: 'phase'; phase: 'model' | 'awaiting_confirmation' | 'tool' }
  | { kind: 'terminal'; terminal: AiTerminal }
);
export interface AiModelEntry { model_id: string; display_name: string; kind: string }
export interface AiProviderEntry { provider_id: string; display_name: string; config_revision: number; models: AiModelEntry[] }
export interface AiSkillEntry extends AiSkillRef { name: string }
export interface AiToolEntry extends AiToolSelection { server_name: string; description: string | null; input_schema: unknown }
