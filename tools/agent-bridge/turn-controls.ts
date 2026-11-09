import { z } from 'zod';
import type { EffortLevel, ModelInfo } from '@anthropic-ai/claude-agent-sdk';

export const turnSettingsSchema = z.object({
  reasoning_effort: z.string().min(1).max(128).nullable().optional(),
  permission_mode: z.enum(['default', 'read_only', 'accept_edits']).optional(),
}).strict();
export type TurnSettings = z.infer<typeof turnSettingsSchema>;
type Control<T> = { available: boolean; options: T[]; current: T | null; source: string; reason: string | null };
export type TurnControls = {
  reasoning: Control<string>;
  permission: Control<NonNullable<TurnSettings['permission_mode']>>;
};

export function projectTurnControls(models: ModelInfo[], boundModel: string, protocolSupported: boolean): TurnControls {
  // No family/name heuristic: aliases qualify only through native resolvedModel metadata.
  const model = models.find(row => row.value === boundModel) ?? models.find(row => row.resolvedModel === boundModel);
  const options = protocolSupported && model?.supportsEffort === true ? [...new Set(model.supportedEffortLevels ?? [])] : [];
  return {
    reasoning: { available: options.length > 0, options, current: null, source: 'claude/supported_models',
      reason: options.length ? 'native_current_unobserved' : 'bound_model_metadata_unavailable' },
    permission: { available: protocolSupported, options: protocolSupported ? ['default', 'accept_edits'] : [],
      current: protocolSupported ? 'default' : null, source: 'claude/query_options', reason: protocolSupported ? null : 'native_protocol_unsupported' },
  };
}

export function validateTurnSettings(controls: TurnControls, settings: TurnSettings): void {
  if (settings.reasoning_effort !== undefined
    && (!controls.reasoning.available || (settings.reasoning_effort !== null && !controls.reasoning.options.includes(settings.reasoning_effort)))) {
    throw new Error('unsupported_reasoning_effort');
  }
  if (settings.permission_mode !== undefined
    && (!controls.permission.available || !controls.permission.options.includes(settings.permission_mode))) throw new Error('unsupported_permission_mode');
}

export async function applyTurnSettings(current: {
  setPermissionMode: (mode: 'default' | 'acceptEdits') => Promise<void>;
  applyFlagSettings: (settings: { effortLevel: EffortLevel | null }) => Promise<void>;
}, controls: TurnControls, settings: TurnSettings): Promise<void> {
  validateTurnSettings(controls, settings);
  // Validation precedes either native mutation. Never resolve pending approvals here.
  if (settings.permission_mode !== undefined) {
    await current.setPermissionMode(settings.permission_mode === 'accept_edits' ? 'acceptEdits' : 'default');
    controls.permission.current = settings.permission_mode;
    controls.permission.source = 'claude/set_permission_mode';
    controls.permission.reason = null;
  }
  if (settings.reasoning_effort !== undefined) {
    // SDK 0.3.317 applyFlagSettings(null) resets to the model default, not a lower settings layer.
    await current.applyFlagSettings({ effortLevel: settings.reasoning_effort as EffortLevel | null });
    // Managed caps can silently downgrade effort. Only a native hook can establish actual effort.
    controls.reasoning.current = null;
    controls.reasoning.source = 'claude/apply_flag_settings';
    controls.reasoning.reason = 'awaiting_native_effort';
  }
}
