// D061: release build gate. Local verification level only; see NOT_COVERED.
//
//   node scripts/release-build.mjs build [tauri build args...]
//   node scripts/release-build.mjs verify <bundled-modules dir>
//
// `build` runs the bundled-modules check, the host contract check, `tauri build`, then
// runs the D049 release gate (`verify_release_bundle`) against the bundled-modules directory
// inside the artifacts that were actually produced (the .app, and the .dmg when present).
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = fileURLToPath(new URL('..', import.meta.url));

export const NOT_COVERED = [
  'platform code signing and notarization',
  'a signed manifest',
  'byte comparison of the notarized installer',
  'Windows and Linux artifacts',
  'the gate binary is built from the same source as the shipped binary but is not the shipped binary',
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

/**
 * Mounts `dmg` through `mount(dmg) -> mountpoint`, verifies every `.app` inside, and always
 * calls `unmount(mountpoint)`. An unmount failure is itself a failure.
 */
export async function verifyDmg(dmg, { run, mount, unmount }) {
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
      const result = await verifyDir(join(mountpoint, app, RESOURCES), run);
      results.push({ ...result, dir: `${dmg}:${app}/${RESOURCES}` });
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
export async function runArtifactGate({ roots, run, mount, unmount, appName, notBefore, log = console.log }) {
  const options = { appName, notBefore };
  const results = [];
  const dirs = roots.flatMap((root) => findBundledModuleDirs(root, options));
  const dmgs = roots.flatMap((root) => findDmgs(root, options));
  for (const dir of dirs) results.push(await verifyDir(dir, run));
  if (dmgs.length > 0) {
    for (const dmg of dmgs) results.push(...(await verifyDmg(dmg, { run, mount, unmount })));
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
    console.error('usage: release-build.mjs verify <bundled-modules dir>');
    return 2;
  }
  const result = await verifyDir(resolve(args[0]), runVerifyExample);
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
  const code = await command(tauri, ['build', ...args]);
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
    console.error('usage: release-build.mjs build [tauri args...] | verify <bundled-modules dir>');
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
