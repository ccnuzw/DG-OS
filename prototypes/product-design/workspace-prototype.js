/* Design prototype workspace shell. No host API is called. */
(() => {
  "use strict";
  var p = window.__dgosProto;
  if (!p) return;
  var apps = [
    { id: "model", name: "模型调试台", icon: "model", tone: "violet", enabled: true, permission: true },
    { id: "canvas", name: "画布", icon: "grid", tone: "teal", enabled: true, permission: true },
    { id: "notes", name: "资料整理", icon: "file", tone: "amber", enabled: true, permission: false },
    { id: "code", name: "代码阅读", icon: "search", tone: "slate", enabled: false, permission: true },
    { id: "data", name: "数据看板", icon: "grid", tone: "blue", enabled: false, permission: true },
    { id: "voice", name: "语音记录", icon: "chat", tone: "pink", enabled: false, permission: true },
    { id: "auto", name: "自动整理", icon: "skill", tone: "purple", enabled: false, permission: true },
    { id: "tools", name: "工具连接", icon: "tool", tone: "green", enabled: true, permission: true }
  ];
  var appOpen = false;
  var focus = false;
  var launcherOpen = false;
  var launcherQuery = "";
  var tabMenuOpen = false;
  var pinned = ["model"];
  var trigger = null;
  var dragAppId = null;
  var pinHoldTimer = null;
  var pinSuppressClick = false;
  var q = function (selector) { return document.querySelector(selector); };
  var esc = p.esc;

  function page() { return p.state.page; }
  function appById(id) { return apps.find(function (item) { return item.id === id; }); }
  function isEnabled(item) { return item.id === "model" ? p.state.enabled : item.enabled; }
  function hasPermission(item) { return item.id === "model" ? p.state.granted : item.permission; }
  function isUsable(item) { return isEnabled(item) && hasPermission(item); }
  function dotClass(item) {
    if (item.id === "model" && appOpen) return "running";
    return "";
  }
  function dotLabel(item) {
    if (item.id === "model" && appOpen) return "运行中";
    if (!isEnabled(item)) return "未启用";
    if (!hasPermission(item)) return "待授权";
    return "已启用";
  }

  function navItem(label, action, iconName, tone, active) {
    return "<button type='button' class='nav-item " + (active ? "active" : "") + "' data-ws-action='" + action + "' title='" + esc(label) + "'" + (active ? " aria-current='page'" : "") + ">" + p.tile(iconName, tone) + "<span class='nav-label'>" + esc(label) + "</span></button>";
  }

  function launchItem(item) {
    var usable = isUsable(item);
    var dot = dotClass(item);
    var action = usable && (item.id === "model" || item.id === "canvas") ? "open-app:" + item.id : "unavailable:" + item.id;
    var title = item.name + " · " + dotLabel(item) + " · 拖到左侧固定";
    return "<button type='button' class='launch-item " + (usable ? "" : "blocked") + "' draggable='" + (usable ? "true" : "false") + "' data-ws-drag-app='" + item.id + "' data-ws-action='" + action + "' title='" + esc(title) + "' aria-label='" + esc(item.name + "，" + dotLabel(item)) + "'>" +
      "<span class='launch-icon tone-" + item.tone + "'>" + p.icon(item.icon) + (dot ? "<span class='launch-dot " + dot + "' aria-hidden='true'></span>" : "") + "</span>" +
      "<span class='launch-label'>" + esc(item.name) + "</span>" +
      "</button>";
  }

  function launcherResults() {
    var query = launcherQuery.trim().toLowerCase();
    var list = apps.filter(function (item) { return item.name.toLowerCase().indexOf(query) >= 0; });
    if (!list.length) return "<p class='launch-empty'>没有匹配的应用。</p>";
    return list.map(launchItem).join("");
  }

  function workspaceTab(id, label, iconName, selected, closeAction, extra) {
    var close = closeAction ? "<button type='button' class='workspace-tab-close' data-ws-action='" + closeAction + "' aria-label='关闭" + esc(label) + "'>" + p.icon("close") + "</button>" : "";
    return "<div class='workspace-tab" + (selected ? " selected" : "") + "'><button type='button' class='workspace-tab-main' data-ws-action='" + id + "' role='tab' aria-selected='" + selected + "'>" + p.icon(iconName) + "<span class='workspace-tab-label'>" + esc(label) + "</span>" + (extra || "") + "</button>" + close + "</div>";
  }
  function workspaceTabMenu() {
    var current = page();
    var rows = [];
    function row(label, iconName, action, closeAction, active) {
      rows.push("<div class='ws-tab-menu-row " + (active ? "active" : "") + "'><button type='button' class='ws-tab-menu-main' role='menuitem' data-ws-action='" + action + "'>" + p.icon(iconName) + "<span>" + label + "</span>" + (active ? "<small>当前</small>" : "") + "</button>" + (closeAction ? "<button type='button' class='ws-tab-menu-close' data-ws-action='" + closeAction + "' aria-label='关闭" + label + "'>" + p.icon("close") + "</button>" : "") + "</div>");
    }
    row("AI 会话", "chat", "open-ai", null, current === "workbench");
    if (appOpen) row(appById("model").name, "model", "open-app:model", "close-app", current === "app");
    if (current === "canvas" || p.state.canvasOpen) row("无限画布", "grid", "open-canvas", "close-canvas", current === "canvas");
    if (current === "market" || p.state.marketOpen) row("应用市场", "shop", "open-market", "close-market", current === "market");
    if (current === "task" || p.state.openTask) row("单任务", "play", "open-task", "close-task", current === "task");
    if (current === "skills" || p.state.openSkills) row("技能中心", "skill", "open-skills", "close-skills", current === "skills");
    if (current === "mcp" || p.state.openMcp) row("MCP 管理", "tool", "open-mcp", "close-mcp", current === "mcp");
    return "<div class='ws-tab-menu' role='menu' aria-label='标签列表'><div class='ws-tab-menu-head'><strong>工作区标签</strong><span>" + rows.length + " 个</span></div>" + rows.join("") + "</div>";
  }

  function injectTabMenu() {
    var old = q(".ws-tab-menu");
    if (old) old.remove();
    if (!tabMenuOpen) return;
    var bar = q(".workspace-tabs");
    if (!bar) return;
    bar.insertAdjacentHTML("beforeend", workspaceTabMenu());
  }

  function workspaceTabs() {
    var current = page();
    if (["workbench", "app", "canvas", "market", "task", "skills", "mcp"].indexOf(current) < 0) return "";
    var appItem = appById("model");
    var aiTab = workspaceTab("open-ai", "AI 会话", "chat", current === "workbench", null, "<span class='ws-session-count'>" + p.chats.length + "</span>");
    var appTab = appOpen ? workspaceTab("open-app:model", appItem.name, "model", current === "app", "close-app", "<span class='ws-running-dot'></span>") : "";
    var canvasTab = (current === "canvas" || p.state.canvasOpen) ? workspaceTab("open-canvas", "无限画布", "grid", current === "canvas", "close-canvas", p.state.canvasDirty ? "<span class='ws-dirty-dot' title='未保存'></span>" : null) : "";
    var marketTab = (current === "market" || p.state.marketOpen) ? workspaceTab("open-market", "应用市场", "shop", current === "market", "close-market", null) : "";
    var taskTab = (current === "task" || p.state.openTask) ? workspaceTab("open-task", "单任务", "play", current === "task", "close-task", null) : "";
    var skillsTab = (current === "skills" || p.state.openSkills) ? workspaceTab("open-skills", "技能中心", "skill", current === "skills", "close-skills", null) : "";
    var mcpTab = (current === "mcp" || p.state.openMcp) ? workspaceTab("open-mcp", "MCP 管理", "tool", current === "mcp", "close-mcp", null) : "";
    return "<div class='workspace-tabs' role='tablist' aria-label='工作区标签'><div class='workspace-tabs-list'>" +
      aiTab + appTab + canvasTab + marketTab + taskTab + skillsTab + mcpTab + "</div><div class='workspace-tabs-actions'><span class='workspace-demo'>交互演示</span><button type='button' class='icon-button workspace-tool' data-ws-action='launcher' aria-label='打开应用程序' title='打开应用程序'>" + p.icon("grid") + "</button><button type='button' class='icon-button workspace-tool ws-tab-menu-trigger' data-ws-action='tab-menu' aria-label='标签列表' title='标签列表'>" + p.icon("collapse") + "</button>" +
      (current === "app" ? "<button type='button' class='workspace-focus-button' data-ws-action='toggle-focus' aria-pressed='" + focus + "'>" + p.icon("collapse") + "<span>" + (focus ? "退出专注" : "最大化") + "</span></button><button type='button' class='workspace-tool' data-ws-action='manage-all' aria-label='管理应用' title='管理应用'>" + p.icon("settings") + "</button>" : "") +
      "</div></div>";
  }

  function launcher() {
    return "<div class='launchpad-layer'><section class='launchpad-panel' role='dialog' aria-modal='false' aria-labelledby='launchpad-title'>" +
      "<header class='launchpad-head'><div><small>工作区</small><h2 id='launchpad-title'>应用程序</h2></div><label class='launchpad-search'>" + p.icon("search") + "<input id='launcher-search' aria-label='搜索应用' placeholder='搜索' value='" + esc(launcherQuery) + "' autocomplete='off'></label></header>" +
      "<div class='launchpad-scroll'><div id='launcher-results' class='launchpad-grid'>" + launcherResults() + "</div><p class='launchpad-hint'>拖拽或聚焦按 P 固定 · 触屏长按固定 · 点击外部或 Esc 关闭</p></div>" +
      "</section></div>";
  }

  function injectNav() {
    var sidebar = q(".sidebar");
    if (!sidebar) return;
    var legacy = sidebar.querySelector('[data-action="page:app"]');
    if (legacy) legacy.remove();
    var task = sidebar.querySelector('[data-action="page:task"]');
    if (task) task.remove();
    var labels = sidebar.querySelectorAll('.eyeline');
    for (var i = 0; i < labels.length; i += 1) {
      if ((labels[i].textContent || '').trim() === '测试工具') labels[i].remove();
    }
    var oldEyeline = sidebar.querySelector('.ws-app-eyeline');
    if (oldEyeline) oldEyeline.remove();
    var oldLauncher = sidebar.querySelector('[data-ws-action="launcher"]');
    if (oldLauncher) oldLauncher.remove();
    var oldPinned = sidebar.querySelectorAll('[data-ws-action^="open-app:"]');
    for (var j = 0; j < oldPinned.length; j += 1) oldPinned[j].remove();
    // The generated app group is re-inserted on every afterRender; drop the previous
    // copies first or the sidebar accumulates duplicate 应用/应用市场 entries.
    var oldMarket = sidebar.querySelectorAll('[data-ws-action="open-market"], [data-ws-action="page:market"]');
    for (var k = 0; k < oldMarket.length; k += 1) oldMarket[k].remove();
    var skills = sidebar.querySelector('[data-action="page:skills"]');
    window.__wsDebugState = { skills: !!skills };
    if (!skills) return;
    var pinnedItems = pinned.map(appById).filter(function (item) { return item && isUsable(item); }).map(function (item) {
      return navItem(item.name, "open-app:" + item.id, item.icon, item.tone, page() === "app" && item.id === "model");
    }).join("");
    // Keep the application group after the MCP entry. Inserting it directly after
    // 技能中心 made MCP jump below 应用 whenever the launcher refreshed the nav.
    var appAnchor = sidebar.querySelector('[data-ws-action="mcp-page"]') || skills;
    window.__wsDebugState.anchor = !!appAnchor;
    window.__wsDebugState.pinned = pinnedItems.length;
    appAnchor.insertAdjacentHTML("afterend", "<div class='eyeline ws-app-eyeline'>应用</div>" + navItem("应用", "launcher", "grid", "purple", page() === "app" || launcherOpen) + navItem("应用市场", "open-market", "shop", "teal", page() === "market") + pinnedItems);
  }

  function injectTabs() {
    tabMenuOpen = false;
    var main = q("#main");
    if (!main) return;
    if (page() === "app") appOpen = true;
    if (page() === "market") p.state.marketOpen = true;
    if (page() !== "app" && focus) focus = false;
    var tabbed = ["workbench", "app", "canvas", "market", "task", "skills", "mcp"].indexOf(page()) >= 0;
    main.classList.toggle("ws-tabs-active", tabbed);
    main.classList.toggle("ws-app-page", page() === "app");
    var old = main.querySelector(".workspace-tabs");
    if (old) old.remove();
    if (tabbed) main.insertAdjacentHTML("afterbegin", workspaceTabs());
    var shell = q(".shell");
    if (shell) {
      shell.classList.toggle("ws-focus", focus && page() === "app");
      shell.classList.toggle("ws-canvas-mode", page() === "canvas");
    }
  }

  function injectLauncher() {
    var old = q(".launchpad-layer");
    if (old) old.remove();
    var main = q("#main");
    if (launcherOpen && p.state.scene === "normal" && main) main.insertAdjacentHTML("beforeend", launcher());
  }

  function afterRender() {
    if (!q("#main")) return;
    injectNav();
    injectTabs();
    injectLauncher();
  }

  function openLauncher() {
    trigger = document.activeElement;
    focus = false;
    launcherOpen = true;
    afterRender();
    setTimeout(function () {
      var input = q("#launcher-search");
      if (input) { input.focus(); input.select(); }
    }, 0);
  }

  function closeLauncher() {
    launcherOpen = false;
    afterRender();
    var next = q('[data-ws-action="launcher"]');
    if (next) next.focus();
    else if (trigger && trigger.isConnected) trigger.focus();
  }

  function openApp(id) {
    var item = appById(id);
    if (!item) return;
    if (!isUsable(item)) { p.toast("该应用尚未启用或授权（演示）。"); return; }
    if (item.id === "canvas") {
      p.state.canvasOpen = true;
      p.state.canvasView = "home";
      p.state.canvasFilter = "projects";
      p.state.canvasDirty = false;
      p.state.page = "canvas";
      p.state.scene = "normal";
      focus = false;
      launcherOpen = false;
      p.render();
      return;
    }
    if (item.id !== "model") { p.toast("该应用为演示图标，尚未接入。"); return; }
    appOpen = true;
    focus = false;
    launcherOpen = false;
    p.go("page:app");
  }

  function closeApp() {
    appOpen = false;
    focus = false;
    launcherOpen = false;
    var layer = q(".ws-close-layer");
    if (layer) layer.remove();
    if (page() === "app") p.go("page:workbench");
    else afterRender();
  }

  function requestCloseApp() {
    if (page() === "app" && p.state.appRunning) {
      var existing = q(".ws-close-layer");
      if (existing) existing.remove();
      document.body.insertAdjacentHTML("beforeend", "<div class='ws-close-layer'><section class='ws-close-dialog' role='dialog' aria-modal='true' aria-labelledby='ws-close-app-title'><h2 id='ws-close-app-title'>应用仍在运行</h2><p>关闭标签不会停止后台运行；再次打开时可以继续查看结果。</p><div class='row'><button type='button' class='secondary' data-ws-action='cancel-close-app'>继续查看</button><button type='button' class='primary' data-ws-action='confirm-close-app'>关闭标签</button></div></section></div>");
      var first = q(".ws-close-dialog button");
      if (first) first.focus();
      return;
    }
    closeApp();
  }

  function confirmCloseApp() {
    p.state.appRunning = false;
    closeApp();
  }

  function closeCanvas() {
    p.state.canvasOpen = false;
    p.state.canvasDirty = false;
    var layer = q(".ws-close-layer");
    if (layer) layer.remove();
    if (page() === "canvas") p.go("page:workbench");
    else afterRender();
  }

  function confirmCloseCanvas() {
    var layer = q(".ws-close-layer");
    if (layer) layer.remove();
    p.state.canvasDirty = false;
    closeCanvas();
  }

  function requestCloseCanvas() {
    if (page() === "canvas" && p.state.canvasView === "editor" && p.state.canvasDirty) {
      var existing = q(".ws-close-layer");
      if (existing) existing.remove();
      document.body.insertAdjacentHTML("beforeend", "<div class='ws-close-layer'><section class='ws-close-dialog' role='dialog' aria-modal='true' aria-labelledby='ws-close-title'><h2 id='ws-close-title'>关闭无限画布？</h2><p>当前画布还有未保存的修改；关闭后这些修改会丢失。</p><div class='row'><button type='button' class='secondary' data-ws-action='cancel-close-canvas'>继续编辑</button><button type='button' class='primary' data-ws-action='confirm-close-canvas'>放弃并关闭</button></div></section></div>");
      var first = q(".ws-close-dialog button");
      if (first) first.focus();
      return;
    }
    closeCanvas();
  }

  function toggleFocus() {
    if (page() !== "app") return;
    focus = !focus;
    afterRender();
    p.toast(focus ? "已进入工作区专注模式（演示）。" : "已退出专注模式（演示）。");
  }

  function pinApp(id) {
    var item = appById(id);
    if (!item) return;
    if (!isUsable(item)) { p.toast("该应用需要先启用或授权（演示）。"); return; }
    if (pinned.indexOf(id) >= 0) { p.toast("该应用已固定在左侧。"); return; }
    if (pinned.length >= 3) { p.toast("固定应用最多 3 个。"); return; }
    pinned.push(id);
    afterRender();
    var input = q("#launcher-search");
    if (input) input.focus();
    p.toast(item.name + " 已固定到左侧。");
  }

  function clearPinHold() {
    if (pinHoldTimer) clearTimeout(pinHoldTimer);
    pinHoldTimer = null;
  }

  function clearDrag() {
    dragAppId = null;
    document.body.classList.remove("ws-dragging-app");
    var sidebar = q(".sidebar");
    if (sidebar) sidebar.classList.remove("ws-pin-target");
    var hint = q(".ws-pin-hint");
    if (hint) hint.remove();
    document.querySelectorAll(".launch-item.dragging").forEach(function (el) { el.classList.remove("dragging"); });
  }

  function startDrag(event) {
    var item = event.target.closest(".launch-item");
    if (!item) return;
    dragAppId = item.getAttribute("data-ws-drag-app");
    if (!dragAppId) return;
    var app = appById(dragAppId);
    if (!app || !isUsable(app)) { event.preventDefault(); dragAppId = null; p.toast("该应用需要先启用或授权（演示）。"); return; }
    item.classList.add("dragging");
    document.body.classList.add("ws-dragging-app");
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "copy";
      event.dataTransfer.setData("text/plain", dragAppId);
    }
    var sidebar = q(".sidebar");
    if (sidebar) {
      sidebar.classList.add("ws-pin-target");
      if (!q(".ws-pin-hint")) sidebar.insertAdjacentHTML("afterbegin", "<div class='ws-pin-hint'>拖到这里固定</div>");
    }
  }

  function dropOnSidebar(event) {
    if (!dragAppId) return;
    var sidebar = q(".sidebar");
    if (!sidebar || !event.target.closest(".sidebar")) return;
    event.preventDefault();
    pinApp(dragAppId);
    clearDrag();
  }

  document.addEventListener("pointerdown", function (event) {
    if (event.button !== 0) return;
    var item = event.target.closest(".launch-item[data-ws-drag-app]");
    if (!item) return;
    var id = item.getAttribute("data-ws-drag-app");
    var app = appById(id);
    if (!app || !isUsable(app)) return;
    clearPinHold();
    pinHoldTimer = setTimeout(function () {
      pinHoldTimer = null;
      pinSuppressClick = true;
      pinApp(id);
      setTimeout(function () { pinSuppressClick = false; }, 700);
    }, 550);
  });
  document.addEventListener("pointerup", clearPinHold);
  document.addEventListener("pointercancel", clearPinHold);
  document.addEventListener("click", function (event) {
    if (!pinSuppressClick || !event.target.closest(".launch-item")) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    pinSuppressClick = false;
  }, true);
  document.addEventListener("keydown", function (event) {
    if (event.key.toLowerCase() !== "p" || event.metaKey || event.ctrlKey || event.altKey) return;
    var item = event.target.closest(".launch-item[data-ws-drag-app]");
    if (!item) return;
    var id = item.getAttribute("data-ws-drag-app");
    if (!appById(id) || !isUsable(appById(id))) return;
    event.preventDefault();
    pinApp(id);
  });
  document.addEventListener("contextmenu", function (event) {
    if (event.target.closest(".launch-item")) event.preventDefault();
  });
  document.addEventListener("dragstart", startDrag);
  document.addEventListener("dragend", clearDrag);
  document.addEventListener("dragover", function (event) {
    if (!dragAppId) return;
    if (event.target.closest(".sidebar")) {
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    }
  });
  document.addEventListener("drop", dropOnSidebar);

  document.addEventListener("click", function (event) {
    if (launcherOpen && !event.target.closest(".launchpad-panel")) {
      var external = event.target.closest("[data-ws-action]");
      if (!external || external.getAttribute("data-ws-action") === "launcher") { closeLauncher(); return; }
      closeLauncher();
    }
    var el = event.target.closest("[data-ws-action]");
    if (!el) return;
    event.preventDefault();
    var action = el.getAttribute("data-ws-action");
    if (tabMenuOpen && el.closest(".ws-tab-menu")) { tabMenuOpen = false; injectTabMenu(); }
    if (action === "launcher") { openLauncher(); return; }
    if (action === "tab-menu") { tabMenuOpen = !tabMenuOpen; injectTabMenu(); return; }
    if (action === "close-launcher") { closeLauncher(); return; }
    if (action === "open-ai") { focus = false; launcherOpen = false; p.go("page:workbench"); return; }
    if (action === "open-canvas") { var settingsDialog = q("#dialog"); if (settingsDialog && settingsDialog.open) p.closeDialog(); p.state.canvasOpen = true; p.state.page = "canvas"; p.state.scene = "normal"; p.render(); return; }
    if (action === "open-market") { p.state.marketOpen = true; p.state.page = "market"; p.state.scene = "normal"; p.render(); return; }
    if (action === "close-market") { p.state.marketOpen = false; if (page() === "market") p.go("page:workbench"); else afterRender(); return; }
    if (action === "module-update") { p.toast("演示：更新前需要先检查权限变化。"); return; }
    if (action === "close-canvas") { requestCloseCanvas(); return; }
    if (action === "confirm-close-canvas") { confirmCloseCanvas(); return; }
    if (action === "cancel-close-canvas") { var closeLayer = q(".ws-close-layer"); if (closeLayer) closeLayer.remove(); return; }
    if (action === "open-task") { p.state.openTask = true; p.go("page:task"); return; }
    if (action === "open-skills") { p.state.openSkills = true; p.go("page:skills"); return; }
    if (action === "open-mcp") { p.state.openMcp = true; p.go("page:mcp"); return; }
    if (action === "close-skills") { p.state.openSkills = false; if (page() === "skills") p.go("page:workbench"); else afterRender(); return; }
    if (action === "close-mcp") { p.state.openMcp = false; if (page() === "mcp") p.go("page:workbench"); else afterRender(); return; }
    if (action === "close-task") { p.state.openTask = false; if (page() === "task") p.go("page:workbench"); else afterRender(); return; }
    if (action.indexOf("open-app:") === 0) { openApp(action.slice(9)); return; }
    if (action.indexOf("unavailable:") === 0) {
      var item = appById(action.slice(12));
      if (item) p.toast(item.name + "：" + dotLabel(item) + "（演示）。");
      return;
    }
    if (action === "close-app") { requestCloseApp(); return; }
    if (action === "confirm-close-app") { confirmCloseApp(); return; }
    if (action === "cancel-close-app") { var appLayer = q(".ws-close-layer"); if (appLayer) appLayer.remove(); return; }
    if (action === "toggle-focus") { toggleFocus(); return; }
    if (action === "manage-all") {
      launcherOpen = false;
      injectLauncher();
      p.openSettings("modules");
      return;
    }
  });

  document.addEventListener("click", function (event) {
    if (!tabMenuOpen) return;
    if (event.target.closest(".ws-tab-menu") || event.target.closest('[data-ws-action="tab-menu"]')) return;
    tabMenuOpen = false;
    injectTabMenu();
  });

  document.addEventListener("input", function (event) {
    if (event.target.id !== "launcher-search") return;
    launcherQuery = event.target.value;
    var results = q("#launcher-results");
    if (results) results.innerHTML = launcherResults();
  });

  document.addEventListener("keydown", function (event) {
    if (event.key !== "Escape") return;
    if (tabMenuOpen) { event.preventDefault(); tabMenuOpen = false; injectTabMenu(); return; }
    if (launcherOpen) { event.preventDefault(); closeLauncher(); return; }
    if (focus) { event.preventDefault(); focus = false; afterRender(); }
  });

  window.__dgosAfterRender = afterRender;
  window.__wsDebug = { injectNav: injectNav, injectTabs: injectTabs, afterRender: afterRender };
  afterRender();
})();
