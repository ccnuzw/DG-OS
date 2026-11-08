import type { Appearance, Variant } from './model';
import { PRESETS, themeColorIssues, type ColorFieldName, type PresetId, type SeedColors, type ThemeColorIssue, type ThemeMode, type ThemeScope } from '../theme/kernel';

export type { ColorFieldName } from '../theme/kernel';
export type AppearanceColorIssue = ThemeColorIssue & { mode: ThemeMode };

/** Each region inherits individual fields from interface; code never inherits reading. */
export function effectiveColors(variant: Variant, mode: ThemeMode, scope: ThemeScope): SeedColors {
  return { ...PRESETS[variant.presetId][mode], ...variant.overrides.interface, ...(scope === 'interface' ? {} : variant.overrides[scope]) };
}

export function overrideCount(variant: Variant): number {
  return Object.values(variant.overrides).reduce((count, colors) => count + Object.keys(colors ?? {}).length, 0);
}

export function withPreset(variant: Variant, presetId: PresetId, keepOverrides: boolean): Variant {
  if (variant.presetId === presetId) return variant;
  return {
    presetId,
    overrides: keepOverrides ? structuredClone(variant.overrides) : { interface: {}, reading: {}, code: {} },
  };
}

export function setColor(variant: Variant, scope: 'interface', field: ColorFieldName, value: string | undefined): Variant;
export function setColor(variant: Variant, scope: ThemeScope, field: Exclude<ColorFieldName, 'accent'>, value: string | undefined): Variant;
export function setColor(variant: Variant, scope: ThemeScope, field: ColorFieldName, value: string | undefined): Variant {
  if (scope !== 'interface' && field === 'accent') throw new Error('Accent overrides belong to interface');
  const colors = { ...variant.overrides[scope] };
  if (value === undefined) delete colors[field];
  else colors[field] = value;
  return { ...variant, overrides: { ...variant.overrides, [scope]: colors } };
}

export function resetRegion(variant: Variant, scope: ThemeScope): Variant {
  return { ...variant, overrides: { ...variant.overrides, [scope]: {} } };
}

/** Use the same validation and derivation as resolveTheme, including inactive mode errors. */
export function colorIssues(appearance: Appearance): AppearanceColorIssue[] {
  return (['light', 'dark'] as const).flatMap(mode => {
    const variant = appearance.variants[mode];
    return themeColorIssues(variant.presetId, mode, variant.overrides).map(issue => ({ mode, ...issue }));
  });
}
