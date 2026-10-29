// Recomputes the v2 integrity (D057) of every module under bundled-modules/ with the same algorithm as
// the host's `digest::read_package` (crates/dgos-modules/src/digest.rs), and keeps three things in
// agreement: the package manifest's artifact.sha256 (the v2 hex, a build consistency value only), and
// the BUILTIN_PINS list in admission.rs (D047, D052). Pins are `v2:` + 64 lowercase hex digits.
//
//   node scripts/bundled-modules.mjs          check only; exit 1 on any disagreement
//   node scripts/bundled-modules.mjs --write  rewrite manifests and the pin list
//
// The Rust test `the_shipped_bundle_equals_the_builtin_pins` is the independent check: it uses the
// Rust implementation, so a divergence between the two digest implementations fails the build.
//
// v2: SHA-256( "dgos-module-digest\0v2\0" || frame(manifest_canon) || u64be(file_count)
//              || for each file sorted by relative path bytes: frame(path) || frame(content) )
// frame(x) = u64be(len(x)) || x. Files are all regular files except the ROOT manifest.json.
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, writeFileSync, lstatSync, existsSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../bundled-modules/', import.meta.url));
const admissionPath = fileURLToPath(new URL('../crates/dgos-modules/src/admission.rs', import.meta.url));
const write = process.argv.includes('--write');

const u64 = (n) => {
  const buffer = Buffer.alloc(8);
  buffer.writeBigUInt64BE(BigInt(n));
  return buffer;
};
const frame = (bytes) => Buffer.concat([u64(bytes.length), bytes]);
const text = (value) => Buffer.from(String(value), 'utf8');

const KNOWN_MANIFEST_KEYS = new Set(['id', 'name', 'version', 'entry', 'artifact', 'permissions', 'requires']);

// Fixed-order encoding of every manifest field except artifact.sha256 (digest.rs `manifest_canon`).
export function manifestCanon(manifest) {
  for (const key of Object.keys(manifest)) {
    if (!KNOWN_MANIFEST_KEYS.has(key)) throw new Error(`unknown manifest field: ${key}`);
  }
  const parts = [];
  for (const value of [manifest.id, manifest.name, manifest.version, manifest.entry, manifest.artifact?.version]) {
    parts.push(frame(text(value)));
  }
  const permissions = manifest.permissions ?? [];
  parts.push(u64(permissions.length));
  for (const permission of permissions) parts.push(frame(text(permission)));
  const requires = manifest.requires;
  if (requires === undefined || requires === null) {
    parts.push(Buffer.from([0]));
  } else {
    parts.push(Buffer.from([1]));
    if (requires.api === undefined || requires.api === null) parts.push(Buffer.from([0]));
    else {
      const api = Buffer.alloc(4);
      api.writeUInt32BE(requires.api);
      parts.push(Buffer.from([1]), api);
    }
    if (requires.react === undefined || requires.react === null) parts.push(Buffer.from([0]));
    else parts.push(Buffer.from([1]), frame(text(requires.react)));
  }
  return Buffer.concat(parts);
}

function collect(dir, base, out) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const stat = lstatSync(path);
    if (stat.isSymbolicLink()) throw new Error(`symbolic links are not allowed: ${path}`);
    if (stat.isDirectory()) collect(path, base, out);
    else if (stat.isFile()) {
      const relativePath = relative(base, path).split(sep).join('/');
      if (relativePath !== 'manifest.json') out.push([relativePath, path]);
    } else throw new Error(`not a regular file: ${path}`);
  }
}

export function packageIntegrity(dir) {
  const manifest = JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8'));
  const files = [];
  collect(dir, dir, files);
  // Rust sorts by String ordering, which is byte-wise on UTF-8.
  files.sort((a, b) => Buffer.compare(Buffer.from(a[0]), Buffer.from(b[0])));
  const hash = createHash('sha256');
  hash.update(Buffer.from('dgos-module-digest\0v2\0', 'utf8'));
  hash.update(frame(manifestCanon(manifest)));
  hash.update(u64(files.length));
  for (const [name, path] of files) {
    hash.update(frame(text(name)));
    hash.update(frame(readFileSync(path)));
  }
  return `v2:${hash.digest('hex')}`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const pins = [];
  const problems = [];
  if (existsSync(root)) {
    for (const id of readdirSync(root).sort()) {
      for (const version of readdirSync(join(root, id)).sort()) {
        const dir = join(root, id, version);
        const integrity = packageIntegrity(dir);
        const hex = integrity.slice(3);
        const manifestPath = join(dir, 'manifest.json');
        const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
        if (manifest.id !== id || manifest.version !== version) problems.push(`${id}@${version}: manifest id/version disagree with the directory`);
        if (manifest.artifact?.version !== version) problems.push(`${id}@${version}: artifact.version disagrees`);
        if (manifest.artifact?.sha256 !== hex) {
          if (write) {
            // artifact.sha256 is not part of the digest, so rewriting it does not change `integrity`.
            manifest.artifact = { version, sha256: hex };
            writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
          } else {
            problems.push(`${id}@${version}: manifest artifact.sha256 is not the recomputed v2 digest`);
          }
        }
        pins.push([id, version, integrity]);
      }
    }
  }

  const block = pins.length
    ? `pub(crate) const BUILTIN_PINS: &[(&str, &str, &str)] = &[\n${pins
        .map(([id, version, digest]) => `    (\n        "${id}",\n        "${version}",\n        "${digest}",\n    ),`)
        .join('\n')}\n];`
    : 'pub(crate) const BUILTIN_PINS: &[(&str, &str, &str)] = &[];';
  const source = readFileSync(admissionPath, 'utf8');
  const pattern = /pub\(crate\) const BUILTIN_PINS: &\[\(&str, &str, &str\)\] = &\[[\s\S]*?\];/;
  if (!pattern.test(source)) throw new Error('BUILTIN_PINS not found in admission.rs');
  // rustfmt reflows the block, so compare the parsed triples rather than the text.
  const parse = (value) => [...value.matchAll(/"([^"]*)",\s*"([^"]*)",\s*"((?:v2:)?[0-9a-f]*)"/g)].map((m) => m.slice(1).join('@'));
  const current = parse(source.match(pattern)[0]).sort().join('\n');
  const wanted = pins.map((pin) => pin.join('@')).sort().join('\n');
  if (current !== wanted) {
    if (write) writeFileSync(admissionPath, source.replace(pattern, block));
    else problems.push('admission.rs: BUILTIN_PINS differs from the recomputed bundle (run with --write, then cargo fmt)');
  }

  if (problems.length) {
    console.error(problems.join('\n'));
    process.exit(1);
  }
  console.log(`${write ? 'Wrote' : 'Checked'} ${pins.length} bundled module pin(s).`);
}
