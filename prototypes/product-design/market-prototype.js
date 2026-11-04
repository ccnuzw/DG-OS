/* Design prototype: application marketplace. No network or real installation. */
(() => {
  "use strict";
  var p = window.__dgosProto;
  if (!p) return;
  var q = function (selector) { return document.querySelector(selector); };
  var esc = p.esc;
  var view = "browse";
  var category = "all";
  var origin = "all";
  var sort = "recommend";
  var query = "";
  var selected = "model";
  var account = null;
  var modal = null;
  var submission = null;

  var apps = [
    { id: "model", name: "模型调试台", publisher: "DGOS", official: true, category: "开发", icon: "model", tone: "purple", desc: "检查模型连接、参数与调用结果，是调试 Provider 的轻量工具。", rating: "4.8", ratingCount: "1,284", downloads: "12.4k", size: "随包", version: "1.0.0", latest: "1.1.0", update: true, installed: true, permissions: ["模型调用"], changelog: "1.1.0：增加流式返回与错误详情。", shots: ["连接", "调试", "结果"], reviews: [{ author: "林一", stars: 5, date: "3 天前", text: "把模型和凭据状态放在同一页，排查连接问题快了很多。", version: "1.0.0" }, { author: "匿名用户", stars: 4, date: "2 周前", text: "希望参数预设能保存成模板。", version: "1.0.0" }], versions: [{ version: "1.1.0", date: "2026-10-28", note: "增加流式返回与错误详情。" }, { version: "1.0.0", date: "2026-09-12", note: "首个随包版本。" }] },
    { id: "canvas", name: "无限画布", publisher: "DGOS", official: true, category: "创作", icon: "grid", tone: "teal", desc: "把模型、技能和工具连成可运行的工作流，适合从想法到交付。", rating: "4.7", ratingCount: "864", downloads: "8.6k", size: "随包", version: "0.1.0", latest: "0.1.0", update: false, installed: true, permissions: ["模型调用", "文件读取"], changelog: "0.1.0：本地创作平台、项目与全屏编辑器。", shots: ["项目", "画布", "节点"], reviews: [{ author: "周野", stars: 5, date: "5 天前", text: "节点连线的反馈很清楚，先把工作流跑通再谈画布细节。", version: "0.1.0" }], versions: [{ version: "0.1.0", date: "2026-10-20", note: "本地创作平台、项目与全屏编辑器。" }] },
    { id: "notes", name: "本地资料整理", publisher: "Northwind Labs", official: false, category: "效率", icon: "file", tone: "amber", desc: "把本地文档整理成可检索的摘要，保留来源和更新时间。", rating: "4.6", ratingCount: "312", downloads: "3.2k", size: "18 MB", version: "1.4.2", latest: "1.4.2", update: false, installed: false, permissions: ["文件读取", "模型调用"], changelog: "1.4.2：修复长文档索引失败。", shots: ["目录", "摘要", "来源"], reviews: [{ author: "陈叙", stars: 5, date: "1 周前", text: "来源和时间戳都保留，整理完还能回溯原始文件。", version: "1.4.2" }, { author: "匿名用户", stars: 4, date: "3 周前", text: "大目录首次索引略慢。", version: "1.4.0" }], versions: [{ version: "1.4.2", date: "2026-10-24", note: "修复长文档索引失败。" }, { version: "1.4.0", date: "2026-09-30", note: "支持按目录批量整理。" }] },
    { id: "meeting", name: "会议纪要", publisher: "Mosaic Studio", official: false, category: "效率", icon: "chat", tone: "green", desc: "从会议转录中提取决定、待办和负责人，输出可分享的纪要。", rating: "4.7", ratingCount: "736", downloads: "8.1k", size: "24 MB", version: "2.1.0", latest: "2.1.0", update: false, installed: false, permissions: ["麦克风", "模型调用"], changelog: "2.1.0：支持按项目归档纪要。", shots: ["录音", "决定", "待办"], reviews: [{ author: "许宁", stars: 5, date: "4 天前", text: "决定和待办分得很清楚，直接贴进任务列表就能用。", version: "2.1.0" }], versions: [{ version: "2.1.0", date: "2026-10-18", note: "支持按项目归档纪要。" }, { version: "2.0.4", date: "2026-09-02", note: "修复长时间录音中断。" }] },
    { id: "data", name: "数据画板", publisher: "Field Notes", official: false, category: "数据", icon: "tool", tone: "purple", desc: "把表格和指标变成可标注的画板，适合复盘和方案讨论。", rating: "4.4", ratingCount: "208", downloads: "1.9k", size: "12 MB", version: "0.9.3", latest: "0.9.3", update: false, installed: false, permissions: ["文件读取"], changelog: "0.9.3：增加 CSV 预览。", shots: ["导入", "图表", "标注"], reviews: [{ author: "匿名用户", stars: 4, date: "2 周前", text: "标注体验不错，图表类型还比较少。", version: "0.9.3" }], versions: [{ version: "0.9.3", date: "2026-10-09", note: "增加 CSV 预览。" }] },
    { id: "flow", name: "流程便签", publisher: "DGOS", official: true, category: "创作", icon: "skill", tone: "amber", desc: "用便签整理流程、角色和边界，适合工作坊和需求梳理。", rating: "4.5", ratingCount: "421", downloads: "5.6k", size: "8 MB", version: "1.1.0", latest: "1.4.0", update: true, installed: true, permissions: [], changelog: "1.4.0：支持导出为 Markdown 与图片。", shots: ["便签", "分区", "导出"], reviews: [{ author: "沈禾", stars: 5, date: "1 周前", text: "工作坊现场用便签做流程很快，导出 Markdown 后可以直接进文档。", version: "1.1.0" }], versions: [{ version: "1.4.0", date: "2026-10-30", note: "支持导出为 Markdown 与图片。" }, { version: "1.1.0", date: "2026-09-15", note: "支持分区与角色标记。" }] },
    { id: "search", name: "本地检索", publisher: "Grain Works", official: false, category: "效率", icon: "search", tone: "teal", desc: "为本地文档建立轻量索引，按内容和时间快速定位。", rating: "4.3", ratingCount: "164", downloads: "2.4k", size: "15 MB", version: "0.6.2", latest: "0.6.2", update: false, installed: false, permissions: ["文件读取"], changelog: "0.6.2：索引重建不再阻塞界面。", shots: ["索引导入", "结果", "预览"], reviews: [{ author: "匿名用户", stars: 4, date: "3 周前", text: "搜索速度很快，索引占空间略大。", version: "0.6.2" }], versions: [{ version: "0.6.2", date: "2026-10-02", note: "索引重建不再阻塞界面。" }] },
    { id: "term", name: "终端助手", publisher: "Basalt", official: false, category: "开发", icon: "terminal", tone: "green", desc: "在只读沙箱里解释命令输出，不会代替你执行命令。", rating: "4.2", ratingCount: "96", downloads: "1.1k", size: "6 MB", version: "1.0.3", latest: "1.0.3", update: false, installed: false, permissions: ["模型调用"], changelog: "1.0.3：支持粘贴日志后按段解释。", shots: ["粘贴", "解释", "建议"], reviews: [{ author: "匿名用户", stars: 4, date: "1 个月前", text: "解释日志比直接问模型更聚焦，权限说明也清楚。", version: "1.0.3" }], versions: [{ version: "1.0.3", date: "2026-09-25", note: "支持粘贴日志后按段解释。" }] }
  ];
  var categories = ["all", "效率", "创作", "数据", "开发"];
  var sorts = [["recommend", "推荐"], ["rating", "评分"], ["downloads", "下载"], ["updated", "最近更新"]];

  function appById(id) { return apps.find(function (item) { return item.id === id; }); }
  function updateCount() { return apps.filter(function (item) { return item.installed && item.update; }).length; }
  function installedCount() { return apps.filter(function (item) { return item.installed; }).length; }
  function selectedApp() { return appById(selected) || apps[0]; }
  function downloadNumber(value) { return parseFloat(value.replace("k", "")) * (value.indexOf("k") >= 0 ? 1000 : 1); }
  function visibleApps() {
    var needle = query.trim().toLowerCase();
    var list = apps.filter(function (item) {
      if (view === "installed" && !item.installed) return false;
      if (view === "updates" && !(item.installed && item.update)) return false;
      if (category !== "all" && item.category !== category) return false;
      if (origin === "official" && !item.official) return false;
      if (origin === "third" && item.official) return false;
      return !needle || (item.name + " " + item.publisher + " " + item.category + " " + item.desc).toLowerCase().indexOf(needle) >= 0;
    });
    return list.sort(function (a, b) {
      if (sort === "rating") return parseFloat(b.rating) - parseFloat(a.rating);
      if (sort === "downloads") return downloadNumber(b.downloads) - downloadNumber(a.downloads);
      if (sort === "updated") return b.versions[0].date < a.versions[0].date ? -1 : 1;
      return 0;
    });
  }
  function appState(app) { return !app.installed ? "install" : app.update ? "update" : "open"; }
  function button(label, action, cls, attrs) {
    return "<button type='button' class='" + (cls || "secondary ws-small") + "' data-market-action='" + action + "'" + (attrs || "") + ">" + label + "</button>";
  }
  function stateBadge(app) {
    var state = appState(app);
    if (state === "update") return p.badge("可更新 " + app.latest, "warn");
    if (state === "open") return p.badge("已安装", "good");
    return "";
  }
  function cardAction(app) {
    var state = appState(app);
    if (state === "install") return button("安装", "install:" + app.id, "primary ws-small");
    if (state === "update") return button("更新", "update:" + app.id, "primary ws-small");
    return button("打开", "open:" + app.id, "secondary ws-small");
  }
  function marketCard(app) {
    return "<article class='market-card" + (app.id === selected ? " selected" : "") + "'>" +
      "<button type='button' class='market-card-main' data-market-action='select:" + app.id + "'>" +
      "<span class='market-card-head'>" + p.tile(app.icon, app.tone) + "<span class='market-card-title'><strong>" + esc(app.name) + "</strong><small>" + esc(app.publisher) + (app.official ? " · 官方" : "") + "</small></span></span>" +
      "<span class='market-card-desc'>" + esc(app.desc) + "</span>" +
      "<span class='market-card-meta'><span>★ " + esc(app.rating) + " (" + esc(app.ratingCount) + ")</span><span>" + esc(app.downloads) + " 次下载</span><span>v" + esc(app.version) + "</span></span>" +
      "</button><div class='market-card-foot'><span class='market-free'>免费</span><span class='market-category'>" + esc(app.category) + "</span>" + stateBadge(app) + cardAction(app) + "</div></article>";
  }
  function shot(label) {
    return "<span class='market-shot'><i></i><i></i><i></i><small>" + label + "</small></span>";
  }
  function reviewsHtml(app) {
    if (!app.reviews.length) return "<p class='resource-caption'>还没有评价。</p>";
    return app.reviews.map(function (review) {
      return "<div class='market-review'><div class='market-review-head'><strong>" + esc(review.author) + "</strong><span aria-label='" + review.stars + " 星'>" + "★★★★★".slice(0, review.stars) + "☆☆☆☆☆".slice(0, 5 - review.stars) + "</span><small>" + esc(review.date) + " · v" + esc(review.version) + "</small></div><p>" + esc(review.text) + "</p></div>";
    }).join("");
  }
  function versionsHtml(app) {
    var currentIndex = app.versions.findIndex(function (entry) { return entry.version === app.version; });
    return "<ul class='market-versions'>" + app.versions.map(function (entry, index) {
      var current = index === currentIndex;
      var older = currentIndex >= 0 && index > currentIndex;
      return "<li" + (current ? " class='current'" : "") + "><div><strong>v" + esc(entry.version) + "</strong>" + (current ? p.badge("当前", "good") : "") + (!current && index < currentIndex && app.installed && app.update ? p.badge("可更新", "warn") : "") + "<small>" + esc(entry.date) + "</small></div><p>" + esc(entry.note) + "</p>" + (older && app.installed ? button("回滚", "rollback:" + app.id + ":" + entry.version, "secondary ws-small") : "") + "</li>";
    }).join("") + "</ul>";
  }
  function marketDetail() {
    var app = selectedApp();
    if (!app) return "<aside class='market-detail resource-panel'><div class='resource-empty'><strong>没有匹配的应用</strong><p>调整搜索、分类或来源后再试。</p></div></aside>";
    var state = appState(app);
    var action = state === "install" ? button("安装", "install:" + app.id, "primary ws-small") : state === "update" ? button("更新到 " + app.latest, "update:" + app.id, "primary ws-small") : button("打开", "open:" + app.id, "primary ws-small");
    return "<aside class='market-detail resource-panel'>" +
      "<div class='market-detail-head'>" + p.tile(app.icon, app.tone) + "<div class='grow'><div class='row wrap'><h2>" + esc(app.name) + "</h2>" + (app.official ? p.badge("DGOS 官方", "good") : p.badge("第三方")) + "</div><p>" + esc(app.publisher) + " · ★ " + esc(app.rating) + "（" + esc(app.ratingCount) + " 条评价） · " + esc(app.downloads) + " 次下载</p></div></div>" +
      "<p class='market-description'>" + esc(app.desc) + "</p>" +
      "<div class='market-shots'>" + app.shots.map(shot).join("") + "</div>" +
      "<dl class='market-facts'><dt>当前版本</dt><dd>v" + esc(app.version) + (app.update ? " · 可更新到 v" + esc(app.latest) : "") + "</dd><dt>大小</dt><dd>" + esc(app.size) + "</dd><dt>分类</dt><dd>" + esc(app.category) + "</dd><dt>价格</dt><dd>免费 · 市场全部免费</dd></dl>" +
      "<div class='market-section'><h3>权限说明</h3><div class='market-permissions'>" + (app.permissions.length ? app.permissions.map(function (permission) { return "<span class='market-permission'>" + esc(permission) + "</span>"; }).join("") : "<span class='resource-caption'>没有额外权限</span>") + "</div></div>" +
      "<div class='market-section'><h3>更新说明</h3><p class='resource-caption'>" + esc(app.changelog) + "</p></div>" +
      "<div class='market-section'><div class='resource-subhead'><h3>版本历史</h3><small>" + app.versions.length + " 个版本</small></div>" + versionsHtml(app) + "</div>" +
      "<div class='market-section'><div class='resource-subhead'><h3>用户评价</h3>" + button("写评价", "review:" + app.id, "secondary ws-small") + "</div>" + reviewsHtml(app) + "</div>" +
      "<div class='market-detail-foot'>" + (app.installed ? "<p class='resource-caption'>已安装 · v" + esc(app.version) + "</p>" : "<p class='resource-caption'>尚未安装 · 安装前会请求以上权限</p>") +
      "<div class='market-actions'>" + action + (app.installed ? button("卸载", "uninstall:" + app.id, "secondary ws-small") : "") + button("在应用与扩展中管理", "manage", "link") + "</div></div>" +
      "</aside>";
  }
  function emptyState() {
    if (view === "installed") return "<div class='resource-empty'><strong>还没有安装应用</strong><p>从浏览页安装后可以在这里卸载或打开。</p>" + button("浏览应用", "view:browse", "primary ws-small") + "</div>";
    if (view === "updates") return "<div class='resource-empty'><strong>没有可用更新</strong><p>已安装应用都是最新版本；市场会在后台检查更新并提示。</p>" + button("查看已安装", "view:installed", "primary ws-small") + "</div>";
    return "<div class='resource-empty'><strong>没有匹配的应用</strong><p>换一个关键词、分类或来源。</p>" + button("清除筛选", "clear-filters", "secondary ws-small") + "</div>";
  }
  function accountChip() {
    return account ? p.badge("云端账号 · " + account.email, "good") + button("退出演示账号", "logout", "secondary ws-small") : p.badge("未登录 · 仅可浏览", "warn") + button("登录云端账号", "login", "primary ws-small");
  }
  function marketPage() {
    var list = visibleApps();
    if (list.length && !list.some(function (item) { return item.id === selected; })) selected = list[0].id;
    var tabs = "<div class='resource-tabs' role='tablist' aria-label='市场视图'>" + button("浏览", "view:browse", view === "browse" ? "primary ws-small" : "secondary ws-small") + button("已安装 " + installedCount(), "view:installed", view === "installed" ? "primary ws-small" : "secondary ws-small") + button("更新 " + updateCount(), "view:updates", view === "updates" ? "primary ws-small" : "secondary ws-small") + "</div>";
    var filters = "<div class='resource-toolbar market-toolbar'><div class='resource-filters' role='group' aria-label='来源'>" + [["all", "全部来源"], ["official", "DGOS 官方"], ["third", "第三方"]].map(function (item) { return button(item[1], "origin:" + item[0], origin === item[0] ? "primary ws-small" : "secondary ws-small"); }).join("") + "</div><label class='resource-search'>" + p.icon("search") + "<span class='visually-hidden'>搜索应用</span><input id='market-search' placeholder='搜索应用、开发者或用途…' value='" + esc(query) + "'></label><label class='market-sort'>排序<select id='market-sort'>" + sorts.map(function (item) { return "<option value='" + item[0] + "'" + (sort === item[0] ? " selected" : "") + ">" + item[1] + "</option>"; }).join("") + "</select></label></div>";
    var chips = "<div class='resource-chips' role='group' aria-label='应用分类'>" + categories.map(function (item) { return button(item === "all" ? "全部" : item, "category:" + item, category === item ? "primary ws-small" : "secondary ws-small"); }).join("") + "</div>";
    var submissionBadge = submission ? p.badge("1 个提交审核中", "warn") : "";
    return "<div class='resource-page market-page'><header class='resource-head'><div><small>应用生态 · V3 预研</small><h1>应用市场</h1><p>中心化官方市场：浏览不需要账号，下载、提交和评价需要云端账号；全部免费。</p></div><div class='resource-head-actions'>" + submissionBadge + accountChip() + (submission ? button("查看提交", "dev-status", "secondary ws-small") : button("开发者提交", "developer", "secondary ws-small")) + "</div></header><div class='resource-scroll'><div class='resource-content'>" +
      "<div class='market-scope'>D010：市场必须登录云端账号才能下载；本原型只演示浏览、权限确认与状态流转，不连接官方服务器，也不安装真实模块。</div>" +
      tabs + filters + chips +
      "<div class='market-layout'><div class='market-results'>" + (list.length ? "<div class='market-grid'>" + list.map(marketCard).join("") + "</div>" : emptyState()) + "</div>" + marketDetail() + "</div>" +
      "</div></div>" + modalMarkup() + "</div>";
  }
  function modalShell(title, kicker, body, actions) {
    return "<div class='market-modal-layer'><section class='market-modal' role='dialog' aria-modal='true' aria-labelledby='market-modal-title'><header class='market-modal-head'><div><small>" + kicker + "</small><h2 id='market-modal-title'>" + esc(title) + "</h2></div><button type='button' class='icon-button' data-market-action='close-modal' aria-label='关闭'>" + p.icon("close") + "</button></header>" + body + "<div class='market-modal-actions'>" + actions + "</div></section></div>";
  }
  function permissionList(app) {
    return "<div class='market-modal-permissions'><strong>请求权限</strong>" + (app.permissions.length ? app.permissions.map(function (permission) { return "<span>" + p.icon("check") + esc(permission) + "</span>"; }).join("") : "<span>没有额外权限</span>") + "</div>";
  }
  function loginBody() {
    return "<p class='market-modal-copy'>市场下载、开发者提交和评价需要云端账号；浏览保持匿名。这里不接收真实凭据。</p><label class='field'>云端账号<input id='market-email' placeholder='name@example.com' autocomplete='off'></label><p class='resource-caption'>演示账号可直接点“以演示账号继续”。</p>";
  }
  function modalMarkup() {
    if (!modal) return "";
    if (modal.type === "login") {
      return modalShell("登录云端账号", "身份", loginBody(), button("取消", "close-modal", "secondary ws-small") + button("以演示账号继续", "login-submit", "primary ws-small"));
    }
    if (modal.type === "developer" || modal.type === "dev-status") {
      if (modal.type === "dev-status" && submission) {
        return modalShell("审核中：" + submission.name, "开发者", "<p class='market-modal-copy'>人工审核通常需要 3 个工作日。审核期间应用不会出现在市场列表。</p><div class='market-review-checks'><span>" + p.icon("check") + "代码安全性</span><span>" + p.icon("check") + "权限合理性</span><span>" + p.icon("check") + "视觉一致性</span><span>" + p.icon("check") + "功能与文档完整性</span></div><p class='resource-caption'>原型只记录提交状态，不连接审核服务。</p>", button("关闭", "close-modal", "primary ws-small"));
      }
      return modalShell("提交应用到市场", "开发者", "<p class='market-modal-copy'>提交需要云端账号、应用资料、截图和人工审核（D010）。这里只演示表单与审核标准。</p><label class='field'>应用名称<input id='dev-name' placeholder='例如：我的工作流工具'></label><label class='field'>简介<textarea id='dev-desc' rows='3' placeholder='说明应用解决的问题和主要能力'></textarea></label><div class='market-form-row'><label class='field'>分类<select id='dev-category'><option>效率</option><option>创作</option><option>数据</option><option>开发</option></select></label><label class='field'>版本<input id='dev-version' placeholder='1.0.0'></label></div><label class='field'>权限声明<select id='dev-permissions'><option>模型调用</option><option>文件读取</option><option>模型调用 + 文件读取</option><option>没有额外权限</option></select></label><div class='market-shots-upload'><span class='market-shot-upload'>截图 1</span><span class='market-shot-upload'>截图 2</span>" + button("添加截图", "dev-shot", "secondary ws-small") + "</div><div class='market-review-checks'><span>审核标准：代码安全性 · 权限合理性 · 视觉一致性 · 功能与文档完整性</span></div>", button("取消", "close-modal", "secondary ws-small") + button("提交审核", "dev-submit", "primary ws-small"));
    }
    var app = appById(modal.appId);
    if (!app) return "";
    if (modal.type === "login-required") return "";
    if (modal.type === "rollback") {
      return modalShell("回滚 " + app.name + "？", "版本管理", "<p class='market-modal-copy'>回滚会把本地版本切换到 v" + esc(modal.version) + "。应用本地数据可能不兼容，回滚不撤销已经完成的操作。</p><div class='market-facts'><span>当前 v" + esc(app.version) + "</span><span>目标 v" + esc(modal.version) + "</span></div>", button("取消", "close-modal", "secondary ws-small") + button("确认回滚", "confirm-rollback", "danger ws-small"));
    }
    if (modal.type === "review") {
      return modalShell("评价 " + app.name, "评分与评论", "<p class='market-modal-copy'>评价会以云端账号公开显示；原型只保存在演示内存。</p><label class='field'>评分<select id='review-stars'>" + [5, 4, 3, 2, 1].map(function (star) { return "<option value='" + star + "'>" + star + " 星</option>"; }).join("") + "</select></label><label class='field'>评价内容<textarea id='review-text' rows='3' placeholder='说说它在你的工作里解决了什么'></textarea></label>", button("取消", "close-modal", "secondary ws-small") + button("发布评价", "review-submit", "primary ws-small"));
    }
    var uninstall = modal.type === "uninstall";
    var update = modal.type === "update";
    var title = uninstall ? "卸载 " + app.name + "？" : update ? "更新 " + app.name + " 到 " + app.latest + "？" : "安装 " + app.name + "？";
    var copy = uninstall ? "卸载后应用会从本机移除；已有数据是否保留取决于应用自己。卸载不需要云端账号。" : update ? "更新会替换当前版本，可能带来新的权限或行为变化。" : "安装前会请求以下权限；你可以稍后在设置 → 应用与扩展中管理。";
    var body = "<p class='market-modal-copy'>" + esc(copy) + "</p>" + (!uninstall ? permissionList(app) : "") + (update ? "<div class='market-section'><h3>更新说明</h3><p class='resource-caption'>" + esc(app.changelog) + "</p></div>" : "");
    var confirm = uninstall ? button("卸载", "confirm-uninstall", "danger ws-small") : update ? button("确认更新", "confirm-update", "primary ws-small") : button("确认安装", "confirm-install", "primary ws-small");
    return modalShell(title, uninstall ? "应用管理" : "安装前确认", body, button("取消", "close-modal", "secondary ws-small") + confirm);
  }
  function needLogin(pending) {
    if (account) return false;
    modal = { type: "login", pending: pending };
    return true;
  }
  function renderMarket() {
    var main = q("#main");
    if (!main) return;
    var tabs = main.querySelector(".workspace-tabs");
    main.innerHTML = "";
    if (tabs) main.appendChild(tabs);
    main.insertAdjacentHTML("beforeend", marketPage());
    syncNavDot();
  }
  function syncNavDot() {
    var nav = q('[data-ws-action="open-market"]');
    if (!nav) return;
    var dot = nav.querySelector(".market-nav-dot");
    if (updateCount() && !dot) nav.insertAdjacentHTML("beforeend", "<span class='market-nav-dot' aria-hidden='true'></span>");
    if (!updateCount() && dot) dot.remove();
  }

  document.addEventListener("click", function (event) {
    var el = event.target.closest("[data-market-action]");
    if (!el) {
      if (modal && event.target.classList && event.target.classList.contains("market-modal-layer")) { modal = null; p.render(); }
      return;
    }
    event.preventDefault();
    var parts = el.getAttribute("data-market-action").split(":");
    var action = parts[0], value = parts[1], extra = parts[2];
    if (action === "select") { selected = value; p.render(); return; }
    if (action === "view") { view = value; category = "all"; modal = null; p.render(); return; }
    if (action === "category") { category = value; p.render(); return; }
    if (action === "origin") { origin = value; p.render(); return; }
    if (action === "clear-filters") { category = "all"; origin = "all"; query = ""; p.render(); return; }
    if (action === "login") { modal = { type: "login" }; p.render(); return; }
    if (action === "logout") { account = null; p.render(); p.toast("已退出云端账号（演示）。浏览不受影响。"); return; }
    if (action === "login-submit") {
      var field = q("#market-email");
      var email = field && field.value.trim() ? field.value.trim() : "demo@example.dgos";
      account = { email: email };
      var pending = modal.pending;
      modal = null;
      if (pending) {
        if (pending.type === "developer") modal = { type: "developer" };
        else modal = { type: pending.type, appId: pending.appId, version: pending.version };
      }
      p.render(); p.toast("已登录 " + email + "（演示）。");
      return;
    }
    if (action === "install" || action === "update") {
      if (needLogin({ type: action, appId: value })) { p.render(); p.toast("市场下载需要云端账号（D010）。"); return; }
      modal = { type: action, appId: value }; p.render(); return;
    }
    if (action === "uninstall") { modal = { type: "uninstall", appId: value }; p.render(); return; }
    if (action === "rollback") { modal = { type: "rollback", appId: value, version: extra }; p.render(); return; }
    if (action === "review") {
      if (needLogin({ type: "review", appId: value })) { p.render(); p.toast("评价需要云端账号（演示）。"); return; }
      modal = { type: "review", appId: value }; p.render(); return;
    }
    if (action === "developer") {
      if (needLogin({ type: "developer" })) { p.render(); p.toast("开发者提交需要云端账号（演示）。"); return; }
      modal = { type: "developer" }; p.render(); return;
    }
    if (action === "dev-status") { modal = { type: "dev-status" }; p.render(); return; }
    if (action === "dev-shot") { p.toast("截图上传为原型占位，不读取本地图片。"); return; }
    if (action === "dev-submit") {
      var nameField = q("#dev-name");
      var name = nameField && nameField.value.trim() ? nameField.value.trim() : "未命名应用";
      submission = { name: name, status: "review" };
      modal = { type: "dev-status" };
      p.render(); p.toast("已提交审核（演示），通常 3 个工作日内完成人工审核。");
      return;
    }
    if (action === "close-modal") { modal = null; p.render(); return; }
    if (action === "confirm-install" || action === "confirm-update") {
      var target = appById(value);
      if (target) { target.installed = true; target.update = false; target.version = target.latest; }
      modal = null; p.render();
      p.toast(action === "confirm-update" && target ? "已更新到 v" + target.latest + "（演示）。" : "应用已安装（演示），可在应用与扩展中管理。");
      return;
    }
    if (action === "confirm-uninstall") {
      var removed = appById(value);
      if (removed) { removed.installed = false; removed.update = false; }
      modal = null; p.render(); p.toast("应用已卸载（演示）。");
      return;
    }
    if (action === "confirm-rollback") {
      var rolled = appById(modal.appId);
      if (rolled && modal.version) { rolled.version = modal.version; rolled.update = modal.version !== rolled.latest; }
      modal = null; p.render(); p.toast("已回滚到 v" + (rolled ? rolled.version : "") + "（演示）。");
      return;
    }
    if (action === "review-submit") {
      var reviewApp = appById(modal.appId);
      var starsField = q("#review-stars");
      var textField = q("#review-text");
      if (reviewApp) {
        reviewApp.reviews = [{ author: account ? account.email : "匿名用户", stars: Number(starsField ? starsField.value : 5), date: "刚刚", text: (textField && textField.value.trim()) || "只打了分，没有写评价。", version: reviewApp.version }].concat(reviewApp.reviews);
      }
      modal = null; p.render(); p.toast("评价已发布（演示）。");
      return;
    }
    if (action === "manage") { p.openSettings("modules"); return; }
    if (action === "open") {
      var openApp = appById(value);
      if (!openApp) return;
      if (openApp.id === "model") { p.go("page:app"); return; }
      if (openApp.id === "canvas") { p.state.canvasOpen = true; p.state.canvasView = "home"; p.state.canvasFilter = "projects"; p.state.canvasDirty = false; p.state.page = "canvas"; p.render(); return; }
      p.toast("该应用为市场演示，未接入真实模块。");
      return;
    }
  });

  document.addEventListener("input", function (event) {
    if (event.target.id !== "market-search") return;
    query = event.target.value;
    p.render();
    var next = q("#market-search");
    if (next) { next.focus(); next.setSelectionRange(query.length, query.length); }
  });
  document.addEventListener("change", function (event) {
    if (event.target.id !== "market-sort") return;
    sort = event.target.value;
    p.render();
  });
  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape" && modal) { modal = null; p.render(); }
  });

  var base = window.__dgosAfterRender;
  window.__dgosAfterRender = function () {
    if (base) base();
    if (p.state.page === "market") renderMarket();
  };
  if (p.state.page === "market") renderMarket();
})();
