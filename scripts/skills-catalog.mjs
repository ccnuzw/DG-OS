import { createHash } from 'node:crypto';
import { readFile, writeFile, readdir, lstat } from 'node:fs/promises';
import { resolve, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../skills');
const catalogPath = resolve(root, 'catalog.v1.json');
const catalog = JSON.parse(await readFile(catalogPath, 'utf8'));
const categories = JSON.parse(await readFile(resolve(root, 'categories.json'), 'utf8'));
const seen = new Set();
const update = process.argv.includes('--write');
if (catalog.schema_version !== '1.0' || !catalog.catalog_version || !Array.isArray(catalog.skills) || catalog.skills.length > 128) throw new Error('Unsupported catalog');

// Hash sorted UTF-8 relative paths, NUL separators and file bytes identically
// to the Rust runtime. The catalog itself is pinned in the compiled binary.
async function packageHash(dir) {
  const files = [];
  let entries = 0;
  if ((await lstat(dir)).isSymbolicLink()) throw new Error(`Symlink rejected: ${dir}`);
  async function walk(path) {
    for (const entry of await readdir(path)) {
      const full = resolve(path, entry);
      const name = relative(dir, full).split('\\').join('/');
      entries += 1;
      if (entries > 256 || name.split('/').length > 8 || !/^[A-Za-z0-9/._-]+$/.test(name)) {
        throw new Error(`Resource path or directory limit exceeded: ${name}`);
      }
      const stat = await lstat(full);
      if (stat.isSymbolicLink()) throw new Error(`Symlink rejected: ${full}`);
      if (stat.isDirectory()) {
        if (!['references', 'templates', 'examples'].includes(name.split('/')[0])) throw new Error(`Unsupported directory: ${name}`);
        await walk(full);
      } else if (stat.isFile()) {
        if (stat.size > 256 * 1024) throw new Error(`File exceeds size limit: ${name}`);
        files.push(name);
      }
      else throw new Error(`Special file rejected: ${full}`);
    }
  }
  await walk(dir);
  if (files.length > 128) throw new Error('Too many package files');
  let total = 0;
  const hash = createHash('sha256');
  for (const file of files.sort()) {
    if (!/^(skill\.json|SKILL\.md|LICENSE|(references|templates|examples)\/[\w./-]+\.(md|txt|json|csv))$/.test(file)) {
      throw new Error(`Unsupported package resource: ${file}`);
    }
    const bytes = await readFile(resolve(dir, file));
    new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    total += bytes.length;
    if (bytes.length > 256 * 1024 || total > 2 * 1024 * 1024) throw new Error('Package exceeds size limits');
    hash.update(file).update('\0').update(bytes).update('\0');
  }
  for (const file of ['skill.json', 'SKILL.md', 'LICENSE']) {
    if (!files.includes(file)) throw new Error(`Missing required file: ${file}`);
    if (!(await lstat(resolve(dir, file))).size) throw new Error(`Empty required file: ${file}`);
  }
  return hash.digest('hex');
}

for (const entry of catalog.skills) {
  if (!/^official\/[a-z0-9-]+\/[a-z0-9-]+$/.test(entry.path)) throw new Error('Invalid package path');
  const dir = resolve(root, entry.path);
  const meta = JSON.parse(await readFile(resolve(dir, 'skill.json'), 'utf8'));
  if (seen.has(entry.id)) throw new Error(`Duplicate ID: ${entry.id}`);
  seen.add(entry.id);
  if (meta.id !== entry.id || meta.version !== entry.version || meta.schema_version !== '1.0' || meta.entry !== 'SKILL.md') throw new Error(`Metadata mismatch: ${entry.id}`);
  if (!categories.some(cat => cat.id === meta.category)) throw new Error('Unknown category');
  if (!/^[a-z0-9-]+$/.test(meta.slug) || meta.id !== `dgos.official.${meta.category}.${meta.slug}` || meta.source.path !== entry.path || entry.path !== `official/${meta.category}/${meta.slug}` || meta.source.kind !== 'dgos_official') throw new Error('Invalid official identity or source');
  if (meta.identity !== 'dgos_official' || !meta.license.redistributable || !meta.license.spdx) throw new Error('Invalid official provenance');
  const hash = await packageHash(dir);
  if (update) entry.sha256 = hash;
  else if (entry.sha256 !== hash) throw new Error(`Content changed: ${entry.id}; bump version when publishing, then run skills:catalog`);
}
if (update) await writeFile(catalogPath, `${JSON.stringify(catalog, null, 2)}\n`);
console.log(`Validated ${catalog.skills.length} bundled skills (${update ? 'catalog updated' : 'hashes match'}).`);
