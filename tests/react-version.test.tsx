// @vitest-environment node
// D060: the React that runs, the React in the lockfile and the React the Rust host
// judges `requires.react` against must be the same exact version.
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import React from 'react';
import ReactDOM from 'react-dom';
import { afterEach, describe, expect, it } from 'vitest';
// @ts-expect-error plain ESM script without type declarations
import { checkReactVersions, satisfies } from '../scripts/react-version.mjs';

const root = join(__dirname, '..');
const lock = JSON.parse(readFileSync(join(root, 'package-lock.json'), 'utf8'));

describe('React version contract (D060)', () => {
  it('runtime React.version equals the package-lock.json version', () => {
    expect(React.version).toBe(lock.packages['node_modules/react'].version);
  });

  it('runtime ReactDOM.version equals the package-lock.json version', () => {
    expect(ReactDOM.version).toBe(lock.packages['node_modules/react-dom'].version);
  });

  it('the repository passes the contract check', () => {
    expect(checkReactVersions(root)).toEqual([]);
  });
});

describe('checkReactVersions drift detection', () => {
  const dirs: string[] = [];
  afterEach(() => {
    for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  });

  function fixture(options: { locked: string; installed: string; range?: string }) {
    const dir = mkdtempSync(join(tmpdir(), 'dgos-react-'));
    dirs.push(dir);
    const range = options.range ?? '^19.1.0';
    writeFileSync(join(dir, 'package.json'), JSON.stringify({ dependencies: { react: range, 'react-dom': range } }));
    writeFileSync(
      join(dir, 'package-lock.json'),
      JSON.stringify({
        packages: {
          'node_modules/react': { version: options.locked },
          'node_modules/react-dom': { version: options.locked },
        },
      }),
    );
    for (const name of ['react', 'react-dom']) {
      mkdirSync(join(dir, 'node_modules', name), { recursive: true });
      writeFileSync(join(dir, 'node_modules', name, 'package.json'), JSON.stringify({ version: options.installed }));
    }
    return dir;
  }

  it('accepts matching versions', () => {
    expect(checkReactVersions(fixture({ locked: '19.3.1', installed: '19.3.1' }))).toEqual([]);
  });

  it('rejects a patch drift between node_modules and the lockfile', () => {
    const problems: string[] = checkReactVersions(fixture({ locked: '19.3.1', installed: '19.3.0' }));
    expect(problems).toHaveLength(2);
    expect(problems[0]).toMatch(/node_modules\/react is 19\.3\.0 but package-lock\.json pins 19\.3\.1.*npm ci/);
  });

  it('rejects a minor drift', () => {
    expect(checkReactVersions(fixture({ locked: '19.3.1', installed: '19.1.0' })).length).toBeGreaterThan(0);
  });

  it('rejects a lockfile version outside the package.json range', () => {
    const problems: string[] = checkReactVersions(fixture({ locked: '19.3.1', installed: '19.3.1', range: '~19.1.0' }));
    expect(problems.join('\n')).toMatch(/outside package\.json range ~19\.1\.0/);
  });

  it('rejects a non-exact lockfile version', () => {
    expect(checkReactVersions(fixture({ locked: '^19.3.1', installed: '19.3.1' })).join('\n')).toMatch(/not an exact/);
  });

  it('reports a missing lockfile', () => {
    const dir = fixture({ locked: '19.3.1', installed: '19.3.1' });
    rmSync(join(dir, 'package-lock.json'));
    expect(checkReactVersions(dir).join('\n')).toMatch(/package-lock\.json is missing/);
  });

  it('reports a missing installation', () => {
    const dir = fixture({ locked: '19.3.1', installed: '19.3.1' });
    rmSync(join(dir, 'node_modules'), { recursive: true });
    expect(checkReactVersions(dir).join('\n')).toMatch(/node_modules.*missing/);
  });
});

describe('satisfies', () => {
  it.each([
    ['19.3.1', '^19.1.0', true],
    ['19.0.9', '^19.1.0', false],
    ['20.0.0', '^19.1.0', false],
    ['19.3.1', '~19.3.0', true],
    ['19.4.0', '~19.3.0', false],
    ['19.3.1', '19.3.1', true],
    ['19.3.2', '19.3.1', false],
    ['19.3.1', '>=19.2.0', true],
    ['19.1.0', '>=19.2.0', false],
    ['0.2.5', '^0.2.1', true],
    ['0.3.0', '^0.2.1', false],
  ])('%s against %s is %s', (version, range, expected) => {
    expect(satisfies(version, range)).toBe(expected);
  });

  it('refuses ranges it does not understand', () => {
    expect(() => satisfies('19.3.1', '>=19 <20')).toThrow(/unsupported version range/);
  });
});
