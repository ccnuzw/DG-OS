#!/usr/bin/env node
// Generates an offline, project-agnostic document viewer for a docs/ tree.
// Output: <docs>/<out>/产品文档浏览器.html + 产品文档浏览器-文档数据.js
// Usage: node scripts/generate-browser.mjs [--docs docs] [--out 90-参考资料] [--config docs/spec-docs.config.json]
//        [--project 名称] [--json] [--strict]

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildModel, loadConfig } from "./lib/docs-model.mjs";

const SKILL_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ASSETS = path.join(SKILL_ROOT, "assets", "browser");
const HTML_NAME = "产品文档浏览器.html";
const DATA_NAME = "产品文档浏览器-文档数据.js";

function parseArgs(argv) {
  const args = { docs: "docs", out: null, config: null, project: null, json: false, strict: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      if (i + 1 >= argv.length) fail(`${arg} 需要一个值`);
      return argv[++i];
    };
    if (arg === "--docs") args.docs = next();
    else if (arg === "--out") args.out = next();
    else if (arg === "--config") args.config = next();
    else if (arg === "--project") args.project = next();
    else if (arg === "--json") args.json = true;
    else if (arg === "--strict") args.strict = true;
    else if (arg === "-h" || arg === "--help") {
      console.log("用法: node generate-browser.mjs [--docs docs] [--out 90-参考资料] [--config 文件] [--project 名称] [--json] [--strict]");
      process.exit(0);
    } else fail(`未知参数: ${arg}`);
  }
  return args;
}

function fail(message) {
  console.error(`错误：${message}`);
  process.exit(2);
}

// JSON is embedded in a <script> file; escape sequences that could end the script or break JS parsing.
function safeJson(value) {
  return JSON.stringify(value)
    .replace(/</g, "\\u003c")
    .replace(/>/g, "\\u003e")
    .replace(/&/g, "\\u0026")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#039;" })[char]);
}

function brandMark(name) {
  const latin = String(name).match(/[A-Za-z0-9]/g);
  if (latin && latin.length >= 2) return latin.slice(0, 2).join("").toUpperCase();
  return Array.from(String(name).trim())[0] || "文";
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const docsRoot = path.resolve(args.docs);
  if (!fs.existsSync(docsRoot) || !fs.statSync(docsRoot).isDirectory()) fail(`docs 目录不存在: ${docsRoot}`);

  const configPath = args.config ? path.resolve(args.config) : path.join(docsRoot, "spec-docs.config.json");
  const config = loadConfig(configPath);
  const outDir = path.join(docsRoot, args.out || (config.browser && config.browser.outDir) || "90-参考资料");
  const outRel = path.relative(docsRoot, outDir).split(path.sep).join("/");
  if (outRel.startsWith("..")) fail("--out 必须位于 docs 目录内");

  const { docs, model } = buildModel(docsRoot, { config, projectName: args.project });
  if (!docs.length) fail(`${docsRoot} 下没有 Markdown 文件`);

  const evolution = model.evolutionDoc ? docs.find(doc => doc.id === model.evolutionDoc) : null;
  const depth = outRel ? outRel.split("/").length : 0;
  const data = {
    schemaVersion: 2,
    generatedAt: new Date().toISOString(),
    generator: "spec-docs-pro/generate-browser",
    docsBase: "../".repeat(depth),
    evolutionPath: evolution ? evolution.path : "",
    ...model,
    content: Object.fromEntries(docs.map(doc => [doc.id, doc.source]))
  };

  const read = name => fs.readFileSync(path.join(ASSETS, name), "utf8");
  const js = ["viewer.js", "viewer-markdown.js", "viewer-views.js"].map(read).join("\n\n");
  const html = read("viewer.html")
    .replace("{{TITLE}}", () => escapeHtml(`${model.project.name} 产品文档浏览器`))
    .replace("{{PROJECT}}", () => escapeHtml(model.project.name))
    .replace("{{BRAND}}", () => escapeHtml(brandMark(model.project.name)))
    .replace("{{DATA_FILE}}", () => encodeURI(DATA_NAME))
    .replace("{{CSS}}", () => read("viewer.css"))
    .replace("{{JS}}", () => js.replace(/<\/script/gi, "<\\/script"));

  fs.mkdirSync(outDir, { recursive: true });
  const htmlPath = path.join(outDir, HTML_NAME);
  const dataPath = path.join(outDir, DATA_NAME);
  fs.writeFileSync(dataPath, `// 由 spec-docs-pro 生成，请勿手工编辑。重新生成：node <skill>/scripts/generate-browser.mjs --docs ${args.docs}\nwindow.SPEC_DOCS_DATA = ${safeJson(data)};\n`, "utf8");
  fs.writeFileSync(htmlPath, html, "utf8");

  const summary = {
    project: model.project.name,
    docs: model.docs.length,
    features: model.features.length,
    featuresWithoutStatus: model.features.filter(item => item.implementation.statusKey === "unknown").length,
    decisions: model.decisions.length,
    adrs: model.adrs.length,
    versions: model.versions.map(item => `${item.id}(${item.kind})`),
    tracks: model.tracks.length,
    warnings: model.warnings.length,
    html: path.relative(process.cwd(), htmlPath),
    data: path.relative(process.cwd(), dataPath),
    sizeKB: {
      html: Number((fs.statSync(htmlPath).size / 1024).toFixed(1)),
      data: Number((fs.statSync(dataPath).size / 1024).toFixed(1))
    }
  };

  if (args.json) {
    console.log(JSON.stringify({ ...summary, warningList: model.warnings }, null, 2));
  } else {
    console.log(`项目：${summary.project}`);
    console.log(`文档 ${summary.docs} · 功能 ${summary.features}（未同步状态 ${summary.featuresWithoutStatus}）· 决策 ${summary.decisions} · ADR ${summary.adrs}`);
    console.log(`版本：${summary.versions.join(" ") || "无"} · 能力主线 ${summary.tracks}`);
    console.log(`输出：${summary.html}（${summary.sizeKB.html} KB）`);
    console.log(`      ${summary.data}（${summary.sizeKB.data} KB）`);
    if (model.warnings.length) {
      console.log(`\n数据质量提示 ${model.warnings.length} 条：`);
      for (const item of model.warnings.slice(0, 20)) console.log(`  [${item.code}] ${item.message}`);
      if (model.warnings.length > 20) console.log(`  … 另有 ${model.warnings.length - 20} 条，使用 --json 查看全部`);
    }
  }
  if (args.strict && model.warnings.length) process.exit(1);
}

main();
