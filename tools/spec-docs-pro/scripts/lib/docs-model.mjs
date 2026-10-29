// Builds a project-agnostic model of a spec-docs-pro style docs/ tree.
// Used by generate-browser.mjs (viewer data) and check-docs.mjs (consistency checks).

import fs from "node:fs";
import path from "node:path";
import {
  parseFrontmatter,
  extractHeadings,
  extractTables,
  extractSections,
  firstParagraph,
  cleanInline,
  truncate,
  similarity
} from "./markdown.mjs";
import { matchImplementationStatus, matchSpecStatus, matchDecisionStatus } from "./status.mjs";

const PALETTE = [
  { accent: "#087d75", soft: "#dcefeb" },
  { accent: "#4056a1", soft: "#e6e9f6" },
  { accent: "#2f7d4f", soft: "#e1f0e8" },
  { accent: "#a96810", soft: "#f6ead6" },
  { accent: "#aa2d4d", soft: "#f4dde4" },
  { accent: "#6d4bb3", soft: "#ebe5f6" }
];
const GRAY = { accent: "#69707a", soft: "#f1eee7" };
const TRACK_COLORS = ["#087d75", "#4056a1", "#aa2d4d", "#a96810", "#2f7d4f", "#6d4bb3", "#69707a"];

const FEATURE_ID = /^V\d+-FR-\d{3}$/;
const collator = new Intl.Collator("zh-Hans-CN", { numeric: true });

export function loadConfig(configPath) {
  if (!configPath || !fs.existsSync(configPath)) return {};
  try {
    return JSON.parse(fs.readFileSync(configPath, "utf8"));
  } catch (error) {
    throw new Error(`无法解析配置文件 ${configPath}: ${error.message}`);
  }
}

export function listMarkdown(root, exclude = []) {
  const results = [];
  const walk = dir => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.name.startsWith(".") || entry.name === "node_modules") continue;
      const full = path.join(dir, entry.name);
      const rel = path.relative(root, full).split(path.sep).join("/");
      if (exclude.some(prefix => rel === prefix || rel.startsWith(prefix.replace(/\/?\*\*$/, "").replace(/\/?$/, "/")))) continue;
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile() && entry.name.toLowerCase().endsWith(".md")) results.push(rel);
    }
  };
  walk(root);
  return results.sort(collator.compare);
}

export function normVersion(value) {
  const match = /^v?(\d+)$/i.exec(String(value || "").trim());
  return match ? `V${Number(match[1])}` : null;
}

function versionNumber(id) {
  const match = /^V(\d+)$/.exec(id || "");
  return match ? Number(match[1]) : Number.POSITIVE_INFINITY;
}

function groupColor(group) {
  const match = /^(\d+)/.exec(group);
  if (!match) return GRAY;
  const n = Number(match[1]);
  if (n >= 90) return GRAY;
  return PALETTE[(n - 1 + PALETTE.length) % PALETTE.length];
}

function pathVersion(rel) {
  for (const part of rel.split("/")) {
    const direct = normVersion(part);
    if (direct) return direct;
    const prefixed = /^(V\d+)-/i.exec(part);
    if (prefixed) return normVersion(prefixed[1]);
  }
  return null;
}

export function classify(rel, fm = {}) {
  if (fm.doc_type) return fm.doc_type;
  const base = path.posix.basename(rel);
  if (/-技术设计\.md$/.test(base)) return "design";
  if (/冻结决策\.md$/.test(base)) return "decision-log";
  if (/(^|\/)ADR\//.test(rel) && base.toLowerCase() !== "readme.md") return "adr";
  if (/功能演进矩阵\.md$/.test(base)) return "evolution";
  if (/功能演进索引\.md$/.test(base)) return "evolution-index";
  if (/版本路线图\.md$/.test(base)) return "roadmap";
  if (/^V\d+-实现状态\.md$/i.test(base)) return "status";
  if (/^V\d+-需求编号\.md$|需求编号\.md$/.test(base)) return "numbering";
  if (/需求追踪矩阵\.md$/.test(base)) return "tracking";
  const isSpecArea = /(^|\/)03-[^/]*\//.test(rel) || rel.includes("功能规格/");
  if (fm.feature_id || (isSpecArea && /^\d{2,3}-.+\.md$/.test(base) && !/^00-/.test(base))) return "feature";
  return "doc";
}

function readDoc(root, rel) {
  const source = fs.readFileSync(path.join(root, ...rel.split("/")), "utf8");
  const fm = parseFrontmatter(source);
  const headings = extractHeadings(fm.body);
  const h1 = headings.find(item => item.level === 1);
  const fallback = path.posix.basename(rel, ".md");
  const group = rel.includes("/") ? rel.split("/")[0] : "根目录";
  return {
    id: rel.replace(/\.md$/i, ""),
    path: rel,
    source,
    body: fm.body,
    fm: fm.data,
    hasFrontmatter: fm.hasFrontmatter,
    frontmatterLines: fm.lineCount,
    title: cleanInline(fm.data.title) || (h1 ? h1.text : fallback),
    headings,
    group,
    kind: classify(rel, fm.data),
    version: normVersion(fm.data.version) || pathVersion(rel)
  };
}

function cardFields(body) {
  const fields = {};
  const table = extractTables(body).find(item => item.header[0] === "项目" && /内容|说明|值/.test(item.header[1] || ""));
  if (!table) return fields;
  for (const row of table.rows) fields[cleanInline(row[0])] = cleanInline(row[1]);
  return fields;
}

function sectionText(body, names) {
  const section = extractSections(body, 2).find(item => names.some(name => item.title.replace(/^\d+[.、\s]*/, "") === name));
  if (!section) return "";
  for (const item of section.lines) {
    if (item.inCode) continue;
    const line = item.line.trim();
    if (!line || /^[|#>!]/.test(line) || /^(包含|不包含)[:：]?$/.test(line)) continue;
    return cleanInline(line.replace(/^[-*+]\s+|^\d+\.\s+/, ""));
  }
  return "";
}

// Expands "AC05～07", "AC05-AC07", "AC01、AC03" into individual IDs.
export function expandAcIds(text) {
  const ids = [];
  for (const match of cleanInline(text).matchAll(/AC(\d{2,3})(?:\s*[～~–—-]\s*(?:AC)?(\d{2,3}))?/g)) {
    const start = Number(match[1]);
    const end = match[2] ? Number(match[2]) : start;
    const width = match[1].length;
    for (let n = start; n <= end && n - start < 50; n += 1) ids.push(`AC${String(n).padStart(width, "0")}`);
  }
  return ids;
}

// Preferred form: "#### AC01 标题" + Given/When/Then lines. A compact table form
// (| AC01 | 描述 |) is also recognised so legacy documents are not reported as empty.
export function parseAcceptance(body) {
  const items = [];
  const headings = extractHeadings(body);
  const lines = body.replace(/\r\n/g, "\n").split("\n");
  headings.forEach((heading, index) => {
    const match = /^(AC\d{2,3})\b[\s:：-]*(.*)$/.exec(heading.text);
    if (!match || heading.level < 3) return;
    const next = headings.slice(index + 1).find(item => item.level <= heading.level);
    const block = lines.slice(heading.line + 1, next ? next.line : lines.length).join("\n");
    items.push({
      id: match[1],
      title: match[2].trim(),
      line: heading.line,
      form: "heading",
      given: /(^|\n)\s*[-*]?\s*\**Given\b/i.test(block) || /前置[:：]|假设[:：]/.test(block),
      when: /(^|\n)\s*[-*]?\s*\**When\b/i.test(block),
      then: /(^|\n)\s*[-*]?\s*\**Then\b/i.test(block)
    });
  });
  if (items.length) return items;
  for (const table of extractTables(body)) {
    const inAcceptance = table.h2 && /验收/.test(table.h2.text);
    const isTestDesign = table.header.some(cell => /测试层级|目标命令|目标资产|测试文件/.test(cell));
    if (!inAcceptance || isTestDesign || !/^(AC|编号)/i.test(table.header[0])) continue;
    for (const row of table.rows) {
      const id = /^(AC\d{2,3})$/.exec(cleanInline(row[0]));
      if (!id) continue;
      const text = row.slice(1).join(" ");
      items.push({
        id: id[1],
        title: cleanInline(row[1] || ""),
        line: table.line,
        form: "table",
        given: /Given/i.test(text),
        when: /When/i.test(text),
        then: /Then/i.test(text)
      });
    }
  }
  return items;
}

export function parseAcTestTable(body) {
  const table = extractTables(body).find(item =>
    /^AC(\s*编号)?$/i.test(item.header[0]) && item.header.some(cell => /测试层级|目标命令|目标资产/.test(cell))
  );
  if (!table) return null;
  const col = pattern => table.header.findIndex(cell => pattern.test(cell));
  const level = col(/测试层级/);
  const asset = col(/目标资产|测试文件/);
  const command = col(/目标命令|命令/);
  const status = col(/状态/);
  return {
    line: table.line,
    rows: table.rows.flatMap(row => {
      const entry = {
        level: level >= 0 ? cleanInline(row[level]) : "",
        asset: asset >= 0 ? cleanInline(row[asset]) : "",
        command: command >= 0 ? cleanInline(row[command]) : "",
        status: status >= 0 ? cleanInline(row[status]) : ""
      };
      const ids = expandAcIds(row[0]);
      return (ids.length ? ids : [cleanInline(row[0])]).map(id => ({ id, ...entry }));
    })
  };
}

function parseFeature(doc, docsByPath) {
  const base = path.posix.basename(doc.path, ".md");
  const numbered = /^(\d{2,3})-(.+)$/.exec(base);
  const version = doc.version || "V1";
  const card = cardFields(doc.body);
  // Only identity-bearing places count; IDs mentioned in prose are references, not identity.
  let id = FEATURE_ID.test(String(doc.fm.feature_id || "").trim()) ? doc.fm.feature_id.trim() : null;
  if (!id) {
    const found = /\b(V\d+-FR-\d{3})\b/.exec(`${doc.title} ${card["编号"] || ""} ${card["功能编号"] || ""}`);
    if (found) id = found[1];
  }
  if (!id && numbered) id = `${version}-FR-${numbered[1].padStart(3, "0")}`;
  if (!id) return null;
  const number = numbered ? numbered[1] : id.slice(-3);
  const designPath = doc.path.replace(/\.md$/, "-技术设计.md");
  const acceptance = parseAcceptance(doc.body);
  const parent = doc.path.split("/").slice(-2, -1)[0] || "";
  const title = doc.title
    .replace(new RegExp(`^${id}\\s*[·:：-]?\\s*`), "")
    .replace(new RegExp(`^0*${Number(number)}\\s+|^${number}\\s+`), "")
    .trim();
  const specRaw = doc.fm.spec_status || card["规格状态"] || "";
  const riskRaw = doc.fm.risk || card["风险等级"] || card["风险"] || "";
  return {
    id,
    number,
    version: normVersion(doc.fm.version) || version,
    title: title || doc.title,
    doc: doc.id,
    design: docsByPath.has(designPath) ? designPath.replace(/\.md$/, "") : null,
    domain: cleanInline(doc.fm.domain) || card["领域"] || parent.replace(/^\d+-/, ""),
    trackHint: cleanInline(doc.fm.track || card["能力主线"] || ""),
    risk: /高/.test(riskRaw) || /high/i.test(riskRaw) || docsByPath.has(designPath) ? "high" : "standard",
    specStatus: (matchSpecStatus(specRaw) || {}).label || (specRaw ? cleanInline(specRaw) : ""),
    legacyStatus: doc.fm.status || card["状态"] || "",
    goal: truncate(sectionText(doc.body, ["目标", "功能目标"]) || firstParagraph(doc.source), 140),
    dependencies: card["上游依赖"] || "",
    acCount: acceptance.length
  };
}

function parseStatusDoc(doc) {
  const entries = new Map();
  const updated = /更新(?:时间|日期)[^\d]{0,6}(\d{4}-\d{2}-\d{2})/.exec(doc.body);
  for (const table of extractTables(doc.body)) {
    const header = table.header;
    const idCol = header.findIndex(cell => /编号|^ID$|^功能$/.test(cell));
    if (idCol < 0 || !header.some(cell => /状态|后端|前端|验证|测试/.test(cell))) continue;
    const col = pattern => header.findIndex((cell, index) => index !== idCol && pattern.test(cell));
    const statusCol = col(/^(当前)?状态$|实现状态|工程状态|当前状态/);
    const backendCol = col(/后端/);
    const frontendCol = col(/前端/);
    const verifyCol = col(/验证|测试/);
    const docCol = col(/^文档/);
    const gapCol = col(/差距|剩余|阻塞/);
    const dateCol = col(/更新|日期/);
    for (const row of table.rows) {
      const rawId = cleanInline(row[idCol]);
      const full = /\b(V\d+-FR-\d{3})\b/.exec(rawId);
      const bare = /^(\d{1,3})$/.exec(rawId);
      if (!full && !bare) continue;
      const key = full ? full[1] : `${doc.version || "V1"}#${bare[1].padStart(3, "0")}`;
      const pick = index => (index >= 0 ? cleanInline(row[index]) : "");
      // Canonical tables have one status column; legacy tables spread it across cells.
      const matched = [statusCol, docCol, backendCol, frontendCol]
        .map(index => matchImplementationStatus(pick(index)))
        .find(Boolean);
      entries.set(key, {
        statusKey: matched ? matched.key : "unknown",
        statusLabel: matched ? matched.label : "未同步",
        statusRaw: pick(statusCol),
        backend: truncate(pick(backendCol), 120),
        frontend: truncate(pick(frontendCol), 120),
        verification: truncate(pick(verifyCol), 120),
        gap: truncate(pick(gapCol), 160),
        updatedAt: pick(dateCol) || (updated ? updated[1] : ""),
        source: doc.id
      });
    }
  }
  return entries;
}

function parseDecisionLog(doc) {
  const decisions = [];
  for (const section of extractSections(doc.body, 2)) {
    const match = /^(D\d{3,})\s*[·:：-]?\s*(.*)$/.exec(section.title);
    if (!match) continue;
    let status = "";
    let summary = "";
    let date = "";
    for (const item of section.lines) {
      if (item.inCode) continue;
      const line = cleanInline(item.line.replace(/^\s*[-*+]\s+/, ""));
      if (!line) continue;
      const statusLine = /^状态[:：]\s*(.+)$/.exec(line);
      const decision = /^(决定|决策内容|结论)[:：]\s*(.+)$/.exec(line);
      const dateLine = /^日期[:：]\s*(\d{4}-\d{2}-\d{2})/.exec(line);
      if (statusLine && !status) status = statusLine[1].replace(/[。.]$/, "");
      else if (dateLine && !date) date = dateLine[1];
      else if (decision) summary = decision[2];
      else if (!summary && !/^(背景|备选方案|影响|实施|理由)[:：]?/.test(line) && !/^[A-C]\.\s/.test(line)) summary = line;
    }
    const matchedStatus = matchDecisionStatus(status);
    decisions.push({
      id: match[1],
      title: match[2].trim(),
      status: matchedStatus ? matchedStatus.label : status || "未标注",
      statusKey: matchedStatus ? matchedStatus.key : "unknown",
      date,
      summary: truncate(summary, 120),
      doc: doc.id,
      line: section.line
    });
  }
  return decisions;
}

function parseAdr(doc) {
  const match = /^(?:ADR-?)?(\d{4})/i.exec(path.posix.basename(doc.path));
  if (!match) return null;
  const statusLine = /状态\**\s*[:：]\s*\**\s*([^\n|]+)/.exec(doc.body);
  const raw = statusLine ? cleanInline(statusLine[1]) : "";
  const matched = matchDecisionStatus(raw);
  return {
    id: `ADR-${match[1]}`,
    title: doc.title.replace(/^ADR-?\d{4}\s*[·:：-]?\s*/i, ""),
    status: raw || "未标注",
    statusKey: matched ? matched.key : "unknown",
    summary: truncate(sectionText(doc.body, ["决策", "决定"]), 120),
    doc: doc.id
  };
}

function stageVersion(stage) {
  const text = cleanInline(stage);
  const version = /^V(\d+)/i.exec(text);
  if (/门禁/.test(text)) return { version: version ? `V${version[1]}` : null, gate: true };
  if (/^(后续|未来|远期)/.test(text)) return { version: "future", gate: false };
  return { version: version ? `V${version[1]}` : null, gate: false };
}

function versionFromHeader(cell) {
  const text = cleanInline(cell);
  const version = /^V(\d+)/i.exec(text);
  if (version) return `V${version[1]}`;
  if (/后续|未来|远期/.test(text)) return "future";
  return null;
}

function parseEvolution(doc) {
  const tables = extractTables(doc.body);
  const tracks = [];
  const overview = tables.find(table => /能力主线|能力/.test(table.header[0]) && table.header.slice(1).some(versionFromHeader));
  for (const table of tables) {
    if (!/^阶段/.test(table.header[0]) || table.header.length < 3 || !table.h2) continue;
    const col = (pattern, fallback) => {
      const index = table.header.findIndex((cell, i) => i > 0 && pattern.test(cell));
      return index >= 0 ? index : fallback;
    };
    const changeCol = col(/变化|增量|能力|说明|内容/, 1);
    const depCol = col(/依赖|边界/, 2);
    const doneCol = col(/完成|判定|验收/, 3);
    const sourceCol = col(/来源|状态/, 4);
    const title = table.h2.text.replace(/^\d+(\.\d+)*[.、\s]*/, "").trim();
    tracks.push({
      title,
      line: table.line,
      columns: table.header.length,
      stages: table.rows.map(row => {
        const parsed = stageVersion(row[0]);
        return {
          stage: cleanInline(row[0]),
          version: parsed.version,
          gate: parsed.gate,
          change: row[changeCol] || "",
          dependencies: row[depCol] || "",
          completion: row[doneCol] || "",
          source: row[sourceCol] || ""
        };
      })
    });
  }
  if (overview) {
    const versionCols = overview.header.map(versionFromHeader);
    for (const row of overview.rows) {
      const label = cleanInline(row[0]);
      if (!label) continue;
      let track = tracks
        .map(item => ({ item, score: similarity(item.title, label) }))
        .sort((a, b) => b.score - a.score)
        .find(entry => entry.score >= 0.5)?.item;
      if (!track) {
        track = { title: label, line: overview.line, columns: 0, stages: [] };
        tracks.push(track);
      }
      track.overview = label;
      track.overviewCells = {};
      versionCols.forEach((version, index) => {
        const cell = cleanInline(row[index] || "");
        if (version && index > 0 && cell && !/^[-–—]$/.test(cell)) track.overviewCells[version] = cell;
      });
    }
  }
  return {
    overviewVersions: overview ? [...new Set(overview.header.map(versionFromHeader).filter(Boolean))] : [],
    tracks: tracks.map((track, index) => ({
      id: `track-${index + 1}`,
      title: track.title,
      overview: track.overview || track.title,
      accent: TRACK_COLORS[index % TRACK_COLORS.length],
      line: track.line,
      columns: track.columns,
      stages: track.stages,
      overviewCells: track.overviewCells || {}
    }))
  };
}

function parseTrackIndex(doc) {
  const mapping = new Map();
  for (const table of extractTables(doc.body)) {
    const idCol = table.header.findIndex(cell => /编号/.test(cell));
    const trackCol = table.header.findIndex(cell => /主责|能力主线|主线/.test(cell));
    if (idCol < 0 || trackCol < 0) continue;
    for (const row of table.rows) {
      const id = /\b(V\d+-FR-\d{3})\b/.exec(cleanInline(row[idCol]));
      if (id && row[trackCol]) mapping.set(id[1], cleanInline(row[trackCol]));
    }
  }
  return mapping;
}

function bestTrack(tracks, label) {
  if (!label) return null;
  const ranked = tracks
    .map(track => ({ track, score: Math.max(similarity(track.title, label), similarity(track.overview, label)) }))
    .sort((a, b) => b.score - a.score);
  return ranked[0] && ranked[0].score >= 0.45 ? ranked[0].track : null;
}

function parseRoadmap(doc) {
  const rows = new Map();
  for (const table of extractTables(doc.body)) {
    if (!/版本/.test(table.header[0])) continue;
    const goalCol = table.header.findIndex(cell => /目标/.test(cell));
    const titleCol = table.header.findIndex(cell => /主题|名称/.test(cell));
    for (const row of table.rows) {
      const id = normVersion(cleanInline(row[0]).replace(/\s.*$/, ""));
      if (!id) continue;
      const fileName = (/V\d+-([^/`)\s]+)\.md/.exec(row.join(" ")) || [])[1] || "";
      const fileTitle = /^(版本总览|产品需求|实现状态|规划)$/.test(fileName) ? "" : fileName;
      rows.set(id, {
        title: titleCol >= 0 ? cleanInline(row[titleCol]) : fileTitle,
        goal: goalCol >= 0 ? cleanInline(row[goalCol]) : "",
        history: /历史|归档|已迁移|已前移/.test(row.join(" "))
      });
    }
  }
  for (const heading of doc.headings) {
    const match = /^(V\d+)\s*[:：·\s-]\s*(.+)$/.exec(heading.text);
    if (!match || heading.level < 2) continue;
    const entry = rows.get(match[1]) || { title: "", goal: "", history: false };
    if (!entry.title) entry.title = match[2].trim();
    rows.set(match[1], entry);
  }
  return rows;
}

function buildVersions(docs, roadmapRows, evolution, features) {
  const versions = new Map();
  const touch = (id, patch = {}) => {
    if (!normVersion(id)) return;
    const current = versions.get(id) || { id, label: id, title: "", goal: "", kind: "", doc: null, history: false };
    for (const [key, value] of Object.entries(patch)) {
      if (value && !current[key]) current[key] = value;
    }
    versions.set(id, current);
  };
  for (const doc of docs) {
    const match = /(?:^|\/)(V\d+)-([^/]+)\.md$/.exec(doc.path);
    if (!match || !/^0?2-|产品与版本|99-|历史归档/.test(doc.path)) continue;
    const id = normVersion(match[1]);
    const name = match[2];
    const generic = /^(版本总览|产品需求|实现状态|规划|文档变更记录|冻结决策)$/.test(name);
    const archived = /^99-|历史归档/.test(doc.path);
    if (doc.path.includes("/当前版本/")) touch(id, { kind: "current" });
    else if (doc.path.includes("/后续版本/")) touch(id, { kind: "planned", title: generic ? "" : name });
    else if (archived) {
      // Archived files only describe a version when that version has no live doc.
      if (!versions.has(id)) {
        touch(id, { history: true, title: generic ? "" : name });
        versions.get(id).doc = doc.id;
      }
      continue;
    }
    const entry = versions.get(id);
    if (!entry.doc || /版本总览|规划/.test(name)) entry.doc = doc.id;
  }
  for (const [id, row] of roadmapRows) {
    const known = versions.get(id);
    if (known && row.title) known.title = row.title;
    touch(id, { title: row.title, goal: row.goal, history: row.history });
  }
  for (const id of evolution.overviewVersions) if (id !== "future") touch(id);
  for (const track of evolution.tracks) for (const stage of track.stages) if (stage.version && stage.version !== "future") touch(stage.version);
  for (const feature of features) touch(feature.version);

  const list = [...versions.values()].sort((a, b) => versionNumber(a.id) - versionNumber(b.id));
  let current = list.find(item => item.kind === "current");
  if (!current) current = list.find(item => features.some(feature => feature.version === item.id)) || list[0];
  for (const item of list) {
    if (item === current) item.kind = "current";
    else if (item.history && item.kind !== "planned") item.kind = "history";
    else if (!item.kind || item.kind === "current") item.kind = versionNumber(item.id) < versionNumber(current.id) ? "released" : "planned";
    if (item.history && item.kind === "planned" && !item.goal) item.kind = "history";
    delete item.history;
  }
  return { list, current: current ? current.id : null };
}

export function buildModel(docsRoot, options = {}) {
  const config = options.config || {};
  const exclude = [...(config.exclude || []), ...(options.exclude || [])];
  const relPaths = listMarkdown(docsRoot, exclude);
  const docs = relPaths.map(rel => readDoc(docsRoot, rel));
  const byPath = new Map(docs.map(doc => [doc.path, doc]));
  const warnings = [];

  const features = [];
  for (const doc of docs.filter(item => item.kind === "feature")) {
    const feature = parseFeature(doc, byPath);
    if (feature) features.push(feature);
  }
  features.sort((a, b) => collator.compare(a.id, b.id));

  const status = new Map();
  for (const doc of docs.filter(item => item.kind === "status")) {
    for (const [key, value] of parseStatusDoc(doc)) status.set(key, value);
  }
  for (const feature of features) {
    const entry = status.get(feature.id) || status.get(`${feature.version}#${feature.number.padStart(3, "0")}`);
    feature.implementation = entry || {
      statusKey: "unknown",
      statusLabel: "未同步",
      backend: "",
      frontend: "",
      verification: "",
      gap: "",
      updatedAt: "",
      source: null
    };
    if (!entry && docs.some(item => item.kind === "status")) {
      warnings.push({ code: "ST001", message: `${feature.id} 未出现在实现状态表中`, doc: feature.doc });
    }
  }

  const decisions = [];
  for (const doc of docs.filter(item => item.kind === "decision-log")) decisions.push(...parseDecisionLog(doc));
  const adrs = docs.filter(item => item.kind === "adr").map(parseAdr).filter(Boolean);

  const evolutionDoc = docs.find(item => item.kind === "evolution");
  const evolution = evolutionDoc ? parseEvolution(evolutionDoc) : { overviewVersions: [], tracks: [] };
  if (evolutionDoc && !evolution.tracks.length) {
    warnings.push({ code: "EV001", message: "功能演进矩阵中没有识别到能力主线表格", doc: evolutionDoc.id });
  }

  const indexMapping = new Map();
  for (const doc of docs.filter(item => item.kind === "evolution-index")) {
    for (const [id, label] of parseTrackIndex(doc)) indexMapping.set(id, label);
  }
  const configTracks = config.featureTracks || {};
  for (const feature of features) {
    const hint = configTracks[feature.id] || feature.trackHint || indexMapping.get(feature.id) || "";
    const track = bestTrack(evolution.tracks, hint) || bestTrack(evolution.tracks, feature.domain);
    feature.track = track ? track.id : null;
    feature.trackName = track ? track.title : "";
    feature.trackSource = !track ? "" : hint ? "explicit" : "domain";
    if (evolution.tracks.length && !track) {
      warnings.push({ code: "EV002", message: `${feature.id} 未关联到任何能力主线`, doc: feature.doc });
    }
    delete feature.trackHint;
  }

  const roadmapDoc = docs.find(item => item.kind === "roadmap");
  const roadmapRows = roadmapDoc ? parseRoadmap(roadmapDoc) : new Map();
  const versions = buildVersions(docs, roadmapRows, evolution, features);

  const projectName =
    options.projectName ||
    (config.project && config.project.name) ||
    detectProjectName(docs) ||
    path.basename(path.resolve(docsRoot, ".."));

  const goals = config.docGoals || {};
  const featureByDoc = new Map(features.map(feature => [feature.doc, feature]));
  const viewDocs = docs.map(doc => {
    const color = groupColor(doc.group);
    const feature = featureByDoc.get(doc.id);
    return {
      id: doc.id,
      path: doc.path,
      title: doc.title,
      group: doc.group,
      groupLabel: doc.group.replace(/^\d+-/, ""),
      subgroup: doc.path.split("/").slice(1, -1).join(" / "),
      kind: doc.kind,
      version: doc.version,
      badge: feature ? feature.number : doc.version || (/^(\d+)-/.exec(doc.group) || [])[1] || "··",
      accent: color.accent,
      soft: color.soft,
      goal: goals[doc.path] || (feature && feature.goal) || firstParagraph(doc.source, 80),
      lines: doc.source.split(/\r?\n/).length,
      featureId: feature ? feature.id : null
    };
  });

  const keyDecisions = (config.keyDecisions || []).length
    ? config.keyDecisions.map(item => ({ id: item.id || "", title: item.text || item.title || "", summary: "", doc: null }))
    : decisions
        .filter(item => item.statusKey === "frozen" || item.statusKey === "unknown")
        .sort((a, b) => collator.compare(b.id, a.id))
        .slice(0, 8)
        .map(item => ({ id: item.id, title: item.title, summary: truncate(item.summary, 60), doc: item.doc }));

  return {
    docs,
    model: {
      project: {
        name: projectName,
        description: (config.project && config.project.description) || ""
      },
      docs: viewDocs,
      features,
      decisions,
      adrs,
      keyDecisions,
      versions: versions.list,
      currentVersion: versions.current,
      tracks: evolution.tracks,
      evolutionDoc: evolutionDoc ? evolutionDoc.id : null,
      statusDocs: docs.filter(item => item.kind === "status").map(item => item.id),
      warnings
    }
  };
}

function detectProjectName(docs) {
  const candidates = ["README.md", "01-项目概览/README.md"];
  for (const rel of candidates) {
    const doc = docs.find(item => item.path === rel);
    if (!doc) continue;
    const match = /^(.+?)\s*(文档中心|文档|项目概览|产品文档|项目)$/.exec(doc.title);
    if (match && match[1].length <= 24 && !/导航|概览/.test(match[1])) return match[1].trim();
  }
  for (const doc of docs.filter(item => item.kind === "evolution" || item.kind === "roadmap")) {
    const match = /^([A-Za-z][\w.-]{1,23})\s+/.exec(doc.title);
    if (match) return match[1];
  }
  return "";
}
