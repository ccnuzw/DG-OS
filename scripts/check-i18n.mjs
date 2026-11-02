import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

const root = process.cwd();
const sourceRoots = ['src'];
const ignoredFiles = new Set([
  'src/i18n/legacy.ts', 'src/i18n/index.ts', 'src/i18n/runtime.ts',
  'src/lan/remote/i18n.ts', 'src/lan/remote/messages.ts',
]);
const files = [];
function walk(dir) {
  for (const entry of readdirSync(join(root, dir))) {
    if (['node_modules', 'dist', 'dist-lan'].includes(entry)) continue;
    const relative = join(dir, entry);
    if (statSync(join(root, relative)).isDirectory()) walk(relative);
    else if (/\.(tsx|ts)$/.test(entry) && !ignoredFiles.has(relative)) files.push(relative);
  }
}
for (const dir of sourceRoots) walk(dir);

const findings = [];
const attributeNames = new Set(['aria-label', 'aria-description', 'placeholder', 'title', 'label', 'alt']);
const allowedTechnical = /^(?:https?:\/\/|wss?:\/\/|[a-z]+:\/\/|\/\S+|\[[^\]]+\]|[A-Z0-9]+(?:[_.-][A-Z0-9]+)*)$/;
function isTranslatedExpression(expression, sourceFile) {
  if (ts.isJsxElement(expression) || ts.isJsxSelfClosingElement(expression)) {
    const tag = expression.openingElement.tagName;
    return ts.isIdentifier(tag) && (tag.text === 'LegacyText' || tag.text === 'Trans');
  }
  return ts.isCallExpression(expression) && ts.isIdentifier(expression.expression)
    && ['legacyValue', 't', 'translate'].includes(expression.expression.text);
}
function isStaticText(node, sourceFile) {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text;
  if (ts.isJsxExpression(node) && node.expression && isTranslatedExpression(node.expression, sourceFile)) return null;
  if (ts.isJsxExpression(node) && node.expression && (ts.isStringLiteral(node.expression) || ts.isNoSubstitutionTemplateLiteral(node.expression))) return node.expression.text;
  return null;
}
for (const file of files) {
  const content = readFileSync(join(root, file), 'utf8');
  const sourceFile = ts.createSourceFile(file, content, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  function visit(node) {
    if (ts.isJsxText(node)) {
      const text = node.getText(sourceFile).replace(/\s+/g, ' ').trim();
      if (text && /[\p{L}\p{N}]/u.test(text) && !/^[{}()[\]<>.,:;|/\s-]+$/.test(text)) {
        const { line } = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
        findings.push(`${file}:${line + 1}: ${text}`);
      }
    } else if (ts.isJsxAttribute(node) && attributeNames.has(node.name.getText(sourceFile)) && node.initializer) {
      const text = isStaticText(node.initializer, sourceFile)?.trim();
      if (text && /[\p{L}\p{N}]/u.test(text) && !allowedTechnical.test(text) && !/^(?:Python|Node\.js|ms|V1\s*·\s*D021|SHA-256:|v|\/\s*\d+|tokens|model-id|https:\/\/your-service\.example\.com\/v1|Bearer|x-api-key)$/i.test(text)) {
        const { line } = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
        findings.push(`${file}:${line + 1}: ${node.name.getText(sourceFile)}="${text}"`);
      }
    } else if (ts.isJsxExpression(node) && node.expression && (ts.isStringLiteral(node.expression) || ts.isNoSubstitutionTemplateLiteral(node.expression)) && ts.isJsxElement(node.parent) && ts.isJsxText(node.parent.children[node.parent.children.indexOf(node)]) === false) {
      const text = node.expression.text.trim();
      if (text && /[\p{L}\p{N}]/u.test(text) && !allowedTechnical.test(text)) {
        const { line } = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
        findings.push(`${file}:${line + 1}: JSX expression text: ${text}`);
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(sourceFile);
}

function parseSource(relativePath) {
  const content = readFileSync(join(root, relativePath), 'utf8');
  return ts.createSourceFile(relativePath, content, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
}
function propertyKey(property, sourceFile) {
  if (!ts.isPropertyAssignment(property)) return null;
  if (ts.isStringLiteral(property.name) || ts.isIdentifier(property.name) || ts.isNumericLiteral(property.name)) return property.name.text;
  return null;
}
function findObject(relativePath, variableName) {
  const sourceFile = parseSource(relativePath);
  let found = null;
  function visit(node) {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === variableName && node.initializer) {
      let initializer = node.initializer;
      while (ts.isAsExpression(initializer) || ts.isSatisfiesExpression(initializer) || ts.isParenthesizedExpression(initializer)) initializer = initializer.expression;
      if (ts.isObjectLiteralExpression(initializer)) found = initializer;
    }
    ts.forEachChild(node, visit);
  }
  visit(sourceFile);
  if (!found) throw new Error(`Cannot find catalog object ${variableName} in ${relativePath}`);
  return { object: found, sourceFile };
}
function collectFlatKeys(object, sourceFile, prefix = '', result = new Map()) {
  const seen = new Set();
  for (const property of object.properties) {
    const key = propertyKey(property, sourceFile);
    if (key === null) continue;
    if (seen.has(key)) findings.push(`${sourceFile.fileName}: duplicate catalog key '${prefix}${key}'`);
    seen.add(key);
    if (!ts.isPropertyAssignment(property)) continue;
    let value = property.initializer;
    while (ts.isAsExpression(value) || ts.isSatisfiesExpression(value) || ts.isParenthesizedExpression(value)) value = value.expression;
    const fullKey = prefix ? `${prefix}.${key}` : key;
    if (ts.isObjectLiteralExpression(value)) collectFlatKeys(value, sourceFile, fullKey, result);
    else result.set(fullKey, value);
  }
  return result;
}
function interpolationNames(value) {
  return [...String(value).matchAll(/\{([a-zA-Z0-9_]+)\}/g)].map(match => match[1]).sort();
}
function checkFlatCatalogPair(enPath, enName, zhPath, zhName) {
  const enSource = findObject(enPath, enName);
  const zhSource = findObject(zhPath, zhName);
  const en = collectFlatKeys(enSource.object, enSource.sourceFile);
  const zh = collectFlatKeys(zhSource.object, zhSource.sourceFile);
  const enKeys = new Set(en.keys());
  const zhKeys = new Set(zh.keys());
  for (const key of enKeys) if (!zhKeys.has(key)) findings.push(`${zhPath}: missing translation key '${key}'`);
  for (const key of zhKeys) if (!enKeys.has(key)) findings.push(`${zhPath}: extra translation key '${key}'`);
  for (const key of enKeys) {
    if (!zhKeys.has(key)) continue;
    const enValue = en.get(key);
    const zhValue = zh.get(key);
    if (!ts.isStringLiteral(enValue) || !ts.isStringLiteral(zhValue)) continue;
    if (JSON.stringify(interpolationNames(enValue.text)) !== JSON.stringify(interpolationNames(zhValue.text))) {
      findings.push(`${zhPath}: interpolation mismatch for '${key}'`);
    }
  }
}
function checkTupleCatalogPair(relativePath, variableName) {
  const { object, sourceFile } = findObject(relativePath, variableName);
  for (const property of object.properties) {
    const key = propertyKey(property, sourceFile);
    if (key === null || !ts.isPropertyAssignment(property)) continue;
    let value = property.initializer;
    while (ts.isAsExpression(value) || ts.isSatisfiesExpression(value) || ts.isParenthesizedExpression(value)) value = value.expression;
    if (!ts.isArrayLiteralExpression(value) || value.elements.length !== 2 || !value.elements.every(ts.isStringLiteral)) {
      findings.push(`${relativePath}: '${key}' must contain an English/Chinese string pair`);
      continue;
    }
    const [zhValue, enValue] = value.elements;
    if (JSON.stringify(interpolationNames(enValue.text)) !== JSON.stringify(interpolationNames(zhValue.text))) {
      findings.push(`${relativePath}: interpolation mismatch for '${key}'`);
    }
  }
}

checkFlatCatalogPair('src/i18n/locales/en-US.ts', 'enUS', 'src/i18n/locales/zh-CN.ts', 'zhCN');
checkFlatCatalogPair('src/i18n/locales/phase2.en-US.ts', 'phase2EnUS', 'src/i18n/locales/phase2.zh-CN.ts', 'phase2ZhCN');
checkTupleCatalogPair('src/i18n/locales/details.ts', 'detailMessages');
checkTupleCatalogPair('src/i18n/locales/chat.ts', 'chatMessages');

const strict = process.argv.includes('--strict');
if (findings.length) {
  console.log(`Static i18n scan: ${findings.length} remaining raw UI literal(s) across ${files.length} source files.`);
  console.log(findings.join('\n'));
  if (strict) process.exit(1);
} else {
  console.log(`Static i18n scan: no raw UI literals across ${files.length} TypeScript source files.`);
}
