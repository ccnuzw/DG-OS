/* Design prototype: skills center and MCP management. No host API is called. */
(() => {
  "use strict";
  var p = window.__dgosProto;
  if (!p) return;
  var q = function (selector) { return document.querySelector(selector); };
  var esc = p.esc;
  var skillView = "browse";
  var skillOrigin = "all";
  var skillCategory = "all";
  var skillQuery = "";
  var selectedSkill = 0;
  var skillDisabled = [];
  var skillNotice = "";
  var skillRegistry = "online";
  var importSource = "";
  var importPreview = null;
  var mcpSelected = "notes";
  var mcpFilter = "all";
  var mcpQuery = "";
  var mcpEditing = null;
  var mcpDelete = null;
  var mcpTool = null;
  var mcpReview = false;
  var mcpResult = null;
  var mcpArgs = "{}";
  var mcpTimeout = "30000";
  var mcpNotice = "";
  var mcpToken = "";
  var skillMeta = [
    { version: "1.0.0", author: "DGOS", identity: "dgos_official", license: "MIT", tags: ["规划", "需求"], installedAt: "2026-10-18", callType: "content" },
    { version: "1.2.0", author: "DGOS", identity: "dgos_official", license: "MIT", tags: ["写作", "表达"], installedAt: "2026-10-20", callType: "content" },
    { version: "0.9.0", author: "社区维护", identity: "legacy_registry", license: "Apache-2.0", tags: ["开发", "代码"], installedAt: "2026-10-12", callType: "legacy" },
    { version: "1.0.0", author: "DGOS", identity: "dgos_official", license: "MIT", tags: ["研究", "摘要"], installedAt: "2026-10-22", callType: "content" }
  ];
  var mcpServers = [
    {
      id: "notes", name: "示例资料工具", transport: "stdio", status: "ready", enabled: true, auto: true, lastError: "",
      tools: [{ name: "read_notes", title: "读取示例资料", description: "读取选定的演示资料，只读。" }],
      logs: [{ time: "10:21", outcome: "成功", tool: "read_notes", duration: "42ms" }],
      connections: [{ time: "10:05", event: "已连接", detail: "stdio · 本地" }]
    },
    {
      id: "files", name: "本地文件服务", transport: "http", status: "degraded", enabled: true, auto: false, lastError: "handshake_timeout",
      tools: [{ name: "list_files", title: "列出目录", description: "列出演示目录中的文件。" }],
      logs: [{ time: "09:48", outcome: "失败", tool: "list_files", duration: "30000ms" }],
      connections: [{ time: "09:47", event: "重连", detail: "第 2 次尝试" }]
    },
    {
      id: "team", name: "团队知识库", transport: "http", status: "disconnected", enabled: false, auto: false, lastError: "",
      tools: [],
      logs: [],
      connections: []
    }
  ];
  function page() { return p.state.page; }
  function metaFor(index) { return skillMeta[index] || { version: "1.0.0", author: "本地", identity: "third_party", license: "MIT", tags: ["导入"], installedAt: "2026-11-03", callType: "content" }; }
  function identityLabel(identity) { return identity === "dgos_official" ? "DGOS 官方" : identity === "legacy_registry" ? "旧版归档" : "第三方"; }
  function isOfficial(index) { return metaFor(index).identity === "dgos_official"; }
  function isInstalled(index) { return p.state.installed.indexOf(index) >= 0; }
  function isEnabled(index) { return skillDisabled.indexOf(index) < 0; }
  function skills() { return p.skills || []; }
  function selectedServer() { return mcpServers.find(function (server) { return server.id === mcpSelected; }); }
  function statusLabel(status) { return { ready: "已连接", degraded: "降级", disconnected: "未连接", connecting: "连接中" }[status] || status; }
  function statusClass(status) { return status === "ready" ? "ready" : status === "degraded" ? "degraded" : "offline"; }

  function navItem(label, action, iconName, tone, active) {
    return "<button type='button' class='nav-item " + (active ? "active" : "") + "' data-ws-action='" + action + "' title='" + esc(label) + "'" + (active ? " aria-current='page'" : "") + ">" + p.tile(iconName, tone) + "<span class='nav-label'>" + esc(label) + "</span></button>";
  }
  function skillButton(label, action, cls) {
    return "<button type='button' class='" + (cls || "secondary ws-small") + "' data-ws-skill-action='" + action + "'>" + label + "</button>";
  }
  function mcpButton(label, action, cls) {
    return "<button type='button' class='" + (cls || "secondary ws-small") + "' data-ws-mcp-action='" + action + "'>" + label + "</button>";
  }

  function skillCards() {
    var list = skills().map(function (skill, index) { return { skill: skill, index: index }; });
    list = list.filter(function (entry) {
      var meta = metaFor(entry.index);
      if (skillView === "installed" && !isInstalled(entry.index)) return false;
      if (skillOrigin === "official" && meta.identity !== "dgos_official") return false;
      if (skillOrigin === "legacy" && meta.identity !== "legacy_registry") return false;
      if (skillCategory !== "all" && entry.skill.type !== skillCategory) return false;
      var haystack = (entry.skill.name + " " + entry.skill.desc + " " + entry.skill.type).toLowerCase();
      return haystack.indexOf(skillQuery.trim().toLowerCase()) >= 0;
    });
    if (!list.length) return "<div class='resource-empty'><strong>没有匹配的技能</strong><p>试试其他关键词或筛选条件。</p>" + skillButton("清除筛选", "clear", "secondary ws-small") + "</div>";
    return list.map(function (entry) {
      var skill = entry.skill, meta = metaFor(entry.index), installed = isInstalled(entry.index), official = meta.identity === "dgos_official";
      return "<article class='skill-card" + (selectedSkill === entry.index ? " selected" : "") + "'>" +
        "<div class='skill-card-head'>" + p.tile(skill.icon, skill.tone) + "<div class='grow'><strong>" + esc(skill.name) + "</strong><small>" + esc(skill.type) + "</small></div>" + p.badge(identityLabel(meta.identity), official ? "good" : "") + "</div>" +
        "<p class='skill-card-desc'>" + esc(skill.desc) + "</p>" +
        "<div class='skill-meta'><span>v" + esc(meta.version) + "</span><span>" + esc(meta.author) + "</span></div>" +
        "<div class='skill-tags'>" + meta.tags.map(function (tag) { return "<span>" + esc(tag) + "</span>"; }).join("") + "</div>" +
        "<div class='skill-card-actions'>" + skillButton("详情", "select:" + entry.index) +
        (installed ? "<span class='skill-added'>已添加</span>" : official ? skillButton("添加到我的技能", "install:" + entry.index, "primary ws-small") : "<span class='skill-added'>旧版只读</span>") +
        "</div></article>";
    }).join("");
  }

  function skillDetail() {
    var skill = skills()[selectedSkill];
    if (!skill) return "<aside class='skill-detail resource-panel'><div class='resource-empty'><strong>选择一个技能</strong><p>查看来源、内容和适用方式。</p></div></aside>";
    var meta = metaFor(selectedSkill), installed = isInstalled(selectedSkill), official = meta.identity === "dgos_official";
    return "<aside class='skill-detail resource-panel'>" +
      "<div class='skill-detail-head'>" + p.tile(skill.icon, skill.tone) + "<div><h2>" + esc(skill.name) + "</h2><p>" + esc(skill.type) + "</p></div></div>" +
      "<p class='skill-detail-desc'>" + esc(skill.desc) + "</p>" +
      "<div class='skill-tags'>" + meta.tags.map(function (tag) { return "<span>" + esc(tag) + "</span>"; }).join("") + "</div>" +
      "<div class='skill-detail-body'><h3>它会怎样帮助你</h3><p>" + esc(skill.body) + "</p></div>" +
      "<dl class='skill-facts'><dt>版本</dt><dd class='mono'>" + esc(meta.version) + "</dd><dt>来源</dt><dd>" + esc(identityLabel(meta.identity)) + " · " + esc(meta.author) + "</dd><dt>许可</dt><dd>" + esc(meta.license) + "</dd><dt>内容</dt><dd>" + (meta.callType === "content" ? "SKILL.md · 本地内容" : "旧版注册表 · 执行不可用") + "</dd></dl>" +
      "<div class='skill-detail-actions'>" +
      (installed ? "<span class='skill-added'>已在我的技能中</span>" + skillButton("在单任务中使用", "use:" + selectedSkill, "primary ws-small") : official ? skillButton("添加到我的技能", "install:" + selectedSkill, "primary ws-small") : "<span class='skill-added'>旧版归档只读</span>") +
      "</div>" +
      "<p class='resource-caption'>添加后可在单任务中选择；执行前仍按任务能力核对。</p>" +
      "</aside>";
  }

  function installedRows() {
    var rows = p.state.installed.map(function (index) { return { skill: skills()[index], index: index }; }).filter(function (entry) { return entry.skill; });
    if (!rows.length) return "<div class='resource-empty'><strong>还没有添加技能</strong><p>从浏览页添加一个官方技能，开始使用。</p>" + skillButton("浏览技能", "view:browse", "primary ws-small") + "</div>";
    return rows.map(function (entry) {
      var meta = metaFor(entry.index), enabled = isEnabled(entry.index), legacy = meta.callType !== "content";
      return "<article class='installed-skill resource-panel'>" +
        "<div class='row'>" + p.tile(entry.skill.icon, entry.skill.tone) + "<div class='grow'><strong>" + esc(entry.skill.name) + "</strong><small>" + esc(entry.skill.type) + " · v" + esc(meta.version) + "</small></div>" + p.badge(enabled ? "已启用" : "已停用", enabled ? "good" : "") + "</div>" +
        "<p>" + esc(entry.skill.desc) + "</p>" +
        "<div class='installed-meta'><span>安装于 " + esc(meta.installedAt) + "</span><span>" + (legacy ? "旧版执行不可用" : "SKILL.md") + "</span></div>" +
        "<div class='row wrap installed-actions'>" + (legacy ? "<span class='skill-added'>旧版归档只读</span>" : skillButton(enabled ? "停用" : "启用", "toggle:" + entry.index)) + skillButton("查看", "select:" + entry.index) + (legacy ? "" : skillButton("在单任务中使用", "use:" + entry.index, "primary ws-small")) + "</div>" +
        "</article>";
    }).join("");
  }

  function importPanel() {
    var preview = importPreview ? "<div class='import-preview'><div><strong>" + esc(importPreview.name) + " · " + esc(importPreview.version) + "</strong><p>" + esc(importPreview.description) + "</p><small>" + esc(importPreview.files) + " 个文件 · " + esc(importPreview.license) + " · sha256 " + esc(importPreview.hash) + "</small></div><div class='row'>" + skillButton("取消", "import-cancel") + skillButton("确认导入", "import-confirm", "primary ws-small") + "</div></div>" : "";
    return "<details class='import-source'" + (preview ? " open" : "") + "><summary><span>" + p.icon("plus") + "从来源安装技能</span><small>URL / ZIP · 预览确认后再安装</small></summary><div class='import-body'><p>支持 SKILL.md 内容包。原型不会读取本地文件或访问真实地址。</p><div class='row wrap'><label class='resource-search grow'><input id='skill-source' aria-label='技能来源' placeholder='https://example.invalid/skill.zip' value='" + esc(importSource) + "'></label>" + skillButton("预览来源", "import-preview") + skillButton("选择 ZIP", "import-zip") + skillButton("模拟依赖失败", "import-dep-fail", "muted-button ws-small") + "</div>" + preview + "</div></details>";
  }

  function skillsPage() {
    var categories = ["all"].concat(Array.from(new Set(skills().map(function (skill) { return skill.type; }))));
    var tabs = "<div class='resource-tabs' role='tablist' aria-label='技能视图'>" + skillButton("浏览", "view:browse", skillView === "browse" ? "primary ws-small" : "secondary ws-small") + skillButton("我的技能 " + p.state.installed.length, "view:installed", skillView === "installed" ? "primary ws-small" : "secondary ws-small") + "</div>";
    var browse = "<div class='resource-toolbar'><div class='resource-filters' role='group' aria-label='内容来源'>" + [["all", "全部内容"], ["official", "DGOS 官方"], ["legacy", "旧版归档"]].map(function (item) { return skillButton(item[1], "origin:" + item[0], skillOrigin === item[0] ? "primary ws-small" : "secondary ws-small"); }).join("") + "</div><label class='resource-search'><span class='visually-hidden'>搜索技能</span><input id='skill-search-v2' aria-label='搜索技能' placeholder='搜索技能名称或用途' value='" + esc(skillQuery) + "'></label></div>" +
      "<div class='resource-chips' role='group' aria-label='技能分类'>" + categories.map(function (cat) { return skillButton(cat === "all" ? "全部" : cat, "category:" + cat, skillCategory === cat ? "primary ws-small" : "secondary ws-small"); }).join("") + "</div>" +
      "<div class='skill-layout'><div class='skill-grid'>" + skillCards() + "</div>" + skillDetail() + "</div>";
    var installed = "<div class='my-skills'>" + installedRows() + "</div>";
    var registryNotice = skillRegistry === "offline" ? "<div class='resource-notice'>技能注册表离线，当前显示本地缓存；安装和来源预览不可用。</div>" : "";
    var registryBadge = p.badge(skillRegistry === "online" ? "注册表在线" : "注册表离线", skillRegistry === "online" ? "good" : "warn");
    var registryButton = skillButton(skillRegistry === "online" ? "模拟离线" : "重试连接", skillRegistry === "online" ? "registry:offline" : "registry:retry", "secondary ws-small");
    return "<div class='resource-page'><header class='resource-head'><div><small>资源</small><h1>技能中心</h1><p>找到适合任务的工作方法，添加后在单任务中使用。</p></div><div class='resource-head-actions'>" + registryBadge + registryButton + "</div></header><div class='resource-scroll'><div class='resource-content'>" + tabs + registryNotice + (skillNotice ? "<div class='resource-notice'>" + esc(skillNotice) + "</div>" : "") + importPanel() + (skillView === "browse" ? browse : installed) + "</div></div></div>";
  }

  function mcpStatusDot(status) { return "<span class='mcp-dot " + statusClass(status) + "' aria-hidden='true'></span>"; }
  function mcpServerCard(server) {
    var connected = server.status === "ready" || server.status === "degraded";
    var confirm = mcpDelete === server.id ? "<div class='mcp-delete-confirm'>删除这个服务？<div class='row'>" + mcpButton("确认删除", "delete-confirm:" + server.id, "danger ws-small") + mcpButton("取消", "delete-cancel", "secondary ws-small") + "</div></div>" : "";
    return "<article class='mcp-server-card" + (mcpSelected === server.id ? " selected" : "") + "'>" +
      "<button type='button' class='mcp-server-main' data-ws-mcp-action='select:" + server.id + "' aria-pressed='" + (mcpSelected === server.id) + "'>" + mcpStatusDot(server.status) + "<span class='grow'><strong>" + esc(server.name) + "</strong><small>" + esc(server.transport) + " · " + esc(statusLabel(server.status)) + (server.auto ? " · 自动连接" : "") + "</small>" + (server.lastError ? "<em>" + esc(server.lastError) + "</em>" : "") + "</span></button>" +
      "<div class='mcp-server-actions'>" + mcpButton(connected ? "断开" : "连接", (connected ? "disconnect:" : "connect:") + server.id) + mcpButton("编辑", "edit:" + server.id) + mcpButton(server.enabled ? "停用" : "启用", "toggle:" + server.id) + mcpButton("删除", "delete:" + server.id, "muted-button ws-small") + "</div>" + confirm + "</article>";
  }

  function mcpForm() {
    var e = mcpEditing;
    return "<section class='resource-panel mcp-form'><header class='resource-panel-head'><div><small>服务配置</small><h2>" + (e.isNew ? "添加 MCP 服务" : "编辑 MCP 服务") + "</h2></div>" + mcpButton("取消", "cancel-edit", "secondary ws-small") + "</header><div class='form-grid'><label class='field'>名称<input id='mcp-name' value='" + esc(e.name) + "' placeholder='例如：本地资料'></label><label class='field'>连接方式<select id='mcp-transport'><option value='stdio'" + (e.transport === "stdio" ? " selected" : "") + ">本地标准输入输出</option><option value='http'" + (e.transport === "http" ? " selected" : "") + ">HTTP</option></select></label>" +
      (e.transport === "stdio" ? "<label class='field full'>可执行文件<input id='mcp-executable' value='" + esc(e.executable) + "' placeholder='/usr/local/bin/mcp-server'></label><label class='field full'>参数（JSON）<input id='mcp-args' value='" + esc(e.args) + "' placeholder='[--root, /tmp]'></label><label class='field'>工作目录<input id='mcp-cwd' value='" + esc(e.cwd) + "' placeholder='可选'></label>" : "<label class='field full'>服务地址<input id='mcp-url' value='" + esc(e.url) + "' placeholder='https://example.invalid/mcp'></label>") +
      "<label class='check-label'><input id='mcp-auto' type='checkbox' " + (e.auto ? "checked" : "") + ">启动时自动连接</label><label class='check-label'><input id='mcp-enabled' type='checkbox' " + (e.enabled ? "checked" : "") + ">启用这个服务</label></div><div class='row form-actions'>" + mcpButton("保存配置", "save", "primary ws-small") + "<small>凭据不写入普通草稿；连接由宿主校验。</small></div></section>";
  }

  function toolCallPanel(server) {
    if (!mcpTool || mcpTool.serverId !== server.id) return "";
    var review = mcpReview ? "<div class='tool-review'><strong>确认调用</strong><p>服务：" + esc(server.name) + " · 工具：" + esc(mcpTool.name) + "</p><pre>" + esc(mcpArgs) + "</pre><div class='row'>" + mcpButton("确认调用", "tool-run", "primary ws-small") + mcpButton("取消调用", "tool-cancel", "secondary ws-small") + mcpButton("模拟未知结果", "tool-unknown", "muted-button ws-small") + "</div></div>" : "";
    var resultTone = mcpResult && (mcpResult.indexOf("结果未知") === 0 || mcpResult.indexOf("已取消") === 0) ? " result-warn" : "";
    var result = mcpResult ? "<div class='tool-result-box" + resultTone + "'><strong>调用结果 · 演示</strong><pre>" + esc(mcpResult) + "</pre></div>" : "";
    return "<section class='resource-panel mcp-tool-call'><header class='resource-panel-head'><div><small>工具调用</small><h2>" + esc(mcpTool.title || mcpTool.name) + "</h2></div>" + mcpButton("关闭", "tool-close", "secondary ws-small") + "</header><p class='resource-caption'>执行前展示参数并请求确认；这是固定演示结果，不会执行真实工具。</p><label class='field'>参数 JSON<textarea id='mcp-tool-args' rows='4'>" + esc(mcpArgs) + "</textarea></label><div class='row'><label class='field'>超时（毫秒）<input id='mcp-tool-timeout' value='" + esc(mcpTimeout) + "'></label>" + (mcpReview || mcpResult ? "" : mcpButton("审核调用", "tool-review", "primary ws-small")) + "</div>" + review + result + "</section>";
  }

  function mcpDetail() {
    var server = selectedServer();
    if (!server) return "<section class='resource-panel mcp-detail'><div class='resource-empty'><strong>选择一个 MCP 服务</strong><p>查看工具、调用记录和连接历史。</p></div></section>";
    var connected = server.status === "ready" || server.status === "degraded";
    var tools = server.tools.length ? server.tools.map(function (tool) { return "<div class='mcp-tool-row'><div class='grow'><strong>" + esc(tool.title || tool.name) + "</strong><small>" + esc(tool.description) + "</small></div><code>" + esc(tool.name) + "</code>" + mcpButton("检查调用", "tool-open:" + server.id + "/" + tool.name) + "</div>"; }).join("") : "<p class='resource-caption'>连接并刷新后显示可用工具。</p>";
    var logs = server.logs.length ? server.logs.map(function (log) { return "<div class='mcp-log-row'><span class='" + (log.outcome === "成功" ? "log-success" : "log-error") + "'>" + esc(log.outcome) + "</span><code>" + esc(log.tool) + "</code><small>" + esc(log.duration) + " · " + esc(log.time) + "</small></div>"; }).join("") : "<p class='resource-caption'>还没有调用记录。</p>";
    var history = server.connections.length ? server.connections.map(function (item) { return "<div class='mcp-log-row'><span>" + esc(item.event) + "</span><small>" + esc(item.detail) + " · " + esc(item.time) + "</small></div>"; }).join("") : "<p class='resource-caption'>还没有连接事件。</p>";
    return "<section class='resource-panel mcp-detail'><header class='resource-panel-head'><div><small>服务详情</small><h2>" + esc(server.name) + "</h2><p>" + mcpStatusDot(server.status) + " " + esc(statusLabel(server.status)) + " · " + esc(server.transport) + "</p></div><div class='row'>" + mcpButton(connected ? "断开连接" : "连接", (connected ? "disconnect:" : "connect:") + server.id, connected ? "secondary ws-small" : "primary ws-small") + mcpButton("刷新工具", "refresh-tools:" + server.id) + "</div></header>" +
      (server.transport === "http" ? "<div class='mcp-credentials'><label class='field'>Bearer Token<input id='mcp-token' type='password' value='" + esc(mcpToken) + "' placeholder='" + (server.tokenSet ? "已保存，输入可替换" : "只输入一次，不会写入普通草稿") + "'></label><div class='row'>" + mcpButton("保存凭据", "token-save", "secondary ws-small") + mcpButton("移除凭据", "token-clear", "muted-button ws-small") + "</div></div>" : "") +
      "<div class='resource-subhead'><h3>可用工具</h3><small>描述不代表已授权；调用前仍由宿主确认。</small></div>" + tools + toolCallPanel(server) +
      "<div class='resource-subhead'><h3>最近使用</h3><small>只记录工具名、结果和耗时。</small></div>" + logs +
      "<div class='resource-subhead'><h3>连接历史</h3><small>连接失败、重连和断开事件。</small></div>" + history +
      "</section>";
  }

  function mcpPage() {
    var header = "<div><small>资源</small><h1>MCP 管理</h1><p>连接本地或远程 MCP 服务，检查工具并查看调用与连接记录。</p></div><div class='resource-head-actions'>" + p.badge("MCP · 演示", "good") + mcpButton("添加服务", "add", "primary ws-small") + mcpButton("刷新状态", "refresh", "secondary ws-small") + "</div>";
    var filters = [['all', '全部'], ['ready', '已连接'], ['issue', '需处理']].map(function (item) {
      return mcpButton(item[1], 'filter:' + item[0], mcpFilter === item[0] ? 'primary ws-small' : 'secondary ws-small');
    }).join("");
    var query = mcpQuery.trim().toLowerCase();
    var visible = mcpServers.filter(function (server) {
      var statusMatch = mcpFilter === 'all' || (mcpFilter === 'ready' ? server.status === 'ready' : server.status !== 'ready');
      var queryMatch = !query || (server.name + ' ' + server.transport + ' ' + statusLabel(server.status)).toLowerCase().indexOf(query) >= 0;
      return statusMatch && queryMatch;
    });
    if (visible.length && !visible.some(function (server) { return server.id === mcpSelected; })) mcpSelected = visible[0].id;
    var list = visible.length ? visible.map(mcpServerCard).join("") : "<div class='mcp-empty'><strong>没有匹配的服务</strong><p>调整筛选或搜索词后再试。</p></div>";
    var detail = visible.length ? mcpDetail() : "<section class='resource-panel mcp-detail'><div class='resource-panel-head'><div><small>服务详情</small><h2>没有可显示的服务</h2></div></div><p class='resource-caption'>当前筛选没有匹配项。清除搜索或切换筛选。</p></section>";
    return "<div class='resource-page'><header class='resource-head'>" + header + "</header><div class='resource-scroll'><div class='resource-content'>" + (mcpNotice ? "<div class='resource-notice'>" + esc(mcpNotice) + "</div>" : "") + (mcpEditing ? mcpForm() : "") + "<div class='mcp-layout'><section class='mcp-servers'><div class='resource-subhead'><h2>我的服务</h2><small>" + visible.length + " / " + mcpServers.length + " 个服务</small></div><div class='mcp-filter-row'><div class='resource-filters'>" + filters + "</div><label class='resource-search'><input id='mcp-search' aria-label='搜索 MCP 服务' placeholder='搜索服务…' value='" + esc(mcpQuery) + "'></label></div>" + list + "</section>" + detail + "</div></div></div></div>";
  }

  function injectMCPNav() {
    var sidebar = q(".sidebar");
    if (!sidebar) return;
    var skills = sidebar.querySelector('[data-action="page:skills"]');
    if (!skills) return;
    var old = sidebar.querySelector('[data-ws-action="mcp-page"]');
    if (old) old.remove();
    skills.insertAdjacentHTML("afterend", navItem("MCP 管理", "mcp-page", "tool", "green", page() === "mcp"));
  }

  var canvasProjects = [
    { id: "ecommerce", name: "电商测试", nodes: 4, updated: "11/3 10:37", visibility: "本地", trashed: false },
    { id: "untitled", name: "未命名画布", nodes: 19, updated: "10/25 16:27", visibility: "本地", trashed: false }
  ];
  var canvasDraftName = "";
  var canvasNewOpen = false;
  function canvasFilter() {
    return p.state.canvasFilter || "projects";
  }
  function canvasVisibleProjects() {
    var filter = canvasFilter();
    return canvasProjects.filter(function (item) {
      return filter === "trash" ? item.trashed : !item.trashed;
    });
  }
  function canvasProject() {
    var id = p.state.canvasProject || "ecommerce";
    return canvasProjects.find(function (item) { return item.id === id; }) || canvasProjects[0] || { name: "未命名项目", updated: "", nodes: 0 };
  }
  function canvasProjectCard(project) {
    var trashed = !!project.trashed;
    var thumb = trashed
      ? "<div class='canvas-project-thumb canvas-project-thumb-disabled'>" + "<span class='canvas-dot-preview'>" + p.icon("grid") + "</span><span class='canvas-project-visibility'>" + esc(project.visibility) + "</span></div>"
      : "<button type='button' class='canvas-project-thumb' data-ws-canvas-action='open:" + project.id + "' aria-label='打开" + esc(project.name) + "'>" + "<span class='canvas-dot-preview'>" + p.icon("grid") + "</span><span class='canvas-project-visibility'>" + esc(project.visibility) + "</span></button>";
    var actions = trashed
      ? "<button type='button' class='secondary small' data-ws-canvas-action='restore:" + project.id + "'>恢复</button><button type='button' class='muted-button small' data-ws-canvas-action='purge:" + project.id + "'>彻底删除</button>"
      : "<button type='button' class='secondary small' data-ws-canvas-action='open:" + project.id + "'>打开</button><button type='button' class='muted-button small' data-ws-canvas-action='delete:" + project.id + "'>移到回收站</button>";
    return "<article class='canvas-project-card " + (trashed ? "canvas-project-card-trashed" : "") + "'>" + thumb +
      "<div class='canvas-project-info'><strong>" + esc(project.name) + "</strong><small>" + (trashed ? "已删除 · " : "") + project.nodes + " 个节点 · " + esc(project.updated) + "</small><div class='canvas-project-actions'>" + actions + "</div></div>" +
      "</article>";
  }
  function canvasCreateForm() {
    return "<section class='canvas-create-form'><label class='field'>项目名称<input id='canvas-project-name' placeholder='例如：电商测试' value='" + esc(canvasDraftName) + "'></label><div class='row'><button type='button' class='primary' data-ws-canvas-action='create-submit'>创建并打开</button><button type='button' class='secondary' data-ws-canvas-action='create-cancel'>取消</button></div><small>本地项目 · 演示数据，不会写入宿主存储。</small></section>";
  }
  function canvasHome() {
    var filter = canvasFilter();
    var visible = canvasVisibleProjects();
    var title = filter === "trash" ? "回收站" : filter === "recent" ? "最近使用" : "全部项目";
    var subtitle = filter === "trash" ? "删除的项目会先进入这里，恢复或彻底删除。" : filter === "recent" ? "按最近打开的项目继续工作。" : "把素材、提示词和生成结果组织在同一个本地项目里。";
    var emptyText = filter === "trash" ? "回收站为空" : filter === "recent" ? "还没有最近项目" : "还没有项目";
    var emptyHint = filter === "trash" ? "删除的项目会先进入这里。" : "新建一个项目，开始编排模型、技能和工具。";
    var cards = visible.length ? visible.map(canvasProjectCard).join("") : "<div class='canvas-empty'><strong>" + emptyText + "</strong><p>" + emptyHint + "</p></div>";
    var headerActions = filter === "trash" ? "" : "<button type='button' class='primary' data-ws-canvas-action='create'>新建项目</button><button type='button' class='secondary' data-ws-canvas-action='import'>导入项目</button>";
    var nav = "<button type='button' class='canvas-nav-item " + (filter === "projects" ? "active" : "") + "' data-ws-canvas-action='filter:projects'>全部项目 <span>" + canvasProjects.filter(function (item) { return !item.trashed; }).length + "</span></button><button type='button' class='canvas-nav-item " + (filter === "recent" ? "active" : "") + "' data-ws-canvas-action='filter:recent'>最近使用</button><button type='button' class='canvas-nav-item " + (filter === "trash" ? "active" : "") + "' data-ws-canvas-action='filter:trash'>回收站 <span>" + canvasProjects.filter(function (item) { return item.trashed; }).length + "</span></button>";
    return "<div class='canvas-platform'><aside class='canvas-platform-nav'><div class='canvas-brand'><span class='canvas-mark'>∞</span><div><strong>无限画布</strong><small>本地创作平台</small></div></div><div class='canvas-nav-section'>项目</div>" + nav + "<div class='canvas-nav-foot'>本地账户 · 演示数据</div></aside><section class='canvas-platform-main'><header class='canvas-platform-head'><div><small>WORKSPACE</small><h1>" + title + "</h1><p>" + subtitle + "</p></div><div class='row'>" + headerActions + "</div></header>" + (canvasNewOpen ? canvasCreateForm() : "") + "<div class='canvas-project-grid'>" + cards + "</div></section></div>";
  }

  function canvasEditor() {
    var project = canvasProject();
    return "<div class='canvas-editor'><header class='canvas-editor-bar'><button type='button' class='canvas-back' data-ws-canvas-action='back'>" + p.icon("arrow") + "<span>全部项目</span></button><div class='canvas-editor-title'><strong>" + esc(project.name) + "</strong><small>本地项目 · 演示画布</small></div><span class='canvas-local-badge'>草稿</span></header><div class='canvas-stage'><svg class='canvas-edges' viewBox='0 0 1440 900' preserveAspectRatio='none' aria-hidden='true'><path d='M489 221 C550 221 555 260 619 260'/><path d='M489 545 C550 545 555 300 619 300'/><path d='M950 270 C975 270 985 270 1008 270'/></svg><div class='canvas-node canvas-node-input' style='left:16%;top:20%'><span class='canvas-node-icon'>" + p.icon("file") + "</span><strong>商品资料</strong><small>文本输入</small></div><div class='canvas-node canvas-node-input' style='left:16%;top:56%'><span class='canvas-node-icon'>" + p.icon("file") + "</span><strong>品牌语气</strong><small>文本输入</small></div><div class='canvas-node canvas-node-llm' style='left:43%;top:19%'><header><span>LLM</span><small>文本生成</small></header><label>模型<select><option>GPT-4.1</option></select></label><label>提示词<textarea rows='3'>根据商品资料生成一段简洁的电商描述。</textarea></label><div class='canvas-node-foot'><span>输出到文本</span><button type='button'>运行</button></div></div><div class='canvas-node canvas-node-output' style='left:70%;top:22%'><span>输出</span><div class='canvas-output-box'>等待运行</div></div><div class='canvas-bottom-bar'><button type='button'>+</button><button type='button' class='active'>" + p.icon("arrow") + "</button><button type='button'>" + p.icon("warning") + "</button><button type='button'>" + p.icon("play") + "</button><button type='button'>" + p.icon("file") + "</button></div><div class='canvas-minimap'><span></span><span></span></div></div></div>";
  }
  function canvasPage() {
    p.state.canvasOpen = true;
    return p.state.canvasView === "editor" ? canvasEditor() : canvasHome();
  }
  function replaceResourcePage(main, markup) {
    var tabs = main.querySelector(".workspace-tabs");
    main.innerHTML = "";
    if (tabs) main.appendChild(tabs);
    main.insertAdjacentHTML("beforeend", markup);
  }
  function afterResources() {
    var main = q("#main");
    if (!main) return;
    injectMCPNav();
    if (page() === "skills") { p.state.openSkills = true; replaceResourcePage(main, skillsPage()); }
    if (page() === "mcp") { p.state.openMcp = true; replaceResourcePage(main, mcpPage()); }
    if (page() === "canvas") { p.state.canvasOpen = true; replaceResourcePage(main, canvasPage()); }
  }

  function rerender(selector) {
    p.render();
    if (selector) setTimeout(function () { var el = q(selector); if (el) { el.focus(); if (el.select) el.select(); } }, 0);
  }
  function syncMcp() {
    p.state.mcp = mcpServers.some(function (server) { return server.enabled && (server.status === "ready" || server.status === "degraded"); });
  }

  document.addEventListener("click", function (event) {
    var mcpPageEl = event.target.closest("[data-ws-action=\"mcp-page\"]");
    if (mcpPageEl) { event.preventDefault(); var settingsDialog = q("#dialog"); if (settingsDialog && settingsDialog.open) p.closeDialog(); p.state.page = "mcp"; p.state.scene = "normal"; p.render(); return; }
    var canvasEl = event.target.closest("[data-ws-canvas-action]");
    if (canvasEl) {
      event.preventDefault();
      var canvasAction = canvasEl.getAttribute("data-ws-canvas-action");
      var canvasParts = canvasAction.split(":");
      var canvasVerb = canvasParts[0], canvasValue = canvasParts[1];
      if (canvasVerb === "create") { p.state.canvasFilter = "projects"; canvasNewOpen = true; canvasDraftName = ""; p.render(); setTimeout(function () { var input = q("#canvas-project-name"); if (input) input.focus(); }, 0); return; }
      if (canvasVerb === "create-cancel") { canvasNewOpen = false; p.render(); return; }
      if (canvasVerb === "create-submit") {
        var name = canvasDraftName.trim() || "未命名项目";
        var id = "canvas-" + Date.now();
        canvasProjects.unshift({ id: id, name: name, nodes: 0, updated: "刚刚", visibility: "本地", trashed: false });
        p.state.canvasProject = id; p.state.canvasView = "editor"; p.state.canvasOpen = true; p.state.canvasDirty = true; p.state.page = "canvas"; p.state.canvasFilter = "projects";
        canvasNewOpen = false; canvasDraftName = ""; p.render(); p.toast("本地项目已创建（演示）。"); return;
      }
      if (canvasVerb === "open") { p.state.canvasProject = canvasValue; p.state.canvasView = "editor"; p.state.canvasOpen = true; p.state.canvasDirty = true; p.state.page = "canvas"; p.render(); return; }
      if (canvasVerb === "back") { p.state.canvasView = "home"; p.state.canvasDirty = false; p.state.canvasFilter = "projects"; p.render(); return; }
      if (canvasVerb === "filter") { p.state.canvasFilter = canvasValue; canvasNewOpen = false; p.render(); return; }
      if (canvasVerb === "delete") {
        var deleted = canvasProjects.find(function (item) { return item.id === canvasValue; });
        if (deleted) { deleted.trashed = true; if (p.state.canvasProject === canvasValue) p.state.canvasProject = null; p.render(); p.toast("项目已移到回收站（演示）。"); }
        return;
      }
      if (canvasVerb === "restore") {
        var restored = canvasProjects.find(function (item) { return item.id === canvasValue; });
        if (restored) { restored.trashed = false; p.render(); p.toast("项目已恢复（演示）。"); }
        return;
      }
      if (canvasVerb === "purge") {
        canvasProjects = canvasProjects.filter(function (item) { return item.id !== canvasValue; });
        if (p.state.canvasProject === canvasValue) p.state.canvasProject = null;
        p.render(); p.toast("项目已彻底删除（演示）。"); return;
      }
      if (canvasVerb === "import") { p.toast("导入项目将在后续版本接入；当前先使用新建项目。"); return; }
      return;
    }
    var skillEl = event.target.closest("[data-ws-skill-action]");
    if (skillEl) {
      event.preventDefault();
      var skillAction = skillEl.getAttribute("data-ws-skill-action");
      var skillParts = skillAction.split(":");
      var verb = skillParts[0], value = skillParts[1];
      if (verb === "view") { skillView = value; rerender(); return; }
      if (verb === "registry") { if (value === "offline") { skillRegistry = "offline"; skillNotice = ""; } else { skillRegistry = "online"; skillNotice = "注册表已恢复（演示）。"; } rerender(); return; }
      if (verb === "origin") { skillOrigin = value; rerender(); return; }
      if (verb === "category") { skillCategory = value; rerender(); return; }
      if (verb === "select") { selectedSkill = Number(value); rerender(); return; }
      if (verb === "install") {
        var index = Number(value);
        if (!isOfficial(index)) { p.toast("旧版归档技能只读，不能添加。"); return; }
        if (skillRegistry === "offline") { skillNotice = "注册表离线，安装失败；请恢复连接后重试。"; rerender(); return; }
        if (p.state.installed.indexOf(index) < 0) p.state.installed.push(index);
        selectedSkill = index; skillNotice = "已添加到我的技能；依赖检查通过（演示）。"; rerender(); return;
      }
      if (verb === "toggle") {
        var toggleIndex = Number(value);
        if (metaFor(toggleIndex).callType !== "content") { p.toast("旧版执行不可用，不能启用。"); return; }
        if (skillDisabled.indexOf(toggleIndex) >= 0) skillDisabled = skillDisabled.filter(function (x) { return x !== toggleIndex; });
        else skillDisabled.push(toggleIndex);
        rerender(); return;
      }
      if (verb === "use") {
        var useIndex = Number(value);
        p.state.skill = useIndex; p.state.taskSkill = skills()[useIndex].name; p.state.task = "idle"; p.go("mode:single"); return;
      }
      if (verb === "clear") { skillOrigin = "all"; skillCategory = "all"; skillQuery = ""; rerender("#skill-search-v2"); return; }
      if (verb === "import-preview") {
        if (skillRegistry === "offline") { skillNotice = "注册表离线，无法检查来源；请恢复连接后重试。"; rerender(); return; }
        if (!importSource.trim()) { p.toast("先填写技能来源。"); return; }
        importPreview = { name: "示例导入技能", version: "1.0.0", description: "从来源预览的演示技能包。", files: 3, license: "MIT", hash: "demo-sha256-8f2c" }; rerender(); return;
      }
      if (verb === "import-zip") { skillNotice = "ZIP 读取失败：演示环境不读取本地文件；请使用来源 URL 预览。"; rerender(); return; }
      if (verb === "import-dep-fail") { importPreview = null; skillNotice = "依赖检查失败：缺少模型调用权限（演示）。请先配置模型服务。"; rerender(); return; }
      if (verb === "import-cancel") { importPreview = null; rerender(); return; }
      if (verb === "import-confirm") {
        var newIndex = skills().length;
        p.skills.push({ name: "示例导入技能", desc: "从来源预览后导入的演示技能。", type: "产品与规划", icon: "skill", tone: "amber", body: "先确认来源、内容与许可，再把技能用于具体任务。" });
        skillMeta.push({ version: "1.0.0", author: "本地导入", identity: "third_party", license: "MIT", tags: ["导入"], installedAt: "2026-11-03", callType: "content" });
        p.state.installed.push(newIndex); selectedSkill = newIndex; importPreview = null; importSource = ""; skillNotice = "示例来源已导入并添加到我的技能。"; rerender(); return;
      }
      return;
    }
    var mcpEl = event.target.closest("[data-ws-mcp-action]");
    if (!mcpEl) return;
    event.preventDefault();
    var action = mcpEl.getAttribute("data-ws-mcp-action");
    var parts = action.split(":");
    var cmd = parts[0], id = parts[1];
    if (cmd === "add") { mcpEditing = { isNew: true, id: "mcp-" + Date.now(), name: "", transport: "stdio", executable: "", args: "[]", cwd: "", url: "", auto: true, enabled: true }; mcpNotice = ""; p.render(); return; }
    if (cmd === "edit") { var editServer = mcpServers.find(function (s) { return s.id === id; }); if (editServer) { mcpEditing = { isNew: false, id: editServer.id, name: editServer.name, transport: editServer.transport, executable: editServer.executable || "", args: editServer.args || "[]", cwd: editServer.cwd || "", url: editServer.url || "", auto: editServer.auto, enabled: editServer.enabled }; p.render(); } return; }
    if (cmd === "cancel-edit") { mcpEditing = null; p.render(); return; }
    if (cmd === "save") {
      if (!mcpEditing || !mcpEditing.name.trim()) { mcpNotice = "请填写服务名称。"; p.render(); return; }
      if (mcpEditing.transport === "stdio" && !mcpEditing.executable.trim()) { mcpNotice = "请填写可执行文件。"; p.render(); return; }
      if (mcpEditing.transport === "http" && !mcpEditing.url.trim()) { mcpNotice = "请填写服务地址。"; p.render(); return; }
      var saved = { id: mcpEditing.id, name: mcpEditing.name, transport: mcpEditing.transport, status: mcpEditing.enabled ? "disconnected" : "disconnected", enabled: mcpEditing.enabled, auto: mcpEditing.auto, executable: mcpEditing.executable, args: mcpEditing.args, cwd: mcpEditing.cwd, url: mcpEditing.url, lastError: "", tools: [], logs: [], connections: [] };
      var existing = mcpServers.find(function (s) { return s.id === saved.id; });
      if (existing) Object.assign(existing, saved); else mcpServers.push(saved);
      mcpSelected = saved.id; mcpEditing = null; mcpNotice = "服务配置已保存（演示）。"; syncMcp(); p.render(); return;
    }
    if (cmd === "select") { mcpSelected = id; mcpTool = null; mcpResult = null; p.render(); return; }
    if (cmd === "filter") { mcpFilter = id; p.render(); return; }
    if (cmd === "connect") { var connectServer = mcpServers.find(function (s) { return s.id === id; }); if (connectServer) { connectServer.status = "ready"; connectServer.lastError = ""; syncMcp(); mcpNotice = connectServer.name + " 已连接（演示）。"; p.render(); } return; }
    if (cmd === "disconnect") { var disconnectServer = mcpServers.find(function (s) { return s.id === id; }); if (disconnectServer) { disconnectServer.status = "disconnected"; syncMcp(); mcpNotice = disconnectServer.name + " 已断开。"; p.render(); } return; }
    if (cmd === "toggle") { var toggleServer = mcpServers.find(function (s) { return s.id === id; }); if (toggleServer) { toggleServer.enabled = !toggleServer.enabled; if (!toggleServer.enabled) toggleServer.status = "disconnected"; syncMcp(); p.render(); } return; }
    if (cmd === "delete") { mcpDelete = id; p.render(); return; }
    if (cmd === "delete-cancel") { mcpDelete = null; p.render(); return; }
    if (cmd === "delete-confirm") { mcpServers = mcpServers.filter(function (s) { return s.id !== id; }); mcpSelected = mcpServers.length ? mcpServers[0].id : null; mcpDelete = null; syncMcp(); mcpNotice = "服务已删除（演示）。"; p.render(); return; }
    if (cmd === "refresh") { mcpNotice = "状态已刷新；未启动真实连接。"; p.render(); return; }
    if (cmd === "refresh-tools") { mcpNotice = "工具列表已刷新（演示）。"; p.render(); return; }
    if (cmd === "tool-open") { var toolParts = (id || "").split("/"); mcpTool = { serverId: toolParts[0], name: toolParts.slice(1).join("/"), title: toolParts.slice(1).join("/") }; var toolServer = mcpServers.find(function (s) { return s.id === mcpTool.serverId; }); var foundTool = toolServer && toolServer.tools.find(function (t) { return t.name === mcpTool.name; }); if (foundTool) mcpTool.title = foundTool.title; mcpReview = false; mcpResult = null; mcpArgs = "{}"; p.render(); return; }
    if (cmd === "tool-review") { mcpReview = true; p.render(); return; }
    if (cmd === "tool-run") { mcpReview = false; mcpResult = "读取完成：示例资料包含 2 个演示片段。"; mcpNotice = "工具调用已完成（演示）。"; var current = mcpServers.find(function (s) { return s.id === (mcpTool && mcpTool.serverId); }); if (current) current.logs.unshift({ time: "刚刚", outcome: "成功", tool: mcpTool.name, duration: "42ms" }); p.render(); return; }
    if (cmd === "tool-close") { mcpTool = null; mcpReview = false; mcpResult = null; p.render(); return; }
    if (cmd === "tool-review-cancel") { mcpReview = false; p.render(); return; }
    if (cmd === "tool-cancel") { mcpReview = false; mcpResult = "已取消：未执行工具调用，外部状态未改变。"; mcpNotice = "工具调用已取消（演示）。"; p.render(); return; }
    if (cmd === "tool-unknown") { mcpReview = false; mcpResult = "结果未知：连接在调用后中断，不能确认工具是否执行成功。"; mcpNotice = "工具调用结果未知（演示）。"; p.render(); return; }
    if (cmd === "token-save") { var tokenServer = selectedServer(); if (tokenServer) tokenServer.tokenSet = true; mcpToken = ""; p.toast("凭据已保存到演示内存。"); p.render(); return; }
    if (cmd === "token-clear") { var tokenClear = selectedServer(); if (tokenClear) tokenClear.tokenSet = false; mcpToken = ""; p.toast("凭据已移除（演示）。"); p.render(); return; }
  });

  document.addEventListener("input", function (event) {
    var id = event.target.id, value = event.target.value;
    if (id === "skill-search-v2") { skillQuery = value; var grid = document.querySelector(".skill-grid"); if (grid) grid.innerHTML = skillCards(); return; }
    if (id === "skill-source") { importSource = value; return; }
    if (id === "canvas-project-name") { canvasDraftName = value; return; }
    if (id === "mcp-search") { mcpQuery = value; p.render(); var nextSearch = document.querySelector("#mcp-search"); if (nextSearch) { nextSearch.focus(); nextSearch.setSelectionRange(value.length, value.length); } return; }
    if (!mcpEditing) {
      if (id === "mcp-tool-args") mcpArgs = value;
      if (id === "mcp-tool-timeout") mcpTimeout = value;
      if (id === "mcp-token") mcpToken = value;
      return;
    }
    if (id === "mcp-name") mcpEditing.name = value;
    if (id === "mcp-executable") mcpEditing.executable = value;
    if (id === "mcp-args") mcpEditing.args = value;
    if (id === "mcp-cwd") mcpEditing.cwd = value;
    if (id === "mcp-url") mcpEditing.url = value;
  });
  document.addEventListener("change", function (event) {
    if (event.target.id === "mcp-transport" && mcpEditing) { mcpEditing.transport = event.target.value; p.render(); return; }
    if (event.target.id === "mcp-auto" && mcpEditing) { mcpEditing.auto = event.target.checked; return; }
    if (event.target.id === "mcp-enabled" && mcpEditing) { mcpEditing.enabled = event.target.checked; return; }
  });

  var baseAfter = window.__dgosAfterRender;
  window.__dgosAfterRender = function () {
    if (baseAfter) baseAfter();
    afterResources();
  };
  syncMcp();
  afterResources();
})();
