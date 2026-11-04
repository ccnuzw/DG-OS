export const FONT_IDS = {
  ui: ['system', 'humanist'],
  reading: ['system', 'humanist', 'serif'],
  code: ['mono', 'system'],
} as const;

export type FontRole = keyof typeof FONT_IDS;
export type FontId<Role extends FontRole = FontRole> = (typeof FONT_IDS)[Role][number];
export const FONT_SIZES = {
  ui: [12, 13, 14, 16],
  reading: [13, 14, 16, 18, 20],
  code: [12, 13, 14, 16, 18],
} as const;
export type LineHeight = 1.5 | 1.75 | 2;

export interface TypographyPreferences {
  uiFont: FontId<'ui'>;
  readingFont: FontId<'reading'>;
  codeFont: FontId<'code'>;
  uiSize: (typeof FONT_SIZES.ui)[number];
  readingSize: (typeof FONT_SIZES.reading)[number];
  codeSize: (typeof FONT_SIZES.code)[number];
  lineHeight: LineHeight;
}

const FONT_STACKS: Record<FontId, string> = {
  system: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
  humanist: '"Segoe UI", "Trebuchet MS", sans-serif',
  serif: 'Georgia, "Songti SC", serif',
  mono: '"SF Mono", Monaco, "Cascadia Code", monospace',
};

export function resolveTypography(preferences: TypographyPreferences): Readonly<Record<string, string>> {
  return {
    '--font-family-ui': FONT_STACKS[preferences.uiFont],
    '--font-family-reading': FONT_STACKS[preferences.readingFont],
    '--font-family-code': FONT_STACKS[preferences.codeFont],
    '--font-size-ui': `${Math.max(11, preferences.uiSize)}px`,
    '--font-size-reading': `${Math.max(11, preferences.readingSize)}px`,
    '--font-size-code': `${Math.max(11, preferences.codeSize)}px`,
    '--line-height-reading': `${preferences.lineHeight}`,
    '--line-height-code': '1.5',
  };
}
