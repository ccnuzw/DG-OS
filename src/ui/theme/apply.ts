import { resolveTheme } from './kernel';
import { resolveTypography } from './typography';
import { parseAppearance, type Appearance, type ResolvedTheme } from '../appearance/model';

const COLOR_NAMES = new Set([
  '--color-primary','--color-primary-hover','--color-primary-active','--color-primary-light','--color-on-primary',
  '--color-bg-primary','--color-bg-secondary','--color-bg-tertiary','--color-bg-rail',
  '--color-text-primary','--color-text-secondary','--color-text-tertiary','--color-border','--color-border-control','--color-divider','--color-focus-ring',
  '--surface-reading','--text-reading','--surface-code','--text-code',
  ...['success','warning','error','info'].flatMap(name=>[`--color-${name}`,`--color-${name}-text`,`--color-${name}-bg`]),'--color-on-error',
]);
const TYPE_NAMES = new Set(['--font-family-ui','--font-family-reading','--font-family-code','--font-size-ui','--font-size-reading','--font-size-code','--line-height-reading','--line-height-code']);

/** D126: only this file can write theme styles; inputs are revalidated before any DOM mutation. */
export function applyTheme(root: HTMLElement, preferences: Appearance, mode: ResolvedTheme): void {
  const values = parseAppearance(preferences);
  if (mode !== 'light' && mode !== 'dark') throw new Error('Invalid resolved theme mode');
  const variant=values.variants[mode];
  const colors=resolveTheme(variant.presetId,mode,variant.overrides);
  const typography=resolveTypography({
    uiFont:values.typography.uiFontId,readingFont:values.typography.readingFontId,codeFont:values.typography.codeFontId,
    uiSize:values.typography.uiSize as 12|13|14|16,readingSize:values.typography.readingSize as 13|14|16|18|20,codeSize:values.typography.codeSize as 12|13|14|16|18,lineHeight:values.typography.lineHeight,
  });
  for (const [name,value] of Object.entries(colors)) {
    if (!COLOR_NAMES.has(name) || !/^#[0-9a-f]{6}$/.test(value)) throw new Error('Invalid resolved theme color');
  }
  for (const [name,value] of Object.entries(typography)) {
    if (!TYPE_NAMES.has(name) || /[;{}<>\\]|url\(/i.test(value)) throw new Error('Invalid resolved typography');
  }
  for (const [name,value] of Object.entries({...colors,...typography})) root.style.setProperty(name,value);
  root.setAttribute('data-theme',mode);
  root.setAttribute('data-preset',variant.presetId);
  root.setAttribute('data-density',String(values.density));
  root.setAttribute('data-reduced-motion',values.effects.reducedMotion);
  root.setAttribute('data-sidebar-treatment',values.effects.sidebarTreatment);
}
