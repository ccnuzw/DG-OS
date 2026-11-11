// Recomputes the v2 integrity (D057) of every module under bundled-modules/ with the same algorithm as
// the host's `digest::read_package` (crates/dgos-modules/src/digest.rs), and keeps three things in
// agreement: each manifest artifact.sha256, current executable BUILTIN_PINS and
// non-executable RETAINED_PINS. Retained artifacts are verified but never installed or run.
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
// Retiring code requires an explicit release decision; never infer it from directory order.
const retainedReleases = new Set(['provider-console@1.0.0', 'provider-console@1.1.0', 'provider-console@1.2.0']);

const u64 = (n) => {
  const buffer = Buffer.alloc(8);
  buffer.writeBigUInt64BE(BigInt(n));
  return buffer;
};
const frame = (bytes) => Buffer.concat([u64(bytes.length), bytes]);
const text = (value) => Buffer.from(String(value), 'utf8');

const KNOWN_MANIFEST_KEYS = new Set(['id', 'name', 'version', 'entry', 'artifact', 'permissions', 'requires', 'views']);

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
  if (manifest.views !== undefined && manifest.views !== null) {
    const views = manifest.views;
    if (Object.keys(views).some(key => !['layout', 'capability'].includes(key)) || !['document', 'full-bleed'].includes(views.layout) || !['single', 'multiple'].includes(views.capability)) throw new Error('invalid view contribution');
    parts.push(frame(text('views')), frame(text(views.layout)), frame(text(views.capability)));
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

  const source = readFileSync(admissionPath, 'utf8');
  const groups = [
    ['BUILTIN_PINS', pins.filter(([id, version]) => !retainedReleases.has(`${id}@${version}`))],
    ['RETAINED_PINS', pins.filter(([id, version]) => retainedReleases.has(`${id}@${version}`))],
  ];
  for (const release of retainedReleases) {
    if (!pins.some(([id, version]) => `${id}@${version}` === release)) problems.push(`retained package missing: ${release}`);
  }
  const currentIds = groups[0][1].map(([id]) => id.toLowerCase());
  if (new Set(currentIds).size !== currentIds.length) problems.push('each module must have exactly one current release; explicitly retire the previous release');
  let updated = source;
  for (const [name, entries] of groups) {
    const block = `pub(crate) const ${name}: &[(&str, &str, &str)] = &[${entries.length ? '\n' + entries.map(([id, version, digest]) => `    (\n        "${id}",\n        "${version}",\n        "${digest}",\n    ),`).join('\n') + '\n' : ''}];`;
    const pattern = new RegExp(`pub\\(crate\\) const ${name}: &\\[\\(&str, &str, &str\\)\\] = &\\[[\\s\\S]*?\\];`);
    if (!pattern.test(source)) throw new Error(`${name} not found in admission.rs`);
    // rustfmt may reflow a block; compare triples rather than formatting.
    const parse = value => [...value.matchAll(/"([^"]*)",\s*"([^"]*)",\s*"((?:v2:)?[0-9a-f]*)"/g)].map(match => match.slice(1).join('@'));
    const current = parse(source.match(pattern)[0]).sort().join('\n');
    const wanted = entries.map(pin => pin.join('@')).sort().join('\n');
    if (current !== wanted) {
      if (write) updated = updated.replace(pattern, block);
      else problems.push(`admission.rs: ${name} differs from the recomputed bundle (run with --write, then cargo fmt)`);
    }
  }
  if (write && updated !== source && !problems.length) writeFileSync(admissionPath, updated);

  if (problems.length) {
    console.error(problems.join('\n'));
    process.exit(1);
  }
  console.log(`${write ? 'Wrote' : 'Checked'} ${pins.length} bundled module pin(s).`);
}
