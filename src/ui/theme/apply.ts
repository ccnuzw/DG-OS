import { resolveTheme } from './kernel';
import { resolveTypography } from './typography';
import { ensureFont, fontStack, type FontRole, type FontSelection } from './fonts';
import { parseAppearance, type Appearance, type ResolvedTheme } from '../appearance/model';

const COLOR_NAMES = new Set([
  '--color-primary','--color-primary-hover','--color-primary-active','--color-primary-light','--color-on-primary',
  '--color-bg-primary','--color-bg-secondary','--color-bg-tertiary','--color-bg-rail',
  '--color-text-primary','--color-text-secondary','--color-text-tertiary','--color-border','--color-border-control','--color-divider','--color-focus-ring',
  '--surface-reading','--text-reading','--surface-code','--text-code',
  ...['success','warning','error','info'].flatMap(name=>[`--color-${name}`,`--color-${name}-text`,`--color-${name}-bg`]),'--color-on-error',
]);
const TYPE_NAMES = new Set(['--font-family-ui','--font-family-reading','--font-family-code','--font-size-ui','--font-size-reading','--font-size-code','--line-height-reading','--line-height-code']);

/** Local copies of kernel-backed roles from tokens/colors.css; static roles remain inherited. */
const SAMPLE_ALIASES: Readonly<Record<string, string>> = Object.freeze({
  '--surface-canvas': '--color-bg-primary', '--surface-rail': '--color-bg-rail',
  '--surface-panel': '--color-bg-secondary', '--surface-card': '--color-bg-secondary',
  '--surface-inset': '--color-bg-tertiary', '--surface-control': '--color-bg-secondary',
  '--surface-field': '--color-bg-primary', '--surface-control-hover': '--color-border',
  '--surface-selection-neutral': '--color-bg-tertiary', '--surface-selected': '--color-primary-light',
  '--surface-hover': '--color-bg-tertiary', '--surface-active': '--color-primary-light',
  '--border-subtle': '--color-border', '--border-control': '--color-border-control',
  '--border-focus': '--color-focus-ring', '--border-divider': '--color-divider',
  '--text-primary': '--color-text-primary', '--text-secondary': '--color-text-secondary',
  '--text-tertiary': '--color-text-tertiary',
  '--accent-default': '--color-primary', '--accent-hover': '--color-primary-hover',
  '--accent-active': '--color-primary-active', '--accent-on': '--color-on-primary',
  '--status-success': '--color-success', '--status-success-surface': '--color-success-bg', '--status-success-text': '--color-success-text',
  '--status-warning': '--color-warning', '--status-warning-surface': '--color-warning-bg', '--status-warning-text': '--color-warning-text',
  '--status-error': '--color-error', '--status-error-surface': '--color-error-bg', '--status-error-text': '--color-error-text', '--status-error-on': '--color-on-error',
  '--status-info': '--color-info', '--status-info-surface': '--color-info-bg', '--status-info-text': '--color-info-text',
  '--surface-popover': '--color-bg-primary', '--surface-modal': '--color-bg-secondary',
  '--text-content': '--color-text-primary', '--surface-content': '--color-bg-primary',
});

/** D126: only this file can write theme styles; inputs are revalidated before any DOM mutation. */
export function applyTheme(root: HTMLElement, preferences: Appearance, mode: ResolvedTheme): void {
  applyResolvedAppearance(root, preferences, mode, false);
}

function applyResolvedAppearance(root: HTMLElement, preferences: Appearance, mode: ResolvedTheme, sample: boolean): void {
  const values = parseAppearance(preferences);
  if (mode !== 'light' && mode !== 'dark') throw new Error('Invalid resolved theme mode');
  const variant=values.variants[mode];
  const colors=resolveTheme(variant.presetId,mode,variant.overrides);
  const typography=resolveTypography({
    uiFont:values.typography.uiFont,readingFont:values.typography.readingFont,codeFont:values.typography.codeFont,
    uiSize:values.typography.uiSize,readingSize:values.typography.readingSize,codeSize:values.typography.codeSize,lineHeight:values.typography.lineHeight,readingFollowUi:values.typography.readingFollowUi,
  });
  for (const [name,value] of Object.entries(colors)) {
    if (!COLOR_NAMES.has(name) || !/^#[0-9a-f]{6}$/.test(value)) throw new Error('Invalid resolved theme color');
  }
  for (const name of Object.keys(typography)) {
    if (!TYPE_NAMES.has(name)) throw new Error('Invalid resolved typography');
  }
  const aliases: Record<string, string> = {};
  if (sample) {
    for (const [name, source] of Object.entries(SAMPLE_ALIASES)) {
      const value = colors[source];
      if (!COLOR_NAMES.has(source) || !/^#[0-9a-f]{6}$/.test(value)) throw new Error('Invalid sample color alias');
      aliases[name] = value;
    }
  }
  for (const [name,value] of Object.entries({...colors,...typography,...aliases})) root.style.setProperty(name,value);
  root.setAttribute('data-theme',mode);
  root.setAttribute('data-preset',variant.presetId);
  root.setAttribute('data-density',String(values.density));
  root.setAttribute('data-reduced-motion',values.effects.reducedMotion);
  root.setAttribute('data-sidebar-treatment',values.effects.sidebarTreatment);
  // Local fonts may settle after the native catalogue; notify independent renderers.
  const roles = ['ui', 'reading', 'code'] as const;
  for (const role of roles) {
    const font = role === 'reading' && values.typography.readingFollowUi ? values.typography.uiFont : values.typography[`${role}Font`];
    void ensureFont(font, role).then(() => root.dispatchEvent(new CustomEvent('appearance-font-ready')), () => root.dispatchEvent(new CustomEvent('appearance-font-error', { detail: { font, role } })));
  }
}

/** Isolated font sample: same validated resolver, a single scoped typography variable. */
export function applyFontSample(root: HTMLElement, font: FontSelection, role: FontRole): void {
  const name = '--font-family-ui';
  const value = fontStack(font, role);
  root.style.setProperty(name, value);
}

function assertLocalSample(root: HTMLElement): void {
  const document = root.ownerDocument;
  if (root === document.documentElement || root === document.body || root.tagName === 'HTML' || root.tagName === 'BODY') throw new Error('Appearance samples require a local element');
}

/** Validate only the chosen color variant, while retaining the complete typography validation. */
export function applyAppearanceSample(root: HTMLElement, appearance: Appearance, mode: ResolvedTheme): void {
  assertLocalSample(root);
  if (mode !== 'light' && mode !== 'dark') throw new Error('Invalid resolved theme mode');
  const otherMode = mode === 'light' ? 'dark' : 'light';
  const variant = appearance.variants[mode];
  const values = {
    ...appearance,
    variants: { ...appearance.variants, [otherMode]: { presetId: variant.presetId, overrides: { interface: {}, reading: {}, code: {} } } },
  };
  applyResolvedAppearance(root, values, mode, true);
}

/** Isolated invalid-contrast example: accept only complete hex pairs, never arbitrary CSS. */
export function applyColorPairSample(root: HTMLElement, background: string, foreground: string, scope: 'reading' | 'code' = 'reading'): void {
  assertLocalSample(root);
  if (scope !== 'reading' && scope !== 'code') throw new Error('Invalid color sample scope');
  if (![background, foreground].every(value => typeof value === 'string' && /^#[0-9a-fA-F]{6}$/.test(value))) throw new Error('Theme colors must be #RRGGBB values');
  const colors = { [`--surface-${scope}`]: background.toLowerCase(), [`--text-${scope}`]: foreground.toLowerCase() };
  for (const [name, value] of Object.entries(colors)) {
    if (!COLOR_NAMES.has(name)) throw new Error('Invalid sample color');
    root.style.setProperty(name, value);
  }
}
