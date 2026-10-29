// `npm run design:check` (FR-005 §9.1, D068, B05-35): ESLint (R1, R2 in scripts) + Stylelint (R2, R3 in CSS)
// + an attribute scan of index.html, compared with the legacy baselines.
//
//   node scripts/design-check.mjs              check; exit 1 on a violation or a stale baseline entry
//   node scripts/design-check.mjs --init       write baselines from the current findings; refuses to overwrite
//   node scripts/design-check.mjs --prune      shrink baselines to the current counts; refuses while violations exist
//   --root <dir>                               scan another root (tests); configs always come from this repo
//
// Exit codes: 0 clean, 1 violations or stale baseline, 2 the tooling itself failed (bad config, parse error).
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { ESLint } from 'eslint';
import stylelint from 'stylelint';
import { scanHtml } from './design-check/html-scan.mjs';
import { ESLINT_BASELINE, HTML_FILES, SCRIPT_GLOBS, STYLE_BASELINE, STYLE_GLOBS } from './design-check/scope.mjs';

const TOOL_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** Stylelint rule -> rule family of FR-005 §9.1. */
const STYLE_TAGS = {
  'color-no-hex': 'R2',
  'color-named': 'R2',
  'function-disallowed-list': 'R2',
  'dgos/defined-custom-properties': 'R3',
  'at-rule-no-unknown': 'R3',
  'declaration-property-unit-disallowed-list': 'R4',
};


function readBaseline(root, location) {
  const path = join(root, location);
  if (!existsSync(path)) return {};
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (error) {
    throw new Error(`${location} is not valid JSON (${error.message})`);
  }
}

function writeBaseline(root, location, baseline) {
  const sorted = Object.fromEntries(
    Object.keys(baseline)
      .sort()
      .map((file) => [file, Object.fromEntries(Object.keys(baseline[file]).sort().map((rule) => [rule, baseline[file][rule]]))]),
  );
  mkdirSync(dirname(join(root, location)), { recursive: true });
  writeFileSync(join(root, location), `${JSON.stringify(sorted, null, 2)}\n`);
}

async function collectEslint(root) {
  const eslint = new ESLint({
    cwd: root,
    overrideConfigFile: join(TOOL_ROOT, 'eslint.config.mjs'),
    errorOnUnmatchedPattern: false,
  });
  const findings = [];
  const toolErrors = [];
  for (const result of await eslint.lintFiles(SCRIPT_GLOBS)) {
    const file = relative(root, result.filePath).replaceAll('\\', '/');
    for (const message of result.messages) {
      if (message.fatal) toolErrors.push(`${file}:${message.line}:${message.column} ${message.message}`);
      else if (message.severity === 2 && message.ruleId) {
        const tag = /^(R\d)/.exec(message.message)?.[1] ?? 'R?';
        findings.push({ file, line: message.line, column: message.column, tag, rule: message.ruleId, message: message.message, baseline: ESLINT_BASELINE });
      }
    }
  }
  return { findings, toolErrors };
}

async function collectStylelint(root) {
  const config = (await import(pathToFileURL(join(TOOL_ROOT, 'stylelint.config.mjs')).href)).default;
  const output = await stylelint.lint({
    config,
    configBasedir: root,
    cwd: root,
    files: STYLE_GLOBS,
    ignoreDisables: true,
    allowEmptyInput: true,
  });
  const findings = [];
  const warnings = [];
  const toolErrors = [];
  for (const result of output.results) {
    const file = relative(root, result.source).replaceAll('\\', '/');
    for (const invalid of result.invalidOptionWarnings) toolErrors.push(`${file}: ${invalid.text}`);
    for (const parse of result.parseErrors ?? []) toolErrors.push(`${file}:${parse.line}:${parse.column} ${parse.text}`);
    for (const warning of result.warnings) {
      const base = { file, line: warning.line, column: warning.column, tag: STYLE_TAGS[warning.rule] ?? 'R?', rule: warning.rule, message: warning.text.replace(/\s*\([^()]*\)$/, '') };
      if (warning.severity === 'warning') warnings.push(base);
      else if (!warning.rule) toolErrors.push(`${file}:${warning.line}:${warning.column} ${warning.text}`);
      else findings.push({ ...base, baseline: STYLE_BASELINE });
    }
  }
  return { findings, warnings, toolErrors };
}

function collectHtml(root) {
  const findings = [];
  for (const file of HTML_FILES) {
    const path = join(root, file);
    if (!existsSync(path)) continue;
    for (const hit of scanHtml(readFileSync(path, 'utf8'))) findings.push({ file, ...hit, baseline: STYLE_BASELINE });
  }
  return findings;
}

/** Group findings by (file, rule) and compare each group with what the baseline allows. */
function compare(findings, baselines) {
  const groups = new Map();
  for (const finding of findings) {
    const key = `${finding.baseline}\u0000${finding.file}\u0000${finding.rule}`;
    if (!groups.has(key)) groups.set(key, { baseline: finding.baseline, file: finding.file, rule: finding.rule, items: [] });
    groups.get(key).items.push(finding);
  }
  const violations = [];
  const exceeded = [];
  const stale = [];
  for (const group of groups.values()) {
    const allowed = baselines[group.baseline]?.[group.file]?.[group.rule]?.count ?? 0;
    if (group.items.length > allowed) {
      violations.push(...group.items);
      exceeded.push({ file: group.file, rule: group.rule, allowed, actual: group.items.length });
    }
  }
  for (const [location, baseline] of Object.entries(baselines)) {
    for (const [file, rules] of Object.entries(baseline)) {
      for (const [rule, { count }] of Object.entries(rules)) {
        const actual = groups.get(`${location}\u0000${file}\u0000${rule}`)?.items.length ?? 0;
        if (actual < count) stale.push({ baseline: location, file, rule, allowed: count, actual });
      }
    }
  }
  return { violations, exceeded, stale };
}

function countByGroup(findings) {
  const baselines = {};
  for (const { baseline, file, rule } of findings) {
    baselines[baseline] ??= {};
    baselines[baseline][file] ??= {};
    baselines[baseline][file][rule] = { count: (baselines[baseline][file][rule]?.count ?? 0) + 1 };
  }
  return baselines;
}

/**
 * @param {string} root directory to scan
 * @param {{init?: boolean, prune?: boolean}} [options]
 */
export async function runDesignCheck(root, options = {}) {
  const eslintRun = await collectEslint(root);
  const styleRun = await collectStylelint(root);
  const findings = [...eslintRun.findings, ...styleRun.findings, ...collectHtml(root)];
  const toolErrors = [...eslintRun.toolErrors, ...styleRun.toolErrors];
  const locations = [ESLINT_BASELINE, STYLE_BASELINE];
  const baselines = Object.fromEntries(locations.map((location) => [location, readBaseline(root, location)]));
  const notes = [];

  if (toolErrors.length === 0 && options.init) {
    const created = countByGroup(findings);
    for (const location of locations) {
      if (existsSync(join(root, location))) throw new Error(`${location} already exists; --init never overwrites a baseline (the baseline may only shrink)`);
      writeBaseline(root, location, created[location] ?? {});
      baselines[location] = created[location] ?? {};
      notes.push(`已写入 ${location}`);
    }
  }

  const { violations, exceeded, stale } = compare(findings, baselines);

  if (toolErrors.length === 0 && options.prune && violations.length === 0) {
    // No group exceeds its allowance here, so the current counts are the shrunken baseline.
    const current = countByGroup(findings);
    for (const location of locations) writeBaseline(root, location, current[location] ?? {});
    notes.push(`已收缩 ${stale.length} 项存量清单`);
    stale.length = 0;
  }

  return {
    ok: toolErrors.length === 0 && violations.length === 0 && stale.length === 0,
    toolErrors,
    violations,
    exceeded,
    stale,
    warnings: styleRun.warnings,
    notes,
    findingCount: findings.length,
  };
}

export function formatReport(result) {
  const lines = [];
  for (const error of result.toolErrors) lines.push(`工具错误 ${error}`);
  for (const v of result.violations) lines.push(`${v.file}:${v.line}:${v.column}  ${v.tag}  ${v.rule}  ${v.message}`);
  for (const e of result.exceeded) {
    lines.push(e.allowed === 0 ? `  ${e.file} [${e.rule}] ${e.actual} 处，不在存量清单内` : `  ${e.file} [${e.rule}] ${e.actual} 处，超过存量清单的 ${e.allowed} 处`);
  }
  for (const s of result.stale) {
    lines.push(`存量清单过期：${s.file} [${s.rule}] 清单 ${s.allowed} 处，实际 ${s.actual} 处。运行 npm run design:prune 收缩清单`);
  }
  if (result.warnings.length > 0) {
    const perFile = new Map();
    for (const w of result.warnings) perFile.set(w.file, (perFile.get(w.file) ?? 0) + 1);
    const top = [...perFile].sort((a, b) => b[1] - a[1]).map(([file, n]) => `${file} ${n}`).join('，');
    lines.push(`R4 px 报告（不阻断）：${result.warnings.length} 处，${top}`);
  }
  lines.push(...result.notes);
  lines.push(result.ok ? 'design:check 通过' : 'design:check 失败');
  return lines.join('\n');
}

export async function main(argv) {
  const args = argv.slice(2);
  const rootIndex = args.indexOf('--root');
  const root = resolve(rootIndex >= 0 ? args[rootIndex + 1] : TOOL_ROOT);
  let result;
  try {
    result = await runDesignCheck(root, { init: args.includes('--init'), prune: args.includes('--prune') });
  } catch (error) {
    console.error(`design:check 无法运行：${error.message}`);
    return 2;
  }
  (result.ok ? console.log : console.error)(formatReport(result));
  if (result.toolErrors.length > 0) return 2;
  return result.ok ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exit(await main(process.argv));
}
