const icons = {
  book:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 5.5A2.5 2.5 0 0 1 6.5 3H20v17H6.5A2.5 2.5 0 0 1 4 17.5z"/><path d="M4 17.5A2.5 2.5 0 0 1 6.5 15H20M8 7h8m-8 4h7"/></svg>',
  blocks:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="11" width="10" height="10" rx="2"/><rect x="11" y="3" width="10" height="10" rx="2"/><path d="M7 3v5m-2.5-2.5h5M17 15v6m-3-3h6"/></svg>',
  search:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="10.8" cy="10.8" r="7.2"/><path d="m16 16 5 5"/></svg>',
  close:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="m6 6 12 12M18 6 6 18"/></svg>',
  plus:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 5v14M5 12h14"/></svg>',
  folder:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M3 6a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
  message:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 5h16v12H9l-5 4z"/></svg>',
  terminal:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3m6 0h4"/></svg>',
  spark:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="m12 3 1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9zM19 16l1 2.5 2.5 1-2.5 1-1 2.5-1-2.5-2.5-1 2.5-1z"/></svg>',
  branch:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="7" r="2"/><path d="M6 7v10m12-8v1a5 5 0 0 1-5 5H8"/></svg>',
  download:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 3v12m-5-5 5 5 5-5M5 17v3h14v-3"/></svg>',
  refresh:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M20 7v5h-5M4 17v-5h5"/><path d="M6 9a7 7 0 0 1 12-2l2 5M4 12l2 5a7 7 0 0 0 12-2"/></svg>',
  globe:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18 14 14 0 0 1 0-18z"/></svg>',
  sliders:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 6h10m4 0h2M4 12h2m4 0h10M4 18h10m4 0h2"/><circle cx="16" cy="6" r="2"/><circle cx="8" cy="12" r="2"/><circle cx="16" cy="18" r="2"/></svg>',
  arrow:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 19V5m-6 6 6-6 6 6"/></svg>',
  check:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m5 12 4 4L19 6"/></svg>',
  warning:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="m12 3 10 18H2z"/><path d="M12 9v5m0 3h.01"/></svg>'
};
const safe = value => String(value ?? '').replace(/[&<>"']/g, char => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
const icon = (name) => icons[name] || icons.spark;
const skillsSeed = [
  {id:'react-guidelines',name:'react-guidelines',description:'Build and review React interfaces with the project conventions.',source:'acme/frontend-skills · main',path:'<workspace>/.agents/skills/react-guidelines',scope:'project',version:'1.4.2',enabled:true,loaded:true,category:'Development',type:'Loose',dependency:'Ready',update:false},
  {id:'api-review',name:'api-review',description:'Review API changes for compatibility, error handling, and security boundaries.',source:'acme/platform-skills',path:'~/.agents/skills/api-review',scope:'user',version:'2.0.0',enabled:true,loaded:false,category:'Development',type:'Loose',dependency:'Not checked',update:true},
  {id:'release-notes',name:'release-notes',description:'Turn merged changes into a concise release summary.',source:'acme/productivity-kit · main',path:'<workspace>/.agents/skills/release-notes',scope:'project',version:'1.0.3',enabled:false,loaded:false,category:'Writing',type:'Loose',dependency:'No requirements',update:false}
];
const skillsCatalog = [
  {id:'design-review',name:'design-review',description:'Review a UI against its design system, accessibility, and interaction requirements.',source:'acme/frontend-skills',category:'Development',version:'1.2.1',scope:'project'},
  {id:'test-author',name:'test-author',description:'Create focused tests around user-visible behavior and edge cases.',source:'acme/quality-toolkit',category:'Development',version:'2.1.0',scope:'project'},
  {id:'migration-helper',name:'migration-helper',description:'Plan incremental, backwards-compatible data migrations.',source:'acme/platform-skills',category:'Engineering',version:'1.1.0',scope:'user'},
  {id:'meeting-brief',name:'meeting-brief',description:'Prepare an agenda and decision-focused meeting notes.',source:'community/workflow-pack',category:'Productivity',version:'3.0.0',scope:'user'},
  {id:'docs-guide',name:'docs-guide',description:'Keep product documentation aligned with implementation and tests.',source:'acme/productivity-kit',category:'Writing',version:'1.5.0',scope:'project'}
];
const mcpCatalog = [
  {id:'docs-mcp',name:'docs-mcp',description:'Search product documentation and fetch selected reference pages.',source:'acme/docs-mcp',transport:'Http',url:'https://docs.example.invalid/mcp',category:'Development tools',count:4},
  {id:'playwright',name:'playwright',description:'Browser automation tools for testing and inspecting local web projects.',source:'microsoft/playwright-mcp',transport:'Stdio',program:'npx',args:['-y','@playwright/mcp@latest'],category:'Development tools',count:12},
  {id:'github',name:'github',description:'Work with repositories, issues, pull requests, and code search.',source:'github/github-mcp-server',transport:'Stdio',program:'docker',args:['run','-i','ghcr.io/github/github-mcp-server'],category:'Development tools',count:26},
  {id:'memory',name:'memory',description:'Persistent knowledge graph tools for structured local context.',source:'modelcontextprotocol/servers',transport:'Stdio',program:'npx',args:['-y','@modelcontextprotocol/server-memory'],category:'Productivity',count:9},
  {id:'fetch',name:'fetch',description:'Retrieve public web pages and transform them into readable text.',source:'modelcontextprotocol/servers',transport:'Stdio',program:'uvx',args:['mcp-server-fetch'],category:'Research',count:2}
];
const repositoriesSeed = [
  {url:'https://github.com/acme/frontend-skills',branch:'main',skills:8,scope:'project',status:'已索引'},
  {url:'https://github.com/acme/platform-skills',branch:'main',skills:5,scope:'user',status:'已索引'},
  {url:'https://github.com/acme/productivity-kit',branch:'main',skills:4,scope:'project',status:'需要刷新'}
];
const state = {
  engine:'codex', scope:'project', manager:'skills', tab:'installed', query:'', category:'全部', selected:'react-guidelines',
  skills:skillsSeed.map(item=>({...item})), repositories:repositoriesSeed.map(item=>({...item})), servers:[], mcpSelected:null,
  skillSource:'keywords', mcpCategory:'全部', observation:false, skillFilter:'全部', noticeTimer:null
};
const manager = document.querySelector('#manager');
const actionDialog = document.querySelector('#action-dialog');
const managerContent = document.querySelector('#manager-content');
const notice = document.querySelector('#notice');
function toast(message){ notice.textContent=message; notice.hidden=false; clearTimeout(state.noticeTimer); state.noticeTimer=setTimeout(()=>notice.hidden=true,2600); }
function selectedSkill(){return state.skills.find(item=>item.id===state.selected&&item.scope===state.scope);}
function selectedServer(){return state.servers.find(item=>item.id===state.mcpSelected&&item.scope===state.scope);}
function scopeLabel(scope){return scope==='user'?'用户级 · 当前系统用户':scope==='local'?'当前工作区 · 本地配置':'当前工作区 · DG-OS';}
function transportLabel(value){return ({Stdio:'stdio · 本地进程',Http:'HTTP · 远程服务',Sse:'SSE · 远程服务',WebSocket:'WebSocket · 只读发现'})[value]||value;}
function scopeOptions(){
  const skill=state.manager==='skills';
  const rows=skill?(state.engine==='claude'?[['project','当前工作区 · 项目技能'],['user','用户级 · 系统用户']]:[['project','当前工作区 · 项目技能'],['user','用户级 · 系统用户']]):(state.engine==='claude'?[['project','当前工作区 · 共享配置'],['local','当前工作区 · 本地配置'],['user','用户级 · 系统用户']]:[['project','当前工作区 · 共享配置'],['user','用户级 · 系统用户']]);
  if(!rows.some(([value])=>value===state.scope))state.scope=rows[0][0];
  return rows.map(([value,label])=>`<option value="${value}" ${state.scope===value?'selected':''}>${safe(label)}</option>`).join('');
}
function describeScope(){
  if(state.scope==='user')return '写入当前操作系统用户的原生 CLI 配置；DG-OS 账户切换不会隔离此位置。';
  if(state.scope==='local')return '写入当前项目的 Claude 本地配置；仅限本机，不是 DG-OS 账户私有目录。';
  return '写入当前工作区配置。Codex 用户级 Skills 目录可能被其他工具共用。';
}
function setScene(name){
  document.querySelectorAll('.review-bar nav [data-scene]').forEach(button=>button.setAttribute('aria-current',button.dataset.scene===name?'true':'false'));
  if(name==='drawer'){manager.close();return;}
  if(name==='skills'){openManager('skills','installed');return;}
  if(name==='discover'){openManager('skills','discover');return;}
  if(name==='mcp'){openManager('mcp','installed');return;}
  if(name==='market'){openManager('mcp','market');return;}
  if(name==='manual'){openManager('mcp','manual');return;}
}
function openManager(kind,tab){
  state.manager=kind; state.tab=tab||'installed'; state.query=''; state.selected=kind==='skills'?(state.skills[0]?.id||null):null; state.mcpSelected=null;
  if(!manager.open)manager.showModal();
  renderManager();
}
function tabs(){
  const rows=state.manager==='skills'?[['installed',`已安装 <span class="tab-count">${state.skills.length}</span>`],['discover','查找技能'],['repositories','技能来源']]:[['installed',`已配置 <span class="tab-count">${state.servers.length}</span>`],['market','MCP 市场'],['manual',state.mcpSelected?'编辑配置':'手动添加']];
  return rows.map(([id,label])=>`<button type="button" data-tab="${id}" aria-pressed="${state.tab===id}">${label}</button>`).join('');
}
function renderManager(restoreFocus=false){
  const active=document.activeElement; const searchWas=restoreFocus&&active?.matches('[data-search]'); const searchPos=searchWas?active.selectionStart:null;
  const title=state.manager==='skills'?'Skills 管理器':'MCP 管理器';
  document.querySelector('#manager-title').textContent=title;
  document.querySelector('#manager-subtitle').textContent=state.manager==='skills'?'CLI 原生技能 · 与 AI 助手独立': 'CLI 原生 MCP 服务 · 使用 CLI 自己的授权策略';
  document.querySelector('#manager-icon').innerHTML=icon(state.manager==='skills'?'book':'blocks');
  document.querySelector('#manager-tabs').innerHTML=tabs();
  document.querySelector('#engine').value=state.engine;
  document.querySelector('#scope').innerHTML=scopeOptions();
  document.querySelector('#scope-note').textContent=describeScope();
  renderDrawer();
  managerContent.innerHTML=state.manager==='skills'?renderSkills():renderMcp();
  document.querySelector('#footer-note').textContent=state.tab==='repositories'?'Git 来源只读元数据不会自动变成已安装资源。':state.manager==='skills'?'修改 Skills 配置不会自动重启 CLI 会话。':'配置不等于当前会话已连接；每次 MCP 工具操作仍由原生 CLI 授权。';
  document.querySelector('#footer-count').textContent=state.manager==='skills'?`${state.skills.length} 个可见条目`:`${state.servers.length} 个已配置服务`;
  if(searchWas){const field=manager.querySelector('[data-search]');field?.focus();if(searchPos!==null)field?.setSelectionRange(searchPos,searchPos);}
}
function renderDrawer(){
  document.querySelector('#skill-count').textContent=state.skills.length;
  document.querySelector('#mcp-count').textContent=state.servers.length||'';
  document.querySelector('#drawer-skills').innerHTML=state.skills.slice(0,3).map(item=>`<button class="drawer-item" data-item="skill:${safe(item.id)}"><strong><span class="dot ${item.enabled?'':'pending'}"></span>${safe(item.name)}</strong><p>${safe(item.description)}</p></button>`).join('')||'<p class="form-hint">还没有已安装的 Skill。</p>';
  document.querySelector('#drawer-mcp').innerHTML=state.servers.slice(0,3).map(item=>`<button class="drawer-item" data-item="mcp:${safe(item.id)}"><strong><span class="dot ${item.observed?'':'pending'}"></span>${safe(item.name)} <span class="badge">${safe(item.transport)}</span></strong><p>${item.observed?'已在当前会话中观测':item.enabled?'已配置 · 尚未观测':'已关闭'}</p></button>`).join('')||'<p class="form-hint">尚未配置 MCP 服务。</p>';
}
function searchBox(placeholder){return `<label class="search">${icon('search')}<input data-search type="search" placeholder="${safe(placeholder)}" value="${safe(state.query)}" aria-label="搜索"/></label>`;}
function filters(values,current,attr){return `<div class="filters">${values.map(value=>`<button type="button" class="filter" data-filter="${safe(value)}" data-filter-kind="${attr}" aria-pressed="${current===value}">${safe(value)}</button>`).join('')}</div>`;}
function renderSkills(){
  if(state.tab==='repositories')return renderRepositories();
  if(state.tab==='discover')return renderSkillDiscovery();
  const items=state.skills.filter(item=>(state.scope==='all'||item.scope===state.scope)&&`${item.name} ${item.description} ${item.source}`.toLowerCase().includes(state.query.toLowerCase()));
  const chosen=selectedSkill();
  return `<section class="list-pane ${chosen?'detail-open':''}"><div class="toolbar">${searchBox('搜索 Skill 名称或用途')}<button type="button" data-action="refresh-skills">${icon('refresh')}重新扫描</button></div><div class="list-meta"><span>显示 ${items.length} 个 Skill</span><span>资源扫描不执行 Skill 内容或脚本</span></div><div class="rows">${items.length?items.map(skillRow).join(''):`<div class="empty">${icon('book')}<h3>此范围内还没有 Skill</h3><p>从可信 GitHub 来源导入一个 Skill，或切换到另一个安装范围。</p><div class="empty-actions"><button class="primary" data-tab="discover">查找技能</button><button data-action="add-skill">添加 Git 来源</button></div></div>`}</div></section>${chosen?skillDetail(chosen):''}`;
}
function skillRow(item){
  const selected=item.id===state.selected;
  const loaded=item.loaded&&item.enabled;
  const label=state.engine==='claude'?(item.rule?`名称规则：${item.rule}`:'本层规则：继承'):(loaded?'当前会话已观测':item.enabled?'待会话加载':'已关闭');
  return `<article class="resource-row ${selected?'selected':''}">
    <span class="resource-icon">${icon('book')}</span>
    <div class="resource-main"><div class="resource-title"><button data-select-skill="${safe(item.id)}">${safe(item.name)}</button><span class="badge">v${safe(item.version)}</span><span class="badge ${state.engine==='codex'&&loaded?'success':''}">${safe(label)}</span></div>
      <p class="resource-description">${safe(item.description)}</p><div class="resource-meta"><span>${safe(item.source)}</span><span>${item.scope==='user'?'用户级':'项目级'}</span><span>${safe(item.category)}</span>${item.update?'<span class="badge warning">有更新</span>':''}</div>
    </div>
    <div class="row-actions"><button data-action="skill-toggle" data-id="${safe(item.id)}">${state.engine==='claude'?'名称规则':item.enabled?'停用':'启用'}</button><button class="icon-button" aria-label="查看 ${safe(item.name)} 详情" data-select-skill="${safe(item.id)}">${icon('search')}</button></div>
  </article>`;
}
function skillDetail(item){
  return `<aside class="detail-pane"><article class="detail-card"><div class="detail-scroll">
    <div class="detail-heading"><span class="resource-icon">${icon('book')}</span><h3>${safe(item.name)}</h3><button class="icon-button" aria-label="关闭详情" data-action="deselect">${icon('close')}</button></div>
    <div class="resource-meta"><span class="badge">${safe(item.type)}</span><span class="badge">v${safe(item.version)}</span></div>
    <p class="detail-description">${safe(item.description)}</p>
    <div class="status-line ${item.loaded?'success':'warning'}">${state.engine==='claude'?`本层名称规则：${safe(item.rule||'inherit')}。未合并更高层有效配置。`:item.loaded&&item.enabled?'当前活动会话中可见。':item.enabled?'已启用，等待会话加载。':'本层配置已关闭。'}</div>
    <section class="detail-section"><h4>安装信息</h4><dl><div><dt>安装范围</dt><dd>${safe(scopeLabel(item.scope))}</dd></div><div><dt>来源</dt><dd>${safe(item.source)}</dd></div><div class="wide"><dt>管理路径</dt><dd class="mono">${safe(item.path)}</dd></div><div><dt>依赖</dt><dd>${safe(item.dependency==='Ready'?'声明已解析 · 未执行检测':item.dependency==='No requirements'?'未声明依赖':item.dependency==='Not checked'?'未检查':item.dependency)}</dd></div><div><dt>类别</dt><dd>${safe(item.category)}</dd></div></dl></section>
    <section class="detail-section"><h4>会话生效</h4><p class="form-hint">资源发现或会话列出都不代表正文已经读取。保存不会自动重启或刷新会话；插件、内置和受管理资源按原生所有者管理，不能直接覆盖。</p></section>
  </div><footer class="detail-footer"><button data-action="skill-deps" data-id="${safe(item.id)}">检查依赖</button><button data-action="skill-toggle" data-id="${safe(item.id)}">${state.engine==='claude'?'名称规则':item.enabled?'停用':'启用'}</button>${item.update?`<button data-action="skill-update" data-id="${safe(item.id)}">检查更新</button>`:''}<button class="danger" data-action="skill-remove" data-id="${safe(item.id)}">移除</button></footer></article></aside>`;
}
function renderSkillDiscovery(){
  const repo=state.skillSource==='repository';
  const results=skillsCatalog.filter(item=>!state.skills.some(installed=>installed.id===item.id)&&`${item.name} ${item.description} ${item.source}`.toLowerCase().includes(state.query.toLowerCase())&&(state.category==='全部'||item.category===state.category));
  return `<section class="list-pane"><div class="discovery-search"><div class="segmented" aria-label="技能来源"><button data-skill-source="keywords" aria-pressed="${!repo}">目录搜索</button><button data-skill-source="repository" aria-pressed="${repo}">Git 仓库</button><button data-skill-source="package" aria-pressed="${state.skillSource==='package'}">ZIP / 本地目录</button></div>${repo?`<div class="toolbar">${searchBox('GitHub 仓库地址或 Skill 子目录')}<button class="primary" data-action="preview-github">预览来源</button></div><p>只支持已批准来源预览；提交前核对仓库、子目录、内容摘要和目标范围。</p>`:state.skillSource==='package'?`<div class="toolbar"><label class="search">${icon('folder')}<input id="package-source" placeholder="选择 ZIP 文件或本地目录" readonly value="${safe(state.packageName||'')}"/></label><button data-action="pick-package">选择文件</button><button data-action="pick-folder">选择目录</button></div><p>原型不访问本地文件。生产需使用受信文件选择器与新的宿主来源授权。</p>`:`<div class="toolbar">${searchBox('按名称、关键词查找 Skill')}<button class="primary" data-action="search-catalog">搜索</button></div><p>原生 CLI 没有共用 DG-OS Skills 市场；此目录是新设计能力，需确定受信来源后才能接入。</p>`}</div>${!repo&&state.skillSource==='keywords'?filters(['全部','Development','Engineering','Productivity','Writing'],state.category,'skill-category'):''}<div class="list-meta"><span>${repo?'仓库来源预览':state.skillSource==='package'?'本地包来源':'可发现的 Skill'} · ${repo?'需在安装前检查内容':`${results.length} 个演示条目`}</span><span class="badge warning">目录内容不能授予权限</span></div><div class="rows">${repo?`<div class="empty">${icon('globe')}<h3>输入 GitHub 仓库或子目录</h3><p>来源需固定到不可变提交并显示实际安装目标。市场排序与搜索暂为交互样例，不代表生产目录。</p></div>`:state.skillSource==='package'?`<div class="empty">${icon('folder')}<h3>预览 ZIP 或本地目录</h3><p>不执行包内代码。生产宿主需要安全扫描全部文件、路径与链接，再展示目标预览。</p></div>`:results.map(catalogSkillRow).join('')||`<div class="empty"><h3>没有匹配的技能</h3><p>试试更短关键词，或切换分类。</p></div>`}</div></section>`;
}
function catalogSkillRow(item){return `<article class="resource-row"><span class="resource-icon">${icon('globe')}</span><div class="resource-main"><div class="resource-title"><button data-action="catalog-detail" data-id="${safe(item.id)}">${safe(item.name)}</button><span class="badge">v${safe(item.version)}</span></div><p class="resource-description">${safe(item.description)}</p><div class="resource-meta"><span>${safe(item.source)}</span><span>${safe(item.category)}</span><span>目标：${safe(item.scope==='user'?'用户级':'项目级')}</span></div></div><div class="row-actions"><button data-action="install-skill" data-id="${safe(item.id)}">${icon('download')}预览安装</button></div></article>`;}
function renderRepositories(){
  return `<section class="list-pane"><div class="toolbar"><div><h3>技能来源</h3><p class="form-hint">管理用于查找和更新 Skills 的 Git 来源。来源与安装配置分开；不会自动同步到另一目标。</p></div><button class="primary" data-action="add-repo">${icon('plus')}添加仓库</button></div><div class="list-meta"><span>${state.repositories.length} 个配置来源</span><span>不访问 Git 凭据库；不执行 Git 命令</span></div><div class="rows">${state.repositories.map((repo,index)=>`<article class="resource-row"><span class="resource-icon">${icon('globe')}</span><div class="resource-main"><div class="resource-title"><button data-action="repo-info" data-id="${index}">${safe(repo.url)}</button><span class="badge ${repo.status==='需要刷新'?'warning':'success'}">${safe(repo.status)}</span></div><p class="resource-description">${repo.skills} 个技能 · 分支提示 ${safe(repo.branch)} · ${scopeLabel(repo.scope)}</p><div class="resource-meta"><span>最近检查：设计样例</span><span>不可变提交需在实际安装时确认</span></div></div><div class="row-actions"><button data-action="repo-refresh" data-id="${index}">${icon('refresh')}刷新</button><button data-action="repo-remove" data-id="${index}" class="danger">移除</button></div></article>`).join('')}</div></section>`;
}
function renderMcp(){
  if(state.tab==='manual')return renderServerForm();
  if(state.tab==='market')return renderMcpMarket();
  const chosen=selectedServer();
  const items=state.servers.filter(item=>item.scope===state.scope&&`${item.name} ${item.url||item.program||''}`.toLowerCase().includes(state.query.toLowerCase()));
  return `<section class="list-pane ${chosen?'detail-open':''}"><div class="stats"><div><span>已配置</span><strong>${state.servers.length}</strong></div><div><span>当前会话已观测</span><strong>${state.servers.filter(item=>item.observed).length}</strong></div><div><span>工具 Schema</span><strong>未提供</strong></div></div><div class="toolbar">${searchBox('筛选已配置的 MCP 服务')}<button data-action="observe-session">${icon('refresh')}读取本会话状态</button><button data-action="manual-tab">${icon('plus')}添加</button></div><div class="list-meta"><span>已保存配置不等于当前会话已连接</span><span>不显示 token 估算</span></div><div class="rows">${items.length?items.map(serverRow).join(''):`<div class="empty">${icon('blocks')}<h3>${state.servers.length?'没有匹配的 MCP 服务':'还没有配置 MCP 服务'}</h3><p>从市场模板开始，或手动添加 Codex / Claude 支持的原生配置。添加配置不会启动服务；连接状态只来自活动 CLI 会话。</p><div class="empty-actions"><button class="primary" data-tab="market">${icon('globe')}浏览 MCP 市场</button><button data-action="manual-tab">手动添加</button></div></div>`}</div></section>${chosen?serverDetail(chosen):''}`;
}
function serverRow(item){return `<article class="resource-row ${item.id===state.mcpSelected?'selected':''}"><span class="resource-icon">${icon('blocks')}</span><div class="resource-main"><div class="resource-title"><button data-select-server="${safe(item.id)}">${safe(item.name)}</button><span class="badge">${safe(item.transport)}</span><span class="badge ${item.observed?'success':item.enabled?'warning':''}">${item.observed?'会话已观测':item.enabled?'待会话加载':'已关闭'}</span></div><p class="resource-description">${safe(item.url||item.program||'原生配置')}</p><div class="resource-meta"><span>${scopeLabel(item.scope)}</span><span>${item.observed?`${item.toolNames?.length||0} 个已观测工具`:'工具列表未观测'}</span></div></div><div class="row-actions">${state.engine==='codex'?`<button data-action="server-toggle" data-id="${safe(item.id)}">${item.enabled?'停用':'启用'}</button>`:''}<button data-action="edit-server" data-id="${safe(item.id)}">编辑</button></div></article>`;}
function serverDetail(item){
  const tools=item.observed?(item.toolNames||[]).map(name=>`<details class="tool"><summary>${safe(name)}</summary><p>当前会话只提供工具名称，未提供参数 Schema。</p></details>`).join(''):'<div class="tool-empty">尚未读取当前会话的工具。</div>';
  return `<aside class="detail-pane"><article class="detail-card"><div class="detail-scroll">
    <div class="detail-heading"><span class="resource-icon">${icon('blocks')}</span><h3>${safe(item.name)}</h3><button class="icon-button" aria-label="关闭详情" data-action="deselect">${icon('close')}</button></div>
    <p class="subtle mono">${safe(transportLabel(item.transport))}</p>
    <div class="status-line ${item.observed?'success':'warning'}">${item.observed?'当前会话已报告连接。':item.enabled?'已保存，等待会话加载。':'本层配置已关闭。'}</div>
    <section class="detail-section"><h4>连接配置</h4><dl><div><dt>传输</dt><dd>${safe(item.transport)}</dd></div><div><dt>认证</dt><dd>${item.auth?'环境变量引用':'未配置凭据引用'}</dd></div><div class="wide"><dt>命令或地址</dt><dd class="mono">${safe(item.url||item.program)}</dd></div><div><dt>范围</dt><dd>${safe(scopeLabel(item.scope))}</dd></div><div><dt>来源</dt><dd>${safe(item.source||'手动配置')}</dd></div></dl></section>
    <section class="detail-section"><div class="tool-heading"><h4>当前会话的工具</h4><button data-action="observe-server" data-id="${safe(item.id)}">${icon('refresh')}读取工具</button></div>${tools}</section>
    <section class="detail-section"><h4>临时连接测试</h4><p class="form-hint">独立测试配置并读取工具 Schema，不调用工具，不代表当前 CLI 会话已连接。</p><button data-action="test-existing" data-id="${safe(item.id)}">测试连接</button></section>
  </div><footer class="detail-footer"><button data-action="edit-server" data-id="${safe(item.id)}">编辑</button>${state.engine==='codex'?`<button data-action="server-toggle" data-id="${safe(item.id)}">${item.enabled?'停用配置':'启用配置'}</button>`:''}<button class="danger" data-action="remove-server" data-id="${safe(item.id)}">移除</button></footer></article></aside>`;
}
function renderMcpMarket(){
  const chosen=mcpCatalog.find(item=>item.id===state.mcpSelected);
  const rows=mcpCatalog.filter(item=>`${item.name} ${item.description} ${item.source}`.toLowerCase().includes(state.query.toLowerCase())&&(state.mcpCategory==='全部'||item.category===state.mcpCategory));
  const installed=state.servers.some(item=>item.marketId===state.mcpSelected);
  return `<section class="list-pane ${chosen?'detail-open':''}"><div class="toolbar">${searchBox('按服务名或关键词搜索')}<button data-action="refresh-market">${icon('refresh')}刷新目录</button></div><div class="list-meta"><span>市场数据为设计样例 · ${rows.length} 个条目</span><span class="badge warning">未验证 Registry 来源</span></div>${filters(['全部','Development tools','Research','Productivity'],state.mcpCategory,'mcp-category')}<div class="rows">${rows.map(item=>`<article class="resource-row ${item.id===state.mcpSelected?'selected':''}"><span class="resource-icon">${icon('globe')}</span><div class="resource-main"><div class="resource-title"><button data-select-market="${safe(item.id)}">${safe(item.name)}</button><span class="badge">${safe(item.transport)}</span>${state.servers.some(s=>s.marketId===item.id)?'<span class="badge success">已配置</span>':''}</div><p class="resource-description">${safe(item.description)}</p><div class="resource-meta"><span>${safe(item.source)}</span><span>${safe(item.category)}</span><span>目录样例，不代表已连接</span></div></div><div class="row-actions"><button data-select-market="${safe(item.id)}">详情</button></div></article>`).join('')}</div></section>${chosen?marketDetail(chosen,installed):''}`;
}
function marketDetail(item,installed){
  return `<aside class="detail-pane"><article class="detail-card"><div class="detail-scroll"><div class="detail-heading"><span class="resource-icon">${icon('globe')}</span><h3>${safe(item.name)}</h3><button class="icon-button" aria-label="关闭详情" data-action="deselect">${icon('close')}</button></div><p class="subtle mono">${safe(item.transport)} · ${safe(item.source)}</p><p class="detail-description">${safe(item.description)}</p><div class="status-line warning">这是未验证的设计市场数据。上架信息不能证明连接可用或工具真实存在。</div><section class="detail-section"><h4>目录信息</h4><dl><div><dt>来源记录</dt><dd>${safe(item.source)}</dd></div><div><dt>传输提示</dt><dd>${safe(item.transport)}</dd></div><div><dt>声明工具数</dt><dd>${item.count}（未读取 Schema）</dd></div></dl></section><section class="detail-section"><div class="tool-heading"><h4>活动会话工具观测</h4><button disabled title="安装并在真实 CLI 会话中观测后才可读取">读取工具</button></div><div class="tool-empty">安装条目或市场元数据不提供真实工具列表。连接与工具名称须来自该 CLI 当前活动会话。</div></section></div><footer class="detail-footer">${installed?'<span class="badge success">已配置</span>':`<button class="primary" data-action="market-configure" data-id="${safe(item.id)}">继续配置</button>`}</footer></article></aside>`;
}
function renderServerForm(){
  const editing=selectedServer();
  const draft=state.serverDraft||editing||{name:'',transport:'Stdio',program:'',args:[],url:'',scope:state.scope,enabled:true,auth:'',headers:'',source:'手动配置'};
  const remote=draft.transport!=='Stdio';
  return `<section class="form-scroll"><form class="server-form" id="server-form">
    <h3>${editing?'编辑 MCP 配置':'添加 MCP 服务'}</h3>
    <div class="segmented form-mode" aria-label="MCP 传输方式">
      <button type="button" data-transport="Stdio" aria-pressed="${!remote}">stdio（本地命令）</button>
      <button type="button" data-transport="Http" aria-pressed="${draft.transport==='Http'}">HTTP</button>
      ${state.engine==='claude'?`<button type="button" data-transport="Sse" aria-pressed="${draft.transport==='Sse'}">SSE</button>`:''}
    </div>
    <label class="field">服务名称<input name="name" required value="${safe(draft.name)}" placeholder="例如：docs-mcp" autocomplete="off"/></label>
    ${remote?`<label class="field">服务地址<input name="url" type="url" required value="${safe(draft.url)}" placeholder="https://example.com/mcp"/></label>`:`
      <label class="field">可执行程序<input name="program" required value="${safe(draft.program)}" placeholder="npx、uvx 或绝对路径" autocomplete="off"/></label>
      <label class="field">命令参数<textarea name="args" rows="3" placeholder="每行一个参数">${safe((draft.args||[]).join('\n'))}</textarea><small>每行保留为一个参数，不经过 Shell 拆分。</small></label>`}
    <label class="field">${remote?'Bearer Token 环境变量名':'转发给子进程的环境变量名'}<input name="auth" value="${safe(draft.auth||'')}" placeholder="例如 API_TOKEN"/><small>仅填写变量名称；不要粘贴密钥。取值来自 CLI 启动环境。</small></label>
    ${remote?`<label class="field">HTTP Header 环境变量<textarea name="headers" rows="2" placeholder="X-API-Key=API_KEY">${safe(draft.headers||'')}</textarea><small>一行一个 Header=环境变量名，不保存密钥值。</small></label>`:''}
    <label class="field">配置范围<select name="scope">${scopeOptions()}</select></label>
    ${state.engine==='codex'?`<label class="form-hint checkbox-field"><input name="enabled" type="checkbox" ${draft.enabled===false?'':'checked'}/>在原生配置中启用</label>`:'<p class="form-hint">Claude 的会话启停与配置保存分开；这里不伪造通用配置开关。</p>'}
    <p class="form-hint">配置保存不会启动服务或重启 CLI。测试连接是一次独立的临时连接，不代表当前会话已连接。</p>
    <p class="field-error" id="form-error" hidden></p>
    <div class="form-actions"><button type="button" data-tab="installed">取消</button><button type="button" data-action="preview-connection">测试连接</button><button class="primary" type="submit">${editing?'预览保存':'预览添加'}</button></div>
  </form></section>`;
}
const engineData = {};
function switchEngine(engine){
  engineData[state.engine]={skills:state.skills,servers:state.servers,repositories:state.repositories};
  state.engine=engine;
  const data=engineData[engine]||{skills:[],servers:[],repositories:[]};
  state.skills=data.skills; state.servers=data.servers; state.repositories=data.repositories;
  state.scope='project'; state.selected=null; state.mcpSelected=null; state.serverDraft=null;
}
function captureServerDraft(){
  const form=document.querySelector('#server-form');
  if(!form)return;
  const data=new FormData(form);
  state.serverDraft={...selectedServer(),...state.serverDraft,name:String(data.get('name')||''),program:String(data.get('program')||''),args:String(data.get('args')||'').split('\n').filter(Boolean),url:String(data.get('url')||''),auth:String(data.get('auth')||''),headers:String(data.get('headers')||''),enabled:data.has('enabled'),scope:String(data.get('scope')||state.scope)};
}
function renderMcpSessionObserve(){
  state.observation=true;
  // Demo observation is explicit and is not a real process query.
  state.servers=state.servers.map(item=>item.enabled&&item.scope===state.scope?{...item,observed:true,toolNames:item.transport==='Http'?['search_docs','read_page']:['list_files','search']}:item);
  if(state.manager==='mcp')renderManager();
  toast('演示：当前会话工具快照已填入；生产状态必须来自 CLI 原生会话 IPC。');
}
function showAction(title,body){document.querySelector('#action-title').textContent=title;document.querySelector('#action-body').innerHTML=body;if(!actionDialog.open)actionDialog.showModal();}
function closeAction(){if(actionDialog.open)actionDialog.close();}
function previewInstall(item){
  item={...item,scope:state.scope};
  if(state.skills.some(existing=>existing.id===item.id&&existing.scope===item.scope)){toast('当前范围已有同名 Skill；请从已安装列表更新。');return;}
  state.pendingInstall=item;
  showAction('预览 Skill 安装',`<p>安装前逐项核对不可变来源、目标范围和包内容。此操作仅更新原生资源；不会执行 Skill 或自动授予工具权限。</p><dl><div class="wide"><dt>Skill</dt><dd>${safe(item.name)}</dd></div><div class="wide"><dt>来源</dt><dd class="mono">${safe(item.source)}</dd></div><div><dt>范围</dt><dd>${safe(scopeLabel(item.scope||state.scope))}</dd></div><div><dt>内容校验</dt><dd>生产需宿主检查提交与文件树</dd></div><div class="wide"><dt>执行行为</dt><dd>不执行；脚本与动态命令作为不可信内容显示</dd></div></dl><div class="form-actions"><button data-close="action">取消</button><button class="primary" data-action="commit-install">确认演示安装</button></div>`);
}
function addSkill(item){const scope=item.scope||state.scope;state.skills.unshift({id:item.id,name:item.name,description:item.description,source:item.source,path:`${scope==='user'?'~':'<workspace>'}/${state.engine==='claude'?'.claude':'.agents'}/skills/${item.id}`,scope,version:item.version||'1.0.0',enabled:true,loaded:false,category:item.category||'Development',type:'Loose',dependency:'Not checked',update:false});state.scope=scope;state.selected=item.id;state.tab='installed';state.query='';}
function catalogInfo(id){return skillsCatalog.find(item=>item.id===id);}
function serverFormFromMarket(item){state.mcpSelected=null;state.serverDraft={id:`mcp-${Date.now()}`,marketId:item.id,name:item.name,transport:item.transport,program:item.program||'',args:item.args||[],url:item.url||'',scope:state.scope,enabled:true,auth:'',headers:'',source:item.source,observed:false};state.tab='manual';renderManager();}
function actionClick(button){
 const action=button.dataset.action,id=button.dataset.id;
 if(action==='deselect'){state.selected=null;state.mcpSelected=null;renderManager();return;}
 if(action==='refresh-skills'){toast('演示：扫描完成；真实扫描不得运行 Skill。');return;}
 if(action==='search-catalog'){toast('演示目录搜索；生产需先确定受信市场来源。');return;}
 if(action==='preview-github'){
  const input=manager.querySelector('[data-search]')?.value.trim()||'';
  if(!/^https:\/\/github\.com\/[\w.-]+\/[\w.-]+/.test(input)){toast('请输入 HTTPS GitHub 仓库或子目录地址。');return;}
  previewInstall({id:'imported-skill',name:'imported-skill',description:'来自所选来源的 Skill · 演示内容',source:input,scope:state.scope,version:'1.0.0',category:'Development'});return;
 }
 if(action==='pick-package'||action==='pick-folder'){
  previewInstall({id:action==='pick-package'?'zip-skill':'local-skill',name:action==='pick-package'?'zip-skill':'local-skill',description:'本地来源导入预览 · 演示内容',source:action==='pick-package'?'example-skill.zip · 模拟文件选择':'example-skill/ · 模拟目录选择',scope:state.scope,version:'1.0.0',category:'Development'});return;
 }
 if(action==='add-skill'){state.tab='discover';state.skillSource='repository';renderManager();return;}
 if(action==='add-repo'){showAction('添加 Git 来源',`<p>来源用于查找可安装的 Skills。保存来源不等于安装其中的任一资源。</p><form id="repo-form"><label class="field">GitHub 仓库地址<input name="repo-url" required placeholder="https://github.com/owner/repository"/></label><label class="field">分支或 Tag 提示<input name="repo-branch" value="main"/></label><label class="field">来源范围<select name="repo-scope"><option value="project">当前工作区</option><option value="user">用户级 · 当前系统用户共享</option></select></label><p class="form-hint">实际安装须固定到不可变提交；本原型不访问网络，也不读取凭据。</p><div class="form-actions"><button type="button" data-close="action">取消</button><button class="primary" type="submit">保存演示来源</button></div></form>`);return;}
 if(action==='repo-info'){const r=state.repositories[Number(id)];showAction('来源信息',`<dl><div class="wide"><dt>URL</dt><dd class="mono">${safe(r.url)}</dd></div><div><dt>分支提示</dt><dd>${safe(r.branch)}</dd></div><div><dt>适用范围</dt><dd>${safe(scopeLabel(r.scope))}</dd></div></dl><p class="form-hint">更新检查需记录原始来源与不可变提交。发现内容不自动覆盖本地修改。</p>`);return;}
 if(action==='repo-refresh'){state.repositories[Number(id)].status='已刷新（演示）';renderManager();toast('演示：来源条目已刷新。');return;}
 if(action==='repo-remove'){showAction('移除技能来源',`<p>这只移除查找来源，不会卸载已经安装的 Skill。</p><div class="form-actions"><button data-close="action">取消</button><button class="danger" data-action="confirm-repo-remove" data-id="${safe(id)}">移除来源</button></div>`);return;}
 if(action==='confirm-repo-remove'){state.repositories.splice(Number(id),1);closeAction();renderManager();toast('演示来源已移除；已安装资源不变。');return;}
 if(action==='catalog-detail'){const item=catalogInfo(id);if(item)showAction(item.name,`<p>${safe(item.description)}</p><dl><div><dt>来源</dt><dd class="mono">${safe(item.source)}</dd></div><div><dt>版本</dt><dd>${safe(item.version)}</dd></div></dl><div class="form-actions"><button data-close="action">关闭</button><button data-action="install-skill" data-id="${safe(id)}">预览安装</button></div>`);return;}
 if(action==='install-skill'){const item=catalogInfo(id);if(item)previewInstall(item);return;}
 if(action==='commit-install'){const item=state.pendingInstall;if(item)addSkill(item);state.pendingInstall=null;closeAction();renderManager();toast('演示安装已加入本地界面状态；没有写入原生配置。');return;}
 if(action==='commit-server'){
  const item=state.pendingServer;if(!item)return;
  state.servers=state.servers.filter(row=>row.id!==item.id).concat(item);state.mcpSelected=item.id;state.serverDraft=null;state.pendingServer=null;state.tab='installed';state.scope=item.scope;
  closeAction();renderManager();toast('演示配置已保存；没有读写原生 CLI 文件。');return;
 }
 if(action==='skill-toggle'){
  const item=state.skills.find(value=>value.id===id);if(!item)return;
  if(state.engine==='claude'){
    state.ruleTarget=id;
    showAction('Claude Skill 名称规则',`<p>规则按原生名称匹配当前和未来同名 Skill，不是实例开关。插件不受此规则影响；更高层策略尚未求值。保存可能触发原生 ConfigChange Hook。</p><label class="field">原生名称<input id="rule-name" value="${safe(item.name)}" readonly/></label><label class="field">本层规则<select id="rule-state"><option value="inherit">继承 · 删除本层规则</option><option value="on">显示名称与描述 / 允许调用</option><option value="name-only">仅显示名称 / 允许调用</option><option value="user-invocable-only">不向模型列示 / 允许手动调用</option><option value="off">不列示 / 禁止调用</option></select></label><div class="form-actions"><button data-close="action">取消</button><button data-action="save-rule" class="primary">保存演示规则</button></div>`);
  }else{item.enabled=!item.enabled;item.loaded=false;renderManager();toast('演示：配置已保存；尚未重新读取活动会话。');}return;
 }
 if(action==='save-rule'){
  const item=state.skills.find(value=>value.id===state.ruleTarget);
  if(item)item.rule=document.querySelector('#rule-state').value;
  closeAction();renderManager();toast('演示名称规则已保存；未推算有效状态或刷新 CLI。');return;
 }
 if(action==='skill-deps'){const item=state.skills.find(value=>value.id===id);if(item){item.dependency='未执行探测；仅解析到声明';renderManager();toast('生产依赖扫描只定位程序，不运行它。');}return;}
 if(action==='skill-update'){const item=state.skills.find(value=>value.id===id);if(item)showAction('检查 Skill 更新',`<p>检查绑定到该实例的不可变来源与现有内容摘要。外部修改会触发冲突，不静默覆盖。</p><dl><div><dt>当前版本</dt><dd>${safe(item.version)}</dd></div><div><dt>目标</dt><dd>${safe(scopeLabel(item.scope))}</dd></div></dl><div class="form-actions"><button data-close="action">取消</button><button class="primary" data-action="confirm-update" data-id="${safe(id)}">重新预览更新</button></div>`);return;}
 if(action==='confirm-update'){const item=state.skills.find(value=>value.id===id);if(item){item.update=false;item.version='2.0.1';item.loaded=false;closeAction();renderManager();toast('演示更新完成；生产需走宿主事务并保留恢复材料。');}return;}
 if(action==='skill-remove'){const item=state.skills.find(value=>value.id===id);if(item)showAction('移除 Skill',`<p>仅移除选定范围的这个实例。卸载不会抹去已进入原生会话上下文的内容。</p><dl><div><dt>Skill</dt><dd>${safe(item.name)}</dd></div><div><dt>安装范围</dt><dd>${safe(scopeLabel(item.scope))}</dd></div></dl><div class="form-actions"><button data-close="action">取消</button><button class="danger" data-action="confirm-skill-remove" data-id="${safe(id)}">预览并移除</button></div>`);return;}
 if(action==='confirm-skill-remove'){state.skills=state.skills.filter(item=>item.id!==id);state.selected=state.skills[0]?.id||null;closeAction();renderManager();toast('演示移除已完成；没有删除磁盘资源。');return;}
 if(action==='observe-session'||action==='observe-server'){renderMcpSessionObserve();return;}
 if(action==='refresh-market'){toast('演示目录刷新；生产市场来源需单独验证与签名策略。');return;}
 if(action==='manual-tab'){state.mcpSelected=null;state.serverDraft=null;state.tab='manual';renderManager();return;}
 if(action==='market-configure'){const item=mcpCatalog.find(row=>row.id===id);if(item)serverFormFromMarket(item);return;}
 if(action==='server-toggle'){if(state.engine!=='codex'){toast('此 Claude 会话的启停需由原生能力门控；不写通用 enabled 配置。');return;}const item=state.servers.find(row=>row.id===id);if(item){item.enabled=!item.enabled;item.observed=false;renderManager();toast('演示：配置状态已变化；活动会话连接尚未重新观测。');}return;}
 if(action==='edit-server'){state.mcpSelected=id;state.serverDraft={...state.servers.find(row=>row.id===id)};state.tab='manual';renderManager();return;}
 if(action==='remove-server'){const item=state.servers.find(row=>row.id===id);if(item)showAction('移除 MCP 配置',`<p>仅从所选原生配置范围移除此服务。CLI 可能需要重新加载；DG-OS 不会结束外部服务进程。</p><dl><div><dt>服务</dt><dd>${safe(item.name)}</dd></div><div><dt>范围</dt><dd>${safe(scopeLabel(item.scope))}</dd></div></dl><div class="form-actions"><button data-close="action">取消</button><button class="danger" data-action="confirm-server-remove" data-id="${safe(id)}">预览并移除</button></div>`);return;}
 if(action==='confirm-server-remove'){state.servers=state.servers.filter(item=>item.id!==id);state.mcpSelected=null;closeAction();renderManager();toast('演示配置已移除；没有更改 CLI 文件。');return;}
 if(action==='test-existing'){
  state.serverDraft={...state.servers.find(item=>item.id===id)};
  actionClick({dataset:{action:'preview-connection'}});return;
 }
 if(action==='preview-connection'){
  captureServerDraft();
  const draft=state.serverDraft||{};
  const remote=draft.transport&&draft.transport!=='Stdio';
  if(!draft.name||!(remote?draft.url:draft.program)){toast('先填写服务名和命令或地址。');return;}
  showAction('预览临时连接测试',`<p>新设计能力：在用户明确开始后执行协议握手和 tools/list，不调用任何工具。stdio 会启动所示程序；远程连接可能发送认证信息。</p><dl><div><dt>目标</dt><dd>${safe(state.engine)} · ${safe(scopeLabel(draft.scope))}</dd></div><div><dt>方式</dt><dd>${safe(transportLabel(draft.transport||'Stdio'))}</dd></div><div class="wide"><dt>命令或地址</dt><dd class="mono">${safe(draft.url||draft.program)}</dd></div><div class="wide"><dt>参数</dt><dd class="mono">${safe((draft.args||[]).join(' | '))||'无'}</dd></div></dl><div class="status-line warning">原型只演示结果，不启动程序、不联网。生产需新增宿主测试契约及超时、取消和子进程回收。</div><div class="form-actions"><button data-close="action">取消</button><button class="primary" data-action="simulate-probe">开始演示测试</button></div>`);return;
 }
 if(action==='simulate-probe'){
  showAction('临时测试结果 · 演示',`<div class="status-line success">握手完成，读取到 2 个演示工具。不代表当前 CLI 会话已连接。</div><details class="tool" open><summary>search_docs</summary><p>搜索文档。以下是临时探针结果，非原生会话工具 Schema。</p><pre>${safe(JSON.stringify({type:'object',properties:{query:{type:'string'}},required:['query']},null,2))}</pre></details><details class="tool"><summary>read_page</summary><p>读取指定文档页。</p></details><p class="form-hint">生产结果需包含测试时间、配置摘要和观测来源；表单变更后标为过期。失败保留表单，不自动重试。</p><div class="form-actions"><button data-close="action">返回配置</button></div>`);return;
 }
}
manager.addEventListener('click',event=>{
 const button=event.target.closest('button');if(!button)return;
 if(button.dataset.tab){if(state.tab==='manual')captureServerDraft();state.tab=button.dataset.tab;state.query='';if(state.tab==='manual'){state.mcpSelected=null;state.serverDraft=null;}renderManager();return;}
 if(button.dataset.action){actionClick(button);return;}
 if(button.dataset.selectSkill){state.selected=button.dataset.selectSkill;renderManager();return;}
 if(button.dataset.selectServer){state.mcpSelected=button.dataset.selectServer;renderManager();return;}
 if(button.dataset.selectMarket){state.mcpSelected=button.dataset.selectMarket;renderManager();return;}
 if(button.dataset.filter){if(button.dataset.filterKind==='skill-category')state.category=button.dataset.filter;if(button.dataset.filterKind==='mcp-category')state.mcpCategory=button.dataset.filter;renderManager();return;}
 if(button.dataset.skillSource){state.skillSource=button.dataset.skillSource;renderManager();return;}
 if(button.dataset.transport){captureServerDraft();state.serverDraft={...state.serverDraft,transport:button.dataset.transport};renderManager();return;}
});
manager.addEventListener('input',event=>{
 if(event.target.matches('[data-search]')){state.query=event.target.value;renderManager(true);}
});
manager.addEventListener('change',event=>{
 if(event.target.id==='engine'){captureServerDraft();switchEngine(event.target.value);renderManager();}
 if(event.target.id==='scope'){captureServerDraft();state.scope=event.target.value;state.selected=null;state.mcpSelected=null;state.serverDraft=null;renderManager();}
});
document.addEventListener('submit',event=>{
 if(event.target.id==='server-form'){
  event.preventDefault();const data=new FormData(event.target);const name=String(data.get('name')||'').trim();const transport=state.serverDraft?.transport||'Stdio';
  const duplicate=state.servers.some(item=>item.name===name&&item.scope===String(data.get('scope')||state.scope)&&item.id!==state.mcpSelected);
  const error=document.querySelector('#form-error');
  if(!name||duplicate){error.textContent=duplicate?'此范围内已有同名服务。':'请输入服务名称。';error.hidden=false;return;}
  const item={...(selectedServer()||{}),...(state.serverDraft||{}),id:state.mcpSelected||state.serverDraft?.id||`mcp-${Date.now()}`,name,transport,program:String(data.get('program')||''),args:String(data.get('args')||'').split('\n').filter(Boolean),url:String(data.get('url')||''),scope:String(data.get('scope')||state.scope),enabled:state.engine==='claude'||data.has('enabled'),auth:String(data.get('auth')||''),headers:String(data.get('headers')||''),observed:false,toolNames:[],source:state.serverDraft?.source||'手动配置'};
  state.pendingServer=item;
  showAction('确认原生配置变更',`<p>预览目标范围和配置差异。保存不启动本地命令、不连接远端，也不重启 CLI。</p><dl><div><dt>服务</dt><dd>${safe(item.name)}</dd></div><div><dt>目标</dt><dd>${safe(state.engine)} · ${safe(scopeLabel(item.scope))}</dd></div><div class="wide"><dt>命令或地址</dt><dd class="mono">${safe(item.url||item.program)}</dd></div><div><dt>凭据</dt><dd>${item.auth?'仅引用环境变量名':'未配置'}</dd></div><div><dt>生效</dt><dd>待原生会话显式加载</dd></div></dl><div class="form-actions"><button data-close="action">取消</button><button class="primary" data-action="commit-server">确认保存演示配置</button></div>`);
 }
 if(event.target.id==='repo-form'){
  event.preventDefault();const data=new FormData(event.target);const url=String(data.get('repo-url')||'').trim();
  if(!/^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+(?:\.git)?(?:\/.*)?$/.test(url)){toast('请输入 HTTPS GitHub 仓库地址。');return;}
  state.repositories.push({url,branch:String(data.get('repo-branch')||'main'),skills:0,scope:String(data.get('repo-scope')||'project'),status:'待首次检查'});closeAction();renderManager();toast('演示来源已保存；尚未连接或安装资源。');
 }
});
document.querySelector('.review-bar nav').addEventListener('click',event=>{const button=event.target.closest('[data-scene]');if(button)setScene(button.dataset.scene);});
document.querySelectorAll('[data-open]').forEach(button=>button.addEventListener('click',()=>openManager(button.dataset.open,'installed')));
document.querySelector('#drawer-toggle').addEventListener('click',event=>{const drawer=document.querySelector('#drawer');drawer.hidden=!drawer.hidden;event.currentTarget.setAttribute('aria-expanded',String(!drawer.hidden));});
document.querySelector('#theme').addEventListener('click',()=>{const root=document.documentElement;const next=root.dataset.theme==='dark'?'light':'dark';if(next==='dark')root.dataset.theme='dark';else delete root.dataset.theme;});
document.addEventListener('click',event=>{
 const button=event.target.closest('button');if(!button)return;
 if(button.dataset.close==='manager')manager.close();
 if(button.dataset.close==='action')closeAction();
 if(button.closest('#action-dialog')&&button.dataset.action)actionClick(button);
 const item=button.dataset.item;
 if(item){const [kind,id]=item.split(':');openManager(kind==='skill'?'skills':'mcp','installed');if(kind==='skill')state.selected=id;else state.mcpSelected=id;renderManager();}
 if(button.dataset.scene&&button.closest('#drawer'))setScene(button.dataset.scene);
});
// Mark the matching review scene without introducing a fake native CLI session.
document.querySelectorAll('[data-icon]').forEach(node=>{const name=node.dataset.icon;node.innerHTML=icon(name);});
document.querySelector('#theme').innerHTML=icon('spark');
document.querySelectorAll('svg').forEach(svg=>svg.setAttribute('aria-hidden','true'));
renderDrawer();
if(location.search){const scene=new URLSearchParams(location.search).get('scene');if(scene)setScene(scene);}
manager.addEventListener('close',()=>{if(state.engine!=='codex')switchEngine('codex');state.scope='project';renderDrawer();});
