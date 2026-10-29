"use strict";
// spec-docs-pro document viewer. Reads only window.SPEC_DOCS_DATA; contains no project-specific data.

const DATA = window.SPEC_DOCS_DATA || null;
const VIEWS = ["overview", "reader", "features", "map", "evolution", "timeline"];
const STORAGE_PREFIX = `spec-docs:${DATA && DATA.project ? DATA.project.name : "default"}:`;
const KIND_LABELS = {
  current: "当前版本",
  planned: "规划",
  released: "已发布",
  history: "历史"
};
const STATUS_ORDER = ["planned", "in-progress", "local-verified", "pending-acceptance", "done", "blocked", "deprecated", "unknown"];

const state = {
  view: "overview",
  activeDocId: null,
  activeFeatureId: null,
  featureFilter: "",
  query: ""
};

const els = {};
let docIndex = new Map();
let featureIndex = new Map();
let trackIndex = new Map();
let versionIndex = new Map();

document.addEventListener("DOMContentLoaded", init);

function init() {
  for (const id of ["loadStatus", "docList", "content", "mainScroll", "outline", "outlineCount", "sideDecisions",
    "searchInput", "searchResults", "clearSearch", "readProgress", "sideQuality", "sideQualityPanel", "sideQualityCount"]) {
    els[id] = document.getElementById(id);
  }
  els.tabs = [...document.querySelectorAll(".segmented button")];

  if (!DATA || !Array.isArray(DATA.docs)) {
    els.loadStatus.textContent = "加载失败";
    els.content.innerHTML = panel("无法读取文档数据", `
      <p>没有找到 <code>window.SPEC_DOCS_DATA</code>。请确认数据文件与当前 HTML 位于同一目录，然后重新运行：</p>
      <pre><code>node &lt;skill&gt;/scripts/generate-browser.mjs --docs docs</code></pre>`);
    return;
  }

  docIndex = new Map(DATA.docs.map(doc => [doc.id, doc]));
  featureIndex = new Map((DATA.features || []).map(feature => [feature.id, feature]));
  trackIndex = new Map((DATA.tracks || []).map(track => [track.id, track]));
  versionIndex = new Map((DATA.versions || []).map(version => [version.id, version]));

  const counts = [`${DATA.docs.length} 份文档`];
  if (DATA.features && DATA.features.length) counts.push(`${DATA.features.length} 个功能`);
  if (location.protocol === "file:") counts.push("离线");
  els.loadStatus.textContent = counts.join(" · ");
  els.loadStatus.title = DATA.generatedAt ? `生成于 ${DATA.generatedAt}` : "";

  renderSideDecisions();
  renderSideQuality();
  bindEvents();
  renderDocList();

  const fromHash = parseHash();
  const saved = {
    view: readStore("view"),
    doc: readStore("doc"),
    feature: readStore("feature")
  };
  state.activeDocId = docIndex.has(fromHash.doc) ? fromHash.doc : docIndex.has(saved.doc) ? saved.doc : DATA.docs[0] && DATA.docs[0].id;
  state.activeFeatureId = featureIndex.has(fromHash.feature) ? fromHash.feature
    : featureIndex.has(saved.feature) ? saved.feature
    : DATA.features && DATA.features[0] ? DATA.features[0].id : null;
  setView(fromHash.view || saved.view || "overview", { replace: true });
}

function readStore(key) {
  try { return localStorage.getItem(STORAGE_PREFIX + key); } catch { return null; }
}

function writeStore(key, value) {
  try { localStorage.setItem(STORAGE_PREFIX + key, value); } catch { /* storage may be disabled for file:// */ }
}

function parseHash() {
  const params = new URLSearchParams(location.hash.replace(/^#/, ""));
  return { view: params.get("view"), doc: params.get("doc"), feature: params.get("feature") };
}

function updateHash(replace) {
  const params = new URLSearchParams({ view: state.view });
  if (state.view === "reader" && state.activeDocId) params.set("doc", state.activeDocId);
  if (state.view === "timeline" && state.activeFeatureId) params.set("feature", state.activeFeatureId);
  const next = `#${params.toString()}`;
  if (next === location.hash) return;
  if (replace) history.replaceState(null, "", next);
  else history.pushState(null, "", next);
}

function bindEvents() {
  els.tabs.forEach(button => button.addEventListener("click", () => setView(button.dataset.view)));

  els.searchInput.addEventListener("input", debounce(() => {
    state.query = els.searchInput.value.trim();
    els.clearSearch.style.display = state.query ? "block" : "none";
    renderSearch();
  }, 150));

  els.clearSearch.addEventListener("click", closeSearch);

  document.addEventListener("keydown", event => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      els.searchInput.focus();
      els.searchInput.select();
    }
    if (event.key === "Escape") els.searchResults.classList.remove("open");
  });

  document.addEventListener("click", event => {
    if (!els.searchResults.contains(event.target) && event.target !== els.searchInput) {
      els.searchResults.classList.remove("open");
    }
    const link = event.target.closest("[data-open-doc],[data-open-feature]");
    if (!link) return;
    event.preventDefault();
    if (link.dataset.openDoc) openDoc(link.dataset.openDoc, link.dataset.anchor);
    else openFeature(link.dataset.openFeature);
  });

  window.addEventListener("popstate", () => {
    const hash = parseHash();
    if (docIndex.has(hash.doc)) state.activeDocId = hash.doc;
    if (featureIndex.has(hash.feature)) state.activeFeatureId = hash.feature;
    setView(hash.view || "overview", { fromHistory: true });
  });

  els.mainScroll.addEventListener("scroll", () => {
    updateProgress();
    updateActiveOutline();
  });
}

function closeSearch() {
  els.searchInput.value = "";
  state.query = "";
  els.clearSearch.style.display = "none";
  els.searchResults.classList.remove("open");
}

function setView(view, options = {}) {
  if (!VIEWS.includes(view)) view = "overview";
  if (view === "timeline" && !(DATA.features || []).length) view = "features";
  state.view = view;
  writeStore("view", view);
  els.tabs.forEach(tab => {
    const active = tab.dataset.view === view;
    tab.classList.toggle("active", active);
    tab.setAttribute("aria-pressed", String(active));
  });

  if (view === "overview") renderOverview();
  if (view === "reader") renderReader();
  if (view === "features") renderFeatures();
  if (view === "map") renderMap();
  if (view === "evolution") renderEvolution();
  if (view === "timeline") renderTimeline();

  renderDocList();
  if (!options.fromHistory) updateHash(options.replace);
  if (!options.keepScroll) els.mainScroll.scrollTop = 0;
  updateProgress();
}

function openDoc(docId, anchor) {
  if (!docIndex.has(docId)) return;
  state.activeDocId = docId;
  writeStore("doc", docId);
  els.searchResults.classList.remove("open");
  setView("reader");
  if (anchor) {
    const target = findHeading(anchor);
    if (target) target.scrollIntoView({ block: "start" });
  }
}

function openFeature(featureId) {
  if (!featureIndex.has(featureId)) return;
  state.activeFeatureId = featureId;
  writeStore("feature", featureId);
  els.searchResults.classList.remove("open");
  setView("timeline");
}

// Accepts a GitHub-style slug, a heading id, or a stable ID prefix such as "D061" / "AC03".
function findHeading(anchor) {
  let decoded = anchor;
  try { decoded = decodeURIComponent(anchor); } catch { /* keep raw */ }
  const wanted = slug(decoded);
  const headings = [...document.querySelectorAll("#article h1,#article h2,#article h3,#article h4")];
  return headings.find(heading => heading.dataset.slug === wanted || heading.id === anchor)
    || headings.find(heading => heading.textContent.trim().toLowerCase().startsWith(`${decoded.toLowerCase()} `))
    || headings.find(heading => heading.dataset.slug.startsWith(wanted));
}

// ---------- shared rendering helpers ----------

function panel(title, body, tag = "") {
  return `
    <section class="panel">
      <div class="panel-head"><h2>${escapeHtml(title)}</h2>${tag ? `<span class="tag">${escapeHtml(tag)}</span>` : ""}</div>
      <div class="panel-body">${body}</div>
    </section>`;
}

function statBlock(label, value, caption) {
  return `
    <div class="stat">
      <span>${escapeHtml(label)}</span>
      <strong>${escapeHtml(String(value))}</strong>
      <span>${escapeHtml(caption)}</span>
    </div>`;
}

function statusPill(implementation) {
  const info = implementation || { statusKey: "unknown", statusLabel: "未同步" };
  return `<span class="status-pill ${escapeHtml(info.statusKey)}">${escapeHtml(info.statusLabel)}</span>`;
}

function docLink(docId, label, anchor = "") {
  if (!docIndex.has(docId)) return escapeHtml(label);
  return `<a href="#view=reader&doc=${encodeURIComponent(docId)}" data-open-doc="${escapeHtml(docId)}"${anchor ? ` data-anchor="${escapeHtml(anchor)}"` : ""}>${escapeHtml(label)}</a>`;
}

function featureLink(featureId, label) {
  return `<a href="#view=timeline&feature=${encodeURIComponent(featureId)}" data-open-feature="${escapeHtml(featureId)}">${escapeHtml(label || featureId)}</a>`;
}

function renderSideDecisions() {
  const items = DATA.keyDecisions || [];
  els.sideDecisions.innerHTML = items.length
    ? items.map(item => `<li>
        ${item.id ? `<strong>${escapeHtml(item.id)}</strong> ` : ""}${item.doc ? docLink(item.doc, item.title, item.id) : escapeHtml(item.title)}
        ${item.summary ? `<small>${escapeHtml(item.summary)}</small>` : ""}
      </li>`).join("")
    : `<li class="muted">未发现冻结决策。在 <code>06-决策记录/V1-冻结决策.md</code> 中按 <code>## D001 标题</code> 记录。</li>`;
}

function renderSideQuality() {
  const warnings = DATA.warnings || [];
  if (!warnings.length) return;
  els.sideQualityPanel.hidden = false;
  els.sideQualityCount.textContent = String(warnings.length);
  els.sideQuality.innerHTML = `<ul class="quality-list">${warnings.slice(0, 12).map(item => `
    <li><code>${escapeHtml(item.code)}</code> ${item.doc ? docLink(item.doc, item.message) : escapeHtml(item.message)}</li>`).join("")}</ul>
    ${warnings.length > 12 ? `<p class="muted">另有 ${warnings.length - 12} 条，运行 <code>check-docs.mjs</code> 查看完整列表。</p>` : ""}`;
}

function renderDocList() {
  const groups = new Map();
  for (const doc of DATA.docs) {
    if (!groups.has(doc.group)) groups.set(doc.group, []);
    groups.get(doc.group).push(doc);
  }
  const html = [];
  for (const [group, docs] of groups) {
    const open = docs.some(doc => doc.id === state.activeDocId) || groups.size <= 3;
    html.push(`<details class="doc-group" ${open ? "open" : ""}>
      <summary><span>${escapeHtml(docs[0].groupLabel || group)}</span><span class="tag">${docs.length}</span></summary>
      ${docs.map(doc => `
        <button type="button" class="doc-item ${doc.id === state.activeDocId && state.view === "reader" ? "active" : ""}" data-open-doc="${escapeHtml(doc.id)}"
          style="--accent:${safeColor(doc.accent)};--accent-soft:${safeColor(doc.soft)}" title="${escapeHtml(doc.path)}">
          <span class="doc-badge">${escapeHtml(doc.badge || "··")}</span>
          <span class="doc-name">
            <strong>${escapeHtml(doc.title)}</strong>
            <span>${escapeHtml(doc.subgroup || doc.groupLabel)} · ${doc.lines} 行</span>
          </span>
        </button>`).join("")}
    </details>`);
  }
  els.docList.innerHTML = html.join("");
}

function renderOutline(items) {
  els.outlineCount.textContent = String(items.length);
  els.outline.innerHTML = items.slice(0, 120).map(item => `
    <a href="#${escapeHtml(item.id)}" class="lv${item.level}" data-target="${escapeHtml(item.id)}">${escapeHtml(item.text)}</a>`).join("");
  els.outline.querySelectorAll("a").forEach(link => {
    link.addEventListener("click", event => {
      event.preventDefault();
      const target = document.getElementById(link.dataset.target);
      if (target) target.scrollIntoView({ block: "start" });
    });
  });
}

function updateProgress() {
  const max = els.mainScroll.scrollHeight - els.mainScroll.clientHeight;
  const value = max > 0 ? Math.min(100, Math.max(0, (els.mainScroll.scrollTop / max) * 100)) : 0;
  els.readProgress.style.width = `${value}%`;
}

function updateActiveOutline() {
  const headings = [...els.content.querySelectorAll("h1[id],h2[id],h3[id],h4[id]")];
  let active = "";
  for (const heading of headings) {
    if (heading.getBoundingClientRect().top < 110) active = heading.id;
  }
  els.outline.querySelectorAll("a").forEach(link => link.classList.toggle("active", link.dataset.target === active));
}

function versionsOrdered() {
  return DATA.versions || [];
}

function currentVersion() {
  return versionIndex.get(DATA.currentVersion) || versionsOrdered()[0] || null;
}

function countBy(list, key) {
  return list.reduce((acc, item) => {
    const value = typeof key === "function" ? key(item) : item[key];
    acc[value] = (acc[value] || 0) + 1;
    return acc;
  }, {});
}

function safeColor(value) {
  return /^#[0-9a-f]{3,8}$/i.test(String(value || "")) ? value : "#69707a";
}

function escapeHtml(value) {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

function escapeRegExp(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function slug(text) {
  return String(text || "")
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/[`*_~]/g, "")
    .replace(/[^\p{Letter}\p{Number}\s-]/gu, "")
    .trim()
    .replace(/\s+/g, "-");
}

function debounce(fn, delay) {
  let timer = null;
  return (...args) => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => fn(...args), delay);
  };
}
