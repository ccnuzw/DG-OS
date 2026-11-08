// @vitest-environment node
// D061: the artifact gate must fail closed and always release a mounted disk image.
import { chmodSync, cpSync, mkdirSync, mkdtempSync, rmSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterAll, describe, expect, it } from 'vitest';
import {
  NOT_COVERED,
  findBundledModuleDirs,
  findDmgs,
  printNotCovered,
  runArtifactGate,
  verifyApp,
  verifyDir,
  verifyDmg,
  // @ts-expect-error plain ESM script without type declarations
} from '../scripts/release-build.mjs';

type Run = (dir: string) => Promise<number>;
type Mount = (dmg: string) => Promise<string>;
type Unmount = (mountpoint: string) => Promise<void>;

const temps: string[] = [];
let expectedRoot: string | undefined;
afterAll(() => {
  for (const dir of temps.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function temp() {
  const dir = mkdtempSync(join(tmpdir(), 'dgos-release-'));
  temps.push(dir);
  return dir;
}

function expectedResources() {
  if (expectedRoot) return expectedRoot;
  const root = temp();
  const bridge = join(root, 'tools', 'agent-bridge', 'bin', 'dgos-claude-bridge');
  mkdirSync(dirname(bridge), { recursive: true });
  writeFileSync(bridge, 'bridge');
  chmodSync(bridge, 0o755);
  mkdirSync(join(root, 'skills'), { recursive: true });
  writeFileSync(join(root, 'skills', 'catalog.v1.json'), '{}');
  mkdirSync(join(root, 'dist-lan'), { recursive: true });
  writeFileSync(join(root, 'dist-lan', 'index.html'), '<html></html>');
  expectedRoot = root;
  return root;
}

function populateResources(app: string, withModules = true) {
  const expected = expectedResources();
  const resources = join(app, 'Contents', 'Resources');
  const executable = join(app, 'Contents', 'MacOS', 'dgos');
  mkdirSync(dirname(executable), { recursive: true });
  writeFileSync(executable, 'app');
  chmodSync(executable, 0o755);
  const bridge = join(resources, 'agent-bridge', 'dgos-claude-bridge');
  mkdirSync(dirname(bridge), { recursive: true });
  cpSync(join(expected, 'tools', 'agent-bridge', 'bin', 'dgos-claude-bridge'), bridge);
  chmodSync(bridge, 0o755);
  cpSync(join(expected, 'skills'), join(resources, 'skills'), { recursive: true });
  cpSync(join(expected, 'dist-lan'), join(resources, 'dist-lan'), { recursive: true });
  if (withModules) mkdirSync(join(resources, 'bundled-modules'), { recursive: true });
}

function makeApp(root: string, name = 'DG-OS.app', withModules = true) {
  const app = join(root, 'release', 'bundle', 'macos', name);
  const resources = join(app, 'Contents', 'Resources');
  mkdirSync(resources, { recursive: true });
  populateResources(app, withModules);
  return join(resources, 'bundled-modules');
}

const quiet = () => {};
const never = async () => {
  throw new Error('must not be called');
};

describe('findBundledModuleDirs', () => {
  it('finds each .app and target-triple variant', () => {
    const root = temp();
    const a = makeApp(root);
    mkdirSync(join(root, 'aarch64-apple-darwin', 'release', 'bundle', 'macos', 'DG-OS.app', 'Contents', 'Resources', 'bundled-modules'), { recursive: true });
    const found: string[] = findBundledModuleDirs(root);
    expect(found).toHaveLength(2);
    expect(found).toContain(a);
  });

  it('filters by app name and skips bundles older than the build', () => {
    const root = temp();
    makeApp(root, 'Other.app');
    const own = makeApp(root, 'DG-OS.app');
    expect(findBundledModuleDirs(root, { appName: 'DG-OS.app' })).toEqual([own]);
    const old = new Date(Date.now() - 3_600_000);
    utimesSync(join(root, 'release', 'bundle', 'macos', 'DG-OS.app'), old, old);
    expect(findBundledModuleDirs(root, { appName: 'DG-OS.app', notBefore: Date.now() - 60_000 })).toEqual([]);
  });

  it('returns nothing when there is no bundle', () => {
    expect(findBundledModuleDirs(temp())).toEqual([]);
    expect(findDmgs(temp())).toEqual([]);
  });
});

describe('verifyDir', () => {
  it('fails when the directory is missing, without running the gate', async () => {
    const result = await verifyDir(join(temp(), 'absent'), never);
    expect(result.ok).toBe(false);
    expect(result.reason).toMatch(/missing/);
  });

  it('fails when the gate exits non-zero', async () => {
    const result = await verifyDir(temp(), (async () => 1) as Run);
    expect(result).toMatchObject({ ok: false, reason: expect.stringMatching(/code 1/) });
  });

  it('fails when the gate cannot run', async () => {
    const result = await verifyDir(temp(), never);
    expect(result).toMatchObject({ ok: false, reason: expect.stringMatching(/could not run/) });
  });

  it('passes on exit code 0 and hands the directory to the runner', async () => {
    const dir = temp();
    const seen: string[] = [];
    const result = await verifyDir(dir, (async (d: string) => (seen.push(d), 0)) as Run);
    expect(result.ok).toBe(true);
    expect(seen).toEqual([dir]);
  });
});

describe('runArtifactGate', () => {
  const noMount = { mount: never as Mount, unmount: never as Unmount, log: quiet, expectedResources: expectedResources() };

  it('fails when no bundle was produced', async () => {
    const gate = await runArtifactGate({ roots: [temp()], run: never, ...noMount });
    expect(gate.ok).toBe(false);
    expect(gate.results[0].reason).toMatch(/no freshly built/);
  });

  it('fails when the delivered app has no bundled-modules directory', async () => {
    const root = temp();
    makeApp(root, 'DG-OS.app', false);
    const gate = await runArtifactGate({ roots: [root], run: never, ...noMount });
    expect(gate.ok).toBe(false);
    expect(gate.results[0].reason).toMatch(/missing/);
  });

  it('fails when the gate reports a violation', async () => {
    const root = temp();
    makeApp(root);
    const gate = await runArtifactGate({ roots: [root], run: (async () => 1) as Run, ...noMount });
    expect(gate.ok).toBe(false);
  });

  it('fails when one of several directories violates', async () => {
    const root = temp();
    const first = makeApp(root, 'A.app');
    makeApp(root, 'B.app');
    const gate = await runArtifactGate({ roots: [root], run: (async (d: string) => (d === first ? 0 : 1)) as Run, ...noMount });
    expect(gate.ok).toBe(false);
    expect(gate.results.filter((r: { ok: boolean }) => r.ok)).toHaveLength(1);
  });

  it('passes when every delivered directory is clean', async () => {
    const root = temp();
    makeApp(root);
    const gate = await runArtifactGate({ roots: [root], run: (async () => 0) as Run, ...noMount });
    expect(gate.ok).toBe(true);
  });

  it('also verifies the app inside a dmg and fails if that copy violates', async () => {
    const root = temp();
    makeApp(root);
    mkdirSync(join(root, 'release', 'bundle', 'dmg'), { recursive: true });
    writeDmg(join(root, 'release', 'bundle', 'dmg', 'DG-OS.dmg'));
    const mountpoint = temp();
    populateResources(join(mountpoint, 'DG-OS.app'));
    const run: Run = async (d) => (d.startsWith(mountpoint) ? 1 : 0);
    const detached: string[] = [];
    const gate = await runArtifactGate({
      roots: [root],
      run,
      mount: async () => mountpoint,
      unmount: async (m) => void detached.push(m),
      log: quiet,
      expectedResources: expectedResources(),
    });
    expect(gate.ok).toBe(false);
    expect(detached).toEqual([mountpoint]);
  });
});

function writeDmg(path: string) {
  // The content is irrelevant to the fake mount; only the name is matched.
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, '');
}

describe('verifyDmg', () => {
  function mounted() {
    const mountpoint = temp();
    populateResources(join(mountpoint, 'DG-OS.app'));
    return mountpoint;
  }

  it('detaches after a successful verification', async () => {
    const mountpoint = mounted();
    const detached: string[] = [];
    const results = await verifyDmg('x.dmg', {
      run: async () => 0,
      mount: async () => mountpoint,
      unmount: async (m: string) => void detached.push(m),
      expectedResources: expectedResources(),
    });
    expect(results.every((r: { ok: boolean }) => r.ok)).toBe(true);
    expect(detached).toEqual([mountpoint]);
  });

  it('detaches even when verification fails', async () => {
    const mountpoint = mounted();
    const detached: string[] = [];
    const results = await verifyDmg('x.dmg', {
      run: async () => 1,
      mount: async () => mountpoint,
      unmount: async (m: string) => void detached.push(m),
      expectedResources: expectedResources(),
    });
    expect(results.some((r: { ok: boolean }) => !r.ok)).toBe(true);
    expect(detached).toEqual([mountpoint]);
  });

  it('detaches even when the runner throws', async () => {
    const mountpoint = mounted();
    const detached: string[] = [];
    const results = await verifyDmg('x.dmg', {
      run: async () => {
        throw new Error('boom');
      },
      mount: async () => mountpoint,
      expectedResources: expectedResources(),
      unmount: async (m: string) => void detached.push(m),
    });
    expect(results.some((r: { ok: boolean }) => !r.ok)).toBe(true);
    expect(detached).toEqual([mountpoint]);
  });

  it('reports a failed detach as a failure', async () => {
    const mountpoint = mounted();
    const results = await verifyDmg('x.dmg', {
      run: async () => 0,
      mount: async () => mountpoint,
      unmount: async () => {
        throw new Error('busy');
      },
      expectedResources: expectedResources(),
    });
    expect(results.some((r: { ok: boolean; reason?: string }) => !r.ok && /could not detach/.test(r.reason ?? ''))).toBe(true);
  });

  it('reports a mount failure without attempting to detach', async () => {
    const results = await verifyDmg('x.dmg', {
      run: never,
      mount: async () => {
        throw new Error('denied');
      },
      unmount: never,
    });
    expect(results).toHaveLength(1);
    expect(results[0].ok).toBe(false);
  });

  it('fails an image that contains no .app', async () => {
    const empty = temp();
    const results = await verifyDmg('x.dmg', { run: never, mount: async () => empty, unmount: async () => {} });
    expect(results[0]).toMatchObject({ ok: false, reason: expect.stringMatching(/no \.app/) });
  });
});

describe('verifyApp', () => {
  it('accepts current resources and rejects a non-executable bridge', async () => {
    const root = temp();
    const modules = makeApp(root);
    const app = join(root, 'release', 'bundle', 'macos', 'DG-OS.app');
    const expected = expectedResources();
    const run: Run = async (dir) => (dir === modules ? 0 : 1);
    expect((await verifyApp(app, run, expected)).ok).toBe(true);
    chmodSync(join(app, 'Contents', 'Resources', 'agent-bridge', 'dgos-claude-bridge'), 0o644);
    expect((await verifyApp(app, run, expected)).reason).toMatch(/non-executable binary/);
  });

  it('rejects stale bundled assets', async () => {
    const root = temp();
    const modules = makeApp(root);
    const app = join(root, 'release', 'bundle', 'macos', 'DG-OS.app');
    writeFileSync(join(app, 'Contents', 'Resources', 'dist-lan', 'index.html'), '<html>stale</html>');
    const run: Run = async (dir) => (dir === modules ? 0 : 1);
    expect((await verifyApp(app, run, expectedResources())).reason).toMatch(/differs from this checkout/);
  });
});

describe('NOT COVERED notice', () => {
  it('lists every uncovered item', () => {
    const lines: string[] = [];
    printNotCovered((line: string) => lines.push(line));
    expect(lines[0]).toBe('NOT COVERED by this gate:');
    expect(NOT_COVERED).toHaveLength(7);
    for (const item of NOT_COVERED) expect(lines.join('\n')).toContain(item);
    expect(lines.join('\n')).toMatch(/signing and notarization/);
    expect(lines.join('\n')).toMatch(/signed manifest/);
    expect(lines.join('\n')).toMatch(/notarized installer/);
    expect(lines.join('\n')).toMatch(/Windows and Linux/);
    expect(lines.join('\n')).toMatch(/not the shipped binary/);
  });
});
