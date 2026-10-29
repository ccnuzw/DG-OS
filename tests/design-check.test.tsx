// @vitest-environment node
// AC11, AC12, AC22 (FR-005, D068): the style gate must fail on violating fixtures, pass on clean ones, and
// only let the legacy baseline shrink. The fixtures are the proof that the rules fire, not the config files.
import { spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
// @ts-expect-error plain ESM script without type declarations
import { formatReport, runDesignCheck } from '../scripts/design-check.mjs';

vi.setConfig({ testTimeout: 60_000 });

const repo = join(__dirname, '..');
const fixtures = join(repo, 'tests', 'fixtures', 'design-check');
const violationsRoot = join(fixtures, 'violations');
const cleanRoot = join(fixtures, 'clean');

type Finding = { file: string; line: number; column: number; tag: string; rule: string; message: string };
const where = (findings: Finding[]) => findings.map((f) => `${f.file}:${f.line} ${f.tag} ${f.rule}`).sort();

const ESLINT = 'no-restricted-syntax';
// Expected findings of tests/fixtures/design-check/violations, written by reading each fixture line.
const EXPECTED = [
  // inline-style.tsx: .style, setAttribute, createElement('style'), adoptedStyleSheets, insertRule,
  // `{ style }` object, h('div', { style }), JSX style attribute
  ...[3, 4, 5, 6, 7, 8, 10, 11].map((line) => `src/inline-style.tsx:${line} R1 ${ESLINT}`),
  // hard-coded-colors.ts: hex, hex in a declaration, rgba(), hsl(), keyword in a declaration, var() fallback,
  // `color: 'red'` and `backgroundColor: 'Navy'` (line 7 has two); lines 8 and 9 are the allowed forms
  ...[1, 2, 3, 4, 5, 6, 7, 7].map((line) => `src/hard-coded-colors.ts:${line} R2 ${ESLINT}`),
  // tailwind-arbitrary.tsx: line 4 is a hex AND an arbitrary value, line 11 is fill="red" and stroke="Blue"
  ...[4, 4, 5, 6, 7, 8, 9, 10, 11, 11].map((line) => `src/tailwind-arbitrary.tsx:${line} R2 ${ESLINT}`),
  // eslint-disable comments do not hide the colour
  ...[2, 4].map((line) => `src/disable-comments.ts:${line} R2 ${ESLINT}`),
  `src/outside-tokens.ts:1 R2 ${ESLINT}`,
  // under src/ui/tokens only the inline style is reported, not the two colour literals above it
  `src/ui/tokens/literal-colors.ts:4 R1 ${ESLINT}`,
  `bundled-modules/demo/1.0.0/entry.js:3 R1 ${ESLINT}`,
  // focus-ring.tsx (R5): bare, `focus:` only, template literal, JSX className. Lines 4-7 (paired, focus-visible:outline-none,
  // outline-hidden, a lookalike word) are the allowed forms and must stay out of this list.
  ...[1, 2, 3, 8].map((line) => `src/focus-ring.tsx:${line} R5 ${ESLINT}`),
  // legacy.css
  ...[1, 1, 5, 11].map((line) => `src/legacy.css:${line} R2 color-no-hex`),
  ...[3, 3, 5].map((line) => `src/legacy.css:${line} R2 color-named`),
  ...[2, 2].map((line) => `src/legacy.css:${line} R2 function-disallowed-list`),
  ...[7, 7, 8].map((line) => `src/legacy.css:${line} R3 dgos/defined-custom-properties`),
  'src/legacy.css:15 R3 at-rule-no-unknown',
  // index.html
  'index.html:4 R1 html/inline-style',
  'index.html:7 R1 html/inline-style',
  'index.html:6 R2 html/hard-coded-color',
  'index.html:6 R2 html/tailwind-arbitrary-value',
].sort();

describe('violating fixtures are reported (AC11, AC12)', () => {
  it('reports exactly the expected file:line and rule', async () => {
    const result = await runDesignCheck(violationsRoot);
    expect(result.toolErrors).toEqual([]);
    expect(result.ok).toBe(false);
    expect(where(result.violations)).toEqual(EXPECTED);
  });

  it('keeps R4 px usage as a non-blocking warning', async () => {
    const result = await runDesignCheck(violationsRoot);
    const warned = result.warnings.map((w: Finding) => `${w.line} ${w.tag}`);
    // font-size, padding x2, gap on line 13, and the px fallback inside margin: var(--gone, 4px) on line 7
    expect(warned.sort()).toEqual(['13 R4', '13 R4', '13 R4', '7 R4', '13 R4'].sort());
    expect(result.violations.some((v: Finding) => v.tag === 'R4')).toBe(false);
  });

  it('does not let eslint-disable or stylelint-disable comments hide a colour', async () => {
    const result = await runDesignCheck(violationsRoot);
    const found = where(result.violations);
    expect(found).toContain(`src/disable-comments.ts:2 R2 ${ESLINT}`);
    expect(found).toContain(`src/disable-comments.ts:4 R2 ${ESLINT}`);
    expect(found).toContain('src/legacy.css:11 R2 color-no-hex');
  });

  it('allows literal colours under src/ui/tokens but still forbids inline styles there', async () => {
    const result = await runDesignCheck(violationsRoot);
    const inTokens = where(result.violations).filter((entry) => entry.startsWith('src/ui/'));
    expect(inTokens).toEqual([`src/ui/tokens/literal-colors.ts:4 R1 ${ESLINT}`]);
  });

  it('rejects a variable that is only defined in the page stylesheet, with or without a fallback', async () => {
    const result = await runDesignCheck(violationsRoot);
    const messages = result.violations.filter((v: Finding) => v.rule === 'dgos/defined-custom-properties').map((v: Finding) => v.message);
    expect(messages.some((m: string) => m.includes('--skills-ink'))).toBe(true);
    expect(messages.some((m: string) => m.includes('--gone'))).toBe(true);
    expect(messages.some((m: string) => m.includes('--page-ink'))).toBe(true);
    // --radix-* is a runtime variable and --color-background is defined in src/ui/tailwind.css
    expect(messages.some((m: string) => m.includes('--radix-'))).toBe(false);
    expect(messages.some((m: string) => m.includes('--color-background'))).toBe(false);
  });
});

describe('clean fixture passes (AC11, AC12)', () => {
  it('has no findings and no warnings', async () => {
    const result = await runDesignCheck(cleanRoot);
    expect(result.toolErrors).toEqual([]);
    expect(result.violations).toEqual([]);
    expect(result.ok).toBe(true);
  });
});

describe('npm run design:check exit code and output (AC11, AC12)', () => {
  const cli = [join(repo, 'scripts', 'design-check.mjs'), '--root'];

  it('exits non-zero and prints file:line with the rule name for a violation', () => {
    const run1 = spawnSync(process.execPath, [...cli, violationsRoot], { encoding: 'utf8' });
    expect(run1.status).toBe(1);
    expect(run1.stderr).toContain('src/inline-style.tsx:3:');
    expect(run1.stderr).toContain('R1');
    expect(run1.stderr).toContain('src/legacy.css:1:');
    expect(run1.stderr).toContain('color-no-hex');
  });

  it('exits zero for the clean fixture', () => {
    const run2 = spawnSync(process.execPath, [...cli, cleanRoot], { encoding: 'utf8' });
    expect(run2.status).toBe(0);
    expect(run2.stdout).toContain('design:check 通过');
  });
});

describe('legacy baseline only shrinks (AC22)', () => {
  const dirs: string[] = [];
  afterEach(() => {
    for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  });

  /** A root with the clean token files plus one stylesheet with `colors` hex literals. */
  function root(colors: number, baseline?: Record<string, unknown>) {
    const dir = mkdtempSync(join(tmpdir(), 'dgos-design-'));
    dirs.push(dir);
    cpSync(join(cleanRoot, 'src', 'ui'), join(dir, 'src', 'ui'), { recursive: true });
    const rules = Array.from({ length: colors }, (_, i) => `.c${i} { color: #00000${i}; }`).join('\n');
    writeFileSync(join(dir, 'src', 'page.css'), `${rules}\n`);
    if (baseline) {
      const path = join(dir, 'scripts', 'design-check', 'baseline', 'legacy-style.json');
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, JSON.stringify(baseline));
    }
    return dir;
  }
  const allow = (count: number) => ({ 'src/page.css': { 'color-no-hex': { count } } });
  const baselineOf = (dir: string) =>
    JSON.parse(readFileSync(join(dir, 'scripts', 'design-check', 'baseline', 'legacy-style.json'), 'utf8'));

  it('fails for a file that is not in the baseline', async () => {
    const result = await runDesignCheck(root(2));
    expect(result.ok).toBe(false);
    expect(result.violations).toHaveLength(2);
    expect(result.exceeded).toEqual([{ file: 'src/page.css', rule: 'color-no-hex', allowed: 0, actual: 2 }]);
  });

  it('passes while the count equals the baseline', async () => {
    const result = await runDesignCheck(root(2, allow(2)));
    expect(result.ok).toBe(true);
  });

  it('fails and lists every occurrence when the count grows past the baseline', async () => {
    const result = await runDesignCheck(root(3, allow(2)));
    expect(result.ok).toBe(false);
    expect(result.exceeded).toEqual([{ file: 'src/page.css', rule: 'color-no-hex', allowed: 2, actual: 3 }]);
    expect(where(result.violations)).toEqual(['src/page.css:1 R2 color-no-hex', 'src/page.css:2 R2 color-no-hex', 'src/page.css:3 R2 color-no-hex']);
  });

  it('fails when the count dropped below the baseline and says how to shrink it', async () => {
    const result = await runDesignCheck(root(1, allow(3)));
    expect(result.ok).toBe(false);
    expect(result.violations).toEqual([]);
    expect(result.stale).toEqual([
      { baseline: 'scripts/design-check/baseline/legacy-style.json', file: 'src/page.css', rule: 'color-no-hex', allowed: 3, actual: 1 },
    ]);
    expect(formatReport(result)).toContain('design:prune');
  });

  it('fails when a baseline entry points at a file that no longer has violations', async () => {
    const result = await runDesignCheck(root(0, allow(1)));
    expect(result.ok).toBe(false);
    expect(result.stale).toHaveLength(1);
  });

  it('--prune shrinks the baseline to the current counts and the next check passes', async () => {
    const dir = root(1, { ...allow(3), 'src/gone.css': { 'color-no-hex': { count: 4 } } });
    const pruned = await runDesignCheck(dir, { prune: true });
    expect(pruned.ok).toBe(true);
    expect(baselineOf(dir)).toEqual(allow(1));
    expect((await runDesignCheck(dir)).ok).toBe(true);
  });

  it('--prune never raises a count: it refuses while a violation exists', async () => {
    const dir = root(3, allow(2));
    const result = await runDesignCheck(dir, { prune: true });
    expect(result.ok).toBe(false);
    expect(baselineOf(dir)).toEqual(allow(2));
  });

  it('--init writes a baseline once and refuses to overwrite it', async () => {
    const dir = root(2);
    const created = await runDesignCheck(dir, { init: true });
    expect(created.ok).toBe(true);
    expect(baselineOf(dir)).toEqual(allow(2));
    await expect(runDesignCheck(dir, { init: true })).rejects.toThrow('already exists');
  });
});

describe('ESLint baseline uses ESLint\'s own suppression format', () => {
  const dirs: string[] = [];
  afterEach(() => {
    for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  });

  it('allows the recorded inline style, and fails when a second one appears', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'dgos-design-'));
    dirs.push(dir);
    cpSync(join(cleanRoot, 'src', 'ui'), join(dir, 'src', 'ui'), { recursive: true });
    const entry = join(dir, 'bundled-modules', 'm', '1.0.0', 'entry.js');
    mkdirSync(dirname(entry), { recursive: true });
    writeFileSync(entry, "export default (host) => host.React.createElement('p', { style: {} });\n");
    const suppressions = join(dir, 'scripts', 'design-check', 'baseline', 'eslint-suppressions.json');
    mkdirSync(dirname(suppressions), { recursive: true });
    writeFileSync(suppressions, JSON.stringify({ 'bundled-modules/m/1.0.0/entry.js': { [ESLINT]: { count: 1 } } }));

    expect((await runDesignCheck(dir)).ok).toBe(true);

    writeFileSync(entry, "export default (host) => [host.React.createElement('p', { style: {} }), host.React.createElement('p', { style: {} })];\n");
    const grown = await runDesignCheck(dir);
    expect(grown.ok).toBe(false);
    expect(grown.violations).toHaveLength(2);
  });
});

describe('this repository', () => {
  it('passes under the committed baseline', async () => {
    const result = await runDesignCheck(repo);
    expect(formatReport(result)).toContain('design:check 通过');
    expect(result.ok).toBe(true);
  });
});
