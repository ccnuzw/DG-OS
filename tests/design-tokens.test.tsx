// @vitest-environment node
// V1-FR-005 Phase 1: AC20 (token contrast) and the invariants of the token files and the Tailwind mapping
// (D064 minimum font size, D066 14px root, D069 paired foreground tokens).
import { describe, expect, it } from 'vitest';
import { cn } from '@dgos/ui';
import pairsFile from './fixtures/design-tokens/contrast-pairs.json';
import { compileUi, contrast, parseBlocks, readUi, themeTokens } from './helpers/designTokens';

const { light, dark, darkOnly } = themeTokens();
const colorNames = (tokens: Map<string, string>) =>
  [...tokens.keys()].filter((name) => name.startsWith('--color-')).map((name) => name.slice('--color-'.length));
const themes = { light, dark } as const;
const pairs = pairsFile.pairs as { kind: 'text' | 'non-text'; fg: string; bg: string }[];
const exempt = Object.keys(pairsFile.exempt);

function colorOf(tokens: Map<string, string>, name: string): string {
  const value = tokens.get(`--color-${name}`);
  if (!value) throw new Error(`pair references undefined token --color-${name}`);
  return value;
}

describe('colour tokens (AC01, AC20)', () => {
  it('light and dark define the same set of colour names', () => {
    expect(colorNames(dark).sort()).toEqual(colorNames(light).sort());
    // every dark override replaces a light token; a dark-only name would be undefined in light
    for (const name of darkOnly.keys()) if (name.startsWith('--color-')) expect(light.has(name)).toBe(true);
  });

  it.each(Object.keys(themes))('every declared pair meets its threshold in %s', (theme) => {
    const tokens = themes[theme as keyof typeof themes];
    const failing = pairs
      .map((pair) => ({ ...pair, ratio: contrast(colorOf(tokens, pair.fg), colorOf(tokens, pair.bg)) }))
      .filter((pair) => pair.ratio < (pair.kind === 'text' ? 4.5 : 3))
      .map((pair) => `${pair.fg} on ${pair.bg} = ${pair.ratio.toFixed(2)} (${pair.kind})`);
    expect(failing).toEqual([]);
  });

  it('every colour token is in a declared pair or explicitly exempt', () => {
    const covered = new Set([...pairs.flatMap((pair) => [pair.fg, pair.bg]), ...exempt]);
    expect(colorNames(light).filter((name) => !covered.has(name))).toEqual([]);
  });

  it('exempt tokens exist (a stale exemption would hide a rename)', () => {
    for (const name of exempt) expect(light.has(`--color-${name}`)).toBe(true);
  });

  it('the control border is darker than the decorative border so controls stay distinguishable', () => {
    for (const tokens of [light, dark]) {
      const surface = colorOf(tokens, 'bg-primary');
      expect(contrast(colorOf(tokens, 'border-control'), surface)).toBeGreaterThan(contrast(colorOf(tokens, 'border'), surface));
    }
  });
});

describe('type scale and minimum size (D064)', () => {
  const sizes = Object.fromEntries(
    [...light]
      .filter(([name]) => /^--font-size-(xs|sm|base|lg|xl|2xl|3xl|4xl)$/.test(name))
      .map(([name, value]) => {
        const match = /^max\(calc\((\d+)rem \/ 14\), var\(--font-size-min\)\)$/.exec(value);
        if (!match) throw new Error(`${name}: unexpected form ${value}`);
        return [name.replace('--font-size-', ''), Number(match[1])];
      }),
  );

  it('matches the FR-005 §4.2.2 px values at the 100% root', () => {
    expect(sizes).toEqual({ xs: 12, sm: 13, base: 14, lg: 16, xl: 18, '2xl': 20, '3xl': 24, '4xl': 30 });
  });

  it('the floor is 11px', () => {
    expect(light.get('--font-size-min')).toBe('11px');
  });

  it.each([75, 100, 125, 150, 175])('no size token renders below 11px at %i%% (and none shrinks past the floor order)', (density) => {
    const root = (14 * density) / 100;
    const effective = Object.values(sizes).map((px) => Math.max((px / 14) * root, 11));
    for (const px of effective) expect(px).toBeGreaterThanOrEqual(11);
    expect(effective).toEqual([...effective].sort((a, b) => a - b));
  });

  it('the five density selectors set the root to 14px x scale', () => {
    const rootSizes = new Map(
      parseBlocks(readUi('tokens', 'density.css')).map((block) => [block.prelude, block.declarations.get('font-size')]),
    );
    expect(Object.fromEntries(rootSizes)).toEqual({
      html: '14px',
      'html[data-density="75"]': '10.5px',
      'html[data-density="125"]': '17.5px',
      'html[data-density="150"]': '21px',
      'html[data-density="175"]': '24.5px',
    });
  });
});

describe('spacing and radius use the 14px root (D064, D066)', () => {
  it('the spacing unit is 4px and every step is an integer multiple of it', () => {
    expect(light.get('--spacing-unit')).toBe('calc(4rem / 14)');
    const steps = [...light].filter(([name]) => /^--spacing-\d+$/.test(name));
    expect(steps.map(([name]) => Number(name.slice('--spacing-'.length)))).toEqual([0, 1, 2, 3, 4, 5, 6, 8, 10, 12]);
    for (const [name, value] of steps) {
      expect(value).toBe(`calc(var(--spacing-unit) * ${name.slice('--spacing-'.length)})`);
    }
  });

  it('radii are the FR-005 §4.4 px values in rem on a 14px root', () => {
    const radii = Object.fromEntries([...light].filter(([name]) => name.startsWith('--radius-')));
    expect(radii).toEqual({
      '--radius-none': '0px',
      '--radius-sm': 'calc(4rem / 14)',
      '--radius-md': 'calc(8rem / 14)',
      '--radius-lg': 'calc(12rem / 14)',
      '--radius-xl': 'calc(16rem / 14)',
      '--radius-full': '9999px',
    });
  });

  it('dark shadows are stronger than light ones at every size (FR-005 §4.5)', () => {
    for (const size of ['sm', 'md', 'lg', 'xl']) {
      expect(Number(dark.get(`--shadow-alpha-${size}`))).toBeGreaterThan(Number(light.get(`--shadow-alpha-${size}`)));
    }
  });
});

describe('Tailwind mapping (D066 spike)', () => {
  const candidates = [
    'p-2', 'gap-3', 'text-sm', 'text-4xl', 'rounded-md', 'font-medium', 'leading-tight',
    'bg-primary', 'bg-background', 'text-foreground', 'text-muted-foreground', 'bg-card', 'bg-secondary',
    'border-input', 'ring-ring', 'bg-destructive', 'text-destructive-foreground', 'text-primary-foreground',
    'bg-white', 'text-red-500', 'bg-blue-500', 'sm:p-2',
  ];

  it('maps utilities onto FR-005 tokens, with the spacing base following the 4px unit', async () => {
    const css = await compileUi(candidates);
    const expected: Record<string, string> = {
      '.p-2': 'padding: calc(var(--spacing-unit) * 2)',
      '.gap-3': 'gap: calc(var(--spacing-unit) * 3)',
      '.text-sm': 'font-size: var(--font-size-sm)',
      '.text-4xl': 'font-size: var(--font-size-4xl)',
      '.rounded-md': 'border-radius: var(--radius-md)',
      '.bg-background': 'background-color: var(--color-bg-primary)',
      '.text-foreground': 'color: var(--color-text-primary)',
      '.text-muted-foreground': 'color: var(--color-text-secondary)',
      '.bg-card': 'background-color: var(--color-bg-secondary)',
      '.bg-secondary': 'background-color: var(--color-bg-tertiary)',
      '.border-input': 'border-color: var(--color-border-control)',
      '.bg-destructive': 'background-color: var(--color-error)',
      '.text-destructive-foreground': 'color: var(--color-on-error)',
      '.text-primary-foreground': 'color: var(--color-on-primary)',
      '.bg-primary': 'background-color: var(--color-primary)',
    };
    for (const [selector, declaration] of Object.entries(expected)) {
      const rule = new RegExp(`${selector.replace('.', '\\.')} \\{([^}]*)\\}`).exec(css);
      expect(rule?.[1], `${selector} missing from the build`).toContain(declaration);
    }
  });

  it('the default palette and breakpoints are gone (bg-white, text-red-500, bg-blue-500, sm:)', async () => {
    const css = await compileUi(candidates);
    for (const selector of ['.bg-white', '.text-red-500', '.bg-blue-500', 'sm\\:p-2']) {
      expect(css).not.toContain(selector);
    }
  });

  it('light tokens are emitted even when no utility uses them (dynamic modules read them)', async () => {
    const css = await compileUi([]);
    for (const name of ['--color-success', '--color-warning', '--shadow-xl', '--opacity-overlay']) {
      expect(css).toContain(`${name}:`);
    }
  });

  it('no custom property refers to itself (the same-name alias trap found in the spike)', async () => {
    const css = await compileUi(candidates);
    const selfReferences = [...css.matchAll(/(--[\w-]+):\s*var\(\1\)/g)].map((match) => match[1]);
    expect(selfReferences).toEqual([]);
  });

  it('every Shadcn semantic colour resolves to a token that exists', () => {
    const mapping = readUi('tailwind.css').matchAll(/--color-([\w-]+):\s*var\(--color-([\w-]+)\)/g);
    const entries = [...mapping];
    expect(entries.length).toBeGreaterThan(10);
    for (const [, key, target] of entries) {
      expect(light.has(`--color-${target}`), `--color-${key} points at missing --color-${target}`).toBe(true);
    }
  });
});

describe('cn() with the custom tokens', () => {
  it('lets the last utility of a kind win and keeps utilities of different kinds', () => {
    expect(cn('bg-primary', 'bg-secondary')).toBe('bg-secondary');
    expect(cn('p-2', 'p-4')).toBe('p-4');
    expect(cn('rounded-sm', 'rounded-md')).toBe('rounded-md');
    expect(cn('text-sm', 'text-lg')).toBe('text-lg');
    // a text size and a text colour are different kinds, including for the token-named colours
    expect(cn('text-sm', 'text-foreground')).toBe('text-sm text-foreground');
    expect(cn('text-muted-foreground', 'text-destructive-foreground')).toBe('text-destructive-foreground');
    expect(cn('px-2', false && 'hidden', undefined, 'py-1')).toBe('px-2 py-1');
  });
});
