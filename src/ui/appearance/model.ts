// FR-005 §13.1 B05-01/B05-07: closed value sets for the two appearance preferences (D064, D065).
export const THEMES = ['light', 'dark', 'auto'] as const;
export const DENSITIES = [75, 100, 125, 150, 175] as const;

export type Theme = (typeof THEMES)[number];
export type ResolvedTheme = Exclude<Theme, 'auto'>;
export type Density = (typeof DENSITIES)[number];

export interface Appearance {
  theme: Theme;
  density: Density;
}

export const DEFAULTS: Readonly<Appearance> = Object.freeze({ theme: 'light', density: 100 });

export function isTheme(value: unknown): value is Theme {
  return typeof value === 'string' && (THEMES as readonly string[]).includes(value);
}

// Numbers only: "125" (a string) is not a density.
export function isDensity(value: unknown): value is Density {
  return typeof value === 'number' && (DENSITIES as readonly number[]).includes(value);
}

function asRecord(raw: unknown): Record<string, unknown> {
  return typeof raw === 'object' && raw !== null ? (raw as Record<string, unknown>) : {};
}

/** B05-22: a missing or illegal field falls back to its default. Illegal values are never echoed back. */
export function sanitize(raw: unknown): Appearance {
  const record = asRecord(raw);
  return {
    theme: isTheme(record.theme) ? record.theme : DEFAULTS.theme,
    density: isDensity(record.density) ? record.density : DEFAULTS.density,
  };
}

/**
 * A write response must be the complete, valid preferences (§12.1). Anything else is an unknown result
 * and is treated as a failed write, not silently turned into defaults.
 */
export function parseConfirmed(raw: unknown): Appearance {
  const record = asRecord(raw);
  if (!isTheme(record.theme) || !isDensity(record.density)) {
    throw new Error('set_appearance_preferences returned incomplete or invalid preferences');
  }
  return { theme: record.theme, density: record.density };
}
