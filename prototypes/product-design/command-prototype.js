/* Design prototype: command palette and desktop app shortcuts. */
(() => {
  "use strict";
  var p = window.__dgosProto;
  if (!p) return;
  var q = function (selector) { return document.querySelector(selector); };
  var esc = p.esc;
  var open = false;
  var query = "";
  var active = 0;
  var returnFocus = null;

  function item(label, group, hint, icon, run) {
    return { label: label, group: group, hint: hint, icon: icon, run: run };
  }
  function allItems() {
    var list = [];
    list.push(item("新建会话", "操作", "打开一个新的 AI 会话", "chat", function () { p.go("new-chat"); }));
    list.push(item("打开应用启动台", "操作", "搜索并打开应用", "grid", function () { var el = q('[data-ws-action="launcher"]'); if (el) el.click(); }));
    list.push(item("打开设置", "操作", "模型、工具、外观与账户", "settings", function () { p.openSettings("providers"); }));
    list.push(item("回到桌面总览", "操作", "最近工作和应用入口", "grid", function () { p.go("page:desktop"); }));
    list.push(item("模型调试台", "应用", "检查模型连接与调用", "model", function () { p.go("page:app"); }));
    list.push(item("无限画布", "应用", "本地创作平台与全屏画布", "grid", function () { p.state.canvasOpen = true; p.state.canvasView = "home"; p.state.page = "canvas"; p.render(); }));
    list.push(item("技能中心", "资源", "浏览、添加和启用技能", "skill", function () { p.state.openSkills = true; p.go("page:skills"); }));
    list.push(item("MCP 管理", "资源", "服务、工具与调用记录", "tool", function () { p.state.openMcp = true; p.state.page = "mcp"; p.render(); }));
    list.push(item("单任务", "应用", "专注执行一个任务", "play", function () { p.state.openTask = true; p.go("page:task"); }));
    if (p.state.canvasOpen) list.push(item("切换到无限画布标签", "标签", "当前已打开", "grid", function () { p.state.page = "canvas"; p.render(); }));
    if (p.state.openSkills) list.push(item("切换到技能中心标签", "标签", "单例资源标签", "skill", function () { p.state.page = "skills"; p.render(); }));
    if (p.state.openMcp) list.push(item("切换到 MCP 管理标签", "标签", "单例资源标签", "tool", function () { p.state.page = "mcp"; p.render(); }));
    if (p.state.openTask) list.push(item("切换到单任务标签", "标签", "任务工作区", "play", function () { p.state.page = "task"; p.render(); }));
    var current = p.state.page;
    if (current === "app") list.push(item("关闭当前应用标签", "标签", "保留后台运行", "close", function () { var el = q('[data-ws-action="close-app"]'); if (el) el.click(); }));
    if (current === "canvas") list.push(item("关闭无限画布标签", "标签", "返回 AI 会话", "close", function () { var el = q('[data-ws-action="close-canvas"]'); if (el) el.click(); }));
    if (current === "task") list.push(item("关闭单任务标签", "标签", "返回 AI 会话", "close", function () { var el = q('[data-ws-action="close-task"]'); if (el) el.click(); }));
    if (current === "skills") list.push(item("关闭技能中心标签", "标签", "返回 AI 会话", "close", function () { var el = q('[data-ws-action="close-skills"]'); if (el) el.click(); }));
    if (current === "mcp") list.push(item("关闭 MCP 管理标签", "标签", "返回 AI 会话", "close", function () { var el = q('[data-ws-action="close-mcp"]'); if (el) el.click(); }));
    return list;
  }
  function filtered() {
    var value = query.trim().toLowerCase();
    return allItems().filter(function (entry) {
      return (entry.label + " " + entry.hint + " " + entry.group).toLowerCase().indexOf(value) >= 0;
    });
  }
  function renderList() {
    var list = filtered();
    if (active >= list.length) active = Math.max(0, list.length - 1);
    var groups = [];
    list.forEach(function (entry, index) {
      if (groups.indexOf(entry.group) < 0) groups.push(entry.group);
      entry.index = index;
    });
    var html = groups.map(function (group) {
      return "<div class='command-group'><small>" + esc(group) + "</small>" + list.filter(function (e) { return e.group === group; }).map(function (e) {
        return "<button type='button' class='command-item " + (e.index === active ? "active" : "") + "' data-command-index='" + e.index + "'><span class='command-icon'>" + p.icon(e.icon) + "</span><span class='command-label'>" + esc(e.label) + "</span><span class='command-hint'>" + esc(e.hint) + "</span></button>";
      }).join("") + "</div>";
    }).join("");
    var results = q("#command-results");
    if (results) results.innerHTML = html || "<p class='command-empty'>没有匹配的命令。</p>";
  }
  function overlay() {
    return "<div class='command-layer'><section class='command-panel' role='dialog' aria-modal='true' aria-label='命令面板'><label class='command-search'>" + p.icon("search") + "<input id='command-input' autocomplete='off' placeholder='搜索应用、资源或动作…' value='" + esc(query) + "'></label><div id='command-results' class='command-results'></div><footer class='command-foot'><span>↑↓ 选择 · Enter 执行</span><span>Esc 关闭</span></footer></section></div>";
  }
  function openCommand() {
    if (open) return;
    open = true;
    returnFocus = document.activeElement;
    query = "";
    active = 0;
    document.body.insertAdjacentHTML("beforeend", overlay());
    renderList();
    setTimeout(function () { var input = q("#command-input"); if (input) { input.focus(); input.select(); } }, 0);
  }
  function closeCommand() {
    if (!open) return;
    open = false;
    var layer = q(".command-layer");
    if (layer) layer.remove();
    if (returnFocus && returnFocus.isConnected) returnFocus.focus();
  }
  function execute(entry) {
    if (!entry) return;
    closeCommand();
    entry.run();
  }
  function injectTriggers() {
    var actions = q(".workspace-tabs-actions");
    if (actions && !actions.querySelector(".command-trigger")) {
      actions.insertAdjacentHTML("beforeend", "<button type='button' class='workspace-tool command-trigger' data-command-open aria-label='打开命令面板' title='命令面板 · ⌘K'>" + p.icon("search") + "</button>");
    }
    if (p.state.page === "desktop") {
      var intro = q(".desktop-intro");
      if (intro && !intro.querySelector(".command-desktop-trigger")) {
        intro.insertAdjacentHTML("beforeend", "<button type='button' class='secondary command-desktop-trigger' data-command-open>命令面板 · ⌘K</button>");
      }
      var content = q(".desktop-content");
      if (content && !content.querySelector(".desktop-command-apps")) {
        var running = [];
        if (p.state.appRunning) running.push("模型调试台");
        if (p.state.canvasOpen) running.push("无限画布");
        if (p.state.openTask) running.push("单任务");
        var runtime = "<div class='desktop-runtime'><small>运行中</small><span>" + (running.length ? running.join(" · ") : "暂无运行中的工作") + "</span></div>";
        var fixedNames = [];
        Array.prototype.forEach.call(document.querySelectorAll(".sidebar [data-ws-action^='open-app:']"), function (button) { var label = button.querySelector(".nav-label"); if (label) fixedNames.push(label.textContent.trim()); });
        var fixed = "<div class='desktop-fixed'><small>固定应用</small><span>" + (fixedNames.length ? fixedNames.join(" · ") : "还没有固定应用") + "</span></div>";
        var chats = (p.chats || []).slice(-3).reverse();
        var recent = "<div class='desktop-recent'><small>最近会话</small><span>" + (chats.length ? chats.map(function (chat) { return "<button type='button' data-command-chat='" + chat.id + "'>" + p.esc(chat.title) + "</button>"; }).join("") : "还没有会话") + "</span></div>";
        var apps = "<section class='desktop-command-apps'><header class='row spread'><h2>应用与运行</h2><button type='button' class='link' data-command-open>全部应用</button></header><div class='desktop-command-grid'>" +
          "<button type='button' class='desktop-command-app' data-command-app='model'>" + p.tile("model", "purple") + "<span><strong>模型调试台</strong><small>应用 · 已启用</small></span></button>" +
          "<button type='button' class='desktop-command-app' data-command-app='canvas'>" + p.tile("grid", "teal") + "<span><strong>无限画布</strong><small>应用 · 本地项目</small></span></button>" +
          "<button type='button' class='desktop-command-app' data-command-app='skills'>" + p.tile("skill", "amber") + "<span><strong>技能中心</strong><small>资源 · " + p.state.installed.length + " 个已添加</small></span></button>" +
          "<button type='button' class='desktop-command-app' data-command-app='mcp'>" + p.tile("tool", "green") + "<span><strong>MCP 管理</strong><small>资源 · 服务与工具</small></span></button>" +
          "</div>" + runtime + fixed + recent + "</section>";
        var boards = content.querySelector(".desktop-boards");
        if (boards) boards.insertAdjacentHTML("beforebegin", apps); else content.insertAdjacentHTML("beforeend", apps);
      }
    }
  }
  function appAction(id) {
    if (id === "model") { p.go("page:app"); return; }
    if (id === "canvas") { p.state.canvasOpen = true; p.state.canvasView = "home"; p.state.page = "canvas"; p.render(); return; }
    if (id === "skills") { p.state.openSkills = true; p.go("page:skills"); return; }
    if (id === "mcp") { p.state.openMcp = true; p.state.page = "mcp"; p.render(); return; }
  }

  document.addEventListener("click", function (event) {
    var openEl = event.target.closest("[data-command-open]");
    if (openEl) { event.preventDefault(); openCommand(); return; }
    var appEl = event.target.closest("[data-command-app]");
    if (appEl) { event.preventDefault(); appAction(appEl.getAttribute("data-command-app")); return; }
    var chatEl = event.target.closest("[data-command-chat]");
    if (chatEl) { event.preventDefault(); p.go("chat-tab:" + chatEl.getAttribute("data-command-chat")); return; }
    var commandEl = event.target.closest("[data-command-index]");
    if (commandEl) { event.preventDefault(); execute(filtered()[Number(commandEl.getAttribute("data-command-index"))]); return; }
    if (open && event.target.classList && event.target.classList.contains("command-layer")) closeCommand();
  });

  document.addEventListener("input", function (event) {
    if (event.target.id !== "command-input") return;
    query = event.target.value;
    active = 0;
    renderList();
  });

  document.addEventListener("keydown", function (event) {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      if (open) closeCommand(); else openCommand();
      return;
    }
    if (!open) return;
    if (event.key === "Escape") { event.preventDefault(); closeCommand(); return; }
    var list = filtered();
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!list.length) return;
      active = (active + (event.key === "ArrowDown" ? 1 : -1) + list.length) % list.length;
      renderList();
      var activeEl = q(".command-item.active");
      if (activeEl) activeEl.scrollIntoView({ block: "nearest" });
      return;
    }
    if (event.key === "Enter") { event.preventDefault(); execute(list[active]); }
  }, true);

  var base = window.__dgosAfterRender;
  window.__dgosAfterRender = function () {
    if (base) base();
    injectTriggers();
  };
  injectTriggers();
})();
