import { isPresetId, resolveTheme, type ScopedOverrides, type PresetId } from '../theme/kernel';
import { FONT_IDS, FONT_SIZES, type FontId, type LineHeight } from '../theme/typography';

export const THEMES = ['light', 'dark', 'auto'] as const;
export const DENSITIES = [75, 100, 125, 150, 175] as const;
export type Theme = (typeof THEMES)[number];
export type ResolvedTheme = Exclude<Theme, 'auto'>;
export type Density = (typeof DENSITIES)[number];
export interface Variant { presetId: PresetId; overrides: ScopedOverrides }
export interface Appearance {
  schemaVersion: 2;
  mode: Theme;
  variants: { light: Variant; dark: Variant };
  typography: { uiFontId: FontId<'ui'>; readingFontId: FontId<'reading'>; codeFontId: FontId<'code'>; uiSize: number; readingSize: number; codeSize: number; lineHeight: LineHeight };
  density: Density;
  effects: { sidebarTreatment: 'solid' | 'soft'; reducedMotion: 'system' | 'reduce' };
}
export interface AppearanceSnapshot extends Appearance { revision: number }
export type AppearanceRead =
  | { kind: 'ready'; snapshot: AppearanceSnapshot; repairWarnings: string[] }
  | { kind: 'corrupt'; safeDefaults: Appearance; recoveryToken: string }
  | { kind: 'unsupported_schema'; safeDefaults: Appearance; schemaVersion: number };
export type AppearanceSave = { kind: 'saved'; snapshot: AppearanceSnapshot } | { kind: 'conflict'; code: 1005007; snapshot: AppearanceSnapshot };
export const DEFAULTS: Appearance = {
  schemaVersion: 2, mode: 'light', density: 100,
  variants: { light: { presetId: 'studio', overrides: { interface: {}, reading: {}, code: {} } }, dark: { presetId: 'studio', overrides: { interface: {}, reading: {}, code: {} } } },
  typography: { uiFontId: 'system', readingFontId: 'system', codeFontId: 'mono', uiSize: 12, readingSize: 13, codeSize: 12, lineHeight: 1.75 },
  effects: { sidebarTreatment: 'solid', reducedMotion: 'system' },
};
export function isTheme(value: unknown): value is Theme { return typeof value === 'string' && (THEMES as readonly string[]).includes(value); }
export function isDensity(value: unknown): value is Density { return typeof value === 'number' && (DENSITIES as readonly number[]).includes(value); }
function exact(raw: unknown, names: readonly string[]): asserts raw is Record<string, unknown> {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw) || Object.keys(raw).length !== names.length || names.some(name => !Object.prototype.hasOwnProperty.call(raw,name))) throw new Error('Invalid appearance object');
}
export function parseAppearance(raw: unknown): Appearance {
  exact(raw,['schemaVersion','mode','variants','typography','density','effects']);
  if (raw.schemaVersion !== 2 || !isTheme(raw.mode) || !isDensity(raw.density)) throw new Error('Invalid appearance mode or density');
  exact(raw.variants,['light','dark']);
  for (const mode of ['light','dark'] as const) {
    const variant=raw.variants[mode]; exact(variant,['presetId','overrides']);
    if (!isPresetId(variant.presetId)) throw new Error('Unknown preset');
    exact(variant.overrides,['interface','reading','code']);
    resolveTheme(variant.presetId,mode,variant.overrides as ScopedOverrides);
  }
  exact(raw.typography,['uiFontId','readingFontId','codeFontId','uiSize','readingSize','codeSize','lineHeight']);
  const typography=raw.typography;
  for (const role of ['ui','reading','code'] as const) {
    if (!(FONT_IDS[role] as readonly unknown[]).includes(typography[`${role}FontId`]) || !(FONT_SIZES[role] as readonly unknown[]).includes(typography[`${role}Size`])) throw new Error('Invalid typography');
  }
  if (![1.5,1.75,2].includes(typography.lineHeight as number)) throw new Error('Invalid line height');
  exact(raw.effects,['sidebarTreatment','reducedMotion']);
  if (!['solid','soft'].includes(raw.effects.sidebarTreatment as string) || !['system','reduce'].includes(raw.effects.reducedMotion as string)) throw new Error('Invalid theme effects');
  return structuredClone(raw) as unknown as Appearance;
}
export function parseConfirmed(raw: unknown): AppearanceSnapshot {
  if (!raw || typeof raw !== 'object' || !('revision' in raw)) throw new Error('Missing appearance revision');
  const {revision,...values}=raw;
  if (!Number.isSafeInteger(revision) || (revision as number)<0) throw new Error('Invalid appearance revision');
  return {...parseAppearance(values),revision:revision as number};
}
export function parseRead(raw: unknown): AppearanceRead {
  if (!raw || typeof raw !== 'object' || !('kind' in raw)) throw new Error('Invalid appearance response');
  if (raw.kind === 'ready' && 'snapshot' in raw) return {kind:'ready',snapshot:parseConfirmed(raw.snapshot),repairWarnings:'repairWarnings' in raw && Array.isArray(raw.repairWarnings) ? raw.repairWarnings.filter((v):v is string=>typeof v==='string') : []};
  if (raw.kind === 'corrupt' && 'recoveryToken' in raw && typeof raw.recoveryToken === 'string') return {kind:'corrupt',safeDefaults:structuredClone(DEFAULTS),recoveryToken:raw.recoveryToken};
  if (raw.kind === 'unsupported_schema' && 'schemaVersion' in raw && typeof raw.schemaVersion === 'number') return {kind:'unsupported_schema',safeDefaults:structuredClone(DEFAULTS),schemaVersion:raw.schemaVersion};
  throw new Error('Invalid appearance response');
}
export function appearanceValues(snapshot: AppearanceSnapshot): Appearance {
  const {revision: _revision,...values}=snapshot;
  return values;
}
