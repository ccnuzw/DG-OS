// Isolated design data: no host invocation, credentials, persistence, or model requests.
const paths = {
  terminal: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="m7 8 4 4-4 4m6 0h4"/>',
  paperclip: '<path d="m9 17 8-8a3 3 0 0 0-4-4L4 14a5 5 0 0 0 7 7L21 11M7 16l8-8"/>',
  settings: '<path d="m9 3-1 3-3 1 1 3-2 2 2 2-1 3 3 1 1 3h4l1-3 3-1-1-3 2-2-2-2 1-3-3-1-1-3z"/><circle cx="11" cy="12" r="3"/>',
  shield: '<path d="M12 3 4 7v6c0 4 5 7 8 9 3-2 8-5 8-9V7z"/>',
  robot: '<rect x="5" y="7" width="14" height="13" rx="3"/><path d="M12 3v4M3 11v5m18-5v5M9 11v3m6-3v3m-6 3h6"/>',
  clipboard: '<rect x="5" y="4" width="14" height="17" rx="2"/><rect x="9" y="2" width="6" height="4" rx="1"/><path d="M9 11h6m-6 5h6"/>',
  bulb: '<path d="M9 18h6m-6 3h6M8 13a6 6 0 1 1 8 0c-2 1-2 3-2 3h-4s0-2-2-3Z"/>',
  bolt: '<path d="m13 2-9 12h7l-1 8 10-13h-7z"/>',
  brain: '<path d="M12 5c-3-5-8-1-7 3-5 1-4 7-1 8-1 5 5 7 8 3m0-14c3-5 8-1 7 3 5 1 4 7 1 8 1 5-5 7-8 3V5Zm-5 3 2 2m-5 6 4-1m9-7-2 2m5 6-4-1"/>',
  cpu: '<rect x="5" y="5" width="14" height="14" rx="4"/><rect x="9" y="9" width="6" height="6" rx="1"/><path d="M9 2v3m6-3v3M9 19v3m6-3v3M2 9h3m-3 6h3m14-6h3m-3 6h3"/>',
  spark: '<path d="m12 2 2 7 8 3-8 2-2 8-2-8-8-2 8-3z"/>',
  server: '<rect x="3" y="3" width="18" height="7" rx="2"/><rect x="3" y="14" width="18" height="7" rx="2"/><path d="M7 6h.01M7 17h.01"/>',
  key: '<circle cx="15" cy="8" r="5"/><path d="m11 12-8 8v-4l3-1v-3h4"/>',
  search: '<circle cx="10" cy="10" r="7"/><path d="m15 15 6 6"/>',
  check: '<path d="m5 12 4 4L20 5"/>',
  chevron: '<path d="m8 10 4 4 4-4"/>',
  right: '<path d="m10 6 6 6-6 6"/>',
  'arrow-up': '<path d="M12 20V4m-6 6 6-6 6 6"/>',
  folder: '<path d="M3 6a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>',
  branch: '<circle cx="6" cy="5" r="3"/><circle cx="18" cy="5" r="3"/><circle cx="6" cy="19" r="3"/><path d="M6 8v8m12-8c0 7-12 4-12 8"/>',
  sliders: '<path d="M3 6h6m4 0h8M3 12h12m4 0h2M3 18h2m4 0h12M9 3v6m6 0v6M5 15v6"/>',
  language: '<path d="M3 5h12M9 2v3M5 5c0 7 8 11 8 11m0-11c0 5-5 9-10 11m11 5 4-12 4 12m-6-4h4"/>',
  moon: '<path d="M20 13A9 9 0 1 1 11 3a7 7 0 0 0 9 10Z"/>',
  feather: '<path d="M4 20 17 7M5 17C-1 8 14-2 20 4c6 6-5 17-15 13Zm5-5h7"/>',
  file: '<path d="M5 3h10l4 4v14H5zm9 0v5h5m-7 3v6m-3-3h6"/>',
  stream: '<path d="M7 4H4v16h3m10-16h3v16h-3M2 12h8m4 0h8M12 2v20"/>',
  code: '<path d="m7 7-5 5 5 5m10-10 5 5-5 5M14 3l-4 18"/>',
  gauge: '<path d="M4 19a10 10 0 1 1 16 0M12 12l5-5"/>',
  layers: '<path d="m12 3 10 5-10 5L2 8zm-10 9 10 5 10-5M2 16l10 5 10-5"/>',
};
const icon = name => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.cpu}</svg>`;
const $ = selector => document.querySelector(selector);
const popover = $('#popover');
const modes = [
  { name: '手动确认', icon: 'shield', detail: '手动确认每项操作，适合谨慎使用' },
  { name: '自动编辑', icon: 'robot', detail: '自动批准文件编辑，其余操作仍需确认' },
  { name: '规划', icon: 'clipboard', detail: '先生成计划，确认后再执行' },
  { name: '深度', icon: 'bulb', detail: '规划＋头脑风暴，深入理解意图后再制定方案' },
  { name: '全自动', icon: 'bolt', detail: '自动批准工具操作【谨慎使用】' },
];
const efforts = ['关', '低', '中', '高', '超高', '极高'];
const platforms = [
  { name: 'Bytro', icon: 'terminal', providers: ['Bytro'], models: ['Auto', 'GPT-6.1 Sol', 'Claude Opus'] },
  { name: 'Anthropic', icon: 'spark', providers: ['Anthropic', 'Anthropic 备用'], models: ['Claude Opus', 'Claude Sonnet', 'Claude Haiku'] },
  { name: 'OpenAI', icon: 'cpu', providers: ['Openai-Gpt', 'OpenAI 备用'], models: ['GPT-6.1 Sol', 'GPT-6.1 Terra', 'GPT-6 Astra', 'GPT-6 Sol', 'GPT-6 Luna', 'GPT-5.6 Sol'] },
  { name: 'Local', icon: 'server', providers: ['本地服务'], models: ['Qwen Coder', 'DeepSeek R1'] },
];
const state = { panel: null, mode: 0, effort: 4, platform: 2, provider: 'Openai-Gpt', model: 'GPT-6.1 Sol', language: '中文', theme: '浅色', concise: '标准', suggestions: true, follow: true, stream: true, highlight: true, usage: false, compressed: false, files: [] };
let trigger = null;
let noticeTimer;
let returnPanel = null;
function hydrateIcons(root = document) { root.querySelectorAll('[data-icon]').forEach(node => { node.innerHTML = icon(node.dataset.icon); }); }
function announce(message) {
  clearTimeout(noticeTimer);
  $('#notice').textContent = message;
  $('#notice').hidden = false;
  noticeTimer = setTimeout(() => { $('#notice').hidden = true; }, 3200);
}
function updateToolbar() {
  const mode = modes[state.mode];
  $('#mode-icon').innerHTML = icon(mode.icon);
  $('#mode-label').textContent = mode.name;
  $('#permission-trigger').classList.toggle('danger', state.mode === 4);
  $('#permission-trigger').setAttribute('aria-label', `运行模式：${mode.name}`);
  $('#model-label').textContent = state.model;
  $('#effort-label').textContent = efforts[state.effort];
  $('#reasoning-trigger').setAttribute('aria-label', `推理强度：${efforts[state.effort]}`);
  $('#reasoning-trigger').title = `推理强度：${efforts[state.effort]}`;
  $('#usage-ring').classList.toggle('filled', state.usage);
}
function closePanel(restore = true) {
  popover.hidden = true;
  document.querySelectorAll('[data-panel]').forEach(button => button.setAttribute('aria-expanded', 'false'));
  state.panel = null;
  if (restore && trigger?.isConnected) trigger.focus({ preventScroll: true });
}
function positionPanel() {
  if (popover.hidden || !trigger) return;
  const box = trigger.getBoundingClientRect();
  const width = popover.offsetWidth;
  const height = popover.offsetHeight;
  const desiredLeft = state.panel === 'settings' ? box.left - 20 : box.right - width + 8;
  const left = Math.max(12, Math.min(desiredLeft, innerWidth - width - 12));
  const above = box.top - height - 10;
  const top = above >= 12 ? above : Math.max(12, Math.min(box.bottom + 10, innerHeight - height - 12));
  popover.style.left = `${left}px`;
  popover.style.top = `${top}px`;
}
function openPanel(name, focus = true) {
  if (state.panel === name && !popover.hidden) { closePanel(); return; }
  closePanel(false);
  state.panel = name;
  trigger = $(`#${name}-trigger`);
  trigger.setAttribute('aria-expanded', 'true');
  popover.className = `popover ${name}-panel`;
  popover.setAttribute('role', 'dialog');
  popover.setAttribute('aria-label', { permission: '运行模式', model: '选择模型', reasoning: '推理强度', settings: '聊天偏好设置', usage: 'Token 用量' }[name]);
  const views = { permission: permissionView, model: modelView, reasoning: reasoningView, settings: settingsView, usage: usageView };
  popover.innerHTML = views[name]();
  popover.hidden = false;
  positionPanel();
  if (focus) (popover.querySelector('[aria-checked="true"], input, button') || popover).focus({ preventScroll: true });
}
function permissionView() {
  return `<h2 class="popup-heading">运行模式</h2><div role="menu" aria-label="选择运行模式">${modes.map((mode, i) => `<button role="menuitemradio" aria-checked="${state.mode === i}" class="mode-row ${state.mode === i ? 'selected' : ''} ${i === 4 ? 'danger' : ''}" data-mode="${i}">${icon(mode.icon)}<span class="mode-copy"><strong>${mode.name}</strong><small>${mode.detail}</small></span><span class="check">${state.mode === i ? icon('check') : ''}</span></button>`).join('')}</div>`;
}
function modelView() {
  const platform = platforms[state.platform];
  return `<div class="model-layout"><aside class="platforms"><div class="platform-title">平台</div>${platforms.map((item, i) => `<button class="platform ${state.platform === i ? 'active' : ''}" aria-pressed="${state.platform === i}" data-platform="${i}">${icon(item.icon)}${item.name}</button>`).join('')}<button class="platform platform-bottom" data-action="configure">${icon('settings')}配置模型</button></aside><div class="model-main"><div class="model-top"><button class="provider-card" data-action="providers" aria-expanded="false">${icon('key')}<span><strong>${platform.providers.includes(state.provider) ? state.provider : platform.providers[0]}</strong><small>${state.platform === 3 ? '本地连接 · localhost' : 'API 密钥 · 已配置'}</small></span>${icon('chevron')}</button><div class="provider-choices" hidden>${platform.providers.map((provider, i) => `<button data-provider="${i}">${provider}</button>`).join('')}</div><label class="search">${icon('search')}<input id="model-search" placeholder="搜索模型…" aria-label="搜索模型" autocomplete="off"></label></div><div class="model-results" role="listbox" aria-label="模型列表">${modelRows('')}</div></div></div>`;
}
function modelRows(query) {
  const models = platforms[state.platform].models.filter(name => name.toLowerCase().includes(query.toLowerCase()));
  return models.length ? models.map((name, i) => `${i === 0 || name.startsWith('GPT-5.6') ? `<div class="model-group" role="presentation">${name.startsWith('GPT-6') ? 'GPT-6' : name.startsWith('GPT-5.6') ? 'GPT-5.6' : platforms[state.platform].name}</div>` : ''}<button role="option" aria-selected="${name === state.model}" class="model-row ${name === state.model ? 'active' : ''}" data-model="${name}"><span>${name}</span><span class="check">${name === state.model ? icon('check') : ''}</span></button>`).join('') : '<p class="empty">没有匹配的模型</p>';
}
function reasoningView() {
  return `<div class="reasoning-heading"><span>推理</span><strong id="effort-value">${efforts[state.effort]}</strong></div><div class="range-wrap" style="--progress:${state.effort * 20}%"><div class="effort-track"><span class="effort-fill"></span><div class="effort-dots">${efforts.map(() => '<span class="effort-dot"></span>').join('')}</div><span class="effort-thumb"></span></div><input class="effort-range" id="effort-range" type="range" min="0" max="5" step="1" value="${state.effort}" aria-label="推理强度" aria-valuetext="${efforts[state.effort]}"><div class="effort-labels">${['关', '低', '中', '高', '超', '极'].map((name, i) => `<button data-effort="${i}" class="${state.effort === i ? 'active' : ''}" aria-label="${efforts[i]}" aria-pressed="${state.effort === i}">${name}</button>`).join('')}</div></div>`;
}
function setEffort(value) {
  state.effort = Number(value);
  updateToolbar();
  $('.range-wrap').style.setProperty('--progress', `${state.effort * 20}%`);
  $('#effort-value').textContent = efforts[state.effort];
  $('#effort-range').value = state.effort;
  $('#effort-range').setAttribute('aria-valuetext', efforts[state.effort]);
  popover.querySelectorAll('[data-effort]').forEach(button => {
    button.classList.toggle('active', Number(button.dataset.effort) === state.effort);
    button.setAttribute('aria-pressed', String(Number(button.dataset.effort) === state.effort));
  });
}
const prefs = [ ['language', 'language', '响应语言'], ['theme', 'moon', '主题'], ['concise', 'feather', '精简模式'] ];
const switches = [ ['suggestions', 'bulb', '下一步提示建议'], ['follow', 'file', '跟随 AI 修改'], ['stream', 'stream', '流式响应'], ['highlight', 'code', '代码高亮'] ];
function settingsRows() {
  return `<div class="setting-rows">${prefs.map(([key, glyph, label]) => `<button class="setting-row" data-pref="${key}">${icon(glyph)}<span>${label}</span><span class="setting-value">${state[key]}${icon('right')}</span></button>`).join('')}${switches.map(([key, glyph, label], i) => `${i === 2 ? '<div class="separator"></div>' : ''}<button class="setting-row" role="switch" aria-checked="${state[key]}" data-toggle="${key}">${icon(glyph)}<span>${label}</span><span class="switch"></span></button>`).join('')}</div>`;
}
function settingsView() {
  return `<h2 class="popup-heading">${icon('sliders')}聊天偏好设置</h2>${settingsRows()}<button class="settings-footer" data-action="full-settings">⌘ / Ctrl + , 打开完整设置</button>`;
}
function showPreference(key, target) {
  const values = { language: ['中文', 'English', '跟随输入'], theme: ['浅色', '深色', '跟随系统'], concise: ['精简', '标准', '详细'] }[key];
  target.innerHTML = `<button class="popup-heading back-heading" data-action="settings-back">‹ ${prefs.find(row => row[0] === key)[2]}</button>${values.map(value => `<button class="preference-option" data-pref-key="${key}" data-pref-value="${value}">${value}<span class="check">${state[key] === value ? icon('check') : ''}</span></button>`).join('')}`;
  if (target === popover) positionPanel();
  target.querySelector('button').focus();
}
function usageView() {
  const used = state.compressed ? '42.1k' : '137.8k';
  const percentage = state.compressed ? 16 : 53;
  return `<h2 class="usage-title">${icon('gauge')}Token 用量</h2><div class="usage-content"><div class="context-card"><div class="context-heading">${icon('layers')}上下文窗口<strong>${percentage}<small> %</small></strong></div><div class="context-progress"><span style="width:${percentage}%"></span></div><div class="context-bottom"><span class="mono">${used} / 258.4k Token</span><button class="compact-button" data-action="compact" ${state.compressed ? 'disabled' : ''}>${state.compressed ? '已压缩' : '压缩'}</button></div></div><dl class="usage-list">${[['总计', '4,807,993', '#2586ed'], ['输入', '254,520', '#377cc8'], ['输出', '25,345', '#68a6e4'], ['缓存读取', '4,528,128', '#20ac85'], ['缓存命中', '94.7%', '#20ac85'], ['总耗时', '2:00:48', '#85877e']].map(([key, value, color]) => `<div class="usage-line" style="--dot:${color}"><dt>${key}</dt><dd>${value}</dd></div>`).join('')}</dl></div>`;
}
function openSettings(provider = false) {
  returnPanel = trigger;
  closePanel(false);
  $('#full-settings-title').textContent = provider ? '模型配置 · 设计预览' : '聊天偏好设置';
  $('#full-settings-content').innerHTML = provider ? `<div class="provider-settings"><h3>${state.provider}</h3><p>模型服务配置沿用 DG-OS 的模型服务设置页；返回时保留消息草稿和模型选择。</p><p>此原型不接收凭据，不连接外部服务。</p></div>` : settingsRows();
  $('#full-settings').showModal();
}
function handleSettings(event, target) {
  const button = event.target.closest('button');
  if (!button) return false;
  if (button.dataset.toggle) {
    const key = button.dataset.toggle;
    state[key] = !state[key];
    button.setAttribute('aria-checked', String(state[key]));
    return true;
  }
  if (button.dataset.pref) { showPreference(button.dataset.pref, target); return true; }
  if (button.dataset.prefKey) {
    state[button.dataset.prefKey] = button.dataset.prefValue;
    document.body.classList.toggle('dark', state.theme === '深色' || state.theme === '跟随系统' && matchMedia('(prefers-color-scheme: dark)').matches);
    target.innerHTML = target === popover ? settingsView() : settingsRows();
    if (target === popover) positionPanel();
    target.querySelector(`[data-pref="${button.dataset.prefKey}"]`).focus();
    return true;
  }
  if (button.dataset.action === 'settings-back') {
    target.innerHTML = target === popover ? settingsView() : settingsRows();
    if (target === popover) positionPanel();
    target.querySelector('button').focus();
    return true;
  }
  return false;
}
popover.addEventListener('click', event => {
  if (state.panel === 'settings' && handleSettings(event, popover)) return;
  const button = event.target.closest('button');
  if (!button) return;
  if (button.dataset.mode !== undefined) { state.mode = Number(button.dataset.mode); updateToolbar(); closePanel(); }
  if (button.dataset.platform !== undefined) {
    state.platform = Number(button.dataset.platform);
    popover.innerHTML = modelView();
    popover.querySelector(`[data-platform="${state.platform}"]`).focus();
  }
  if (button.dataset.provider !== undefined) {
    state.provider = platforms[state.platform].providers[Number(button.dataset.provider)];
    popover.innerHTML = modelView();
    popover.querySelector('[data-action="providers"]').focus();
  }
  if (button.dataset.model) {
    state.model = button.dataset.model;
    if (!platforms[state.platform].providers.includes(state.provider)) state.provider = platforms[state.platform].providers[0];
    updateToolbar(); closePanel();
  }
  if (button.dataset.effort !== undefined) setEffort(button.dataset.effort);
  if (button.dataset.action === 'providers') {
    const choices = $('.provider-choices'); choices.hidden = !choices.hidden;
    button.setAttribute('aria-expanded', String(!choices.hidden)); positionPanel();
  }
  if (button.dataset.action === 'configure') openSettings(true);
  if (button.dataset.action === 'full-settings') openSettings();
  if (button.dataset.action === 'compact') {
    state.compressed = true; popover.innerHTML = usageView();
    popover.querySelector('[data-action="compact"]').focus();
    announce('压缩效果演示：上下文占用降至 16%');
  }
});
popover.addEventListener('input', event => {
  if (event.target.id === 'model-search') $('.model-results').innerHTML = modelRows(event.target.value);
  if (event.target.id === 'effort-range') setEffort(event.target.value);
});
popover.addEventListener('keydown', event => {
  if (event.target.matches('input') && !['Tab', 'Escape'].includes(event.key)) return;
  const items = [...popover.querySelectorAll('button:not(:disabled), input:not(:disabled)')].filter(item => item.getClientRects().length);
  const index = items.indexOf(document.activeElement);
  if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    items[next]?.focus();
  }
  if (event.key === 'Tab' && (event.shiftKey && index === 0 || !event.shiftKey && index === items.length - 1)) {
    event.preventDefault(); closePanel();
  }
});
document.querySelectorAll('[data-panel]').forEach(button => button.addEventListener('click', () => openPanel(button.dataset.panel)));
document.querySelectorAll('[data-scene]').forEach(button => button.addEventListener('click', () => {
  closePanel(false);
  document.querySelectorAll('[data-scene]').forEach(item => item.setAttribute('aria-current', String(item === button)));
  if (button.dataset.scene !== 'rest') {
    state.mode = 4;
    if (button.dataset.scene === 'usage') state.usage = true;
    updateToolbar(); openPanel(button.dataset.scene, false);
  }
}));
document.addEventListener('pointerdown', event => {
  if (!popover.hidden && !popover.contains(event.target) && !event.target.closest('[data-panel], [data-scene]')) closePanel(false);
});
document.addEventListener('keydown', event => {
  if (event.key === 'Escape' && !popover.hidden) { event.preventDefault(); closePanel(); }
  if ((event.metaKey || event.ctrlKey) && event.key === ',') { event.preventDefault(); openSettings(); }
});
$('#full-settings-content').addEventListener('click', event => handleSettings(event, $('#full-settings-content')));
$('#full-settings').addEventListener('close', () => { returnPanel?.focus(); });
$('#attach').addEventListener('click', () => { closePanel(false); $('#file-input').click(); });
function renderAttachments() {
  const container = $('#attachments'); container.replaceChildren(); container.hidden = !state.files.length;
  state.files.forEach((file, index) => {
    const chip = document.createElement('span'); chip.className = 'attachment-chip';
    const label = document.createElement('span'); label.textContent = file;
    const remove = document.createElement('button'); remove.textContent = '×'; remove.setAttribute('aria-label', `移除 ${file}`);
    remove.addEventListener('click', () => { state.files.splice(index, 1); renderAttachments(); updateSend(); });
    chip.append(label, remove); container.append(chip);
  });
}
$('#file-input').addEventListener('change', event => {
  state.files.push(...[...event.target.files].map(file => file.name));
  renderAttachments(); updateSend(); event.target.value = '';
});
function updateSend() { $('#send').disabled = !$('#prompt').value.trim() && !state.files.length; }
function send() {
  if ($('#send').disabled) return;
  closePanel(false);
  const message = document.createElement('p'); message.className = 'message';
  message.textContent = $('#prompt').value.trim() || state.files.join('、');
  const receipt = document.createElement('p'); receipt.className = 'message receipt';
  receipt.textContent = `设计演示：${state.provider} / ${state.model} · ${modes[state.mode].name} · 推理${efforts[state.effort]}。未发送至 CLI。`;
  $('#conversation').append(message, receipt);
  $('#prompt').value = ''; state.files = []; state.usage = true;
  renderAttachments(); updateToolbar(); updateSend(); $('#prompt').focus();
}
$('#prompt').addEventListener('input', updateSend);
$('#prompt').addEventListener('keydown', event => {
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) { event.preventDefault(); send(); }
});
$('#send').addEventListener('click', send);
window.addEventListener('resize', positionPanel);
window.addEventListener('scroll', positionPanel, true);
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', event => {
  if (state.theme === '跟随系统') document.body.classList.toggle('dark', event.matches);
});
hydrateIcons(); updateToolbar();
const initialScene = new URLSearchParams(location.search).get('scene');
if (['permission', 'model', 'reasoning', 'settings', 'usage', 'rest'].includes(initialScene)) $(`[data-scene="${initialScene}"]`).click();
