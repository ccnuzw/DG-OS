#!/usr/bin/env node
// Project entry for the vendored spec-docs-pro tools (tools/spec-docs-pro).
//
//   node scripts/docs-tools.mjs check     run check-docs on docs/ (read-only; exit 1 on errors)
//   node scripts/docs-tools.mjs browser   regenerate the offline doc browser in docs/90-参考资料
//   node scripts/docs-tools.mjs verify    regenerate into a temp copy and fail if the committed browser is stale
//
// The generated data carries a generatedAt timestamp. `verify` ignores that one field so an unchanged
// docs tree compares equal; the vendored scripts stay byte-identical to the upstream skill.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const tools = path.join(root, 'tools', 'spec-docs-pro', 'scripts');
const docs = path.join(root, 'docs');
const OUT = '90-参考资料';
const FILES = ['产品文档浏览器.html', '产品文档浏览器-文档数据.js'];
const mode = process.argv[2];

const fail = (message) => {
  console.error(`docs-tools: ${message}`);
  process.exit(1);
};
const run = (script, args) => {
  const result = spawnSync(process.execPath, [path.join(tools, script), ...args], { stdio: 'inherit' });
  if (result.error) fail(result.error.message);
  return result.status ?? 1;
};
// Two values differ between runs without any docs change: the timestamp, and the --docs path echoed in
// the data file's header comment (a temp directory during `verify`).
const stable = (text) =>
  text
    .replace(/"generatedAt":"[^"]*"/, '"generatedAt":""')
    .replace(/--docs [^\n]*\n/, '--docs <docs>\n');

if (mode === 'check') {
  process.exit(run('check-docs.mjs', ['--docs', docs]));
}

if (mode === 'browser') {
  process.exit(run('generate-browser.mjs', ['--docs', docs, '--strict']));
}

if (mode === 'verify') {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'dgos-docs-'));
  try {
    fs.cpSync(docs, path.join(tmp, 'docs'), { recursive: true });
    const status = spawnSync(
      process.execPath,
      [path.join(tools, 'generate-browser.mjs'), '--docs', path.join(tmp, 'docs'), '--strict', '--json'],
      { stdio: ['ignore', 'ignore', 'inherit'] },
    ).status;
    if (status !== 0) fail('generate-browser failed or reported data-quality warnings');
    const stale = FILES.filter((name) => {
      const committed = path.join(docs, OUT, name);
      const fresh = path.join(tmp, 'docs', OUT, name);
      return !fs.existsSync(committed) || stable(fs.readFileSync(committed, 'utf8')) !== stable(fs.readFileSync(fresh, 'utf8'));
    });
    if (stale.length) fail(`stale, run "npm run docs:browser": ${stale.join(', ')}`);
    console.log('doc browser is current');
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  process.exit(0);
}

fail('usage: node scripts/docs-tools.mjs <check|browser|verify>');
