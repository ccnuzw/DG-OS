// D060: React/ReactDOM installed versions must equal package-lock.json, and
// package.json's ranges must admit them. The Rust host reads the same lockfile
// entry in crates/dgos-modules/build.rs, so all three agree.
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const EXACT = /^(\d+)\.(\d+)\.(\d+)$/;
const PACKAGES = ['react', 'react-dom'];

function readJson(path, hint) {
  if (!existsSync(path)) throw new Error(`${path} is missing. ${hint}`);
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (error) {
    throw new Error(`${path} is not valid JSON (${error.message}). ${hint}`);
  }
}

function parseExact(version, where) {
  const match = EXACT.exec(version ?? '');
  if (!match) throw new Error(`${where}: ${JSON.stringify(version)} is not an exact x.y.z version`);
  return match.slice(1).map(Number);
}

function compare(a, b) {
  for (let i = 0; i < 3; i += 1) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

// Only the range forms package.json uses for these packages are supported
// (exact, ^, ~, >=). Anything else fails loudly instead of being guessed.
export function satisfies(version, range) {
  const v = parseExact(version, 'version');
  const match = /^(\^|~|>=|=)?\s*(\d+)\.(\d+)\.(\d+)$/.exec(range.trim());
  if (!match) throw new Error(`unsupported version range ${JSON.stringify(range)} (use exact, ^, ~ or >=)`);
  const [, op = '', ...rest] = match;
  const base = rest.map(Number);
  if (compare(v, base) < 0) return false;
  if (op === '>=') return true;
  if (op === '' || op === '=') return compare(v, base) === 0;
  if (op === '~') return v[0] === base[0] && v[1] === base[1];
  // ^: the leftmost non-zero component is fixed.
  if (base[0] > 0) return v[0] === base[0];
  if (base[1] > 0) return v[0] === 0 && v[1] === base[1];
  return v[0] === 0 && v[1] === 0 && v[2] === base[2];
}

/** Returns the list of problems (empty when consistent) for the repo at `root`. */
export function checkReactVersions(root) {
  const problems = [];
  let lock;
  let manifest;
  try {
    lock = readJson(join(root, 'package-lock.json'), 'Run `npm install` to create it.');
    manifest = readJson(join(root, 'package.json'), 'The repository is incomplete.');
  } catch (error) {
    return [error.message];
  }
  for (const name of PACKAGES) {
    const locked = lock.packages?.[`node_modules/${name}`]?.version;
    try {
      parseExact(locked, `package-lock.json packages["node_modules/${name}"].version`);
    } catch (error) {
      problems.push(`${error.message}. Run \`npm install\` to regenerate the lockfile.`);
      continue;
    }
    const range = manifest.dependencies?.[name];
    if (!range) {
      problems.push(`package.json has no dependencies.${name}`);
    } else {
      try {
        if (!satisfies(locked, range)) {
          problems.push(`package-lock.json pins ${name}@${locked}, outside package.json range ${range}. Run \`npm install\`.`);
        }
      } catch (error) {
        problems.push(`package.json dependencies.${name}: ${error.message}`);
      }
    }
    let installed;
    try {
      installed = readJson(join(root, 'node_modules', name, 'package.json'), 'Run `npm ci`.').version;
    } catch (error) {
      problems.push(error.message);
      continue;
    }
    if (installed !== locked) {
      problems.push(`node_modules/${name} is ${installed} but package-lock.json pins ${locked}. Run \`npm ci\` to install the locked version.`);
    } else if (range) {
      try {
        if (!satisfies(installed, range)) problems.push(`installed ${name}@${installed} is outside package.json range ${range}`);
      } catch {
        // Already reported against the lockfile version above.
      }
    }
  }
  return problems;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const problems = checkReactVersions(fileURLToPath(new URL('..', import.meta.url)));
  if (problems.length > 0) {
    console.error(`React version contract failed (D060):\n  ${problems.join('\n  ')}`);
    process.exit(1);
  }
  console.log('react and react-dom installed versions match package-lock.json and package.json.');
}
