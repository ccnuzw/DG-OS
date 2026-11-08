import { getFontCatalogue, validFontFamily } from './fontCatalogue';

export type FontRole = 'ui' | 'reading' | 'code';
export type FontSelection = { kind: 'system-ui' } | { kind: 'system-mono' } | { kind: 'local'; family: string };
export const SYSTEM_UI = 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", sans-serif';
export const SYSTEM_MONO = 'ui-monospace, "SF Mono", Menlo, Monaco, Consolas, "Liberation Mono", "Noto Sans Mono CJK SC", "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", monospace';

export function isFontSelection(value: unknown, role: FontRole): value is FontSelection {
  if (!value || typeof value !== 'object' || !('kind' in value)) return false;
  if (value.kind === 'local') return Object.keys(value).length === 2 && 'family' in value && validFontFamily(value.family);
  return Object.keys(value).length === 1 && value.kind === (role === 'code' ? 'system-mono' : 'system-ui');
}
/** CSS string escapes, never a raw declaration fragment. Handles quotes and non-Latin names. */
export function quoteFontFamily(family: string): string {
  if (!validFontFamily(family)) throw new Error('Invalid font family');
  return `"${[...family].map(char => /[\p{L}\p{N} _-]/u.test(char) ? char : `\\${char.codePointAt(0)!.toString(16)} `).join('')}"`;
}
export type FontAvailability = 'system' | 'available' | 'missing' | 'unconfirmed';
export function fontAvailability(font: FontSelection): FontAvailability {
  if (font.kind !== 'local') return 'system';
  const catalogue = getFontCatalogue();
  if (catalogue.status !== 'ready') return 'unconfirmed';
  return catalogue.families.some(item => item.family === font.family) ? 'available' : 'missing';
}
export function fontStack(font: FontSelection, role: FontRole): string {
  if (!isFontSelection(font, role)) throw new Error(`Invalid ${role} font selection`);
  const fallback = role === 'code' ? SYSTEM_MONO : SYSTEM_UI;
  return font.kind === 'local' && fontAvailability(font) === 'available' ? `${quoteFontFamily(font.family)}, ${fallback}` : fallback;
}
/** Native faces are not web FontFace entries. Zero web faces is valid for a local family. */
export async function ensureFont(font: FontSelection, role: FontRole, sample = 'Aa 0123456789 中文字体'): Promise<void> {
  const stack = fontStack(font, role);
  if (typeof document === 'undefined' || !document.fonts || typeof document.fonts.load !== 'function') return;
  await document.fonts.load(`12px ${stack}`, sample);
  await document.fonts.ready;
}
