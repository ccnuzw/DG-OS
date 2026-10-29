#!/usr/bin/env node
// Regenerates the data of the offline documentation browser from docs/.
//
//   node scripts/generate-doc-browser.mjs            write the derived files
//   node scripts/generate-doc-browser.mjs --check    fail if they are stale
//   --docs <dir>                                     docs root (default: docs)
//
// Markdown is authoritative. Derived from it: every document's full text and
// title, the decision list (D### headings of the frozen decisions), and the
// feature goal, dependencies and implementation status. Curated, kept from
// the existing viewer: the viewer HTML itself (layout, SVG diagrams, script),
// document ids, versions, phases, accents, short goals of existing documents,
// and each feature's track. The evolution timeline is built by the viewer at
// runtime from 功能演进矩阵.md, so it needs no generated data.

import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';

const args = process.argv.slice(2);
const check = args.includes('--check');
const docsArg = args.indexOf('--docs');
const docs = path.resolve(docsArg >= 0 ? args[docsArg + 1] : 'docs');
const outDir = path.join(docs, '90-参考资料');
const VIEWER = path.join(outDir, '产品文档浏览器.html');
const DATA = path.join(outDir, '产品文档浏览器-文档数据.js');
const SINGLE = path.join(outDir, '产品文档浏览器-单文件版.html');
const SCRIPT_TAG = '<script src="产品文档浏览器-文档数据.js"></script>';
const ACCENTS = ['#087d75', '#4056a1', '#2f7d4f', '#a96810', '#aa2d4d'];
const STATUS_KEYS = { '🟢': 'local-verified', '🟡': 'partial', '🔴': 'foundation' };

const fail = (message) => {
  console.error(`generate-doc-browser: ${message}`);
  process.exit(1);
};
const read = (file) => fs.readFileSync(file, 'utf8');

function walk(dir) {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .filter((entry) => !entry.name.startsWith('.'))
    .flatMap((entry) => {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) return walk(full);
      return entry.name.endsWith('.md') ? [full] : [];
    })
    .sort();
}

// Find the JSON array literal assigned to `const NAME = [` (string aware).
function arrayBounds(html, name) {
  const marker = `const ${name} = [`;
  const at = html.indexOf(marker);
  if (at < 0) fail(`viewer has no "${marker}"`);
  const start = at + marker.length - 1;
  let depth = 0;
  let inString = false;
  for (let i = start; i < html.length; i += 1) {
    const ch = html[i];
    if (inString) {
      if (ch === '\\') i += 1;
      else if (ch === '"') inString = false;
    } else if (ch === '"') inString = true;
    else if (ch === '[') depth += 1;
    else if (ch === ']' && --depth === 0) return [start, i + 1];
  }
  return fail(`unterminated array ${name}`);
}

const viewer = read(VIEWER);
const [docsStart, docsEnd] = arrayBounds(viewer, 'DOCS');
const oldDocs = JSON.parse(viewer.slice(docsStart, docsEnd));

const sandbox = { window: {} };
vm.createContext(sandbox);
vm.runInContext(read(DATA), sandbox);
const oldFeatures = sandbox.window.DGOS_FEATURE_BROWSER_DATA ?? [];

const firstHeading = (text) =>
  (text.replace(/^```[\s\S]*?^```/gm, '').match(/^#\s+(.+)$/m) ?? [])[1]?.trim();

// ---- documents -------------------------------------------------------------
const relFile = (file) => path.relative(docs, path.resolve(outDir, file)).split(path.sep).join('/');
const existing = new Map(oldDocs.map((doc) => [relFile(doc.file), doc]));
const phaseStyle = new Map(oldDocs.map((doc) => [doc.phase, { accent: doc.accent, soft: doc.soft }]));
const sources = walk(docs).map((file) => path.relative(docs, file).split(path.sep).join('/'));
const sourceSet = new Set(sources);
const phaseOf = (rel) => {
  if (rel.startsWith('99-')) return '归档';
  if (rel.startsWith('90-')) return '参考';
  if (rel.startsWith('06-')) return '决策';
  if (rel.startsWith('04-')) return '架构';
  if (rel.startsWith('03-')) return '规格';
  if (rel.startsWith('02-')) return '产品';
  return rel === 'README.md' ? '概览' : '其他';
};
let nextId = Math.max(0, ...oldDocs.map((d) => Number(/^doc-(\d+)$/.exec(d.id)?.[1] ?? 0))) + 1;
const texts = {};
const docList = [];
const keep = oldDocs.filter((doc) => sourceSet.has(relFile(doc.file)));
const added = sources.filter((rel) => !existing.has(rel));
for (const rel of [...keep.map((doc) => relFile(doc.file)), ...added]) {
  const text = read(path.join(docs, rel));
  const old = existing.get(rel);
  const heading = firstHeading(text);
  const archived = rel.startsWith('99-') && rel !== '99-历史归档/README.md';
  const base = heading ?? old?.title ?? path.basename(rel, '.md');
  const title = archived && !base.startsWith('历史：') ? `历史：${base}` : base;
  let doc = old;
  if (!doc) {
    const style = phaseStyle.get(phaseOf(rel)) ?? phaseStyle.get('其他') ?? { accent: ACCENTS[0], soft: '#e3f1ef' };
    const paragraph = text
      .split(/\n\s*\n/)
      .map((p) => p.trim())
      .find((p) => p && !p.startsWith('#') && !p.startsWith('|') && !p.startsWith('```'));
    doc = {
      id: `doc-${String(nextId++).padStart(3, '0')}`,
      version: /(?:^|\/)V(\d)/.exec(rel)?.[0] ? `V${/(?:^|\/)V(\d)/.exec(rel)[1]}` : '00',
      phase: phaseOf(rel),
      accent: style.accent,
      soft: style.soft,
      file: path.relative(outDir, path.join(docs, rel)).split(path.sep).join('/'),
      goal: (paragraph ?? title).replace(/[>*`#]/g, '').replace(/\s+/g, ' ').trim().slice(0, 140),
    };
  }
  const { id, version, phase, accent, soft, file, goal } = doc;
  docList.push({ id, version, phase, accent, soft, title, file, goal });
  texts[id] = text;
}

// ---- decisions -------------------------------------------------------------
const decisionsText = read(path.join(docs, '06-决策记录', 'V1-冻结决策.md'));
const headings = [...decisionsText.matchAll(/^## (D\d+ .+)$/gm)].map((m) => m[1].trim());
const oldDecisionText = new Map();
const [decStart, decEnd] = arrayBounds(viewer, 'DECISIONS');
for (const item of JSON.parse(viewer.slice(decStart, decEnd))) oldDecisionText.set(item.text.split(' ')[0], item.text);
const decisions = headings.map((heading, index) => {
  const id = heading.split(' ')[0];
  const previous = oldDecisionText.get(id);
  // Keep a curated suffix such as "（Skills 部分由 D016 取代）" while the heading is unchanged.
  const text = previous?.startsWith(heading) ? previous : heading;
  return { accent: ACCENTS[index % ACCENTS.length], text };
});

// ---- features --------------------------------------------------------------
const statusText = read(path.join(docs, '02-产品与版本', '当前版本', 'V1-实现状态.md'));
const updatedAt = /\*\*最后更新\*\*：(.+)/.exec(statusText)?.[1]?.trim();
if (!updatedAt) fail('V1-实现状态.md has no "**最后更新**：" line');
const statusRows = new Map();
for (const line of statusText.split('\n')) {
  if (!/^\| V1-FR-\d+ \|/.test(line)) continue;
  const cells = line.split('|').slice(1, -1).map((cell) => cell.trim());
  const [id, , status, , , ...notes] = cells;
  const key = STATUS_KEYS[[...status][0]];
  if (!key) fail(`${id}: unknown status marker in "${status}"`);
  statusRows.set(id, { key, label: status.replace(/^\S+\s*/, ''), gap: notes.join('|').trim() });
}
const meta = (text, ...names) => {
  for (const name of names) {
    const value = new RegExp(`^\\| ${name} \\| (.+) \\|\\s*$`, 'm').exec(text)?.[1]?.trim();
    if (value) return value;
  }
  return undefined;
};
const features = [];
for (const rel of sources.filter((rel) => /^03-功能规格\/V1\/\d\d-[^/]+\/\d\d-[^/]+\.md$/.test(rel))) {
  const text = read(path.join(docs, rel));
  const id = /^V1-FR-\d+/.exec(meta(text, '功能编号') ?? '')?.[0];
  if (!id) continue;
  const old = oldFeatures.find((feature) => feature.id === id);
  if (!old) fail(`${id}: add its track/trackName to the existing feature data before generating`);
  const implementation = statusRows.get(id);
  if (!implementation) fail(`${id}: missing from the V1-实现状态.md table`);
  const goal = /### 2\.1[^\n]*\n+([\s\S]*?)\n\n/.exec(text)?.[1]?.trim();
  features.push({
    id,
    title: meta(text, '功能名称')?.replace(/（原[^）]*）$/, ''),
    track: old.track,
    trackName: old.trackName,
    domain: meta(text, '所属领域') ?? meta(text, '所属版本 / 领域')?.split('/').pop()?.trim(),
    goal,
    file: path.relative(outDir, path.join(docs, rel)).split(path.sep).join('/'),
    dependencies: meta(text, '依赖功能', '依赖'),
    implementation: { ...implementation, updatedAt },
  });
}
features.sort((a, b) => a.id.localeCompare(b.id));

// ---- output ----------------------------------------------------------------
const date = /^(\d{4}-\d{2}-\d{2})/.exec(updatedAt)?.[1] ?? 'unknown';
const dataJs =
  `// DGOS 产品文档浏览器 - 文档数据\n// 由项目文档生成，请勿手工编辑。权威内容以 Markdown 为准。\n// 生成时间: ${date}\n\n` +
  `window.DGOS_DOC_BROWSER_DATA = Object.freeze({\n` +
  Object.entries(texts).map(([id, text]) => `  ${JSON.stringify(id)}: ${JSON.stringify(text)}`).join(',\n') +
  `\n});\n\nwindow.DGOS_FEATURE_BROWSER_DATA = Object.freeze(${JSON.stringify(features, null, 2)});\n`;
const replaceArray = (html, name, value) => {
  const [start, end] = arrayBounds(html, name);
  return html.slice(0, start) + JSON.stringify(value, null, 2) + html.slice(end);
};
let nextViewer = replaceArray(viewer, 'DOCS', docList);
nextViewer = replaceArray(nextViewer, 'DECISIONS', decisions);
if (!nextViewer.includes(SCRIPT_TAG)) fail(`viewer has no ${SCRIPT_TAG}`);
const single = nextViewer.replace(SCRIPT_TAG, () => `<script>\n${dataJs}</script>`);

const outputs = [
  [VIEWER, nextViewer],
  [DATA, dataJs],
  [SINGLE, single],
];
const stale = outputs.filter(([file, content]) => !fs.existsSync(file) || read(file) !== content);
if (check) {
  if (stale.length) fail(`stale, run "node scripts/generate-doc-browser.mjs": ${stale.map(([f]) => path.basename(f)).join(', ')}`);
  console.log(`doc browser is current: ${docList.length} docs, ${decisions.length} decisions, ${features.length} features`);
} else {
  for (const [file, content] of stale) fs.writeFileSync(file, content);
  console.log(
    `doc browser: ${docList.length} docs (${added.length} new), ${decisions.length} decisions, ${features.length} features; wrote ${stale.length} file(s)`,
  );
}
