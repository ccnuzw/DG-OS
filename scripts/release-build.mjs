// D061: release build gate. Local verification level only; see NOT_COVERED.
//
//   node scripts/release-build.mjs build [tauri build args...]
//   node scripts/release-build.mjs verify <bundled-modules dir>
//
// `build` checks modules and host contracts, runs `tauri build`, then checks the
// actual .app/.dmg resources, executable permissions and D049 module pins.
// `verify` accepts a .app for the same checks, or a module directory for D049 only.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, lstatSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = fileURLToPath(new URL('..', import.meta.url));

export const NOT_COVERED = [
  'platform code signing and notarization',
  'a signed manifest',
  'byte comparison of the notarized installer',
  'Windows and Linux artifacts',
  'the gate binary is built from the same source as the shipped binary but is not the shipped binary',
  'launching the installed app, Finder environment discovery and lifecycle recovery',
  'proving that the main executable was compiled from the current checkout',
];

const RESOURCES = join('Contents', 'Resources', 'bundled-modules');

function subdirs(dir) {
  try {
    return readdirSync(dir, { withFileTypes: true }).filter((entry) => entry.isDirectory()).map((entry) => entry.name);
  } catch {
    return [];
  }
}

function matchesApp(name, { appName, notBefore }, path) {
  if (!name.endsWith('.app')) return false;
  if (appName && name !== appName) return false;
  // A leftover .app from an earlier build must not satisfy (or fail) this build.
  if (notBefore !== undefined && statSync(path).mtimeMs < notBefore) return false;
  return true;
}

/** `<root>/release/bundle` and `<root>/<target triple>/release/bundle` directories that exist. */
function bundleDirs(root) {
  const candidates = [join(root, 'release', 'bundle'), ...subdirs(root).map((name) => join(root, name, 'release', 'bundle'))];
  return candidates.filter((dir) => existsSync(dir));
}

/**
 * Expected `bundled-modules` paths inside every macOS `.app` under a cargo target directory.
 * The paths are returned even when the directory is absent so that `verifyDir` reports it.
 * Options: `appName` (exact `X.app`), `notBefore` (ms epoch; skips older bundles).
 */
export function findBundledModuleDirs(root, options = {}) {
  const found = [];
  for (const bundle of bundleDirs(root)) {
    const macos = join(bundle, 'macos');
    for (const name of subdirs(macos)) {
      const app = join(macos, name);
      if (matchesApp(name, options, app)) found.push(join(app, RESOURCES));
    }
  }
  return found;
}

/** `.dmg` files under `<root>/release/bundle/dmg` (and target-triple variants). */
export function findDmgs(root, options = {}) {
  const found = [];
  for (const bundle of bundleDirs(root)) {
    const dmgDir = join(bundle, 'dmg');
    let names = [];
    try {
      names = readdirSync(dmgDir);
    } catch {
      continue;
    }
    for (const name of names) {
      const path = join(dmgDir, name);
      if (!name.endsWith('.dmg')) continue;
      if (options.notBefore !== undefined && statSync(path).mtimeMs < options.notBefore) continue;
      found.push(path);
    }
  }
  return found;
}

/**
 * Runs the gate on one directory. `run(dir)` resolves to the exit code of the gate binary.
 * Never throws: every failure becomes `{ ok: false, reason }`.
 */
export async function verifyDir(dir, run) {
  let isDir = false;
  try {
    isDir = statSync(dir).isDirectory();
  } catch {
    // reported below
  }
  if (!isDir) return { dir, ok: false, reason: 'bundled-modules directory is missing' };
  try {
    const code = await run(dir);
    if (code === 0) return { dir, ok: true };
    return { dir, ok: false, reason: `release gate exited with code ${code}` };
  } catch (error) {
    return { dir, ok: false, reason: `release gate could not run: ${error?.message ?? error}` };
  }
}

// Compare complete resource trees, not just their entry points. A missing asset or
// a stale bridge must fail even if the module-only gate succeeds. No code is run.
function resourceDigest(path) {
  const hash = createHash('sha256');
  function frame(value) {
    const bytes = Buffer.isBuffer(value) ? value : Buffer.from(String(value));
    const length = Buffer.alloc(8);
    length.writeBigUInt64BE(BigInt(bytes.length));
    hash.update(length).update(bytes);
  }
  function visit(full, relative) {
    const stat = lstatSync(full);
    if (stat.isSymbolicLink()) throw new Error(`resource symlink is not allowed: ${full}`);
    frame(relative);
    if (stat.isDirectory()) {
      frame('directory');
      const names = readdirSync(full).sort();
      frame(names.length);
      for (const name of names) visit(join(full, name), `${relative}/${name}`);
    } else if (stat.isFile()) {
      frame('file');
      frame(readFileSync(full));
    } else throw new Error(`resource is not a regular file or directory: ${full}`);
  }
  visit(path, '');
  return hash.digest('hex');
}

function requireExecutable(path) {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.size === 0 || (stat.mode & 0o111) === 0) {
    throw new Error(`missing, empty or non-executable binary: ${path}`);
  }
}

export async function verifyApp(app, run, expectedResources = repoRoot) {
  const resources = join(app, 'Contents', 'Resources');
  const modules = await verifyDir(join(resources, 'bundled-modules'), run);
  if (!modules.ok) return { ...modules, dir: app };
  try {
    const config = JSON.parse(readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'));
    const binaryName = config.mainBinaryName ?? 'dgos';
    requireExecutable(join(app, 'Contents', 'MacOS', binaryName));
    requireExecutable(join(resources, 'agent-bridge', 'dgos-claude-bridge'));
    for (const [source, target] of [
      [join(expectedResources, 'tools/agent-bridge/bin/dgos-claude-bridge'), 'agent-bridge/dgos-claude-bridge'],
      [join(expectedResources, 'skills'), 'skills'],
      [join(expectedResources, 'dist-lan'), 'dist-lan'],
    ]) {
      if (resourceDigest(source) !== resourceDigest(join(resources, target))) {
        throw new Error(`bundled ${target} differs from this checkout's build resources; rebuild before verifying`);
      }
    }
    if (!statSync(join(resources, 'dist-lan', 'index.html')).isFile()) {
      throw new Error('bundled LAN index.html is missing');
    }
    return { dir: app, ok: true };
  } catch (error) {
    return { dir: app, ok: false, reason: `application resources invalid: ${error?.message ?? error}` };
  }
}

/**
 * Mounts `dmg` through `mount(dmg) -> mountpoint`, verifies every `.app` inside, and always
 * calls `unmount(mountpoint)`. An unmount failure is itself a failure.
 */
export async function verifyDmg(dmg, { run, mount, unmount, expectedResources = repoRoot }) {
  let mountpoint;
  try {
    mountpoint = await mount(dmg);
  } catch (error) {
    return [{ dir: dmg, ok: false, reason: `could not mount: ${error?.message ?? error}` }];
  }
  const results = [];
  try {
    const apps = subdirs(mountpoint).filter((name) => name.endsWith('.app'));
    if (apps.length === 0) {
      results.push({ dir: dmg, ok: false, reason: 'no .app found inside the disk image' });
    }
    for (const app of apps) {
      const result = await verifyApp(join(mountpoint, app), run, expectedResources);
      results.push({ ...result, dir: `${dmg}:${app}` });
    }
  } catch (error) {
    results.push({ dir: dmg, ok: false, reason: `verification aborted: ${error?.message ?? error}` });
  } finally {
    try {
      await unmount(mountpoint);
    } catch (error) {
      results.push({ dir: dmg, ok: false, reason: `could not detach ${mountpoint}: ${error?.message ?? error}` });
    }
  }
  return results;
}

/**
 * Locates and verifies the delivered resources under each target root.
 * Fails when nothing was found, or when any directory fails.
 */
export async function runArtifactGate({ roots, run, mount, unmount, appName, notBefore, expectedResources = repoRoot, log = console.log }) {
  const options = { appName, notBefore };
  const results = [];
  const dirs = roots.flatMap((root) => findBundledModuleDirs(root, options));
  const dmgs = roots.flatMap((root) => findDmgs(root, options));
  for (const dir of dirs) results.push(await verifyApp(dirname(dirname(dirname(dir))), run, expectedResources));
  if (dmgs.length > 0) {
    for (const dmg of dmgs) results.push(...(await verifyDmg(dmg, { run: (dir) => run(dir), mount, unmount, expectedResources })));
  }
  if (dirs.length === 0) {
    results.push({
      dir: roots.map((root) => join(root, 'release', 'bundle', 'macos')).join(' | '),
      ok: false,
      reason: `no freshly built ${appName ?? '*.app'} with a bundled-modules directory was found`,
    });
  }
  for (const result of results) {
    log(result.ok ? `verified: ${result.dir}` : `FAILED:   ${result.dir}: ${result.reason}`);
  }
  return { ok: results.every((result) => result.ok), results };
}

export function printNotCovered(log = console.log) {
  log('NOT COVERED by this gate:');
  for (const item of NOT_COVERED) log(`  - ${item}`);
}

function exitCode(child) {
  return new Promise((resolveCode, reject) => {
    child.on('error', reject);
    child.on('close', (code, signal) => resolveCode(code ?? (signal ? 128 : 1)));
  });
}

function command(cmd, args, options = {}) {
  return exitCode(spawn(cmd, args, { cwd: repoRoot, stdio: 'inherit', ...options }));
}

/** Default runner: the D049 gate example compiled from this checkout. */
export function runVerifyExample(dir) {
  return command('cargo', ['run', '--offline', '-p', 'dgos-modules', '--example', 'verify_release_bundle', '--', dir]);
}

async function defaultMount(dmg) {
  const mountpoint = mkdtempSync(join(tmpdir(), 'dgos-dmg-'));
  const code = await command('hdiutil', ['attach', '-readonly', '-nobrowse', '-noverify', '-mountpoint', mountpoint, dmg]);
  if (code !== 0) {
    rmSync(mountpoint, { recursive: true, force: true });
    throw new Error(`hdiutil attach exited with code ${code}`);
  }
  return mountpoint;
}

async function defaultUnmount(mountpoint) {
  let code = await command('hdiutil', ['detach', mountpoint]);
  if (code !== 0) code = await command('hdiutil', ['detach', '-force', mountpoint]);
  if (code !== 0) throw new Error(`hdiutil detach exited with code ${code}`);
  rmSync(mountpoint, { recursive: true, force: true });
}

async function mainVerify(args) {
  if (args.length !== 1) {
    console.error('usage: release-build.mjs verify <app.app | bundled-modules dir>');
    return 2;
  }
  const target = resolve(args[0]);
  const result = basename(target).endsWith('.app')
    ? await verifyApp(target, runVerifyExample)
    : await verifyDir(target, runVerifyExample);
  console.log(result.ok ? `verified: ${result.dir}` : `FAILED:   ${result.dir}: ${result.reason}`);
  printNotCovered();
  return result.ok ? 0 : 1;
}

async function mainBuild(args) {
  const steps = [
    ['bundled-modules check', process.execPath, ['scripts/bundled-modules.mjs']],
    ['host contract check', process.execPath, ['scripts/check-host-contracts.mjs']],
  ];
  for (const [label, cmd, cmdArgs] of steps) {
    console.log(`\n== ${label} ==`);
    const code = await command(cmd, cmdArgs);
    if (code !== 0) {
      console.error(`${label} failed (exit ${code}); release build stopped.`);
      printNotCovered(console.error);
      return code;
    }
  }
  console.log('\n== tauri build ==');
  const startedAt = Date.now();
  const tauri = join(repoRoot, 'node_modules', '.bin', process.platform === 'win32' ? 'tauri.cmd' : 'tauri');
  // FR-010: the remote SPA (`dist-lan/`, built by `npm run build`) ships as a resource. It is
  // merged here, not in tauri.conf.json, because tauri-build copies resources on every cargo
  // build and `dist-lan/` exists only after the frontend build.
  const lanResource = JSON.stringify({
    bundle: {
      resources: { ...JSON.parse(readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8')).bundle.resources, '../dist-lan/': 'dist-lan/' },
    },
  });
  const code = await command(tauri, ['build', '--config', lanResource, ...args]);
  if (code !== 0) {
    console.error(`tauri build failed (exit ${code}); no artifact gate was run.`);
    printNotCovered(console.error);
    return code;
  }
  console.log('\n== artifact gate (D061) ==');
  const config = JSON.parse(readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'));
  const gate = await runArtifactGate({
    roots: [join(repoRoot, 'target'), join(repoRoot, 'src-tauri', 'target')],
    run: runVerifyExample,
    mount: defaultMount,
    unmount: defaultUnmount,
    appName: `${config.productName}.app`,
    notBefore: startedAt,
  });
  console.log(gate.ok ? '\nRelease artifact gate PASSED (local verification only).' : '\nRelease artifact gate FAILED.');
  printNotCovered();
  return gate.ok ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [mode, ...rest] = process.argv.slice(2);
  const handler = mode === 'build' ? mainBuild : mode === 'verify' ? mainVerify : null;
  if (!handler) {
    console.error('usage: release-build.mjs build [tauri args...] | verify <app.app | bundled-modules dir>');
    process.exit(2);
  }
  handler(rest).then(
    (code) => process.exit(code),
    (error) => {
      console.error(error?.stack ?? error);
      printNotCovered(console.error);
      process.exit(1);
    },
  );
}
