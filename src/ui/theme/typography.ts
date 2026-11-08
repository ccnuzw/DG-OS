import { fontStack, type FontSelection } from './fonts';
export type { FontRole } from './fonts';
export const FONT_SIZES = {
  ui: Array.from({ length: 9 }, (_, index) => index + 12),
  reading: Array.from({ length: 13 }, (_, index) => index + 12),
  code: Array.from({ length: 13 }, (_, index) => index + 12),
} as const;
export type LineHeight = 1.5 | 1.75 | 2;
export interface TypographyPreferences {
  uiFont: FontSelection;
  readingFont: FontSelection;
  codeFont: FontSelection;
  uiSize: number;
  readingSize: number;
  codeSize: number;
  lineHeight: LineHeight;
  readingFollowUi: boolean;
}

export function resolveTypography(preferences: TypographyPreferences): Readonly<Record<string, string>> {
  const follow = preferences.readingFollowUi === true;
  return {
    '--font-family-ui': fontStack(preferences.uiFont, 'ui'),
    '--font-family-reading': fontStack(follow ? preferences.uiFont : preferences.readingFont, 'reading'),
    '--font-family-code': fontStack(preferences.codeFont, 'code'),
    '--font-size-ui': `${preferences.uiSize}px`,
    '--font-size-reading': `${follow ? preferences.uiSize : preferences.readingSize}px`,
    '--font-size-code': `${preferences.codeSize}px`,
    '--line-height-reading': `${preferences.lineHeight}`,
    '--line-height-code': '1.5',
  };
}
