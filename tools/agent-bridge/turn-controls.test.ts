import { describe, expect, test } from 'bun:test';
import type { ModelInfo } from '@anthropic-ai/claude-agent-sdk';
import { applyTurnSettings, projectTurnControls, turnSettingsSchema, validateTurnSettings } from './turn-controls';

const model: ModelInfo = { value: 'sonnet', resolvedModel: 'exact-provider-model', displayName: 'Native', description: '', supportsEffort: true, supportedEffortLevels: ['low', 'high'] };

describe('pinned Claude turn settings protocol', () => {
  test('only exact native model or resolved alias metadata supplies effort options', () => {
    expect(projectTurnControls([model], 'exact-provider-model', true).reasoning.options).toEqual(['low', 'high']);
    expect(projectTurnControls([model], 'sonnet', true).reasoning.available).toBe(true);
    expect(projectTurnControls([model], 'sonnet-lookalike', true).reasoning.available).toBe(false);
    expect(projectTurnControls([model], 'exact-provider-model', false).reasoning.available).toBe(false);
    expect(projectTurnControls([{ ...model, supportsEffort: false }], 'sonnet', true).reasoning.options).toEqual([]);
  });

  test('dangerous modes, null permissions and extra keys cannot enter the wire', () => {
    for (const permission_mode of ['bypassPermissions', 'danger-full-access', 'auto', 'acceptEdits']) {
      expect(turnSettingsSchema.safeParse({ reasoning_effort: null, permission_mode }).success).toBe(false);
    }
    expect(turnSettingsSchema.safeParse({ permission_mode: null }).success).toBe(false);
    expect(turnSettingsSchema.safeParse({ permission_mode: 'default' }).success).toBe(true);
    expect(turnSettingsSchema.safeParse({}).success).toBe(true);
    expect(turnSettingsSchema.safeParse({ reasoning_effort: null, permission_mode: 'default', always_allow: true }).success).toBe(false);
  });

  test('unsupported effort or engine permission rejects before any native mutation', async () => {
    const controls = projectTurnControls([model], 'sonnet', true);
    const calls: unknown[] = [];
    const native = {
      setPermissionMode: async (mode: 'default' | 'acceptEdits') => { calls.push(mode); },
      applyFlagSettings: async (settings: unknown) => { calls.push(settings); },
    };
    await expect(applyTurnSettings(native, controls, { reasoning_effort: 'medium', permission_mode: 'default' })).rejects.toThrow('unsupported_reasoning_effort');
    await expect(applyTurnSettings(native, controls, { reasoning_effort: 'low', permission_mode: 'read_only' })).rejects.toThrow('unsupported_permission_mode');
    expect(calls).toEqual([]);
    expect(() => validateTurnSettings(projectTurnControls([], 'unknown', true), { reasoning_effort: 'high', permission_mode: 'default' })).toThrow('unsupported_reasoning_effort');
  });

  test('explicit accept_edits uses native acceptEdits and nullable effort resets existing query', async () => {
    const controls = projectTurnControls([model], 'sonnet', true);
    const calls: unknown[] = [];
    const native = {
      setPermissionMode: async (mode: 'default' | 'acceptEdits') => { calls.push(['permission', mode]); },
      applyFlagSettings: async (settings: unknown) => { calls.push(['settings', settings]); },
    };
    await applyTurnSettings(native, controls, { reasoning_effort: 'high', permission_mode: 'accept_edits' });
    expect(calls).toEqual([['permission', 'acceptEdits'], ['settings', { effortLevel: 'high' }]]);
    expect(controls.permission.current).toBe('accept_edits');
    expect(controls.reasoning.current).toBeNull();
    expect(controls.reasoning.reason).toBe('awaiting_native_effort');
    await applyTurnSettings(native, controls, { reasoning_effort: null, permission_mode: 'default' });
    expect(calls.slice(2)).toEqual([['permission', 'default'], ['settings', { effortLevel: null }]]);
  });

  test('missing model metadata preserves effort while allowing the verified native permission control', async () => {
    const controls = projectTurnControls([], 'unknown', true);
    const calls: unknown[] = [];
    await applyTurnSettings({
      setPermissionMode: async mode => { calls.push(mode); },
      applyFlagSettings: async settings => { calls.push(settings); },
    }, controls, { permission_mode: 'accept_edits' });
    expect(calls).toEqual(['acceptEdits']);
    expect(controls.reasoning.reason).toBe('bound_model_metadata_unavailable');
  });
  test('empty settings preserve native state and reasoning-only changes never reset permissions', async () => {
    const controls = projectTurnControls([model], 'sonnet', true);
    controls.permission.current = 'accept_edits';
    const calls: unknown[] = [];
    const native = {
      setPermissionMode: async (mode: 'default' | 'acceptEdits') => { calls.push(mode); },
      applyFlagSettings: async (settings: unknown) => { calls.push(settings); },
    };
    await applyTurnSettings(native, controls, {});
    expect(calls).toEqual([]);
    expect(controls.permission.current).toBe('accept_edits');
    controls.permission.available = false;
    await applyTurnSettings(native, controls, { reasoning_effort: null });
    expect(calls).toEqual([{ effortLevel: null }]);
    expect(controls.permission.current).toBe('accept_edits');
    await applyTurnSettings(native, controls, turnSettingsSchema.parse({}));
    expect(calls).toEqual([{ effortLevel: null }]);
    expect(controls.permission.current).toBe('accept_edits');
    expect(controls.reasoning.reason).toBe('awaiting_native_effort');
  });
});
