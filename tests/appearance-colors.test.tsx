import { describe, expect, it } from 'vitest';
import { colorIssues, effectiveColors, overrideCount, resetRegion, setColor, withPreset } from '../src/ui/appearance/colors';
import { DEFAULTS, type Variant } from '../src/ui/appearance/model';
import { applyAppearanceSample, applyColorPairSample } from '../src/ui/theme/apply';
import { PRESETS, PRESET_IDS, resolveTheme, ThemeColorError, type ThemeMode, type ThemeScope } from '../src/ui/theme/kernel';

const variant = (): Variant => ({
  presetId: 'studio',
  overrides: {
    interface: { background: '#ffffff', foreground: '#111111', accent: '#224488' },
    reading: { background: '#ffffee' },
    code: { foreground: '#222222' },
  },
});

describe('appearance color inheritance and editing', () => {
  it('inherits each field directly from interface and keeps code independent of reading', () => {
    const value = variant();
    expect(effectiveColors(value, 'light', 'interface')).toEqual({ background: '#ffffff', foreground: '#111111', accent: '#224488' });
    expect(effectiveColors(value, 'light', 'reading')).toEqual({ background: '#ffffee', foreground: '#111111', accent: '#224488' });
    expect(effectiveColors(value, 'light', 'code')).toEqual({ background: '#ffffff', foreground: '#222222', accent: '#224488' });
    const restored = setColor(value, 'interface', 'background', undefined);
    expect(effectiveColors(restored, 'dark', 'code').background).toBe(PRESETS.studio.dark.background);
    expect(effectiveColors(restored, 'dark', 'reading').background).toBe('#ffffee');
  });

  it('counts explicit fields even when equal to the inherited preset', () => {
    const value = structuredClone(DEFAULTS.variants.light);
    expect(overrideCount(value)).toBe(0);
    expect(overrideCount(setColor(value, 'interface', 'accent', PRESETS.studio.light.accent))).toBe(1);
    expect(overrideCount(variant())).toBe(5);
  });

  it('sets and restores one field without changing any other field or the source', () => {
    const value = variant();
    const before = structuredClone(value);
    const changed = setColor(value, 'reading', 'foreground', '#333333');
    expect(changed.overrides).toEqual({ ...before.overrides, reading: { background: '#ffffee', foreground: '#333333' } });
    const restored = setColor(changed, 'reading', 'background', undefined);
    expect(restored.overrides).toEqual({ ...before.overrides, reading: { foreground: '#333333' } });
    expect(effectiveColors(restored, 'light', 'reading').background).toBe('#ffffff');
    expect(value).toEqual(before);
    expect(changed.overrides.interface).toBe(value.overrides.interface);
    expect(changed.overrides.code).toBe(value.overrides.code);
  });

  it.each(['interface', 'reading', 'code'] as const)('resets only the %s region', scope => {
    const value = variant();
    const cleared = resetRegion(value, scope);
    expect(cleared.overrides).toEqual({ ...value.overrides, [scope]: {} });
    expect(value.overrides[scope]).not.toEqual({});
    if (scope === 'interface') {
      expect(effectiveColors(cleared, 'light', 'reading')).toEqual({ ...PRESETS.studio.light, background: '#ffffee' });
      expect(effectiveColors(cleared, 'light', 'code')).toEqual({ ...PRESETS.studio.light, foreground: '#222222' });
    }
  });

  it('keeps or resets all overrides when changing preset, with a no-op for the same preset', () => {
    const value = variant();
    expect(withPreset(value, 'studio', false)).toBe(value);
    expect(withPreset(value, 'studio', true)).toBe(value);
    const kept = withPreset(value, 'paper', true);
    expect(kept).toEqual({ ...value, presetId: 'paper' });
    expect(kept.overrides).not.toBe(value.overrides);
    expect(kept.overrides.reading).not.toBe(value.overrides.reading);
    expect(withPreset(value, 'forest', false)).toEqual({ presetId: 'forest', overrides: { interface: {}, reading: {}, code: {} } });
    expect(value.presetId).toBe('studio');
    expect(overrideCount(value)).toBe(5);
  });

  it('rejects non-interface accent writes at runtime and in the public type contract', () => {
    expect(() => {
      // @ts-expect-error Reading has no independent accent.
      setColor(variant(), 'reading', 'accent', '#224488');
    }).toThrow('Accent overrides belong to interface');
    expect(() => {
      // @ts-expect-error Code has no independent accent.
      setColor(variant(), 'code', 'accent', undefined);
    }).toThrow('Accent overrides belong to interface');
  });
});

describe('shared kernel color diagnostics', () => {
  it.each(PRESET_IDS)('keeps both %s variants free of diagnostics', presetId => {
    const appearance = structuredClone(DEFAULTS);
    appearance.variants.light = { presetId, overrides: {} };
    appearance.variants.dark = { presetId, overrides: {} };
    expect(colorIssues(appearance)).toEqual([]);
  });

  it('collects independent contrast problems in both modes, including inactive regions', () => {
    const appearance = structuredClone(DEFAULTS);
    appearance.variants.light.overrides = { interface: { accent: '#ffffff' }, reading: { background: '#ffffff', foreground: '#ffffff' } };
    appearance.variants.dark.overrides = { code: { background: '#111111', foreground: '#111111' } };
    expect(colorIssues(appearance)).toEqual(expect.arrayContaining([
      { mode: 'light', scope: 'interface', fields: ['accent', 'background', 'foreground'], kind: 'accentContrast' },
      { mode: 'light', scope: 'reading', fields: ['background', 'foreground'], kind: 'textContrast' },
      { mode: 'dark', scope: 'code', fields: ['background', 'foreground'], kind: 'textContrast' },
    ]));
  });

  it('reports every malformed explicit field without parsing it as CSS', () => {
    const appearance = structuredClone(DEFAULTS);
    appearance.variants.light.overrides = { interface: { accent: 'url(secret)' }, reading: { background: '#fff' } };
    appearance.variants.dark.overrides = { code: { foreground: 'transparent' } };
    expect(colorIssues(appearance)).toEqual([
      { mode: 'light', scope: 'interface', fields: ['accent'], kind: 'hex' },
      { mode: 'light', scope: 'reading', fields: ['background'], kind: 'hex' },
      { mode: 'dark', scope: 'code', fields: ['foreground'], kind: 'hex' },
    ]);
    expect(() => resolveTheme('studio', 'light', appearance.variants.light.overrides)).toThrow(ThemeColorError);
    expect(appearance.variants.light.overrides.interface?.accent).toBe('url(secret)');
  });

  it('reports derived contrast failure without changing an otherwise valid seed pair', () => {
    const appearance = structuredClone(DEFAULTS);
    appearance.variants.light.overrides = { interface: { background: '#000000', foreground: '#ffffff', accent: '#ffffff' } };
    const before = structuredClone(appearance);
    expect(colorIssues(appearance)).toEqual([
      { mode: 'light', scope: 'interface', fields: ['accent', 'background', 'foreground'], kind: 'derivedContrast' },
    ]);
    expect(() => resolveTheme('studio', 'light', appearance.variants.light.overrides)).toThrow(ThemeColorError);
    expect(appearance).toEqual(before);
  });

  it.each(['light', 'dark'] as const)('diagnoses the same %s seeds that the resolver rejects', mode => {
    const cases: { scope: ThemeScope; overrides: Variant['overrides']; kind: string }[] = [
      { scope: 'interface', overrides: { interface: { background: '#777777', foreground: '#777777' } }, kind: 'textContrast' },
      { scope: 'interface', overrides: { interface: { accent: PRESETS.studio[mode].background } }, kind: 'accentContrast' },
      { scope: 'reading', overrides: { reading: { background: '#ffffff', foreground: '#ffffff' } }, kind: 'textContrast' },
      { scope: 'code', overrides: { code: { background: '#ffffff', foreground: '#ffffff' } }, kind: 'textContrast' },
    ];
    for (const test of cases) {
      const appearance = structuredClone(DEFAULTS);
      appearance.variants[mode].overrides = test.overrides;
      expect(() => resolveTheme('studio', mode, test.overrides)).toThrow(ThemeColorError);
      expect(colorIssues(appearance)).toEqual(expect.arrayContaining([expect.objectContaining({ mode, scope: test.scope, kind: test.kind })]));
    }
  });
});

describe('isolated controlled appearance samples', () => {
  it.each(['light', 'dark'] as const)('renders valid %s despite an invalid opposite mode without touching the document root', mode => {
    const appearance = structuredClone(DEFAULTS);
    const otherMode: ThemeMode = mode === 'light' ? 'dark' : 'light';
    appearance.variants[otherMode].overrides = { interface: { accent: 'partial' } };
    const beforeRoot = document.documentElement.getAttribute('style');
    const beforeBody = document.body.getAttribute('style');
    const sample = document.createElement('section');
    applyAppearanceSample(sample, appearance, mode);
    const expected = resolveTheme('studio', mode);
    expect(sample.style.getPropertyValue('--color-bg-primary')).toBe(expected['--color-bg-primary']);
    expect(sample.style.getPropertyValue('--accent-default')).toBe(expected['--color-primary']);
    expect(sample.style.getPropertyValue('--surface-canvas')).toBe(expected['--color-bg-primary']);
    expect(sample.style.getPropertyValue('--text-primary')).toBe(expected['--color-text-primary']);
    expect(sample.style.getPropertyValue('--status-error-text')).toBe(expected['--color-error-text']);
    expect(sample.style.getPropertyValue('--surface-code')).toBe(expected['--surface-code']);
    expect(sample.style.getPropertyValue('--font-size-ui')).toBe('12px');
    expect(sample.dataset.theme).toBe(mode);
    expect(document.documentElement.getAttribute('style')).toBe(beforeRoot);
    expect(document.body.getAttribute('style')).toBe(beforeBody);
    expect(appearance.variants[otherMode].overrides.interface?.accent).toBe('partial');
  });

  it('does not mutate a local sample when its chosen variant or typography is invalid', () => {
    const appearance = structuredClone(DEFAULTS);
    const sample = document.createElement('section');
    applyAppearanceSample(sample, appearance, 'light');
    const before = sample.outerHTML;
    appearance.variants.light.overrides = { interface: { foreground: '#ffffff' } };
    expect(() => applyAppearanceSample(sample, appearance, 'light')).toThrow();
    expect(sample.outerHTML).toBe(before);
    appearance.variants.light = structuredClone(DEFAULTS.variants.light);
    appearance.typography.uiSize = 999;
    expect(() => applyAppearanceSample(sample, appearance, 'light')).toThrow();
    expect(sample.outerHTML).toBe(before);
  });

  it.each(['html', 'body'] as const)('rejects global %s as a sample root', tag => {
    const root = tag === 'html' ? document.documentElement : document.body;
    const before = root.outerHTML;
    expect(() => applyAppearanceSample(root, DEFAULTS, 'light')).toThrow('local element');
    expect(() => applyColorPairSample(root, '#ffffff', '#ffffff')).toThrow('local element');
    expect(root.outerHTML).toBe(before);
  });

  it('shows an invalid contrast pair only through two permitted reading sample variables', () => {
    const root = document.createElement('section');
    const beforeGlobal = document.documentElement.getAttribute('style');
    applyColorPairSample(root, '#FFFFFF', '#FFFFFF');
    expect(root.style.length).toBe(2);
    expect(root.style.getPropertyValue('--surface-reading')).toBe('#ffffff');
    expect(root.style.getPropertyValue('--text-reading')).toBe('#ffffff');
    expect(root.style.getPropertyValue('--surface-code')).toBe('');
    expect(document.documentElement.getAttribute('style')).toBe(beforeGlobal);
    const before = root.getAttribute('style');
    expect(() => applyColorPairSample(root, '#000000', 'url(secret)')).toThrow();
    expect(() => applyColorPairSample(root, 'transparent', '#000000')).toThrow();
    expect(root.getAttribute('style')).toBe(before);
  });

  it.each(['reading', 'code'] as const)('writes only the selected %s pair for an isolated low-contrast sample', scope => {
    const root = document.createElement(scope === 'code' ? 'pre' : 'section');
    applyColorPairSample(root, '#AAAAAA', '#AAAAAA', scope);
    expect(root.style.length).toBe(2);
    expect(root.style.getPropertyValue(`--surface-${scope}`)).toBe('#aaaaaa');
    expect(root.style.getPropertyValue(`--text-${scope}`)).toBe('#aaaaaa');
    const otherScope = scope === 'reading' ? 'code' : 'reading';
    expect(root.style.getPropertyValue(`--surface-${otherScope}`)).toBe('');
    expect(root.style.getPropertyValue(`--text-${otherScope}`)).toBe('');
    const before = root.getAttribute('style');
    expect(() => applyColorPairSample(root, '#000000', 'url(secret)', scope)).toThrow();
    expect(root.getAttribute('style')).toBe(before);
  });

  it('rejects pair scopes outside reading and code without mutation', () => {
    const root = document.createElement('section');
    expect(() => {
      // @ts-expect-error Interface colors cannot be written through the pair sample API.
      applyColorPairSample(root, '#ffffff', '#ffffff', 'interface');
    }).toThrow('Invalid color sample scope');
    expect(root.getAttribute('style')).toBeNull();
  });
});
