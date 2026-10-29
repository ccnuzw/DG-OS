// ---------- views ----------

function renderOverview() {
  const features = DATA.features || [];
  const current = currentVersion();
  const statusCounts = countBy(features, item => (item.implementation || {}).statusKey || "unknown");
  const done = statusCounts.done || 0;
  const frozen = (DATA.decisions || []).filter(item => item.statusKey === "frozen").length;
  const versions = versionsOrdered();

  els.content.innerHTML = `
    <section class="doc-header">
      <div class="doc-kicker"><span class="tag">总览</span><span>${escapeHtml(DATA.project.name)}</span>${current ? `<span>当前版本 ${escapeHtml(current.id)}</span>` : ""}</div>
      <h2>${escapeHtml(DATA.project.name)} 文档总览</h2>
      ${DATA.project.description ? `<p class="muted">${escapeHtml(DATA.project.description)}</p>` : ""}
    </section>

    <section class="stats">
      ${statBlock("文档", DATA.docs.length, "Markdown")}
      ${statBlock("功能", features.length, current ? `${current.id} 等版本` : "功能主文档")}
      ${statBlock("已完成", features.length ? `${done}/${features.length}` : "—", "以实现状态表为准")}
      ${statBlock("冻结决策", frozen || (DATA.decisions || []).length, `${(DATA.adrs || []).length} 份 ADR`)}
    </section>

    ${versions.length ? panel("版本链路", `
      <div class="version-strip">
        ${versions.map(version => `
          <button type="button" class="version-node ${escapeHtml(version.kind)}" ${version.doc ? `data-open-doc="${escapeHtml(version.doc)}"` : "disabled"}>
            <strong>${escapeHtml(version.id)}${version.title ? ` · ${escapeHtml(version.title)}` : ""}</strong>
            <span class="tag">${escapeHtml(KIND_LABELS[version.kind] || version.kind)}</span>
            <span>${escapeHtml(version.goal || "（版本路线图未写目标）")}</span>
          </button>`).join("")}
      </div>`, `${versions.length} 个版本`) : ""}

    ${features.length ? panel("实现状态分布", statusBars(statusCounts, features.length), "来源：实现状态表") : ""}

    ${panel("文档结构", groupTable(), `${new Set(DATA.docs.map(doc => doc.group)).size} 个分区`)}
  `;
  renderOutline([]);
}

function statusBars(counts, total) {
  const labels = {
    planned: "规划中", "in-progress": "开发中", "local-verified": "本地验证", "pending-acceptance": "待验收",
    done: "已完成", blocked: "阻塞", deprecated: "已废弃", unknown: "未同步"
  };
  return `<div class="status-bars">${STATUS_ORDER.filter(key => counts[key]).map(key => `
    <div class="status-bar">
      <span class="status-pill ${key}">${labels[key]}</span>
      <span class="bar"><i style="width:${((counts[key] / total) * 100).toFixed(1)}%"></i></span>
      <strong>${counts[key]}</strong>
    </div>`).join("")}</div>
    ${counts.unknown ? `<p class="muted">“未同步”表示该功能没有出现在版本实现状态表中。状态只在实现状态表维护，功能主文档不重复记录。</p>` : ""}`;
}

function groupTable() {
  const groups = new Map();
  for (const doc of DATA.docs) {
    const entry = groups.get(doc.group) || { label: doc.groupLabel, count: 0, lines: 0, first: doc.id };
    entry.count += 1;
    entry.lines += doc.lines;
    groups.set(doc.group, entry);
  }
  return `<table class="matrix"><thead><tr><th>分区</th><th>文档</th><th>行数</th></tr></thead><tbody>
    ${[...groups].map(([group, entry]) => `<tr><td>${docLink(entry.first, group)}</td><td>${entry.count}</td><td>${formatNumber(entry.lines)}</td></tr>`).join("")}
  </tbody></table>`;
}

function renderReader() {
  const doc = docIndex.get(state.activeDocId) || DATA.docs[0];
  if (!doc) {
    els.content.innerHTML = panel("没有文档", "<p>docs/ 目录下没有 Markdown 文件。</p>");
    return;
  }
  state.activeDocId = doc.id;
  const source = (DATA.content || {})[doc.id];
  if (typeof source !== "string") {
    els.content.innerHTML = panel("缺少文档内容", `<p>数据文件中没有 <code>${escapeHtml(doc.path)}</code> 的内容，请重新生成。</p>`);
    renderOutline([]);
    return;
  }
  const rendered = renderMarkdown(stripFrontmatter(source), doc.path);
  const feature = doc.featureId ? featureIndex.get(doc.featureId) : null;

  els.content.innerHTML = `
    <section class="doc-header" style="--accent:${safeColor(doc.accent)}">
      <div class="doc-kicker">
        <span class="tag">${escapeHtml(doc.groupLabel)}</span>
        ${doc.version ? `<span class="tag">${escapeHtml(doc.version)}</span>` : ""}
        <code>${escapeHtml(doc.path)}</code>
      </div>
      <h2>${escapeHtml(doc.title)}</h2>
      <div class="doc-meta">
        <span class="tag">${doc.lines} 行</span>
        <span class="tag">${rendered.outline.length} 章节</span>
        ${feature ? `<span class="tag">${escapeHtml(feature.id)}</span>${statusPill(feature.implementation)}${featureLink(feature.id, "查看时间线")}` : ""}
      </div>
      ${doc.goal ? `<p class="muted">${escapeHtml(doc.goal)}</p>` : ""}
    </section>
    <article class="article" id="article">${rendered.html}</article>`;
  renderOutline(rendered.outline);
}

function renderFeatures() {
  const features = DATA.features || [];
  if (!features.length) {
    els.content.innerHTML = panel("还没有功能主文档", `
      <p>生成器会把 <code>03-功能规格/V{n}/{领域}/NN-功能名.md</code> 识别为功能主文档。
      建议在 frontmatter 中写 <code>feature_id: V1-FR-001</code>。</p>`);
    renderOutline([]);
    return;
  }
  const query = state.featureFilter.toLowerCase();
  const rows = features.filter(item => !query || [item.id, item.title, item.domain, item.trackName, item.goal,
    (item.implementation || {}).statusLabel].some(value => String(value || "").toLowerCase().includes(query)));
  const domains = [...new Set(features.map(item => item.domain || "未分组"))];

  els.content.innerHTML = `
    <section class="doc-header">
      <div class="doc-kicker"><span class="tag">功能</span><span>${features.length} 个功能 · ${domains.length} 个领域</span></div>
      <h2>功能清单</h2>
      <div class="feature-controls">
        <div class="feature-field">
          <label for="featureFilter">筛选（编号 / 名称 / 领域 / 状态）</label>
          <input id="featureFilter" value="${escapeHtml(state.featureFilter)}" placeholder="例如 V1-FR-005、支付、开发中">
        </div>
      </div>
    </section>
    <div class="table-wrap feature-table"><table>
      <thead><tr><th>编号</th><th>功能</th><th>领域</th><th>风险</th><th>规格</th><th>实现状态</th><th>AC</th><th>主责能力</th></tr></thead>
      <tbody>${rows.map(item => `<tr>
        <td><code>${escapeHtml(item.id)}</code></td>
        <td>${docLink(item.doc, item.title)}${item.design ? ` · ${docLink(item.design, "技术设计")}` : ""}</td>
        <td>${escapeHtml(item.domain || "—")}</td>
        <td>${item.risk === "high" ? `<span class="risk high">高风险</span>` : `<span class="risk">标准</span>`}</td>
        <td>${escapeHtml(item.specStatus || "—")}</td>
        <td>${statusPill(item.implementation)}</td>
        <td>${item.acCount || 0}</td>
        <td>${item.trackName ? featureLink(item.id, item.trackName.replace(/演进$/, "")) : "—"}</td>
      </tr>`).join("") || `<tr><td colspan="8" class="empty">没有匹配项</td></tr>`}</tbody>
    </table></div>`;

  const input = document.getElementById("featureFilter");
  input.addEventListener("input", debounce(() => {
    state.featureFilter = input.value.trim();
    const caret = input.selectionStart;
    renderFeatures();
    const next = document.getElementById("featureFilter");
    next.focus();
    next.setSelectionRange(caret, caret);
  }, 200));
  renderOutline([]);
}

function renderEvolution() {
  const doc = DATA.evolutionDoc ? docIndex.get(DATA.evolutionDoc) : null;
  const tracks = DATA.tracks || [];
  if (!doc) {
    els.content.innerHTML = panel("没有功能演进矩阵", `
      <p>在 <code>02-产品与版本/功能演进矩阵.md</code> 中按能力主线编写表格后，这里会显示跨版本演进。
      格式见 skill 的 <code>assets/templates/feature-evolution-matrix.md</code>。</p>`);
    renderOutline([]);
    return;
  }
  const versions = versionsOrdered();
  const columns = [...versions.map(version => version.id), "future"];
  const cell = (track, version) => {
    const stages = stagesFor(track, version);
    if (stages.length) return stages.map(stage => `<div class="cell-stage ${stage.gate ? "gate" : ""}">${escapeHtml(stage.stage)}</div>`).join("");
    const overview = track.overviewCells && track.overviewCells[version];
    return overview ? `<div class="cell-overview">${escapeHtml(overview)}</div>` : `<span class="muted">—</span>`;
  };
  const rendered = renderMarkdown(stripFrontmatter((DATA.content || {})[doc.id] || ""), doc.path);

  els.content.innerHTML = `
    <section class="doc-header">
      <div class="doc-kicker"><span class="tag">演进</span><span>${tracks.length} 条能力主线 · ${versions.length} 个版本</span></div>
      <h2>${escapeHtml(doc.title)}</h2>
      <div class="doc-meta">${docLink(doc.id, "打开原文")}</div>
    </section>
    ${tracks.length ? `<div class="table-wrap evolution-grid"><table>
      <thead><tr><th>能力主线</th>${columns.map(id => `<th>${escapeHtml(id === "future" ? "后续" : id)}${versionIndex.get(id) ? `<br><small>${escapeHtml(KIND_LABELS[versionIndex.get(id).kind] || "")}</small>` : ""}</th>`).join("")}</tr></thead>
      <tbody>${tracks.map(track => `<tr>
        <th style="border-left:4px solid ${safeColor(track.accent)}">${escapeHtml(track.title)}</th>
        ${columns.map(id => `<td>${cell(track, id)}</td>`).join("")}
      </tr>`).join("")}</tbody>
    </table></div>` : panel("未识别到能力主线表格", `<p>每条能力主线需要一个 <code>## 能力名称演进</code> 章节和一张以“阶段”开头的表格。</p>`)}
    <article class="article" id="article">${rendered.html}</article>`;
  renderOutline(rendered.outline);
}

function stagesFor(track, versionId) {
  return (track.stages || []).filter(stage => stage.version === versionId);
}

function renderTimeline() {
  const features = DATA.features || [];
  const feature = featureIndex.get(state.activeFeatureId) || features[0];
  if (!feature) return renderFeatures();
  state.activeFeatureId = feature.id;
  const track = feature.track ? trackIndex.get(feature.track) : null;
  const versions = versionsOrdered();
  const impl = feature.implementation || {};
  const statusDoc = impl.source || (DATA.statusDocs || [])[0];

  const milestones = versions.map(version => milestone(feature, track, version));
  const futureStages = track ? stagesFor(track, "future") : [];
  if (futureStages.length || (track && track.overviewCells && track.overviewCells.future)) {
    milestones.push(milestone(feature, track, { id: "future", label: "后续", kind: "planned", title: "后续独立增强" }));
  }

  els.content.innerHTML = `
    <section class="feature-query" id="feature-picker">
      <h2>功能时间线</h2>
      <p>当前版本节点来自功能主文档和实现状态表；其他版本节点来自功能演进矩阵中主责能力的阶段，表示“关联能力规划”，不代表该功能已获得后续版本编号或开发承诺。</p>
      <div class="feature-controls">
        <div class="feature-field">
          <label for="featureSelect">选择功能（${features.length} 个）</label>
          <select id="featureSelect">${features.map(item => `<option value="${escapeHtml(item.id)}" ${item.id === feature.id ? "selected" : ""}>${escapeHtml(item.id)} · ${escapeHtml(item.title)}</option>`).join("")}</select>
        </div>
      </div>
    </section>

    <section class="feature-summary" id="feature-summary" style="border-left-color:${safeColor(track ? track.accent : "")}">
      <div>
        <div class="feature-summary-meta">
          <span class="tag">${escapeHtml(feature.id)}</span>
          ${statusPill(impl)}
          <span class="tag">${escapeHtml(feature.domain || "未标注领域")}</span>
          ${feature.risk === "high" ? `<span class="risk high">高风险</span>` : ""}
          <span class="tag">主责：${escapeHtml(track ? track.title : "未关联能力主线")}</span>
        </div>
        <h2>${escapeHtml(feature.title)}</h2>
        <p>${escapeHtml(feature.goal || "功能目标以主文档为准。")}</p>
      </div>
      <div class="feature-summary-links">
        ${docLink(feature.doc, "打开功能主文档")}
        ${feature.design ? docLink(feature.design, "打开技术设计") : ""}
        ${statusDoc ? docLink(statusDoc, "查看实现状态") : ""}
      </div>
    </section>

    <section class="feature-timeline-panel" id="feature-timeline">
      <div class="feature-timeline-head">
        <div class="feature-timeline-title">
          <h3>${escapeHtml(feature.id)} · 版本时间线</h3>
          <p>${track ? (feature.trackSource === "domain" ? "主责能力按领域名称推断，可在功能主文档 frontmatter 中用 track 字段指定。" : "主责能力来自显式映射。") : "该功能没有关联能力主线，只显示当前版本节点。"}</p>
        </div>
        <div class="timeline-legend" aria-label="图例">
          <span class="current"><i></i>当前版本</span>
          <span class="gate"><i></i>发布门禁</span>
          <span><i></i>规划</span>
          <span class="history"><i></i>历史或无规划</span>
        </div>
      </div>
      <div class="feature-timeline-scroll"><div class="feature-version-timeline">${milestones.join("")}</div></div>
    </section>`;

  document.getElementById("featureSelect").addEventListener("change", event => openFeature(event.target.value));
  renderOutline([
    { id: "feature-picker", text: "选择功能", level: 2 },
    { id: "feature-summary", text: `${feature.id} ${feature.title}`, level: 2 },
    { id: "feature-timeline", text: "版本时间线", level: 2 }
  ]);
}

function milestone(feature, track, version) {
  const axis = (label, caption) => `
    <div class="milestone-axis"><span class="milestone-dot" aria-hidden="true"></span><strong>${escapeHtml(label)}</strong><small>${escapeHtml(caption)}</small></div>`;
  const mobile = (label, caption) => `<div class="milestone-mobile-meta"><strong>${escapeHtml(label)}</strong><span>${escapeHtml(caption)}</span></div>`;
  const stages = track ? stagesFor(track, version.id) : [];
  const ownVersion = version.id === feature.version;

  if (ownVersion) {
    const impl = feature.implementation || {};
    const gates = stages.filter(stage => stage.gate);
    return `
      <article class="feature-milestone current">
        ${axis(version.id, "本功能所在版本")}
        <div class="milestone-card">
          <div class="milestone-state-line">${mobile(version.id, "本功能所在版本")}<span class="milestone-state current">${escapeHtml(KIND_LABELS[version.kind] || "版本")}</span>${statusPill(impl)}</div>
          <h4>${escapeHtml(feature.title)}</h4>
          <p>${escapeHtml(feature.goal || "功能行为以主文档为准。")}</p>
          <div class="milestone-facts">
            <div class="milestone-fact"><span>后端</span><strong>${escapeHtml(impl.backend || "—")}</strong></div>
            <div class="milestone-fact"><span>前端</span><strong>${escapeHtml(impl.frontend || "—")}</strong></div>
            <div class="milestone-fact"><span>验证</span><strong>${escapeHtml(impl.verification || "—")}</strong></div>
          </div>
          ${impl.gap ? `<div class="milestone-detail"><strong>当前差距</strong><span>${escapeHtml(impl.gap)}</span></div>` : ""}
          ${feature.dependencies ? `<div class="milestone-detail"><strong>上游依赖</strong><span>${escapeHtml(feature.dependencies)}</span></div>` : ""}
          ${gates.map(stage => `<div class="milestone-detail"><strong>${escapeHtml(stage.stage)}</strong><span>${inline(stage.change, DATA.evolutionPath)}</span></div>`).join("")}
          <div class="milestone-source">状态更新：${escapeHtml(impl.updatedAt || "未标注")}${impl.statusKey === "unknown" ? " · 实现状态表中没有该功能" : ""}</div>
        </div>
      </article>`;
  }

  if (version.kind === "history" && !stages.length) {
    return `
      <article class="feature-milestone history">
        ${axis(version.id, "历史")}
        <div class="milestone-card">
          <div class="milestone-state-line">${mobile(version.id, "历史")}<span class="milestone-state history">历史范围</span></div>
          <h4>${escapeHtml(version.title || "不参与当前规划")}</h4>
          <p>${escapeHtml(version.goal || "该版本已归档，不参与当前规划。")}</p>
        </div>
      </article>`;
  }

  if (!stages.length) {
    const overview = track && track.overviewCells ? track.overviewCells[version.id] : "";
    const caption = overview ? "总览关联" : "无单独规划";
    return `
      <article class="feature-milestone ${overview ? "planned" : "empty"}">
        ${axis(version.label || version.id, caption)}
        <div class="milestone-card">
          <div class="milestone-state-line">${mobile(version.label || version.id, caption)}<span class="milestone-state ${overview ? "" : "empty"}">${overview ? "总览关联" : "无阶段"}</span></div>
          <h4>${escapeHtml(overview ? `${version.label || version.id} 总览` : "该能力在此版本没有阶段变化")}</h4>
          <p>${escapeHtml(overview || "空白不代表取消；调整规划时先更新版本路线图和功能演进矩阵。")}</p>
        </div>
      </article>`;
  }

  const main = stages.find(stage => !stage.gate) || stages[0];
  const caption = main.gate ? "发布门禁" : "关联能力规划";
  return `
    <article class="feature-milestone ${main.gate ? "gate" : "planned"}">
      ${axis(version.label || version.id, caption)}
      <div class="milestone-card">
        <div class="milestone-state-line">${mobile(version.label || version.id, caption)}<span class="milestone-state ${main.gate ? "gate" : ""}">${escapeHtml(caption)}</span></div>
        <h4>${escapeHtml(main.stage)}</h4>
        <p>${inline(main.change, DATA.evolutionPath)}</p>
        ${main.dependencies ? `<div class="milestone-detail"><strong>依赖与边界</strong><span>${inline(main.dependencies, DATA.evolutionPath)}</span></div>` : ""}
        ${main.completion ? `<div class="milestone-detail"><strong>完成判定</strong><span>${inline(main.completion, DATA.evolutionPath)}</span></div>` : ""}
        ${main.source ? `<div class="milestone-source">${inline(main.source, DATA.evolutionPath)}</div>` : ""}
        ${stages.length > 1 ? `<div class="milestone-source">同版本另有 ${stages.length - 1} 个阶段，详见功能演进矩阵。</div>` : ""}
      </div>
    </article>`;
}

function renderSearch() {
  if (!state.query) {
    els.searchResults.classList.remove("open");
    els.searchResults.innerHTML = "";
    return;
  }
  const query = state.query.toLowerCase();
  const featureHits = (DATA.features || []).filter(item => [item.id, item.title, item.domain, item.goal]
    .some(value => String(value || "").toLowerCase().includes(query))).slice(0, 12);
  const decisionHits = (DATA.decisions || []).filter(item => `${item.id} ${item.title} ${item.summary}`.toLowerCase().includes(query)).slice(0, 8);
  const lineHits = [];
  for (const doc of DATA.docs) {
    const source = (DATA.content || {})[doc.id] || "";
    const lines = source.split(/\r?\n/);
    for (let index = 0; index < lines.length && lineHits.length < 80; index += 1) {
      if (lines[index].toLowerCase().includes(query)) lineHits.push({ doc, line: lines[index].trim(), lineNo: index + 1 });
    }
  }
  const item = (attrs, top, right, body) => `
    <button type="button" class="result-item" ${attrs}>
      <span class="result-top"><span>${top}</span><span>${right}</span></span>
      <span class="result-line">${body}</span>
    </button>`;
  const html = [
    ...featureHits.map(feature => item(`data-open-feature="${escapeHtml(feature.id)}"`, `${escapeHtml(feature.id)} · 功能`, escapeHtml((feature.implementation || {}).statusLabel || ""), highlight(`${feature.title} · ${feature.goal || ""}`, state.query))),
    ...decisionHits.map(decision => item(`data-open-doc="${escapeHtml(decision.doc)}" data-anchor="${escapeHtml(decision.id)}"`, `${escapeHtml(decision.id)} · 决策`, escapeHtml(decision.status), highlight(`${decision.title} ${decision.summary || ""}`, state.query))),
    ...lineHits.map(hit => item(`data-open-doc="${escapeHtml(hit.doc.id)}"`, escapeHtml(hit.doc.title), `第 ${hit.lineNo} 行`, highlight(hit.line || "(空行)", state.query)))
  ];
  els.searchResults.innerHTML = html.length ? html.join("") : `<div class="empty">没有匹配项</div>`;
  els.searchResults.classList.add("open");
}

function renderMap() {
  const sections = [
    diagram("版本路线", "按版本顺序展示目标和状态；来源：版本路线图与版本文档。", versionRoadmapSvg()),
    diagram("能力 × 版本热力图", "每格表示该能力主线在该版本是否有阶段变化；来源：功能演进矩阵。", heatmapSvg()),
    diagram("领域与实现状态", "按功能领域统计实现状态；来源：功能主文档与实现状态表。", domainStatusSvg()),
    diagram("风险分布", "高风险功能需要独立技术设计；来源：功能主文档与技术设计文件。", riskSvg())
  ].filter(Boolean);
  els.content.innerHTML = `
    <section class="doc-header">
      <div class="doc-kicker"><span class="tag">图谱</span><span>全部由文档数据生成</span></div>
      <h2>${escapeHtml(DATA.project.name)} 结构图谱</h2>
    </section>
    <section class="map-stack">${sections.join("") || panel("暂无可视化数据", "<p>补充版本路线图、功能演进矩阵或功能主文档后再生成。</p>")}</section>`;
  renderOutline([]);
}

function diagram(title, caption, svg) {
  if (!svg) return "";
  return `
    <section class="diagram">
      <div class="diagram-head"><h3>${escapeHtml(title)}</h3></div>
      <div class="diagram-body">${svg}</div>
      <div class="diagram-caption">${escapeHtml(caption)}</div>
    </section>`;
}

function svgText(x, y, text, attrs = "") {
  return `<text x="${x}" y="${y}" ${attrs}>${escapeHtml(text)}</text>`;
}

function clip(text, max) {
  const value = String(text || "");
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

function versionRoadmapSvg() {
  const versions = versionsOrdered();
  if (!versions.length) return "";
  const colors = { current: "#2f7d4f", planned: "#4056a1", released: "#087d75", history: "#8b929a" };
  const perRow = 4;
  const width = 960;
  const cardW = 210;
  const gap = (width - 72 - perRow * cardW) / (perRow - 1);
  const rows = Math.ceil(versions.length / perRow);
  const height = 70 + rows * 120;
  const cards = versions.map((version, index) => {
    const x = 36 + (index % perRow) * (cardW + gap);
    const y = 56 + Math.floor(index / perRow) * 120;
    const color = colors[version.kind] || colors.planned;
    const dashed = version.kind === "history" ? ` stroke-dasharray="5 5"` : "";
    return `<g>
      <rect x="${x}" y="${y}" width="${cardW}" height="92" rx="8" fill="#ffffff" stroke="${color}"${dashed}></rect>
      <rect x="${x}" y="${y}" width="6" height="92" rx="3" fill="${color}"></rect>
      ${svgText(x + 18, y + 26, `${version.id}${version.title ? ` · ${clip(version.title, 10)}` : ""}`, `font-size="14" font-weight="700" fill="#1d1f22"`)}
      ${svgText(x + 18, y + 46, KIND_LABELS[version.kind] || version.kind, `font-size="11" fill="${color}"`)}
      ${svgText(x + 18, y + 68, clip(version.goal || "", 15), `font-size="11" fill="#69707a"`)}
      ${svgText(x + 18, y + 84, clip((version.goal || "").slice(15), 15), `font-size="11" fill="#69707a"`)}
    </g>`;
  }).join("");
  return `<svg viewBox="0 0 ${width} ${height}" role="img" aria-label="版本路线">
    <rect width="${width}" height="${height}" rx="8" fill="#fbfaf7"></rect>
    ${svgText(36, 34, `${versions.length} 个版本`, `font-size="16" font-weight="700" fill="#1d1f22"`)}
    ${cards}
  </svg>`;
}

function heatmapSvg() {
  const tracks = DATA.tracks || [];
  const versions = versionsOrdered();
  if (!tracks.length || !versions.length) return "";
  const columns = [...versions.map(version => version.id), "future"];
  const labelW = 230;
  const cellW = Math.max(56, Math.floor((960 - labelW - 48) / columns.length));
  const cellH = 34;
  const width = labelW + cellW * columns.length + 48;
  const height = 70 + tracks.length * (cellH + 6);
  const header = columns.map((id, index) => svgText(labelW + index * cellW + cellW / 2, 52, id === "future" ? "后续" : id, `font-size="12" text-anchor="middle" fill="#69707a"`)).join("");
  const rows = tracks.map((track, row) => {
    const y = 62 + row * (cellH + 6);
    const cells = columns.map((id, index) => {
      const stages = stagesFor(track, id);
      const overview = track.overviewCells && track.overviewCells[id];
      const fill = stages.some(stage => stage.gate) ? "#f6ead6" : stages.length ? "#dcefeb" : overview ? "#eef1f8" : "#ffffff";
      const stroke = stages.length ? safeColor(track.accent) : "#dedbd2";
      const label = stages.length ? String(stages.length) : overview ? "·" : "";
      const x = labelW + index * cellW;
      return `<rect x="${x + 3}" y="${y}" width="${cellW - 6}" height="${cellH}" rx="6" fill="${fill}" stroke="${stroke}"></rect>
        ${label ? svgText(x + cellW / 2, y + 22, label, `font-size="12" text-anchor="middle" fill="#1d1f22"`) : ""}`;
    }).join("");
    return `${svgText(36, y + 22, clip(track.title.replace(/演进$/, ""), 14), `font-size="12" fill="#1d1f22"`)}${cells}`;
  }).join("");
  return `<svg viewBox="0 0 ${width} ${height}" role="img" aria-label="能力版本热力图">
    <rect width="${width}" height="${height}" rx="8" fill="#fbfaf7"></rect>
    ${svgText(36, 30, "数字 = 阶段数；· = 仅在总览中提及；浅橙 = 含发布门禁", `font-size="12" fill="#69707a"`)}
    ${header}${rows}
  </svg>`;
}

function domainStatusSvg() {
  const features = DATA.features || [];
  if (!features.length) return "";
  const colors = {
    planned: "#c6c1b5", "in-progress": "#a96810", "local-verified": "#087d75", "pending-acceptance": "#4056a1",
    done: "#2f7d4f", blocked: "#aa2d4d", deprecated: "#8b929a", unknown: "#e5e1d8"
  };
  const domains = [...new Set(features.map(item => item.domain || "未分组"))];
  const barW = 600;
  const height = 70 + domains.length * 38 + 30;
  const rows = domains.map((domain, row) => {
    const list = features.filter(item => (item.domain || "未分组") === domain);
    const counts = countBy(list, item => (item.implementation || {}).statusKey || "unknown");
    let x = 260;
    const y = 56 + row * 38;
    const segments = STATUS_ORDER.filter(key => counts[key]).map(key => {
      const w = (counts[key] / list.length) * barW;
      const rect = `<rect x="${x}" y="${y}" width="${Math.max(w, 2)}" height="24" fill="${colors[key]}"><title>${escapeHtml(key)}: ${counts[key]}</title></rect>`;
      x += w;
      return rect;
    }).join("");
    return `${svgText(36, y + 17, `${clip(domain, 12)}（${list.length}）`, `font-size="12" fill="#1d1f22"`)}${segments}`;
  }).join("");
  const legendLabels = { planned: "规划中", "in-progress": "开发中", "local-verified": "本地验证", "pending-acceptance": "待验收", done: "已完成", blocked: "阻塞", deprecated: "已废弃", unknown: "未同步" };
  const legend = STATUS_ORDER.map((key, index) => `<rect x="${36 + index * 110}" y="${height - 26}" width="12" height="12" fill="${colors[key]}"></rect>${svgText(54 + index * 110, height - 16, legendLabels[key], `font-size="11" fill="#69707a"`)}`).join("");
  return `<svg viewBox="0 0 960 ${height}" role="img" aria-label="领域与实现状态">
    <rect width="960" height="${height}" rx="8" fill="#fbfaf7"></rect>
    ${svgText(36, 32, `${domains.length} 个领域 · ${features.length} 个功能`, `font-size="14" font-weight="700" fill="#1d1f22"`)}
    ${rows}${legend}
  </svg>`;
}

function riskSvg() {
  const features = DATA.features || [];
  if (!features.length) return "";
  const high = features.filter(item => item.risk === "high");
  const missingDesign = high.filter(item => !item.design).length;
  const width = 960;
  const height = 150;
  const ratio = features.length ? high.length / features.length : 0;
  return `<svg viewBox="0 0 ${width} ${height}" role="img" aria-label="风险分布">
    <rect width="${width}" height="${height}" rx="8" fill="#fbfaf7"></rect>
    ${svgText(36, 36, `高风险 ${high.length} / ${features.length}`, `font-size="16" font-weight="700" fill="#1d1f22"`)}
    <rect x="36" y="56" width="888" height="26" rx="6" fill="#e1f0e8"></rect>
    <rect x="36" y="56" width="${(888 * ratio).toFixed(1)}" height="26" rx="6" fill="#f4dde4" stroke="#aa2d4d"></rect>
    ${svgText(36, 112, missingDesign ? `${missingDesign} 个高风险功能缺少独立技术设计` : "所有高风险功能都有独立技术设计", `font-size="13" fill="${missingDesign ? "#aa2d4d" : "#2f7d4f"}"`)}
  </svg>`;
}

function formatNumber(value) {
  return new Intl.NumberFormat("zh-CN").format(value);
}

function highlight(line, query) {
  const escaped = escapeHtml(clip(line, 220));
  if (!query) return escaped;
  return escaped.replace(new RegExp(escapeRegExp(escapeHtml(query)), "gi"), match => `<mark>${match}</mark>`);
}
