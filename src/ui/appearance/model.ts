import { isPresetId, resolveTheme, type ScopedOverrides, type PresetId } from '../theme/kernel';
import { FONT_SIZES, type LineHeight } from '../theme/typography';
import { isFontSelection, type FontSelection, type FontRole } from '../theme/fonts';

export const THEMES = ['light', 'dark', 'auto'] as const;
export const DENSITIES = [60, 70, 80, 90, 100, 110, 120, 130, 140, 150] as const;
export type Theme = (typeof THEMES)[number];
export type ResolvedTheme = Exclude<Theme, 'auto'>;
export type Density = (typeof DENSITIES)[number];
export interface Variant { presetId: PresetId; overrides: ScopedOverrides }
export interface Appearance {
  schemaVersion: 6;
  mode: Theme;
  variants: { light: Variant; dark: Variant };
  typography: { uiFont: FontSelection; readingFont: FontSelection; codeFont: FontSelection; uiSize: number; readingSize: number; codeSize: number; lineHeight: LineHeight; readingFollowUi: boolean };
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
  schemaVersion: 6, mode: 'light', density: 100,
  variants: { light: { presetId: 'studio', overrides: { interface: {}, reading: {}, code: {} } }, dark: { presetId: 'studio', overrides: { interface: {}, reading: {}, code: {} } } },
  typography: { uiFont: {kind:'system-ui'}, readingFont: {kind:'system-ui'}, codeFont: {kind:'system-mono'}, uiSize: 12, readingSize: 12, codeSize: 12, lineHeight: 1.75, readingFollowUi: true },
  effects: { sidebarTreatment: 'solid', reducedMotion: 'system' },
};
export function isTheme(value: unknown): value is Theme { return typeof value === 'string' && (THEMES as readonly string[]).includes(value); }
export function isDensity(value: unknown): value is Density { return typeof value === 'number' && (DENSITIES as readonly number[]).includes(value); }
function exact(raw: unknown, names: readonly string[]): asserts raw is Record<string, unknown> {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw) || Object.keys(raw).length !== names.length || names.some(name => !Object.prototype.hasOwnProperty.call(raw,name))) throw new Error('Invalid appearance object');
}
export function parseAppearance(raw: unknown): Appearance {
  exact(raw,['schemaVersion','mode','variants','typography','density','effects']);
  if (raw.schemaVersion !== 6 || !isTheme(raw.mode) || !isDensity(raw.density)) throw new Error('Invalid appearance mode or density');
  exact(raw.variants,['light','dark']);
  for (const mode of ['light','dark'] as const) {
    const variant=raw.variants[mode]; exact(variant,['presetId','overrides']);
    if (!isPresetId(variant.presetId)) throw new Error('Unknown preset');
    exact(variant.overrides,['interface','reading','code']);
    resolveTheme(variant.presetId,mode,variant.overrides as ScopedOverrides);
  }
  exact(raw.typography,['uiFont','readingFont','codeFont','uiSize','readingSize','codeSize','lineHeight','readingFollowUi']);
  const typography=raw.typography;
  if (typeof typography.readingFollowUi !== 'boolean') throw new Error('Invalid reading inheritance');
  for (const role of ['ui','reading','code'] as const) {
    if (!isFontSelection(typography[`${role}Font`], role) || !(FONT_SIZES[role] as readonly unknown[]).includes(typography[`${role}Size`])) throw new Error('Invalid typography');
  }
  if (![1.5,1.75,2].includes(typography.lineHeight as number)) throw new Error('Invalid line height');
  exact(raw.effects,['sidebarTreatment','reducedMotion']);
  if (!['solid','soft'].includes(raw.effects.sidebarTreatment as string) || !['system','reduce'].includes(raw.effects.reducedMotion as string)) throw new Error('Invalid theme effects');
  return structuredClone(raw) as unknown as Appearance;
}
const LEGACY_FAMILIES: Readonly<Record<string, string>> = {
  inter: 'Inter', 'source-sans-3': 'Source Sans 3', 'noto-sans-cjk-sc': 'Noto Sans CJK SC',
  'jetbrains-mono': 'JetBrains Mono', 'source-code-pro': 'Source Code Pro', 'noto-sans-mono-cjk-sc': 'Noto Sans Mono CJK SC',
  humanist: 'Segoe UI', serif: 'Georgia',
};
function migrateFont(id: unknown, role: FontRole): FontSelection {
  if (id === 'system' || (role === 'code' && id === 'mono')) return {kind: role === 'code' ? 'system-mono' : 'system-ui'};
  if (typeof id !== 'string' || !Object.prototype.hasOwnProperty.call(LEGACY_FAMILIES, id)) throw new Error('Invalid legacy font');
  return {kind:'local', family:LEGACY_FAMILIES[id]};
}
function migrateDensity(value: unknown): Density {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new Error('Invalid legacy density');
  return Math.round(Math.min(150, Math.max(60, value)) / 10) * 10 as Density;
}
export function parseConfirmed(raw: unknown): AppearanceSnapshot {
  if (!raw || typeof raw !== 'object' || !('revision' in raw)) throw new Error('Missing appearance revision');
  const {revision,...values}=raw;
  if (!Number.isSafeInteger(revision) || (revision as number)<0) throw new Error('Invalid appearance revision');
  // Legacy values exist only at this confirmed-read boundary, never in drafts or writes.
  if ('schemaVersion' in values && [2,3,4,5].includes(values.schemaVersion as number)) {
    const density = migrateDensity('density' in values ? values.density : undefined);
    if ([2,3].includes(values.schemaVersion as number) && 'typography' in values && values.typography && typeof values.typography === 'object') {
      const old = values.typography as Record<string, unknown>;
      const {uiFontId, readingFontId, codeFontId, ...rest} = old;
      return {...parseAppearance({...values, schemaVersion: 6, density, typography: {...rest,
        uiFont: migrateFont(uiFontId, 'ui'), readingFont: migrateFont(readingFontId, 'reading'), codeFont: migrateFont(codeFontId, 'code'),
        readingFollowUi: values.schemaVersion === 2 ? false : old.readingFollowUi,
      }}), revision: revision as number};
    }
    return {...parseAppearance({...values, schemaVersion: 6, density}),revision:revision as number};
  }
  return {...parseAppearance(values),revision:revision as number};
}
export function parseRead(raw: unknown): AppearanceRead {
  if (!raw || typeof raw !== 'object' || !('kind' in raw)) throw new Error('Invalid appearance response');
  if (raw.kind === 'ready' && 'snapshot' in raw) {
    const snapshot = parseConfirmed(raw.snapshot);
    const repairWarnings = 'repairWarnings' in raw && Array.isArray(raw.repairWarnings) ? raw.repairWarnings.filter((v):v is string=>typeof v==='string') : [];
    const source = raw.snapshot as Record<string, unknown>;
    if ([2,3,4,5].includes(source.schemaVersion as number) && source.density !== snapshot.density && !repairWarnings.includes('density_migrated')) repairWarnings.push('density_migrated');
    return {kind:'ready',snapshot,repairWarnings};
  }
  if (raw.kind === 'corrupt' && 'recoveryToken' in raw && typeof raw.recoveryToken === 'string') return {kind:'corrupt',safeDefaults:structuredClone(DEFAULTS),recoveryToken:raw.recoveryToken};
  if (raw.kind === 'unsupported_schema' && 'schemaVersion' in raw && typeof raw.schemaVersion === 'number') return {kind:'unsupported_schema',safeDefaults:structuredClone(DEFAULTS),schemaVersion:raw.schemaVersion};
  throw new Error('Invalid appearance response');
}
export function appearanceValues(snapshot: AppearanceSnapshot): Appearance {
  const {revision: _revision,...values}=snapshot;
  return values;
}
