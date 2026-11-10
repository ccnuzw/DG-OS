import { z } from 'zod';
import type { EffortLevel, ModelInfo, PermissionMode } from '@anthropic-ai/claude-agent-sdk';

export const responsePreferencesSchema = z.object({
  language: z.enum(['follow_input', 'zh', 'en']),
  brevity: z.enum(['concise', 'standard', 'detailed']),
  next_step_suggestions: z.boolean(),
}).strict();
export const turnSettingsSchema = z.object({
  reasoning_effort: z.string().min(1).max(128).nullable().optional(),
  permission_mode: z.enum(['default', 'read_only', 'accept_edits', 'full_auto']).optional(),
  collaboration_mode: z.enum(['default', 'plan', 'deep']).optional(),
  response_preferences: responsePreferencesSchema.optional(),
}).strict();
export type TurnSettings = z.infer<typeof turnSettingsSchema>;
type Control<T> = { available: boolean; options: T[]; current: T | null; source: string; reason: string | null };
export type TurnControls = {
  reasoning: Control<string>;
  permission: Control<NonNullable<TurnSettings['permission_mode']>>;
  collaboration: Control<NonNullable<TurnSettings['collaboration_mode']>>;
};

export function responseInstructions(preferences: TurnSettings['response_preferences']): string {
  if (!preferences) return '';
  const language = { follow_input: "Match the user's input language unless the user explicitly requests another language.", zh: 'Respond in Chinese unless the user explicitly requests another language.', en: 'Respond in English unless the user explicitly requests another language.' }[preferences.language];
  const brevity = { concise: 'Give a direct, concise answer.', standard: 'Use normal detail appropriate to the task.', detailed: 'Give a thorough explanation with useful details.' }[preferences.brevity];
  return `Session response preferences: ${language} ${brevity} ${preferences.next_step_suggestions ? 'Include useful next-step suggestions when appropriate.' : 'Omit unsolicited next-step suggestions.'}`;
}
export const deepPlanInstructions = "Clarify the user's goals and unresolved constraints first; compare viable approaches and their tradeoffs; then produce a concrete plan and wait for explicit user approval before implementation.";
export function nativePermission(controls: TurnControls, settings: TurnSettings): PermissionMode {
  const collaboration = settings.collaboration_mode ?? controls.collaboration.current;
  if (collaboration === 'plan' || collaboration === 'deep') return 'plan';
  const permission = settings.permission_mode ?? controls.permission.current ?? 'default';
  if (permission === 'full_auto') return 'bypassPermissions';
  if (permission === 'accept_edits') return 'acceptEdits';
  return 'default';
}

export function projectTurnControls(models: ModelInfo[], boundModel: string, protocolSupported: boolean): TurnControls {
  const exact = models.filter(row => row.value === boundModel);
  const matches = exact.length ? exact : models.filter(row => row.resolvedModel === boundModel);
  const levels = ['low', 'medium', 'high', 'xhigh', 'max'] as const;
  const optionsFor = (row: ModelInfo) => row.supportsEffort === true ? levels.filter(level => row.supportedEffortLevels?.includes(level)) : [];
  const first = matches[0];
  const candidate = first ? optionsFor(first) : [];
  const ambiguous = matches.some(row => row.supportsEffort !== first?.supportsEffort || optionsFor(row).join(',') !== candidate.join(','));
  const options = protocolSupported && !ambiguous ? candidate : [];
  return {
    reasoning: { available: options.length > 0, options, current: null, source: 'claude/supported_models', reason: !protocolSupported ? 'native_protocol_unsupported' : ambiguous ? 'bound_model_metadata_ambiguous' : options.length ? 'native_current_unobserved' : 'bound_model_metadata_unavailable' },
    permission: { available: protocolSupported, options: protocolSupported ? ['default', 'accept_edits', 'full_auto'] : [], current: protocolSupported ? 'default' : null, source: 'claude/query_options', reason: protocolSupported ? null : 'native_protocol_unsupported' },
    collaboration: { available: protocolSupported, options: protocolSupported ? ['default', 'plan', 'deep'] : [], current: protocolSupported ? 'default' : null, source: 'claude/native_plan', reason: protocolSupported ? null : 'native_protocol_unsupported' },
  };
}
export function validateTurnSettings(controls: TurnControls, settings: TurnSettings): void {
  if (settings.reasoning_effort !== undefined && (!controls.reasoning.available || (settings.reasoning_effort !== null && !controls.reasoning.options.includes(settings.reasoning_effort)))) throw new Error('unsupported_reasoning_effort');
  if (settings.permission_mode !== undefined && (!controls.permission.available || !controls.permission.options.includes(settings.permission_mode))) throw new Error('unsupported_permission_mode');
  if (settings.collaboration_mode !== undefined && (!controls.collaboration.available || !controls.collaboration.options.includes(settings.collaboration_mode))) throw new Error('unsupported_collaboration_mode');
}
export async function applyTurnSettings(current: {
  setPermissionMode: (mode: PermissionMode) => Promise<void>;
  applyFlagSettings: (settings: { effortLevel: EffortLevel | null }) => Promise<void>;
}, controls: TurnControls, settings: TurnSettings): Promise<void> {
  validateTurnSettings(controls, settings);
  // Settings never resolve an outstanding approval. Native plan mode supplies its own read-only policy.
  if (settings.permission_mode !== undefined || settings.collaboration_mode !== undefined) {
    await current.setPermissionMode(nativePermission(controls, settings));
    if (settings.permission_mode !== undefined) controls.permission.current = settings.permission_mode;
    controls.permission.source = 'claude/set_permission_mode'; controls.permission.reason = null;
    if (settings.collaboration_mode !== undefined) controls.collaboration.current = settings.collaboration_mode;
    controls.collaboration.source = 'claude/native_plan'; controls.collaboration.reason = null;
  }
  if (settings.reasoning_effort !== undefined) {
    await current.applyFlagSettings({ effortLevel: settings.reasoning_effort as EffortLevel | null });
    controls.reasoning.current = null; controls.reasoning.source = 'claude/apply_flag_settings'; controls.reasoning.reason = 'awaiting_native_effort';
  }
}
