import presetSeeds from '../tokens/presets.json';

export const PRESET_IDS = ['precision', 'studio', 'paper', 'forest', 'slate'] as const;
export type PresetId = (typeof PRESET_IDS)[number];
export type ThemeMode = 'light' | 'dark';
export type ThemeScope = 'interface' | 'reading' | 'code';
export type SeedColors = Readonly<{ accent: string; background: string; foreground: string }>;
export type ColorOverrides = Readonly<Partial<SeedColors>>;
export type ScopedOverrides = Readonly<Partial<Record<ThemeScope, ColorOverrides>>>;
export type ResolvedTheme = Readonly<Record<string, string>>;
export const PRESETS: Readonly<Record<PresetId, Readonly<Record<ThemeMode, SeedColors>>>> = presetSeeds;

type RGB = readonly [number, number, number];
const BLACK: RGB = [0, 0, 0];
const WHITE: RGB = [255, 255, 255];
function rgb(color: string): RGB {
  if (!/^#[0-9a-fA-F]{6}$/.test(color)) throw new Error('Theme colors must be #RRGGBB values');
  return [Number.parseInt(color.slice(1, 3), 16), Number.parseInt(color.slice(3, 5), 16), Number.parseInt(color.slice(5, 7), 16)];
}
function hex(color: RGB): string {
  return `#${color.map((value) => Math.round(value).toString(16).padStart(2, '0')).join('')}`;
}
function mix(from: RGB, to: RGB, amount: number): RGB {
  return [from[0] + (to[0] - from[0]) * amount, from[1] + (to[1] - from[1]) * amount, from[2] + (to[2] - from[2]) * amount];
}
function luminance(color: RGB): number {
  const channels = color.map((value) => value / 255).map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}
function ratio(first: RGB, second: RGB): number {
  const a = luminance(first);
  const b = luminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}
export function contrastRatio(first: string, second: string): number {
  return ratio(rgb(first), rgb(second));
}
function solve(from: RGB, toward: RGB, surfaces: readonly RGB[], target: number): RGB {
  for (let step = 0; step <= 100; step += 1) {
    const candidate = rgb(hex(mix(from, toward, step / 100)));
    if (surfaces.every((surface) => ratio(candidate, surface) >= target)) return candidate;
  }
  throw new Error('Theme contrast target cannot be met');
}

export function resolveTheme(presetId: PresetId, mode: ThemeMode, overrides: ScopedOverrides = {}): ResolvedTheme {
  const base = PRESETS[presetId]?.[mode];
  if (!base) throw new Error('Unknown theme preset');
  for (const [scope, colors] of Object.entries(overrides)) {
    if (!['interface', 'reading', 'code'].includes(scope) || !colors || typeof colors !== 'object' || Array.isArray(colors)) throw new Error('Invalid theme scope');
    for (const [name, value] of Object.entries(colors)) {
      if (!['background', 'foreground', ...(scope === 'interface' ? ['accent'] : [])].includes(name)) throw new Error('Invalid theme color field');
      rgb(value);
    }
  }
  const ui = { ...base, ...overrides.interface };
  const bg = rgb(ui.background);
  const fg = rgb(ui.foreground);
  const panel = rgb(hex(mix(bg, fg, 0.035)));
  const rail = rgb(hex(mix(bg, fg, 0.055)));
  const inset = rgb(hex(mix(bg, fg, 0.075)));
  const surfaces = [bg, panel, rail, inset];
  if (!surfaces.every((surface) => ratio(fg, surface) >= 4.5)) throw new Error('Interface text contrast must be at least 4.5:1');
  const accentSeed = rgb(ui.accent);
  if (!surfaces.every((surface) => ratio(accentSeed, surface) >= 3)) throw new Error('Accent contrast must be at least 3:1');
  const brand = solve(accentSeed, fg, surfaces, 4.5);
  const ink = mode === 'dark' ? bg : fg;
  const on = ratio(brand, WHITE) >= ratio(brand, ink) ? WHITE : ink;
  if (ratio(on, brand) < 4.5) throw new Error('Accent text contrast must be at least 4.5:1');
  const interactionTarget = mode === 'light' ? fg : WHITE;
  const hover = solve(rgb(hex(mix(brand, interactionTarget, 0.08))), brand, [on], 4.5);
  const active = solve(rgb(hex(mix(brand, interactionTarget, 0.16))), brand, [on], 4.5);
  const control = solve(bg, fg, surfaces, 3);
  const muted = solve(bg, fg, surfaces, 4.5);
  const selected = mix(bg, brand, 0.1);
  const result: Record<string, string> = {
    '--color-primary': hex(brand), '--color-primary-hover': hex(hover), '--color-primary-active': hex(active),
    '--color-primary-light': hex(selected), '--color-on-primary': hex(on),
    '--color-bg-primary': hex(bg), '--color-bg-secondary': hex(panel), '--color-bg-tertiary': hex(inset), '--color-bg-rail': hex(rail),
    '--color-text-primary': hex(fg), '--color-text-secondary': hex(muted), '--color-text-tertiary': hex(muted),
    '--color-border': hex(mix(bg, fg, 0.16)), '--color-border-control': hex(control), '--color-divider': hex(mix(bg, fg, 0.1)),
    '--color-focus-ring': hex(brand),
  };
  for (const scope of ['reading', 'code'] as const) {
    const colors = { ...ui, ...overrides[scope] };
    if (contrastRatio(colors.background, colors.foreground) < 4.5) throw new Error(`${scope} text contrast must be at least 4.5:1`);
    result[`--surface-${scope}`] = colors.background.toLowerCase();
    result[`--text-${scope}`] = colors.foreground.toLowerCase();
  }
  // Status colors remain independent of the user's brand and overrides.
  const statusSeeds = presetSeeds.precision[mode];
  const light = mode === 'light';
  const status: Record<string, RGB> = {
    success: light ? [22, 112, 72] : [137, 216, 175],
    warning: light ? [139, 76, 0] : [233, 190, 118],
    error: light ? [185, 28, 28] : [252, 165, 165],
    info: light ? [29, 78, 216] : [147, 197, 253],
  } as const;
  for (const [name, seed] of Object.entries(status)) {
    const surface = rgb(hex(mix(rgb(statusSeeds.background), seed, 0.12)));
    const text = solve(seed, light ? BLACK : WHITE, [surface, ...surfaces], 4.5);
    result[`--color-${name}`] = hex(text);
    result[`--color-${name}-text`] = hex(text);
    result[`--color-${name}-bg`] = hex(surface);
  }
  const error = rgb(result['--color-error']);
  result['--color-on-error'] = ratio(error, WHITE) >= ratio(error, BLACK) ? hex(WHITE) : hex(BLACK);
  return Object.freeze(result);
}

export function isPresetId(value: unknown): value is PresetId {
  return typeof value === 'string' && (PRESET_IDS as readonly string[]).includes(value);
}
