/* Design prototype only. No host API, credential storage or model requests. */
(() => {
  'use strict';
  const $ = s => document.querySelector(s);
  const esc = v => String(v ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const paths = {search:'M11 3a8 8 0 1 0 0 16 8 8 0 0 0 0-16zm6 14 4 4',chat:'M4 4h16v12H9l-5 4V4zm4 4h8m-8 4h5',grid:'M4 4h6v6H4zm10 0h6v6h-6zM4 14h6v6H4zm10 0h6v6h-6z',shop:'M4 9h16l-1 11H5L4 9zm3 0a5 5 0 0 1 10 0',skill:'M5 4h14v16H5zm4 4h6m-6 4h6m-6 4h3',settings:'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zm0-5v2m0 14v2M3 12h2m14 0h2M5.6 5.6 7 7m10 10 1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4',model:'M7 7h10v10H7zM9 3v4m6-4v4M9 17v4m6-4v4M3 9h4m-4 6h4m10-6h4m-4 6h4',tool:'m14 6 4 4m-9 5-5 5-2-2 5-5M13 3a6 6 0 0 0-7 7l8 8a6 6 0 0 0 7-7l-4 4-8-8 4-4z',network:'M12 3v5M5 12V8h14v4M3 14h4v6H3zm7 0h4v6h-4zm7 0h4v6h-4z',user:'M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM4 21v-2a8 8 0 0 1 16 0v2',plus:'M12 5v14M5 12h14',close:'m6 6 12 12M6 18 18 6',collapse:'M3 4h18v16H3zM9 4v16m5-12-3 4 3 4',arrow:'M12 20V4m-6 6 6-6 6 6',check:'m5 12 4 4L19 6',copy:'M8 8h12v13H8zM16 8V3H3v13h5',sun:'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v2m0 16v2M2 12h2m16 0h2M5 5l1 1m12 12 1 1M5 19l1-1M18 6l1-1',info:'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zm0 7v6m0-9v.1',file:'M5 3h9l5 5v13H5zM14 3v6h5M8 13h8m-8 4h6',stop:'M6 6h12v12H6z',play:'m8 4 12 8-12 8V4z',warning:'m12 3 10 18H2L12 3zm0 6v5m0 3v.1',terminal:'M4 5h16v14H4zM8 10l2.5 2.5L8 15M13 15h4',storage:'M12 3c4.4 0 8 1.3 8 3s-3.6 3-8 3-8-1.3-8-3 3.6-3 8-3zM4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3',folder:'M3 6h6l2 2h10v11H3z',refresh:'M20 12a8 8 0 1 1-2.3-5.7M20 4v4h-4'};
  const icon = n => `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[n]||paths.grid}"/></svg>`;
  const tile = (n,t='') => `<span class="icon-tile ${t}">${icon(n)}</span>`;
  const btn = (l,a,c='',attrs='') => `<button type="button" class="${c}" data-action="${a}" ${attrs}>${l}</button>`;
  const badge = (l,t='') => `<span class="badge ${t}">${t==='good'?'<span class="dot"></span>':''}${l}</span>`;
  const note = (l,t='') => `<div class="notice ${t}" role="${t==='error'?'alert':'status'}">${l}</div>`;
  const s={page:'workbench',mode:'chat',chat:'sample',draft:'',message:'我想把 AI 会话做得更像一个真正的智能体工作区：让用户在输入区选择模型、组合 Skills，并能看到完整的执行过程和交付结果。',running:false,task:'idle',taskDraft:'请将这段需求整理成清晰的目标、约束和验收要点。',taskSkill:'需求梳理',tool:false,skill:0,filter:'all',search:'',installed:[0],configured:true,provider:'我的模型服务',mcp:true,enabled:true,granted:true,demoModule:'pending',demoGranted:true,demoUpdate:'idle',moduleFilter:'all',appRunning:false,providerId:'openai',providerTokenSet:false,lan:false,paired:false,theme:'light',density:'1',contentSize:'14',collapsed:false,review:false,section:'providers',settingsOpen:false,dirty:false,formName:'我的模型服务',endpoint:'https://api.example.invalid/v1',test:'',account:'本地用户',scene:'normal',remoteMessage:'',remoteSent:false,loginError:'',pairError:'',desktopItems:['chat','task','skills','app'],boards:['recent','connections'],sessionList:true,agent:null};
  const skills=[{name:'需求梳理',desc:'把零散想法整理成目标、边界与验收要点。',type:'产品与规划',icon:'grid',tone:'purple',body:'先明确要解决的问题，再整理目标用户、使用场景与约束。遇到缺失信息时明确列出，不代替用户编造结论。'},{name:'清晰写作',desc:'保留原意，调整结构，让内容更容易理解。',type:'写作与表达',icon:'file',tone:'green',body:'识别读者和表达目的；梳理信息顺序，减少重复，保持术语与语气一致。'},{name:'代码阅读',desc:'解释代码结构，梳理调用关系和边界。',type:'开发与技术',icon:'model',tone:'',body:'从输入与输出开始理解代码，标记关键分支、依赖和失败路径，区分事实与推测。'},{name:'信息提炼',desc:'从长文本中提取重点，保留事实与出处。',type:'阅读与研究',icon:'skill',tone:'amber',body:'按问题组织材料，提取事实与限制，保留来源线索，说明材料没有回答的问题。'}];

  const cliTools=[
    {id:'codex',name:'Codex CLI',desc:'OpenAI 的 Codex 编码代理',version:'0.133.0',path:'/Users/demo/Library/Application Support/DGOS/tools/codex',managed:true,status:'ready'},
    {id:'claude',name:'Claude Code',desc:'Anthropic 的 Claude Code CLI',version:'未检测',path:'',managed:false,status:'warn'},
    {id:'gemini',name:'Gemini CLI',desc:'Google 的 Gemini CLI 工具',version:'0.66.0',path:'/Users/demo/.local/bin/gemini',managed:false,status:'ready'},
    {id:'node',name:'Node.js',desc:'JavaScript 运行时，部分 CLI 依赖它',version:'22.23.0',path:'/Users/demo/.local/bin/node',managed:false,status:'ready'}
  ];
  const storageSegments=[
    {id:'db',label:'数据库',bytes:225,status:'violet'},
    {id:'files',label:'会话文件',bytes:265,status:'teal'},
    {id:'tmp',label:'临时文件',bytes:0.011,status:'amber'},
    {id:'cache',label:'缓存',bytes:24,status:'green'},
    {id:'logs',label:'活动日志',bytes:160,status:'muted'}
  ];
  const storageTasks=[
    {id:'clean-chats',label:'清理旧对话',icon:'chat',bytes:'225 MB',segment:'db',desc:'删除超过保留期限的对话及其持久化会话数据；置顶对话不受影响。'},
    {id:'vacuum',label:'优化数据库',icon:'storage',bytes:'—',segment:'',done:'已优化',desc:'回收碎片空间并优化搜索索引（VACUUM）。'},
    {id:'clean-files',label:'清理会话文件',icon:'file',bytes:'265 MB',segment:'files',desc:'删除会话目录下已过期的记录文件。'},
    {id:'clean-tmp',label:'清理临时文件',icon:'folder',bytes:'11 KB',segment:'tmp',desc:'删除调试日志与临时目录。'},
    {id:'clean-cache',label:'清理缓存',icon:'grid',bytes:'24 MB',segment:'cache',desc:'删除已缓存的扩展包，下次安装时会重新下载。'},
    {id:'clean-logs',label:'清理活动记录',icon:'info',bytes:'160 MB',segment:'logs',desc:'删除活动热力图数据与健康检查历史。'}
  ];
  let cliScanning=false,storageScanning=false,storageRetention='7',storageCleaned=[],storageConfirm='';

  const sections=[['providers','模型服务','model','purple'],['mcp','工具连接','tool','green'],['modules','应用与扩展','grid','amber'],['cli','CLI 路径','terminal','teal'],['storage','存储','storage','amber'],['appearance','外观与语言','sun','teal'],['account','账户','user','purple'],['lan','局域网访问','network','green'],['about','关于 DGOS','info','amber']];
  // All capabilities below are isolated, synthetic design examples.
  const modelProviders = [
    {id:'openai',name:'OpenAI',mark:'O',models:[{id:'gpt-4.1',name:'GPT-4.1',detail:'通用 · 长上下文',context:'1M',tag:'推荐'},{id:'gpt-4.1-mini',name:'GPT-4.1 mini',detail:'轻量 · 快速响应',context:'1M'},{id:'o4-mini',name:'o4-mini',detail:'推理 · 复杂任务',context:'200K'}]},
    {id:'anthropic',name:'Anthropic',mark:'A',models:[{id:'claude-sonnet-4',name:'Claude Sonnet 4',detail:'均衡 · 编程与写作',context:'200K'},{id:'claude-opus-4',name:'Claude Opus 4',detail:'深度 · 复杂分析',context:'200K'}]},
    {id:'local',name:'本地服务',mark:'L',models:[{id:'qwen3-32b',name:'Qwen3 32B',detail:'本地 · 中文与推理',context:'32K'},{id:'deepseek-r1',name:'DeepSeek R1',detail:'本地 · 深度推理',context:'64K'}]}
  ];
  const sampleContext=[{id:'brief',name:'产品需求.md',detail:'文档 · 4.2 KB',icon:'file'},{id:'screen',name:'界面参考.svg',detail:'图像 · 128 KB',icon:'grid'},{id:'navigation',name:'navigation.ts',detail:'代码 · 1.8 KB',icon:'model'}];
  const defaultAgent=()=>({provider:'openai',model:'gpt-4.1',skills:[],context:[],mode:'协作',effort:'标准',length:'适中',temperature:'0.7',tools:true,instructions:'回答使用中文。先给出结论，保留重要约束与来源。',approval:'pending',recovered:false});
  const cloneAgent=agent=>({...agent,skills:[...agent.skills],context:[...agent.context]});
  s.agent=defaultAgent();
  let recording=false;
  let composerMenu='',menuProvider='openai',modelQuery='',skillQuery='',skillCategory='全部',composerTrigger='';
  const selectedModel=()=>modelProviders.find(x=>x.id===s.agent.provider).models.find(x=>x.id===s.agent.model);
  const chevron='<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m8 10 4 4 4-4"/></svg>';
  const paperclip='<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m8 13 7-7a3 3 0 0 1 4 4L9 20a5 5 0 0 1-7-7L13 2m-3 13 7-7"/></svg>';
  function modelResults(){
    const provider=modelProviders.find(x=>x.id===menuProvider),query=modelQuery.toLowerCase().trim();
    const list=provider.models.filter(x=>(x.name+' '+x.detail).toLowerCase().includes(query));
    return list.length?list.map(x=>btn(`<span class="grow"><strong>${x.name}</strong><small>${x.detail}</small></span><span class="model-context">${x.context}</span>${x.tag?`<span class="model-recommend">${x.tag}</span>`:''}${s.agent.provider===provider.id&&s.agent.model===x.id?icon('check'):''}`,'agent-model:'+provider.id+'/'+x.id,'model-option',`aria-pressed="${s.agent.provider===provider.id&&s.agent.model===x.id}"`)).join(''):'<p class="picker-empty">没有匹配的模型，试试其他名称。</p>';
  }
  function skillResults(){
    const list=skills.map((x,i)=>({...x,i})).filter(x=>(skillCategory==='全部'||x.type===skillCategory)&&(x.name+x.desc).includes(skillQuery.trim()));
    return list.length?list.map(x=>btn(`${tile(x.icon,x.tone)}<span class="grow"><strong>${x.name}</strong><small>${x.desc}</small></span><span class="skill-shortcut">/${['plan','write','code','summarize'][x.i]}</span>${s.agent.skills.includes(x.i)?icon('check'):''}`,'agent-skill:'+x.i,'skill-option',`aria-pressed="${s.agent.skills.includes(x.i)}"`)).join(''):'<p class="picker-empty">没有匹配的技能，换个关键词试试。</p>';
  }
  function composerMenuContent(){
    const title={model:'选择模型',skills:'选择 Skills',settings:'会话设置',context:'添加上下文'}[composerMenu];
    const header=`<header class="composer-popover-header"><strong id="composer-menu-title">${title}</strong><small>演示数据</small>${btn(icon('close'),'composer-close','icon-button','aria-label="关闭会话弹层"')}</header>`;
    if(composerMenu==='model')return `${header}<div class="model-picker-body"><nav class="provider-list" aria-label="Provider">${modelProviders.map(x=>btn(`<span class="provider-mark">${x.mark}</span><span>${x.name}</span>${x.id===menuProvider?chevron:''}`,'agent-provider:'+x.id,'provider-option',`aria-pressed="${x.id===menuProvider}"`)).join('')}${btn(`${icon('settings')}配置模型服务`,'agent-manage-models','picker-manage')}</nav><section class="model-list"><label class="picker-search">${icon('search')}<input id="agent-model-search" aria-label="搜索当前 Provider 的模型" placeholder="搜索模型…" value="${esc(modelQuery)}" autocomplete="off"></label><div class="picker-caption">${modelProviders.find(x=>x.id===menuProvider).name} 模型</div><div id="agent-model-results" class="picker-results">${modelResults()}</div></section></div><footer class="picker-footnote">选择后用于下一次演示回复，不会建立真实连接。</footer>`;
    if(composerMenu==='skills')return `${header}<label class="picker-search">${icon('search')}<input id="agent-skill-search" aria-label="搜索会话技能" placeholder="搜索技能名称或用途…" value="${esc(skillQuery)}" autocomplete="off"></label><nav class="skill-categories" aria-label="技能分类">${['全部',...new Set(skills.map(x=>x.type))].map(x=>btn(x,'agent-category:'+x,'',`aria-pressed="${x===skillCategory}"`)).join('')}</nav><div id="agent-skill-results" class="picker-results">${skillResults()}</div><footer class="picker-footnote"><span>可选择多个技能，在输入区查看或移除</span>${btn('完成','composer-close','link')}</footer>`;
    if(composerMenu==='context')return `${header}<p class="picker-description">选择示例附件，预览上下文的呈现方式。不会读取本机文件。</p><div class="picker-results">${sampleContext.map(x=>btn(`${tile(x.icon)}<span class="grow"><strong>${x.name}</strong><small>${x.detail}</small></span>${s.agent.context.includes(x.id)?icon('check'):icon('plus')}`,'agent-context:'+x.id,'skill-option',`aria-pressed="${s.agent.context.includes(x.id)}"`)).join('')}</div><footer class="picker-footnote">附件仅在当前原型会话中保留。</footer>`;
    return `${header}<div class="session-preferences"><p class="picker-description">只调整当前演示会话，不改变全局设置。</p><fieldset><legend>工作方式</legend><div class="session-choices">${['对话','规划','协作'].map(x=>btn(x,'agent-mode:'+x,'',`aria-pressed="${s.agent.mode===x}"`)).join('')}</div><small>协作模式展示计划、工具调用与结果确认。</small></fieldset><fieldset><legend>思考强度</legend><div class="session-choices">${['轻量','标准','深入'].map(x=>btn(x,'agent-effort:'+x,'',`aria-pressed="${s.agent.effort===x}"`)).join('')}</div></fieldset><fieldset><legend>回复长度</legend><div class="session-choices">${['简洁','适中','详细'].map(x=>btn(x,'agent-length:'+x,'',`aria-pressed="${s.agent.length===x}"`)).join('')}</div></fieldset><label class="session-range" for="agent-temperature"><span>创造性 <output id="agent-temperature-value">${s.agent.temperature}</output></span><input id="agent-temperature" type="range" min="0" max="1" step="0.1" value="${s.agent.temperature}"><small>精确稳定<span>多样发散</span></small></label><label class="session-tool-toggle"><span>工具调用演示<small>仅展示交互，每次调用仍需确认</small></span><input id="agent-tools" type="checkbox" role="switch" ${s.agent.tools?'checked':''}></label><label class="field">会话指令<textarea id="agent-instructions" rows="2">${esc(s.agent.instructions)}</textarea></label></div><footer class="picker-footnote">修改即时生效于原型${btn('恢复默认','agent-reset-settings','link')}</footer>`;
  }
  function positionComposerMenu(){
    const menu=$('#composer-popover'),anchor=document.querySelector(`[data-action="composer-menu:${composerMenu}"]`);if(!menu||!anchor)return;
    const rect=anchor.getBoundingClientRect();menu.style.maxHeight=Math.max(160,rect.top-20)+'px';
    const box=menu.getBoundingClientRect();menu.style.left=Math.max(10,Math.min(composerMenu==='model'?rect.right-box.width:rect.left,innerWidth-box.width-10))+'px';menu.style.top=Math.max(10,rect.top-box.height-10)+'px';
  }
  function refreshComposer(focus=''){
    const wrap=$('.tabbed-session .composer-wrap');if(!wrap)return;wrap.outerHTML=composer();positionComposerMenu();if(focus)document.querySelector(focus)?.focus({preventScroll:true});
  }
  function closeComposerMenu(restore=false){const trigger=composerTrigger;composerMenu='';$('#composer-popover')?.remove();document.querySelectorAll('[data-action^="composer-menu:"]').forEach(x=>x.setAttribute('aria-expanded','false'));if(restore)document.querySelector(`[data-action="${trigger}"]`)?.focus({preventScroll:true});}
  function openComposerMenu(name){
    if(composerMenu===name){closeComposerMenu(true);return;}
    composerMenu=name;composerTrigger='composer-menu:'+name;menuProvider=s.agent.provider;modelQuery='';skillQuery='';skillCategory='全部';refreshComposer(name==='model'?'#agent-model-search':name==='skills'?'#agent-skill-search':'#composer-popover button');
  }
  function demoReply(){return `
    <p>已围绕<strong>“权限范围先于界面操作”</strong>整理这轮修改。查询、详情和生成表单使用一致的角色与数据范围；无法确认的边界保留为待核实项。</p>
    <div class="reply-meta-strip"><span>${icon('check')}已完成示例修改</span><span>7 个文件</span><span class="change-count"><b>+617</b><i>−129</i></span><span>本地演示</span></div>
    <nav class="reply-index" aria-label="回复演示章节">${btn('执行记录','reply-jump:execution')}${btn('结果与代码','reply-jump:content-section')}${btn('产物与确认','reply-jump:artifacts')}</nav>
    <section id="reply-execution" class="reply-section">
      <details class="operation-group" open><summary>${icon('play')}<strong>执行记录</strong><span>4 项操作 · 7 个文件 · 已完成</span>${chevron}</summary>
        <details class="agent-plan"><summary>${icon('check')}<strong>实施计划</strong><span>3 / 3 完成</span>${chevron}</summary><ol><li>${icon('check')}梳理页面、Controller 与 Service 权限入口</li><li>${icon('check')}统一列表和生成表单使用的数据范围</li><li>${icon('check')}运行验证并整理遗留风险</li></ol></details>
        <div class="execution-log" aria-label="模拟工具记录">
          <details class="operation-entry" open><summary>${icon('search')}<span>检查权限调用链</span><code>页面 → Controller → Service</code><small>4 处入口</small>${chevron}</summary><div class="operation-detail"><p>已核对角色来源、列表过滤与表单提交路径。文件名和输出仅为固定演示。</p><div class="operation-file-list">${['GenTableController.java','GenTableService.java','GenTableServiceImpl.java','GenTableMapper.xml'].map(name=>`<div class="operation-file">${icon('file')}<code>${name}</code><span>已检查</span>${icon('check')}</div>`).join('')}</div></div></details>
          <details class="operation-entry" open><summary>${icon('file')}<span>更新权限过滤</span><code>GenTableServiceImpl.java</code><span class="diff-added">+12</span><span class="diff-removed">−3</span>${chevron}</summary><div class="diff-preview"><div class="diff-file-heading"><span>差异预览 · 模拟片段</span><small>演示内容</small></div><div class="diff-line removed"><span>−</span><code>return mapper.selectList(query);</code></div><div class="diff-line added"><span>+</span><code>PermissionScope scope = currentUser.getScope();</code></div><div class="diff-line added"><span>+</span><code>query.applyScope(scope);</code></div></div><details class="raw-output"><summary>查看完整差异 <span>3 行</span></summary><pre>- 查询直接返回未过滤的列表
 从宿主身份解析当前数据范围
 列表与详情复用相同过滤条件</pre></details></details>
          <details class="operation-entry"><summary>${icon('file')}<span>调整生成表单</span><code>GenTableForm.vue</code><span class="diff-added">+8</span><span class="diff-removed">−2</span>${chevron}</summary><div class="diff-preview"><div class="diff-line removed"><span>−</span><code>&lt;FormItem label="角色"&gt;</code></div><div class="diff-line added"><span>+</span><code>&lt;FormItem v-if="canManageRole" label="角色"&gt;</code></div><div class="diff-line added"><span>+</span><code>&lt;RoleSelect v-model="form.roleId" /&gt;</code></div></div></details>
          <details class="operation-entry"><summary>${icon('check')}<span>运行界面验证</span><code>mvn test · npm run build</code><small>演示通过</small>${chevron}</summary><div class="operation-detail"><p>这是原型里的固定状态示例，不代表实际运行了命令。</p><details class="raw-output"><summary>查看运行输出 <span>6 行</span></summary><pre>✓ 权限列表可见性
 ✓ 生成表单角色字段
 ✓ 无权限状态提示
 ✓ 错误恢复入口
 ✓ 窄窗口布局
 ✓ 构建产物检查</pre></details></div></details>
        </div>
      </details>
      <details class="operation-group"><summary>${icon('file')}<strong>修改文件</strong><span>7 个文件 · 展开清单</span>${chevron}</summary><div class="operation-file-list expanded-file-list">${['GenTableController.java','GenTableService.java','GenTableServiceImpl.java','GenTableMapper.xml','GenTable.vue','GenTableForm.vue','PermissionScope.java'].map(name=>`<button type="button" class="operation-file-button" data-action="reply-file:${name}">${icon('file')}<code>${name}</code><span>查看差异</span>${chevron}</button>`).join('')}</div></details>
      <details class="operation-group review-group"><summary>${icon('info')}<strong>Review · 核对项</strong><span>3 项通过 · 1 项待确认</span>${chevron}</summary><ul class="review-checks"><li>${icon('check')}调用者身份由宿主上下文派生</li><li>${icon('check')}列表和详情使用同一数据范围</li><li>${icon('check')}拒绝状态有清楚的恢复入口</li><li class="review-pending">${icon('warning')}真实角色矩阵仍需业务方确认</li></ul></details>
    </section>
    <section id="reply-content-section" class="reply-section"><h3>处理结果</h3>
      <p>权限过滤在查询入口执行；无权修改的字段不会因为前端隐藏而变成可写。失败与拒绝显示明确状态，不伪装成空结果。${btn('[1]','reply-source:0','inline-citation','aria-label="查看引用 1"')}</p>
      <blockquote>新增或更新数据时，由服务端重新核对调用者身份和数据范围。隐藏界面字段不是授权边界。</blockquote>
      <div class="code-panel response-code"><header><span>Java · 权限过滤示例</span>${btn(icon('copy'),'reply-copy:session-code','','aria-label="复制代码片段" title="复制代码"')}</header><details class="code-disclosure" open><summary>查看代码 · 4 行</summary><pre id="session-code"><code>PermissionScope scope = permissionService
    .scopeFor(currentUser);

query.applyScope(scope);
return tableMapper.selectList(query);</code></pre></details></div>
      <div class="source-list"><small>依据 · 演示引用</small>${btn(`${icon('file')}权限规则.md <span>数据范围</span>`,'reply-source:0')}${btn(`${icon('file')}接口说明.md <span>错误语义</span>`,'reply-source:1')}</div>
    </section>
    <section id="reply-artifacts" class="reply-section"><h3>交付产物</h3>
      <div class="reply-artifacts">${btn(`${tile('file','amber')}<span class="grow"><strong>权限改造摘要.md</strong><small>Markdown · 文件与验证清单</small></span><span>预览</span>${chevron}`,'reply-artifact:checklist','artifact-card')}${btn(`${tile('model','purple')}<span class="grow"><strong>PermissionScope.java</strong><small>Java · 权限范围示例</small></span><span>预览</span>${chevron}`,'reply-artifact:navigation','artifact-card')}</div>
      <details class="agent-approval" open><summary>${icon('tool')}<strong>${s.agent.approval==='pending'?'下一步需要你的确认':s.agent.approval==='allowed'?'已允许本次演示调用':'已拒绝本次演示调用'}</strong>${badge(s.agent.approval==='pending'?'等待确认':s.agent.approval==='allowed'?'已允许':'已拒绝',s.agent.approval==='pending'?'warn':s.agent.approval==='allowed'?'good':'')}${chevron}</summary><p>${s.agent.approval==='pending'?'示例工具请求读取权限说明，用于补齐 Review 清单。只读，不修改文件。':s.agent.approval==='allowed'?'已展示读取结果：示例资料包含 6 个核对点。没有调用真实工具。':'没有读取资料。你仍可查看现有回复或继续补充问题。'}</p><code>demo.read_reference({ document: "permission-notes" })</code><div class="row">${s.agent.approval==='pending'?`${btn('拒绝','reply-approval:denied','secondary')}${btn('允许本次调用','reply-approval:allowed','primary')}`:btn('重新演示确认','reply-approval:pending','secondary')}<small>交互样例</small></div></details>
      <details class="operation-group"><summary>${icon('warning')}<strong>失败与恢复</strong><span>连接中断 · 结果未知</span>${chevron}</summary><div class="reply-recovery ${s.agent.recovered?'recovered':''}"><strong>${s.agent.recovered?'已重新连接 · 演示':'连接中断，回答未完成'}</strong><p>${s.agent.recovered?'演示内容已保留，可以继续发送消息。':'已保留输入与当前内容。检查连接后再继续，不会自动重复发送。'}</p>${btn(s.agent.recovered?'再次演示失败':'模拟重新连接','reply-recover','secondary')}</div><div class="reply-recovery unknown"><strong>工具结果未知</strong><p>调用后连接中断，无法确认最终结果。先核对工具侧记录，不直接重复执行。</p></div></details>
    </section>
    <p class="reply-conclusion">下方可调整本轮 Provider、模型与 Skills。本回复中的文件、命令、日志和工具调用都是固定界面演示。</p>
    <footer class="reply-footer"><span>${icon('check')}演示回复已完成</span><small>12.8s · 1,248 tokens · 非实际计量</small></footer>
    <div class="response-actions row">${btn(`${icon('copy')}复制回答`,'reply-copy:reply-content','muted-button')}${btn(`${icon('check')}有帮助`,'reply-feedback','muted-button')}${btn('继续细化交互','reply-followup','muted-button')}</div>`;}
  let timer,returnFocus;
  let nextChatId=3, activeChatId=1;
  let chats=[{id:1,title:'产品体验整理',chat:'sample',draft:'',message:'我想把 AI 会话做得更像一个真正的智能体工作区：让用户在输入区选择模型、组合 Skills，并能看到完整的执行过程和交付结果。',agent:null},{id:2,title:'文字与表达',chat:'sample',draft:'',message:'请帮我把这段介绍写得更清楚。',agent:null}];
  chats[0].agent=cloneAgent(defaultAgent());
  chats[1].agent={...cloneAgent(defaultAgent()),provider:'anthropic',model:'claude-sonnet-4',skills:[1],context:['brief']};
  s.agent=cloneAgent(chats[0].agent);
  function saveChat(){if(s.page==='workbench'){const chat=chats.find(x=>x.id===activeChatId);if(chat)Object.assign(chat,{chat:s.chat,draft:s.draft,message:s.message,agent:cloneAgent(s.agent)});}}
  function selectChat(id){const chat=chats.find(x=>x.id===id);if(!chat)return;closeComposerMenu();activeChatId=id;s.page='workbench';s.mode='chat';Object.assign(s,{chat:chat.chat,draft:chat.draft,message:chat.message,agent:cloneAgent(chat.agent||defaultAgent())});}
  function createChat(){saveChat();const chat={id:nextChatId++,title:'新会话',chat:'empty',draft:'',message:'',agent:defaultAgent()};chats.push(chat);selectChat(chat.id);}
  function toast(t){$('#toast').textContent=t;clearTimeout(timer);timer=setTimeout(()=>$('#toast').textContent='',3500);}
  function theme(){/* AppearanceEditor owns the prototype's visual preferences. */}
  function nav(l,a,n,t='',on=false){return btn(`${tile(n,t)}<span class="nav-label">${l}</span>`,a,`nav-item ${on?'active':''}`,`title="${l}" ${on?'aria-current="page"':''}`);}
  function bar(title,sub='',actions=''){return `<header class="pagebar"><div class="row">${tile(s.page==='skills'?'skill':s.page==='app'?'model':'chat',s.page==='skills'?'amber':'green')}<div><h1>${title}</h1>${sub?`<small>${sub}</small>`:''}</div></div><div class="row">${actions}</div></header>`;}
  function shell(){return `<div class="shell ${s.collapsed?'collapsed':''}"><aside class="sidebar" aria-label="主导航"><div class="brand"><span class="brand-mark">D</span><span class="brand-text">我的工作空间</span></div><div class="workspace-label nav-label">概览</div>${nav('桌面总览','page:desktop','grid','purple',s.page==='desktop')}${nav('AI 会话','page:workbench','chat','green',s.page==='workbench')}<div class="eyeline">资源</div>${nav('技能中心','page:skills','skill','amber',s.page==='skills')}${s.enabled?nav('模型调试台','page:app','model','purple',s.page==='app'):''}<div class="eyeline">测试工具</div>${nav('单任务测试','page:task','play','blue',s.page==='task')}<div class="nav-bottom"><small class="prototype-label nav-label">设计原型 · 演示数据</small>${nav('设置','settings:providers','settings','blue')}<div class="sidebar-footer">${btn(`<span class="avatar">${esc(s.account.slice(0,1))}</span><span class="nav-label">${esc(s.account)}</span>`,'settings:account','nav-item','title="账户设置"')}${btn(icon('collapse') + '<span class="sidebar-collapse-label">收起</span>','collapse','icon-button sidebar-collapse',`aria-label="收起或展开导航" title="${s.collapsed?'展开导航':'收起导航'}" aria-expanded="${!s.collapsed}"`)}</div></div></aside><main class="workspace" id="main">${s.page==='desktop'?desktop():s.page==='skills'?skillPage():s.page==='app'?appPage():s.page==='account'?accountPage():s.page==='task'?`${bar('单任务测试','',badge('测试功能'))}${taskPage()}`:workbench()}</main></div>${review()}`;}
  const desktopEntries=[['chat','AI 会话','交流、思考与创作','chat','green','new-chat'],['task','单任务','技能与工具协同','play','purple','mode:single'],['skills','技能中心','选择工作方法','skill','amber','page:skills'],['app','模型调试台','已启用的应用','model','','page:app']];
  function desktop(){return `${bar('桌面总览','',btn('整理桌面','customize-desktop','secondary'))}
    <div class="scroll"><div class="desktop-content">
      <div class="desktop-intro"><div><h2>你好，开始今天的工作</h2><p>我的工作空间</p></div><small>本地桌面 · 演示数据</small></div>
      ${!s.configured?`<div class="row spread setup-notice">${note('连接一个模型服务，即可开始使用 AI 会话。')}${btn('配置模型服务','settings:providers','primary')}</div>`:''}
      <div class="studio-overview">
        <section class="launch-panel"><header class="row">${tile('chat','purple')}<div><h2>新建会话</h2><p>从一个问题、一段文字或一个想法开始。</p></div></header>
          <div class="launch-options">${btn('梳理想法','launch:idea')}${btn('清晰写作','launch:writing')}${btn('使用技能','page:skills')}</div>
          <div class="launch-config"><div><span class="muted">${icon('network')}模型服务</span>${btn(esc(s.provider),'settings:providers','link')}</div><div><span class="muted">${icon('model')}当前模型</span><strong>${s.configured?'示例文本模型':'尚未配置'}</strong></div><div><span class="muted">${icon('file')}内容归属</span><strong>本地会话</strong></div></div>
          ${btn(`开始会话 ${icon('arrow')}`,'new-chat','primary')}</section>
        ${s.boards.includes('connections')?`<section class="resource-overview"><header class="row spread"><h2>可用能力</h2>${btn('管理','settings:providers','link')}</header>${settingRow('模型服务',esc(s.provider),badge(s.configured?'已配置':'待配置',s.configured?'good':'warn'))}${settingRow('工具连接','示例资料工具',badge(s.mcp?'已连接':'未连接',s.mcp?'good':''))}${settingRow('我的技能',`${s.installed.length} 个演示技能`,btn('浏览','page:skills','link'))}<p class="footer-caption">按需选择能力，执行前确认。</p></section>`:''}
      </div>
      <section class="desktop-launch" aria-label="桌面快捷入口">${desktopEntries.filter(([id])=>s.desktopItems.includes(id)&&(id!=='app'||s.enabled)).map(([,name,desc,ic,tone,action])=>btn(`${tile(ic,tone)}<strong>${name}</strong><small>${desc}</small>`,action,'desktop-shortcut')).join('')}${btn(`${icon('plus')}<span>添加入口</span>`,'customize-desktop','desktop-shortcut add-shortcut')}</section>
      <div class="desktop-boards">${s.boards.includes('recent')?`<section class="desktop-board"><header class="row spread"><h2>最近的工作</h2>${btn('查看会话','page:workbench','link')}</header>${btn(`${tile('chat','green')}<span class="grow"><strong>产品体验整理</strong><small>会话 · 示例文本模型</small></span><small>今天</small>`,'history','desktop-record')}${btn(`${tile('file','amber')}<span class="grow"><strong>一段更清晰的表达</strong><small>会话 · 写作整理</small></span><small>昨天</small>`,'history:writing','desktop-record')}${btn(`${tile('play','purple')}<span class="grow"><strong>需求要点整理</strong><small>单任务 · 演示记录</small></span>${badge('已完成','good')}`,'recent-task','desktop-record')}</section>`:''}<section class="desktop-board"><header><h2>继续一项专注的任务</h2></header><p class="muted">选择技能、准备输入，在执行前确认工具调用。</p>${btn(`${icon('play')}打开单任务`,'mode:single','secondary')}<div class="divider"></div><small class="muted">所有记录与连接状态均为原型样例。</small></section></div>
    </div></div>`;}
  function workbench(){const current=chats.find(x=>x.id===activeChatId);return `<div class="workbench-shell"><section class="workbench-stage"><header class="pagebar session-bar"><div class="row">${icon('file')}<h1>AI 会话</h1><span class="session-context">${esc(current?.title||'新会话')}</span></div><div class="row"><small class="session-demo-label">交互演示</small>${badge(s.configured?'就绪':'需要配置',s.configured?'good':'warn')}</div></header><div class="conversation-tabs-row"><div class="conversation-tabs" role="tablist" aria-label="会话标签">${chats.map(x=>`<div class="conversation-tab ${x.id===activeChatId?'selected':''}">${btn(esc(x.title),'chat-tab:'+x.id,'chat-tab-label',`role="tab" id="chat-tab-${x.id}" aria-selected="${x.id===activeChatId}" aria-controls="chat-panel" tabindex="${x.id===activeChatId?0:-1}"`)}${btn(icon('close'),'close-chat:'+x.id,'close-chat','aria-label="关闭会话 '+esc(x.title)+'"')}</div>`).join('')}</div>${btn(icon('plus'),'new-chat','icon-button new-chat-button','aria-label="新建对话" title="新建对话"')}</div><section class="session-main tabbed-session" id="chat-panel" role="tabpanel" aria-labelledby="chat-tab-${activeChatId}"><div class="scroll chat-scroll"><div class="conversation">${chatContent()}</div></div>${composer()}</section></section></div>`;}
  function chatContent(){
    if(!s.configured)return `<div class="empty">${tile('model','purple')}<h2>先连接一个模型服务</h2><p>配置服务地址和模型后，就可以在这里开始对话。<br>你的对话保存在本机。</p>${btn('配置模型服务','settings:providers','primary')}<p class="footer-caption">原型使用虚构地址，请勿输入真实密钥。</p></div>`;
    if(s.chat==='empty')return `<div class="empty">${tile('chat','green')}<h2>今天，想从哪里开始？</h2><p>写下问题、整理想法，或者把一段内容变得更清楚。</p><div class="suggestions">${btn('梳理一个想法','suggest:idea','secondary')}${btn('改写一段文字','suggest:writing','secondary')}</div></div>`;
    return `<div class="conversation-divider"><span>今天 · 演示会话</span></div>
      <div class="user-message"><small>你</small><div class="bubble">${esc(s.message||'我已经完成了功能开发的准备，接下来如何把这些能力组织成一个完整的产品？')}</div></div>
      <div class="row assistant-meta">${tile('model','green')}<strong>DGOS 助手</strong><small class="mono">${selectedModel().name}</small><small>${s.running?'生成中':'演示回复'}</small></div>
      <article id="reply-content" class="assistant-content">${s.chat==='error'?`${note('连接中断，本次回答未完成。输入已保留，请检查模型服务后重新发送。','error')}${btn('检查模型服务','settings:providers','secondary')}${btn('重新发送','send','primary')}`:s.chat==='cancelled'?'<p>已停止生成。你可以编辑输入，再次发送。</p>':s.running?`<div class="agent-working">${icon('model')}<span>正在整理权限边界与操作记录…</span></div><p class="muted">输入与能力设置都保留在当前会话中。</p>${btn('模拟完成','finish','link')}`:demoReply()}</article>`;
  }
  const microphone='<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a3 3 0 0 0-3 3v5a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3Zm-7 8a7 7 0 0 0 14 0m-7 7v3m-4 0h8"/></svg>';
  function composer(){const provider=modelProviders.find(x=>x.id===s.agent.provider);return `<div class="composer-wrap"><div class="agent-composer-shell">
    <div class="composer-workspace-status" aria-label="演示工作区文件变更"><span class="workspace-status-title">${icon('file')}工作区</span>${btn('GenTableServiceImpl.java','reply-file:GenTableServiceImpl.java','workspace-file-chip')}${btn('GenTableForm.vue','reply-file:GenTableForm.vue','workspace-file-chip')}${btn('+14 项','reply-files-list','workspace-file-more')}${btn(`${icon('check')}审查 <b>+617</b> <i>−129</i>`,'reply-review','workspace-review-chip')}<span class="workspace-status-note">界面演示</span></div>
    <form id="chat-form" class="composer agent-composer">
      ${s.agent.skills.length||s.agent.context.length?`<div class="composer-context" aria-label="本次会话能力">${s.agent.skills.map(i=>btn(`${icon('skill')}<span>${skills[i].name}</span>${icon('close')}`,'agent-remove-skill:'+i,'context-chip skill-chip','aria-label="移除技能 '+skills[i].name+'"')).join('')}${s.agent.context.map(id=>{const x=sampleContext.find(x=>x.id===id);return btn(`${icon(x.icon)}<span>${x.name}</span>${icon('close')}`,'agent-remove-context:'+id,'context-chip','aria-label="移除附件 '+x.name+'"');}).join('')}</div>`:''}
      <label class="visually-hidden" for="chat-input">消息</label><textarea id="chat-input" placeholder="向智能体提问… 输入 / 选择技能" ${!s.configured?'disabled':''}>${esc(s.draft)}</textarea>
      <div class="composer-footer agent-composer-footer"><div class="composer-tools">
        ${btn(icon('plus'),'composer-menu:context','icon-button','aria-label="添加上下文" title="添加文件、图片或内容" aria-haspopup="dialog"')}
        ${btn(paperclip,'composer-menu:context','icon-button','aria-label="附件" title="添加示例附件" aria-haspopup="dialog"')}
        ${btn(microphone,'composer-action:record',`icon-button ${recording?'recording':''}`,`aria-label="${recording?'停止录音演示':'语音输入'}" title="语音输入演示" aria-pressed="${recording}"`)}
        ${btn(icon('tool'),'composer-action:tools',`icon-button ${s.agent.tools?'tool-enabled':''}`,`aria-label="${s.agent.tools?'关闭':'开启'}工具调用" title="工具调用" aria-pressed="${s.agent.tools}"`)}
        ${btn(`${icon('skill')}<span>Skills</span>${s.agent.skills.length?`<i>${s.agent.skills.length}</i>`:''}`,'composer-menu:skills','composer-skill-button',`aria-label="选择 Skills" title="选择技能" aria-expanded="${composerMenu==='skills'}" aria-haspopup="dialog"`)}
        ${btn(icon('settings'),'composer-menu:settings','icon-button',`aria-label="会话设置" title="会话设置" aria-expanded="${composerMenu==='settings'}" aria-haspopup="dialog"`)}
      </div><div class="composer-model-tools">
        ${btn(`<span class="provider-mini">${provider.mark}</span><span class="model-button-label"><small>${provider.name}</small><span>${selectedModel().name}</span></span>${chevron}`,'composer-menu:model','composer-model-button',`aria-label="选择 Provider 和模型" aria-expanded="${composerMenu==='model'}" aria-haspopup="dialog" ${!s.configured?'disabled':''}`)}
        ${btn(`<span class="effort-label">${s.agent.effort}</span>${chevron}`,'composer-menu:settings','composer-effort-button',`aria-label="思考强度：${s.agent.effort}" title="调整思考强度" aria-expanded="${composerMenu==='settings'}" aria-haspopup="dialog"`)}
        ${s.running?btn(icon('stop'),'stop','agent-stop','aria-label="停止" title="停止生成"'):`<button type="submit" class="primary agent-send" aria-label="发送" title="发送消息" ${!s.configured?'disabled':''}>${icon('arrow')}</button>`}
      </div></div>
    </form>
    ${composerMenu?`<section id="composer-popover" class="composer-popover ${composerMenu}-popover" role="dialog" aria-labelledby="composer-menu-title">${composerMenuContent()}</section>`:''}
  </div></div>`;}
  function taskPage(){return `<div class="scroll"><div class="content"><div class="section-heading"><h1>一次专注的任务</h1><p>选择模型和技能，明确输入，再查看执行结果。</p></div>${!s.configured?note('请先在设置中配置模型服务。','warn'):''}<div class="task-grid"><div class="stack"><section class="panel"><label class="field" for="task-input"><strong>任务内容</strong><textarea id="task-input" rows="6" ${s.running?'disabled':''}>${esc(s.taskDraft)}</textarea></label><div class="row spread" style="margin-top:16px"><small>不会自动执行</small>${s.running?btn('停止任务','stop','secondary'):btn(`${icon('play')}运行任务`,'run-task','primary',!s.configured?'disabled':'')}</div></section>${taskResult()}</div><aside class="stack"><section class="panel"><h2>本次使用</h2><div class="stack"><label class="field">模型<select ${s.running?'disabled':''}><option>示例文本模型</option></select></label><label class="field">技能<select id="task-skill" ${s.running?'disabled':''}><option value="">不使用技能</option>${s.installed.map(i=>`<option ${s.taskSkill===skills[i].name?'selected':''}>${skills[i].name}</option>`).join('')}</select></label>${btn('去技能中心选择','page:skills','link')}<div class="divider" style="margin:0"></div><label class="check-label"><input id="task-tool" type="checkbox" ${s.tool?'checked':''} ${!s.mcp||s.running?'disabled':''}>使用演示工具</label><small>${s.mcp?'执行前展示操作并请求确认。':'请先在设置中连接工具。'}</small>${btn('管理工具连接','settings:mcp','link')}</div></section><section class="panel"><h2>最近任务</h2><div class="row spread"><span>需求要点整理</span>${badge('已完成','good')}</div><p class="footer-caption">演示记录 · 非真实执行</p></section></aside></div></div></div>`;}
  function taskResult(){
    if(s.task==='idle')return `<section class="panel"><h2>执行结果</h2><p class="muted">运行后，结果和关键状态会显示在这里。</p></section>`;
    if(s.task==='approval')return `<section class="panel pending"><div class="row spread"><h2>需要你的确认</h2>${badge('等待批准','warn')}</div><p>演示工具请求读取示例资料，用于整理需求要点。</p><div class="tool-result mono">工具：demo.read_notes<br>参数：{"document":"sample-notes"}<br>作用：读取演示内容，不修改文件</div><div class="row">${btn('拒绝','deny-tool','secondary')}${btn('允许本次调用','allow-tool','primary')}</div></section>`;
    if(s.task==='running')return `<section class="panel"><h2>正在执行</h2><p>已提交演示任务，正在整理输出。</p>${btn('模拟完成','finish-task','primary')}</section>`;
    if(s.task==='unknown')return `<section class="panel"><h2>工具结果未知</h2>${note('连接在执行后中断，无法确认最终结果。请先核查工具侧记录，再决定是否重新执行。','warn')}</section>`;
    if(['denied','cancelled'].includes(s.task))return `<section class="panel"><h2>${s.task==='denied'?'已拒绝工具调用':'任务已停止'}</h2><p class="muted">没有生成最终结果，输入已保留。</p></section>`;
    return `<section class="panel"><div class="row spread"><h2>任务结果</h2>${badge('已完成','good')}</div><p>已按${esc(s.taskSkill||'当前输入')}整理出以下要点：</p><ol><li>目标：让现有功能拥有明确入口。</li><li>约束：沿用本地优先与现有能力边界。</li><li>验收：完成从配置到结果的完整流程。</li></ol><div class="row">${btn('复制结果','copy','secondary')}<small>固定演示输出</small></div></section>`;
  }
  function skillPage(){const list=skills.map((x,i)=>({...x,i})).filter(x=>(s.filter!=='mine'||s.installed.includes(x.i))&&(x.name+x.desc).includes(s.search)),x=skills[s.skill];return `${bar('技能中心','找到适合任务的工作方法',btn(`${icon('plus')}导入技能`,'import','secondary'))}<div class="scroll"><div class="content"><div class="section-heading"><h1>让每次工作，都有章可循</h1><p>先了解技能，再添加到自己的工具箱。</p></div><div class="toolbar"><label class="visually-hidden" for="skill-search">搜索技能</label><input id="skill-search" class="search" placeholder="搜索技能名称或用途" value="${esc(s.search)}">${btn('全部技能','filter:all',s.filter==='all'?'primary':'secondary')}${btn('我的技能','filter:mine',s.filter==='mine'?'primary':'secondary')}</div><div class="resource-layout"><div class="resource-list">${list.length?list.map(x=>btn(`${tile(x.icon,x.tone)}<div class="grow"><strong>${x.name}</strong><p>${x.desc}</p><small>${x.type}${s.installed.includes(x.i)?'　·　已添加':''}</small></div>`,'skill:'+x.i,'resource '+(s.skill===x.i?'active':''),`aria-pressed="${s.skill===x.i}"`)).join(''):`<div class="panel"><h2>没有匹配的技能</h2><p class="muted">试试其他关键词。</p>${btn('清除筛选','clear-search','secondary')}</div>`}</div><section class="panel detail">${tile(x.icon,x.tone)}<h2>${x.name}</h2><p class="muted">${x.desc}</p><div class="row">${badge(x.type)}${badge('示例技能')}</div><div class="divider"></div><h3>它会怎样帮助你</h3><p>${x.body}</p><dl><dt>版本</dt><dd class="mono">1.0.0</dd><dt>来源</dt><dd>原型样例，非真实目录</dd><dt>使用方式</dt><dd>添加后在单任务中选择</dd></dl><div class="divider"></div>${s.installed.includes(s.skill)?`<div class="stack">${badge('已添加到我的技能','good')}${btn('在单任务中使用','use-skill','primary')}</div>`:btn(`${icon('plus')}添加到我的技能`,'add-skill','primary')}<p class="footer-caption">更新与卸载仍依赖技能功能收口。本原型演示浏览、添加和使用。</p></section></div></div></div>`;}
  function appPage(){return `${bar('模型调试台','已启用的应用',btn('管理应用','settings:modules','secondary'))}<div class="scroll"><div class="content"><section class="panel"><div class="row">${tile('model','purple')}<div><h1 style="margin:0">模型调试台</h1><p class="muted" style="margin:5px 0 0">应用使用入口与配置管理各有位置</p></div></div><div class="divider"></div>${!s.granted?`${note('此应用尚未获得模型调用权限。','warn')}${btn('查看并授予权限','settings:modules','primary')}`:`<label class="field">测试输入<textarea rows="5" id="app-input" placeholder="输入一段测试文本"></textarea></label><div class="row" style="margin-top:16px">${btn('模拟调用','app-run','primary')}${badge('已授权模型调用','good')}</div><div id="app-result" class="task-output"></div>`}</section><p class="footer-caption">没有装载或执行真实模块包。</p></div></div>`;}
  function accountPage(){return `${bar('账户管理','本机账户与数据归属')}<div class="scroll"><div class="content" style="max-width:800px"><div class="section-heading"><h1>你的本地工作空间</h1><p>本机账户用于区分数据，不是云端登录。</p></div><section class="panel"><div class="row">${tile('user')}<div class="grow"><strong>${esc(s.account)}</strong><p class="muted" style="margin:4px 0 0">当前账户 · 演示</p></div>${badge('已登录','good')}</div><div class="divider"></div>${settingRow('切换账户','会结束当前会话并停止正在运行的任务。',btn('切换','switch-account','secondary'))}${settingRow('账户密码','通过身份验证后修改本机密码。',btn('修改密码','password','secondary'))}${settingRow('退出当前账户','退出后显示本机登录页。',btn('退出登录','logout','secondary'))}${settingRow('删除账户','删除该账户的数据与授权，正式产品中不可撤销。',btn('删除账户','delete-account','danger'))}</section></div></div>`;}
  function accountSettings(){return `<section class="panel"><div class="row">${tile('user')}<div class="grow"><strong>${esc(s.account)}</strong><p class="muted" style="margin:4px 0 0">当前账户 · 本地数据归属</p></div>${badge('已登录','good')}</div><div class="divider"></div>${settingRow('账户范围','对话、项目、技能和凭据按当前账户隔离。',badge('本地'))}${settingRow('切换账户','会结束当前会话并停止正在运行的任务。',btn('切换','switch-account','secondary'))}${settingRow('账户密码','通过身份验证后修改本机密码。',btn('修改密码','password','secondary'))}${settingRow('退出当前账户','退出后显示本机登录页。',btn('退出登录','logout','secondary'))}${settingRow('删除账户','删除该账户的数据与授权，正式产品中不可撤销。',btn('删除账户','delete-account','danger'))}<p class="footer-caption">账户是系统状态，不进入工作区标签。</p></section>`;}

  function cliStatusLabel(status){return status==='ready'?'已就绪':status==='warn'?'未检测到':'需检查';}
  function cliSettings(){
    const rows=cliTools.map(tool=>{
      const status=cliStatusLabel(tool.status);
      return '<div class="cli-row"><div class="cli-row-head"><span class="cli-dot '+tool.status+'" aria-hidden="true"></span><strong>'+esc(tool.name)+'</strong><span class="badge">'+(tool.status==='ready'?'v'+esc(tool.version):'未检测')+'</span><span class="muted cli-desc">'+esc(tool.desc)+'</span><small class="cli-managed">'+(tool.managed?'由 DG-OS 自动管理':status)+'</small></div>'+
        '<label class="cli-path"><span class="cli-prompt" aria-hidden="true">&gt;_</span><input id="cli-path-'+tool.id+'" data-cli-path="'+tool.id+'" value="'+esc(tool.path)+'" placeholder="留空时按系统 PATH 查找" autocomplete="off" spellcheck="false" aria-label="'+esc(tool.name)+' 可执行文件路径"'+(tool.managed?' readonly':'')+'>'+
        (tool.managed?'':'<button type="button" class="icon-button cli-browse" data-action="cli-browse:'+tool.id+'" aria-label="选择 '+esc(tool.name)+' 可执行文件">'+icon('folder')+'</button>')+'</label></div>';
    }).join('');
    return '<section class="panel cli-settings"><div class="row spread cli-head"><div class="row">'+tile('terminal','teal')+'<div><h2>CLI 可执行文件路径</h2><p class="muted">配置 DG-OS 调用本机 CLI 时使用的可执行文件</p></div></div>'+btn(icon('refresh')+(cliScanning?'检测中…':'自动检测'),'cli-detect','secondary',cliScanning?'disabled':'')+'</div><div class="divider"></div><div class="cli-list">'+rows+'</div>'+(cliScanning?note('正在检测本机 CLI…（演示，不执行真实命令）'):'')+'<p class="footer-caption">留空时按系统 PATH 查找。自动检测只读取版本号，不执行模型请求，也不读取 CLI 凭据；该原型不会访问本机文件。</p></section>';
  }
  function storageSize(bytes){return bytes>=1?Math.round(bytes)+' MB':bytes>0?Math.round(bytes*1024)+' KB':'0 B';}
  function storageTotal(){return storageSegments.reduce((sum,item)=>sum+item.bytes,0);}
  function storageSettings(){
    const total=storageTotal();
    const bar=storageSegments.filter(item=>item.bytes>0).map(item=>'<span class="storage-seg '+item.status+'" style="width:'+(item.bytes/total*100).toFixed(2)+'%" title="'+esc(item.label)+'"></span>').join('');
    const legend=storageSegments.map(item=>'<span class="storage-legend-item"><span class="storage-dot '+item.status+'" aria-hidden="true"></span>'+esc(item.label)+' <strong>'+storageSize(item.bytes)+'</strong></span>').join('');
    const tasks=storageTasks.map(task=>{
      const cleaned=storageCleaned.includes(task.id);
      const confirming=storageConfirm===task.id;
      const actions=cleaned?badge(task.done||'已清理','good'):confirming?btn('取消','storage-cancel','link')+btn('确认清理','storage-confirm:'+task.id,'danger'):btn('清理','storage-clean:'+task.id,'secondary');
      return '<div class="storage-task'+(cleaned?' cleaned':'')+'"><span class="storage-task-icon">'+tile(task.icon)+'</span><div class="grow"><div class="row wrap storage-task-head"><strong>'+esc(task.label)+'</strong><span class="storage-task-size">'+esc(task.bytes)+'</span></div><p>'+esc(task.desc)+'</p>'+(confirming?'<p class="storage-confirm-copy">确认后将从演示数据中移除这部分占用。</p>':'')+'</div><div class="row storage-task-actions">'+actions+'</div></div>';
    }).join('');
    return '<section class="panel storage-usage"><div class="row spread"><div class="row">'+tile('storage','purple')+'<div><h2>存储用量</h2><p class="muted">'+(storageScanning?'正在扫描…（5/6）':'已扫描 6 项 · 共 '+storageSize(total))+'</p></div></div>'+btn(icon('refresh')+(storageScanning?'扫描中':'重新扫描'),'storage-scan','secondary',storageScanning?'disabled':'')+'</div><div class="divider"></div><div class="storage-bar" role="img" aria-label="存储构成，共 '+storageSize(total)+'">'+bar+'</div><div class="storage-legend">'+legend+'</div><div class="storage-stats"><div><strong>333</strong><small>对话</small></div><div><strong>6,367</strong><small>消息</small></div><div><strong>440</strong><small>会话文件</small></div><div><strong>—</strong><small>持久会话</small></div></div></section>'+
      '<section class="panel"><div class="setting-row"><div><strong>保留期限</strong><p>清理时删除超过此期限的对话与会话文件。</p></div><select id="storage-retention" aria-label="保留期限">'+[['3','3 天'],['7','7 天'],['30','30 天'],['90','90 天'],['keep','永久保留']].map(([value,label])=>'<option value="'+value+'"'+(storageRetention===value?' selected':'')+'>'+label+'</option>').join('')+'</select></div><p class="footer-caption">保留期限只决定清理范围，不影响当前打开的会话。</p></section>'+
      '<section class="panel"><div class="row spread storage-tasks-head"><div><h2>清理操作</h2><p class="muted">清理缓存和旧数据以释放磁盘空间</p></div>'+badge('当前占用 '+storageSize(total))+'</div><div class="divider"></div><div class="storage-tasks">'+tasks+'</div><p class="footer-caption">原型只更新演示数值，不会删除本机任何文件。</p></section>';
  }
  function refreshSettings(){const scroller=document.querySelector('.settings-main');const top=scroller?scroller.scrollTop:0;updateSettings();const next=document.querySelector('.settings-main');if(next)next.scrollTop=top;}

  function settingRow(title,desc,action){return `<div class="setting-row"><div><strong>${title}</strong><p>${desc}</p></div>${action}</div>`;}
  function settingsView(){
    const content=({providers:providerSettings,mcp:mcpSettings,modules:moduleSettings,cli:cliSettings,storage:storageSettings,appearance:appearanceSettings,account:accountSettings,lan:lanSettings,about:aboutSettings})[s.section]();
    const footer=s.section==='appearance'?window.AppearanceEditor.footer():`<small id="dirty-status">${s.dirty?'有未保存的修改':'仅用于交互评审，刷新即重置'}</small><div class="row">${s.section==='providers'?`${btn('放弃修改','discard-provider','secondary')}${btn(`${icon('check')}保存并应用`,'save-provider','primary')}`:btn('返回工作区','close-settings','secondary')}</div>`;
    return `<section class="settings-window"><header class="window-header"><div class="row">${tile('settings')}<h1 id="dialog-title">设置</h1><small>主窗口内悬浮</small></div><div class="row">${badge('演示数据')}${btn(icon('close'),'close-settings','icon-button','aria-label="关闭设置"')}</div></header><div class="settings-body"><nav class="settings-nav" aria-label="设置分类">${sections.map(([id,l,n,t])=>nav(l,'section:'+id,n,t,s.section===id)).join('')}</nav><div class="settings-main"><div class="section-heading"><h1>${sections.find(x=>x[0]===s.section)[1]}</h1><p>${({providers:'管理模型连接，在会话中选择使用。',mcp:'连接外部工具，在任务中逐次确认调用。',modules:'管理可用应用及其权限。',cli:'配置本机 CLI 的可执行文件路径与检测状态。',storage:'查看本机数据占用，设置保留期限并清理。',appearance:'分别调整操作界面、阅读与代码。',account:'本地账户的数据归属与入口。',lan:'允许可信设备访问本机的远程聊天。',about:'本地优先的 AI 工作空间。'})[s.section]}</p></div>${content}</div></div><footer class="savebar">${footer}</footer></section>`;
  }
  function providerSettings(){
    var selected = modelProviders.find(function (provider) { return provider.id === s.providerId; }) || modelProviders[0];
    var providerList = modelProviders.map(function (provider) {
      var status = provider.id === 'local' ? '本地服务' : '已配置';
      return btn(tile('model', provider.id === 'anthropic' ? 'amber' : provider.id === 'local' ? 'green' : 'purple') + '<div class="grow"><strong>' + esc(provider.name) + '</strong><p>' + status + '</p></div>', 'provider-select:' + provider.id, 'resource ' + (provider.id === selected.id ? 'active' : ''));
    }).join('');
    var models = selected.models.map(function (model) {
      var isDefault = s.agent.provider === selected.id && s.agent.model === model.id;
      return '<div class="provider-model-row"><div class="grow"><strong>' + esc(model.name) + '</strong><small>' + esc(model.detail) + '</small></div><span class="model-context">' + esc(model.context) + '</span>' + (isDefault ? badge('默认', 'good') : btn('设为默认', 'provider-model:' + selected.id + '/' + model.id, 'link')) + '</div>';
    }).join('');
    return '<div class="resource-layout"><div class="resource-list">' + providerList + btn(icon('plus') + '添加服务', 'add-provider', 'secondary') + '</div><div class="stack"><section class="panel"><div class="row spread"><h2>连接配置</h2>' + badge(s.configured ? '已配置' : '待配置', s.configured ? 'good' : 'warn') + '</div><div class="form-grid"><label class="field">显示名称<input id="provider-name" value="' + esc(s.formName) + '" autocomplete="off"></label><label class="field">接口类型<select><option>OpenAI 兼容接口</option></select></label><label class="field full">服务地址<input id="provider-endpoint" value="' + esc(s.endpoint) + '" autocomplete="off"><small>原型使用虚构域名，不会访问此地址。</small></label><label class="field full">API Key<input id="provider-token" type="password" placeholder="' + (s.providerTokenSet ? '已保存，输入可替换' : '演示模式不接收真实凭据') + '" autocomplete="off"><small>原型不保存真实密钥；保存后只显示状态。</small></label></div><div class="row wrap" style="margin-top:22px">' + btn('测试连接', 'test-provider', 'secondary') + btn('演示连接失败', 'fail-provider', 'muted-button') + '<span id="provider-test">' + s.test + '</span></div><div class="row wrap provider-credential-row">' + badge(s.providerTokenSet ? '凭据已保存' : '未保存凭据', s.providerTokenSet ? 'good' : '') + btn('保存凭据', 'save-provider-token', 'secondary') + btn('移除凭据', 'clear-provider-token', 'muted-button') + '</div></section><section class="panel"><div class="row spread"><h2>可用模型</h2>' + badge(selected.models.length + ' 个模型') + '</div>' + models + '</section></div></div>';
  }
  function mcpSettings(){return `<section class="panel"><div class="row">${tile('tool','green')}<div class="grow"><h2 style="margin-bottom:4px">MCP 工具连接</h2><small>工具服务由左侧 MCP 管理统一维护</small></div>${badge(s.mcp?'已连接':'未连接',s.mcp?'good':'warn')}</div><div class="divider"></div><p class="muted" style="margin:0">完整管理已移至左侧「MCP 管理」资源页：服务配置、连接、工具检查、调用确认和记录都在那里完成；设置只保留兼容入口。</p><div class="row wrap" style="margin-top:16px">${btn('在单任务中使用','mcp-use','primary',!s.mcp?'disabled':'')}<button type="button" class="secondary" data-ws-action="mcp-page">打开 MCP 管理</button></div><p class="footer-caption">这是原型入口跳转，不会启动真实 MCP 服务。</p></section>`;}
  function moduleSettings(){
    var filter=s.moduleFilter||'all';
    var modelActive=!!(s.enabled&&s.granted);
    var demoActive=s.demoModule==='installed'&&s.demoGranted;
    var activeCount=(modelActive?1:0)+1+(demoActive?1:0);
    var pendingCount=(modelActive?0:1)+(demoActive?0:1);
    var filterButtons=[['all','全部'],['active','已启用'],['pending','待处理']].map(function(item){
      var count=item[0]==='all'?3:item[0]==='active'?activeCount:pendingCount;
      return '<button type="button" class="module-filter '+(filter===item[0]?'active':'')+'" data-action="module-filter:'+item[0]+'">'+item[1]+' <span>'+count+'</span></button>';
    }).join('');
    var modelCard='<article class="module-card"><div class="row">'+tile('model','purple')+'<div class="grow"><strong>模型调试台</strong><small>随包应用 · provider-console</small></div>'+badge(s.enabled?(s.granted?'已启用':'权限异常'):'已停用',s.enabled?(s.granted?'good':'warn'):'')+'</div><div class="module-meta"><span>版本 1.0.0</span><span>模型调用 '+(s.granted?'已授权':'待授权')+'</span></div><div class="row module-actions">'+btn(s.enabled?'停用':'启用','toggle-module','secondary')+btn(s.granted?'撤销权限':'授予权限','toggle-permission','secondary')+btn('打开应用','open-app','primary',!s.enabled?'disabled':'')+'</div></article>';
    var canvasCard='<article class="module-card"><div class="row">'+tile('grid','teal')+'<div class="grow"><strong>无限画布</strong><small>随包应用 · 本地创作平台</small></div>'+badge('已启用','good')+'</div><div class="module-meta"><span>版本 0.1.0</span><span>本地项目</span></div><div class="row module-actions"><button type="button" class="primary" data-ws-action="open-canvas">打开画布</button><span class="module-note">全屏编辑器</span></div></article>';
    var demoBadge='';
    var demoMeta='';
    var demoActions='';
    var demoClass='';
    if(s.demoModule==='pending'){
      demoBadge=badge('未安装','warn');
      demoMeta='<span>版本 1.2.0</span><span>可安装 · 演示模块</span>';
      demoActions=btn('安装示例模块','module-install','primary')+'<span class="module-note">安装后在本机启用</span>';
      demoClass='module-disabled';
    } else {
      var crashed=s.demoModule==='crashed';
      var disabled=s.demoModule==='disabled';
      demoBadge=badge(crashed?'已崩溃':disabled?'已停用':!s.demoGranted?'权限不足':'已启用',crashed||!s.demoGranted?'warn':disabled?'':( 'good'));
      demoMeta='<span>版本 '+(s.demoUpdate==='updated'?'1.2.1':'1.2.0')+'</span><span>'+(crashed?'上次运行崩溃 · 可重启':disabled?'已停用 · 不加载能力':!s.demoGranted?'工具权限未授予 · 更新被阻止':s.demoUpdate==='failed'?'更新失败 · 可重试':s.demoUpdate==='updated'?'已是最新':'工具调用 已授权')+'</span>';
      demoActions=(crashed?btn('重启','module-recover','primary')+btn('卸载','module-remove','secondary'):btn(disabled?'启用':'停用','module-toggle','secondary')+btn('模拟崩溃','module-crash','muted-button')+btn('卸载','module-remove','secondary'))+btn(s.demoGranted?'撤销权限':'授予权限','module-grant','secondary')+btn(s.demoUpdate==='failed'?'重试更新':s.demoUpdate==='updated'?'已是最新':'检查更新','module-update','secondary',s.demoUpdate==='updated'?'disabled':'');
      demoClass=(crashed||disabled||!s.demoGranted)?'module-disabled':'';
    }
    var demoCard='<article class="module-card '+demoClass+'"><div class="row">'+tile('file','amber')+'<div class="grow"><strong>本地资料整理</strong><small>第三方模块 · 示例</small></div>'+demoBadge+'</div><div class="module-meta">'+demoMeta+'</div><div class="row module-actions module-actions-wrap">'+demoActions+'</div></article>';
    var cards=[];
    if(filter==='all'||filter===(modelActive?'active':'pending'))cards.push(modelCard);
    if(filter==='all'||filter==='active')cards.push(canvasCard);
    if(filter==='all'||filter===(demoActive?'active':'pending'))cards.push(demoCard);
    if(!cards.length)cards.push('<div class="module-empty"><strong>没有符合条件的模块</strong><p>切换筛选，或安装/启用模块后再查看。</p></div>');
    return '<section class="panel"><div class="row spread"><div><h2 style="margin:0">模块清单</h2><p class="muted" style="margin:5px 0 0">应用使用在工作区标签中打开，安装、权限和更新在这里管理。</p></div>'+badge(activeCount+' 个已启用','good')+'</div><div class="divider"></div><div class="module-filter-row">'+filterButtons+'</div><div class="module-list">'+cards.join('')+'</div><p class="footer-caption">应用实例和标签由工作区管理；这里不直接创建实例。</p></section>';
  }
  function appearanceSettings(){return window.AppearanceEditor.render();}
  function lanSettings(){return `<section class="panel">${settingRow('允许局域网访问','远程设备只能使用配对后创建的聊天。',btn(s.lan?'关闭访问':'开启访问','toggle-lan',s.lan?'secondary':'primary'))}${s.lan?`${settingRow('配对地址','<span class="mono">http://192.0.2.10:8635/pair/demo</span><br>文档保留地址，仅为示意',btn('预览远程访问','remote-preview','secondary'))}${settingRow('演示设备',s.paired?'已配对 · 远程聊天':'尚未配对',s.paired?btn('撤销访问','revoke','danger'):badge('无连接'))}`:note('开启后显示配对地址与已连接设备。')}<p class="footer-caption">不启动网络监听、不生成真实凭据、不连接其他设备。</p></section>`;}
  function aboutSettings(){return `<section class="panel"><div class="row"><span class="brand-mark">D</span><h2 style="margin:0">DGOS</h2></div>${settingRow('原型基线','V2-FR-009','')}${settingRow('工程版本基线','0.1.0','')}${settingRow('宿主','Tauri 2（原型未连接）','')}<p class="footer-caption">实际产品版本和运行环境由宿主提供。</p></section>`;}

  document.addEventListener('input',e=>{
    if(!e.target.closest)return;
    const field=e.target.closest('[data-cli-path]');if(!field)return;
    const tool=cliTools.find(item=>item.id===field.dataset.cliPath);if(!tool)return;
    tool.path=field.value;tool.status=field.value.trim()?'ready':'warn';
  });
  document.addEventListener('change',e=>{
    if(e.target.id!=='storage-retention')return;
    storageRetention=e.target.value;refreshSettings();toast('保留期限已更新（演示）。');
  });

  function review(){return `<aside class="review" aria-label="原型评审控制">${s.review?`<div class="review-panel"><h2>评审场景</h2><p>原型评审控制，数据与执行均为模拟。</p><div class="row wrap">${[['desktop','桌面总览'],['normal','常规对话'],['first','首次使用'],['error','连接失败'],['unknown','任务结果未知'],['login','登录页'],['startup','启动失败'],['notfound','页面不存在'],['remote','远程访问']].map(([v,l])=>btn(l,'scene:'+v,'secondary')).join('')}</div><div class="row wrap" style="margin-top:14px">${btn(s.theme==='light'?'切换暗色':'切换亮色','toggle-theme','secondary')}${btn('重置演示','reset','secondary')}</div><p class="footer-caption">设置固定在主窗口内悬浮。界面尺寸和内容字号在“外观与语言”中分别评审。</p></div>`:''}${btn(`${icon('grid')}${s.review?'收起评审面板':'评审场景'}`,'review','review-toggle',`aria-expanded="${s.review}"`)}</aside>`;}
  function authPage(){return `<header class="topbar"><span class="wordmark">DGOS</span>${badge('登录场景 · 模拟')}</header><main class="center-page"><form id="login-form" class="panel login-panel"><div class="brand-mark">D</div><h1>回到你的工作空间</h1><p class="muted">选择本机账户，继续上一次的工作。</p><div class="stack"><label class="field">账户<select id="login-user"><option>本地用户</option><option>演示账户 B</option></select></label><label class="field">密码<input id="login-password" type="password" autocomplete="off" placeholder="演示口令：demo"><small>仅输入 demo，请勿使用真实密码。</small></label>${s.loginError?note(s.loginError,'error'):''}<button type="submit" class="primary">登录</button></div><div class="divider"></div>${btn('预览无账户首次启动','scene:first','link')}</form></main>${review()}`;}
  function errorPage(){const startup=s.scene==='startup';return `<header class="topbar"><span class="wordmark">DGOS</span>${badge('异常场景 · 模拟')}</header><main class="center-page"><section class="panel login-panel">${tile('warning','amber')}<h1 style="margin-top:22px">${startup?'无法打开本机数据库':'找不到这个页面'}</h1><p class="muted">${startup?'应用未能完成启动，当前工作空间尚未打开。':'入口可能已变更，你可以返回工作台继续。'}</p>${startup?'<div class="tool-result">阶段：打开数据库<br>原因：演示的文件访问失败<br>位置：/example/dgos/data.db</div><p class="footer-caption">正式产品此处不提供自动修复。通过评审场景退出本演示。</p>':btn('返回工作台','scene:normal','primary')}</section></main>${review()}`;}
  function remotePage(){return `<header class="topbar"><span class="wordmark">DGOS</span>${badge('远程访问原型')}</header>${!s.paired?`<main class="center-page"><form id="pair-form" class="panel login-panel">${tile('network')}<h1 style="margin-top:20px">连接你的 DGOS</h1><p class="muted">使用主机提供的配对信息，开启本次远程聊天。</p>${!s.lan?note('主机尚未开启局域网访问，请返回主机设置开启。','warn'):''}<label class="field">配对码<input id="pair-code" placeholder="演示口令：demo" autocomplete="off"></label>${s.pairError?note(s.pairError,'error'):''}<button type="submit" class="primary" style="margin-top:18px" ${!s.lan?'disabled':''}>配对并进入</button>${btn('返回主机原型','scene:normal','link')}</form></main>`:`<main class="remote-shell">${bar('远程聊天','仅本次配对创建的会话',btn('结束访问','remote-exit','secondary'))}<div class="scroll chat-scroll">${s.remoteSent?`<div class="user-message"><div class="bubble">${esc(s.remoteMessage)}</div></div><section class="assistant-content"><p>这是远程聊天的演示回复。桌面设置、原有会话和应用不会出现在远程入口。</p></section>`:`<div class="empty">${tile('chat','green')}<h2>开始本次会话</h2><p>本次配对的聊天与主机已有会话区分。</p></div>`}</div><div class="composer-wrap"><form id="remote-form" class="composer"><label class="field">消息<textarea id="remote-input" rows="3" placeholder="输入消息"></textarea></label><div class="row spread" style="margin-top:12px"><small>模拟远程响应</small><button class="primary" type="submit">发送</button></div></form></div></main>`}${review()}`;}
  function render(){theme();$('#app').innerHTML=s.scene==='login'?authPage():['startup','notfound'].includes(s.scene)?errorPage():s.scene==='remote'?remotePage():shell();window.__dgosAfterRender?.();}
  function updateSettings(){if(s.settingsOpen){$('#dialog').innerHTML=settingsView();window.AppearanceEditor.show();}}
  function openSettings(section){s.section=section;s.formName=s.provider;s.dirty=false;s.test='';returnFocus=document.activeElement;s.settingsOpen=true;const d=$('#dialog');d.className='overlay-settings';d.innerHTML=settingsView();d.showModal();window.AppearanceEditor.show();}
  function closeDialog(){window.AppearanceEditor.discard();const d=$('#dialog');d.close();d.className='';s.settingsOpen=false;if(returnFocus?.isConnected)returnFocus.focus();}
  function confirm(title,text,action,label='确认'){returnFocus=document.activeElement;const d=$('#dialog');d.className='';d.innerHTML=`<h2 id="dialog-title">${title}</h2><p>${text}</p><div class="dialog-actions">${btn('取消','cancel-dialog','secondary')}${btn(label,action,'primary')}</div>`;if(!d.open)d.showModal();}
  function go(action){const [k,v]=action.split(':');saveChat();if(k==='page'){s.page=v;s.scene='normal';s.mode=v==='task'?'single':'chat';if(v==='task')s.openTask=true;if(v==='workbench')selectChat(activeChatId);}if(k==='mode'){s.page=v==='single'?'task':'workbench';s.mode=v;if(v==='single')s.openTask=true;if(v==='chat')selectChat(activeChatId);}if(action==='new-chat')createChat();if(k==='chat-tab')selectChat(Number(v));if(k==='close-chat'){const id=Number(v),index=chats.findIndex(x=>x.id===id);chats=chats.filter(x=>x.id!==id);if(id===activeChatId){if(chats.length)selectChat(chats[Math.min(index,chats.length-1)].id);else createChat();}}render();if(k==='chat-tab'||k==='close-chat'){$(`#chat-tab-${activeChatId}`)?.focus();$(`#chat-tab-${activeChatId}`)?.scrollIntoView({block:'nearest',inline:'nearest'});}else{const heading=$('#main h1');heading?.setAttribute('tabindex','-1');heading?.focus({preventScroll:true});}}
  function safeGo(action){if(s.running){confirm('离开将停止当前任务','你可以留在当前任务继续等待，或停止后前往其他页面。','leave:'+action,'停止并离开');return;}go(action);}
  function send(){if(!s.configured||s.running)return;if(!s.draft.trim()&&s.chat!=='error'){toast('先输入一条消息。');$('#chat-input')?.focus();return;}s.message=s.draft.trim()||s.message||'重新发送演示请求';s.draft='';s.chat='running';s.running=true;render();}
  function closeSettings(){if(s.dirty){toast('还有未保存修改，请先保存或放弃修改。');return;}closeDialog();}
  function modal(html){returnFocus=document.activeElement;$('#dialog').className='';$('#dialog').innerHTML=html;$('#dialog').showModal();}
  function clearAccount(){s.running=false;s.task='idle';s.taskDraft='';s.draft='';s.message='';s.chat='empty';s.tool=false;s.remoteMessage='';s.remoteSent=false;s.paired=false;chats=[{id:nextChatId++,title:'新会话',chat:'empty',draft:'',message:''}];activeChatId=chats[0].id;}
  document.addEventListener('click',e=>{
    if(composerMenu&&!$('#composer-popover')?.contains(e.target)&&!e.target.closest('[data-action^="composer-menu:"]'))closeComposerMenu();
    const el=e.target.closest('[data-action]');if(!el)return;const a=el.dataset.action,[k,v]=a.split(':');
    if(k==='page'||k==='mode'||a==='new-chat'){safeGo(a);return;}
    if(k==='chat-tab'){if(Number(v)!==activeChatId)safeGo(a);return;}
    if(k==='close-chat'){if(Number(v)===activeChatId)safeGo(a);else go(a);return;}
    if(k==='settings'){openSettings(v);return;}
    if(k==='composer-menu'){openComposerMenu(v);return;}
    if(k==='agent-provider'){menuProvider=v;$('#composer-popover').innerHTML=composerMenuContent();positionComposerMenu();$('#agent-model-search')?.focus({preventScroll:true});return;}
    if(k==='agent-model'){const [provider,model]=v.split('/');s.agent.provider=provider;s.agent.model=model;closeComposerMenu();refreshComposer('[data-action="composer-menu:model"]');toast('本次会话模型已切换（演示）。');return;}
    if(k==='agent-skill'){const index=Number(v);s.agent.skills=s.agent.skills.includes(index)?s.agent.skills.filter(x=>x!==index):[...s.agent.skills,index];refreshComposer('#agent-skill-search');return;}
    if(k==='composer-action'&&v==='record'){recording=!recording;refreshComposer('[data-action="composer-action:record"]');toast(recording?'语音输入演示已开始。':'语音输入演示已停止。');return;}
    if(k==='composer-action'&&v==='tools'){s.agent.tools=!s.agent.tools;refreshComposer('[data-action="composer-action:tools"]');toast(s.agent.tools?'工具调用已在本轮开启（演示）。':'工具调用已在本轮关闭。');return;}
    if(k==='reply-file'){const content=`${v}\n\n演示差异：\n- 页面入口直接读取未过滤列表\n+ 服务端按当前调用者范围过滤\n+ 无权限时保留明确错误状态\n\n此内容为原型展示，不对应真实仓库文件。`;modal(`<h2 id="dialog-title">${icon('file')} ${esc(v)}</h2><p>文件差异 · 固定演示内容</p><pre class="artifact-preview">${esc(content)}</pre><div class="dialog-actions">${btn('关闭预览','cancel-dialog','primary')}</div>`);return;}
    if(a==='reply-files-list'){modal(`<h2 id="dialog-title">修改文件 · 7 项</h2><p>本轮文件变更清单，文件名与差异均为界面演示。</p><div class="operation-file-list">${['GenTableController.java','GenTableService.java','GenTableServiceImpl.java','GenTableMapper.xml','GenTable.vue','GenTableForm.vue','PermissionScope.java'].map(name=>`<button type="button" class="operation-file-button" data-action="reply-file:${name}">${icon('file')}<code>${name}</code><span>打开演示差异</span>${chevron}</button>`).join('')}</div><div class="dialog-actions">${btn('关闭','cancel-dialog','primary')}</div>`);return;}
    if(a==='reply-review'){const group=$('.review-group');if(group){group.open=!group.open;group.scrollIntoView({block:'nearest',behavior:'smooth'});}return;}
    if(k==='session-rail'){if(v==='settings'){openSettings('providers');return;}if(v==='skill'){safeGo('page:skills');return;}if(v==='account'){openSettings('account');return;}if(v==='desktop'){safeGo('page:desktop');return;}if(v==='files'){toast('修改文件清单已显示在输入区上方（演示）。');return;}toast(`${v==='chat'?'会话':'工具'}入口为原型导航示例。`);return;}
    if(k==='agent-category'){skillCategory=v;$('#composer-popover').innerHTML=composerMenuContent();positionComposerMenu();$('#agent-skill-search')?.focus({preventScroll:true});return;}
    if(k==='agent-context'){s.agent.context=s.agent.context.includes(v)?s.agent.context.filter(x=>x!==v):[...s.agent.context,v];refreshComposer('#composer-popover button');return;}
    if(k==='agent-remove-skill'){s.agent.skills=s.agent.skills.filter(x=>x!==Number(v));refreshComposer('#chat-input');return;}
    if(k==='agent-remove-context'){s.agent.context=s.agent.context.filter(x=>x!==v);refreshComposer('#chat-input');return;}
    if(k==='agent-manage-models'){closeComposerMenu();openSettings('providers');return;}
    if(a==='cli-detect'){if(cliScanning)return;cliScanning=true;refreshSettings();setTimeout(()=>{const claude=cliTools.find(item=>item.id==='claude');if(claude){claude.version='0.3.295';claude.path='/Users/demo/.local/bin/claude';claude.status='ready';}cliScanning=false;refreshSettings();toast('已检测到 4 个 CLI（演示，未执行真实命令）。');},900);return;}
    if(k==='cli-browse'){toast('原型不打开系统文件选择器。');return;}
    if(a==='storage-scan'){if(storageScanning)return;storageScanning=true;refreshSettings();setTimeout(()=>{storageScanning=false;refreshSettings();toast('扫描完成：共占用 '+storageSize(storageTotal())+'（演示）。');},900);return;}
    if(k==='storage-clean'){storageConfirm=v;refreshSettings();return;}
    if(a==='storage-cancel'){storageConfirm='';refreshSettings();return;}
    if(k==='storage-confirm'){const task=storageTasks.find(item=>item.id===v);if(!task)return;storageCleaned=[...storageCleaned,v];if(task.segment){const segment=storageSegments.find(item=>item.id===task.segment);if(segment)segment.bytes=0;}storageConfirm='';refreshSettings();toast(task.label+'已完成（演示）。');return;}

    if(k==='agent-mode'||k==='agent-effort'||k==='agent-length'){s.agent[k==='agent-mode'?'mode':k==='agent-effort'?'effort':'length']=v;refreshComposer('#composer-popover button');return;}
    if(a==='composer-close'){closeComposerMenu(true);return;}
    if(a==='agent-reset-settings'){Object.assign(s.agent,{mode:'协作',effort:'标准',length:'适中',temperature:'0.7',tools:true,instructions:'回答使用中文。先给出结论，保留重要约束与来源。'});refreshComposer('#composer-popover button');return;}
    if(k==='reply-jump'){const target=$('#reply-'+v);target?.scrollIntoView({behavior:'smooth',block:'start'});return;}
    if(k==='reply-source'){const x=sampleContext[Number(v)];modal(`<h2 id="dialog-title">${icon(x.icon)} ${x.name}</h2><p>引用片段 · 演示资料</p><blockquote>${Number(v)===0?'把模型与技能选择放在会话输入区，让当前任务上下文保持连续。':'悬浮选择器从输入区向上展开，不遮挡主要回复内容。'}</blockquote><div class="dialog-actions">${btn('返回会话','cancel-dialog','primary')}</div>`);return;}
    if(k==='reply-artifact'){const content=v==='checklist'?'输入区能力选择\n- Provider 与模型\n- 多个会话 Skills\n- 当前会话参数\n- 显式工具确认':'export const session = {\n  provider: "openai",\n  model: "gpt-4.1",\n  skills: ["需求梳理", "清晰写作"],\n};';modal(`<h2 id="dialog-title">${v==='checklist'?'会话交互清单.md':'navigation.ts'}</h2><p>产物预览 · 固定演示内容</p><pre class="artifact-preview">${esc(content)}</pre><div class="dialog-actions">${btn('关闭预览','cancel-dialog','primary')}</div>`);return;}
    if(k==='reply-approval'){s.agent.approval=v;saveChat();render();return;}
    if(a==='reply-recover'){s.agent.recovered=!s.agent.recovered;refreshComposer();return;}
    if(a==='reply-feedback'){el.innerHTML=`${icon('check')}谢谢，已记录（演示）`;el.disabled=true;return;}
    if(a==='reply-followup'){s.draft='请继续细化：把模型选择和 Skills 选择的交互状态也纳入设计。';refreshComposer('#chat-input');return;}
    if(k==='reply-copy'){const target=$(v==='session-code'?'#session-code':'#reply-content');const text=target?.innerText||'';if(navigator.clipboard)navigator.clipboard.writeText(text).then(()=>toast('已复制演示内容。')).catch(()=>toast('浏览器未允许复制。'));else toast('此环境不支持复制，请手动选择内容。');return;}
    if(k==='leave'){s.running=false;s.task='cancelled';if(s.chat==='running')s.chat='cancelled';closeDialog();go(a.slice(6));return;}
    if(k==='provider-select'){if(s.dirty){toast('请先保存或放弃当前修改。');return;}var chosen=modelProviders.find(function(provider){return provider.id===v;});if(!chosen)return;s.providerId=chosen.id;s.formName=chosen.name;s.endpoint='https://api.example.invalid/v1';s.configured=chosen.id!=='local';s.test='';s.dirty=false;updateSettings();return;}
    if(k==='provider-model'){var pair=v.split('/');var provider=modelProviders.find(function(x){return x.id===pair[0];});if(!provider)return;s.agent.provider=provider.id;s.agent.model=pair[1];s.provider=provider.name;updateSettings();toast('默认模型已切换（演示）。');return;}
    if(k==='module-filter'){s.moduleFilter=v;render();updateSettings();return;}
    if(k==='section'){if(s.dirty){toast('请先保存或放弃当前修改。');return;}if(s.section==='appearance'&&v!=='appearance')window.AppearanceEditor.discard();s.section=v;updateSettings();return;}
    if(k==='skill'){s.skill=Number(v);render();return;}
    if(k==='filter'){s.filter=v;render();return;}
    if(k==='suggest'){s.draft=v==='idea'?'帮我梳理这个想法的目标和边界：':'请保留原意，把下面这段话改得更清晰：';render();$('#chat-input').focus();return;}
    if(k==='launch'){createChat();s.draft=v==='idea'?'帮我梳理这个想法的目标和边界：':'请保留原意，把下面这段话改得更清晰：';render();$('#chat-input').focus();return;}
    if(k==='history'){const id=v==='writing'?2:1;if(chats.some(x=>x.id===id))safeGo('chat-tab:'+id);else safeGo('new-chat');return;}
    if(k==='scene'){saveChat();s.running=false;s.scene=v;s.review=false;s.loginError='';s.pairError='';if(v==='desktop'){s.scene='normal';s.page='desktop';}if(v==='normal'){s.page='workbench';s.chat='sample';s.mode='chat';s.configured=true;}if(v==='first'){s.scene='normal';s.page='desktop';s.mode='chat';s.chat='empty';s.configured=false;s.account='未创建账户';chats=[{id:nextChatId++,title:'新会话',chat:'empty',draft:'',message:''}];activeChatId=chats[0].id;}if(v==='error'){s.scene='normal';s.page='workbench';s.mode='chat';s.configured=true;s.chat='error';}if(v==='unknown'){s.scene='normal';s.page='task';s.mode='single';s.task='unknown';}render();return;}
    switch(a){
      case 'collapse':s.collapsed=!s.collapsed;render();break;
      case 'review':s.review=!s.review;render();break;
      case 'reset':location.reload();break;
      case 'toggle-theme':window.AppearanceEditor.toggleMode();render();break;
      case 'recent-task':s.task='done';safeGo('mode:single');break;
      case 'customize-desktop':modal(`<h2 id="dialog-title">整理桌面</h2><p>选择桌面上的功能入口与看板。原型中仅在本次浏览保留。</p><div class="stack"><h3>功能入口</h3>${desktopEntries.map(([id,label])=>`<label class="check-label"><input type="checkbox" data-desktop-item="${id}" ${s.desktopItems.includes(id)?'checked':''}>${label}</label>`).join('')}<h3>看板</h3>${[['recent','最近的工作'],['connections','可用能力']].map(([id,label])=>`<label class="check-label"><input type="checkbox" data-desktop-board="${id}" ${s.boards.includes(id)?'checked':''}>${label}</label>`).join('')}</div><div class="dialog-actions">${btn('取消','cancel-dialog','secondary')}${btn('应用到桌面','save-desktop','primary')}</div>`);break;
      case 'save-desktop':s.desktopItems=[...$('#dialog').querySelectorAll('[data-desktop-item]:checked')].map(x=>x.dataset.desktopItem);s.boards=[...$('#dialog').querySelectorAll('[data-desktop-board]:checked')].map(x=>x.dataset.desktopBoard);closeDialog();render();break;
      case 'send':send();break;
      case 'finish':s.running=false;s.chat='sample';render();break;
      case 'stop':s.running=false;if(s.mode==='single')s.task='cancelled';else s.chat='cancelled';render();toast('演示任务已停止。');break;
      case 'run-task':if(!s.taskDraft.trim()){toast('请输入任务内容。');break;}s.running=true;s.task=s.tool?'approval':'running';render();break;
      case 'allow-tool':s.task='running';render();break;
      case 'deny-tool':s.task='denied';s.running=false;render();break;
      case 'finish-task':s.task='done';s.running=false;render();break;
      case 'clear-search':s.search='';s.filter='all';render();break;
      case 'add-skill':if(!s.installed.includes(s.skill))s.installed.push(s.skill);render();toast('已添加到我的技能（模拟）。');break;
      case 'use-skill':s.taskSkill=skills[s.skill].name;s.task='idle';safeGo('mode:single');break;
      case 'import':modal(`<h2 id="dialog-title">导入技能</h2><p>演示来源预览与确认。不会读取文件或安装内容。</p><label class="field">来源<input value="sample-skill.zip" readonly></label><div class="tool-result">名称：示例导入技能<br>来源：本地 ZIP<br>许可：演示许可<br>内容：1 个 SKILL.md</div><div class="dialog-actions">${btn('取消','cancel-dialog','secondary')}${btn('确认导入','confirm-import','primary')}</div>`);break;
      case 'confirm-import':closeDialog();toast('已演示导入确认，未安装真实内容。');break;
      case 'cancel-dialog':closeDialog();break;
      case 'close-settings':closeSettings();break;
      case 'account-page':closeDialog();safeGo('page:account');break;
      case 'provider-select':toast('正在查看该服务的连接配置。');break;
      case 'add-provider':s.formName='新模型服务';s.endpoint='https://api.example.invalid/v1';s.dirty=true;updateSettings();$('#provider-name')?.focus();break;
      case 'save-provider-token':s.providerTokenSet=true;updateSettings();toast('凭据状态已保存（演示，不接收真实密钥）。');break;
      case 'clear-provider-token':s.providerTokenSet=false;updateSettings();toast('凭据已移除（演示）。');break;
      case 'test-provider':s.test=badge('连接成功（模拟）','good');$('#provider-test').innerHTML=s.test;break;
      case 'fail-provider':s.test=note('连接失败：请检查服务地址。此错误为模拟。','error');$('#provider-test').innerHTML=s.test;break;
      case 'save-provider':if(!s.formName.trim()||!/^https?:\/\//.test(s.endpoint)){s.test=note('填写名称及以 http:// 或 https:// 开头的地址。','error');$('#provider-test').innerHTML=s.test;return;}s.provider=s.formName.trim();s.configured=true;s.dirty=false;render();updateSettings();toast('配置已保存到演示内存。');break;
      case 'discard-provider':s.formName=s.provider;s.endpoint='https://api.example.invalid/v1';s.dirty=false;s.test='';updateSettings();break;
      case 'toggle-mcp':s.mcp=!s.mcp;if(!s.mcp)s.tool=false;render();updateSettings();break;
      case 'mcp-fail':$('#mcp-feedback').innerHTML=note('无法连接工具服务，请检查配置后重试。（模拟）','error');break;
      case 'mcp-use':closeDialog();s.tool=s.mcp;safeGo('mode:single');break;
      case 'toggle-module':s.enabled=!s.enabled;render();updateSettings();break;
      case 'toggle-permission':s.granted=!s.granted;render();updateSettings();break;
      case 'open-app':closeDialog();safeGo('page:app');break;
      case 'module-install':s.demoModule='installed';s.demoGranted=true;s.demoUpdate='idle';render();updateSettings();toast('示例模块已安装（演示）。');break;
      case 'module-toggle':s.demoModule=s.demoModule==='disabled'?'installed':'disabled';render();updateSettings();break;
      case 'module-crash':s.demoModule='crashed';render();updateSettings();toast('示例模块已模拟崩溃。');break;
      case 'module-recover':s.demoModule='installed';render();updateSettings();toast('示例模块已重启（演示）。');break;
      case 'module-remove':s.demoModule='pending';s.demoUpdate='idle';s.demoGranted=true;render();updateSettings();toast('示例模块已卸载。');break;
      case 'module-grant':s.demoGranted=!s.demoGranted;if(!s.demoGranted)s.demoUpdate='idle';render();updateSettings();toast(s.demoGranted?'示例模块已获得工具权限。':'示例模块已撤销工具权限。');break;
      case 'module-update':if(!s.demoGranted){toast('请先授予工具权限。');break;}s.demoUpdate=s.demoUpdate==='failed'?'updated':s.demoUpdate==='updated'?'idle':'failed';render();updateSettings();toast(s.demoUpdate==='failed'?'更新检查失败（演示），请重试。':s.demoUpdate==='updated'?'示例模块已更新到 1.2.1。':'已重置更新状态。');break;
      case 'app-run':s.appRunning=true;$('#app-result').innerHTML=note('模拟调用完成。这是固定演示结果，没有执行真实模型请求。');break;
      case 'toggle-lan':s.lan=!s.lan;if(!s.lan)s.paired=false;render();updateSettings();break;
      case 'remote-preview':closeDialog();s.scene='remote';s.review=false;render();break;
      case 'revoke':s.paired=false;render();updateSettings();toast('已撤销演示设备访问。');break;
      case 'remote-exit':s.paired=false;render();break;
      case 'switch-account':confirm('切换到演示账户 B','将停止任务并清除当前账户草稿。正式产品还需要重新认证。','confirm-switch','模拟切换');break;
      case 'confirm-switch':closeDialog();clearAccount();s.account='演示账户 B';s.page='desktop';render();break;
      case 'logout':clearAccount();s.scene='login';render();break;
      case 'delete-account':confirm('删除这个本地账户？','正式产品会删除账户的数据与授权，无法撤销。本次只展示确认与退出流程。','confirm-delete','模拟删除');break;
      case 'password':modal(`<h2 id="dialog-title">修改密码</h2><p>只输入演示口令 demo，不要使用真实密码。</p><form id="password-form" class="stack"><label class="field">当前密码<input type="password" name="current" required autocomplete="off"></label><label class="field">新密码<input type="password" name="next" required autocomplete="off"></label><div id="password-error"></div><div class="row">${btn('取消','cancel-dialog','secondary')}<button type="submit" class="primary">模拟保存</button></div></form>`);break;
      case 'preview-action':toast('主要操作反馈示例。');break;
      case 'copy':if(navigator.clipboard)navigator.clipboard.writeText('DGOS 原型演示：工作台、技能中心、应用与设置。').then(()=>toast('已复制演示摘要。')).catch(()=>toast('浏览器未允许复制，请手动选择内容。'));else toast('此环境不支持复制，请手动选择内容。');break;
    }
  });
  document.addEventListener('input',e=>{const {id,value}=e.target;if(id==='chat-input')s.draft=value;if(id==='task-input')s.taskDraft=value;if(id==='provider-name'||id==='provider-endpoint'){s[id==='provider-name'?'formName':'endpoint']=value;s.dirty=true;$('#dirty-status').textContent='有未保存的修改';}if(id==='skill-search'){const pos=e.target.selectionStart;s.search=value;render();$('#skill-search').focus();$('#skill-search').setSelectionRange(pos,pos);}});
  document.addEventListener('change',e=>{if(e.target.id==='task-skill')s.taskSkill=e.target.value;if(e.target.id==='task-tool')s.tool=e.target.checked;});
  document.addEventListener('input',e=>{
    if(e.target.id==='agent-model-search'){modelQuery=e.target.value;const start=e.target.selectionStart;$('#agent-model-results').innerHTML=modelResults();e.target.setSelectionRange(start,start);positionComposerMenu();return;}
    if(e.target.id==='agent-skill-search'){skillQuery=e.target.value;const start=e.target.selectionStart;$('#agent-skill-results').innerHTML=skillResults();e.target.setSelectionRange(start,start);positionComposerMenu();return;}
    if(e.target.id==='agent-temperature'){s.agent.temperature=e.target.value;$('#agent-temperature-value').textContent=e.target.value;return;}
    if(e.target.id==='agent-instructions'){s.agent.instructions=e.target.value;return;}
  });
  document.addEventListener('change',e=>{if(e.target.id==='agent-tools')s.agent.tools=e.target.checked;});
  document.addEventListener('keydown',e=>{
    if(composerMenu&&e.key==='Escape'){e.preventDefault();closeComposerMenu(true);return;}
    if(e.target.matches('[role="tab"][data-action^="chat-tab:"]')){
      const index=chats.findIndex(x=>x.id===activeChatId);
      if(['ArrowLeft','ArrowRight','Home','End'].includes(e.key)){e.preventDefault();const next=e.key==='Home'?0:e.key==='End'?chats.length-1:(index+(e.key==='ArrowRight'?1:-1)+chats.length)%chats.length;safeGo('chat-tab:'+chats[next].id);}
      if(e.key==='Delete'){e.preventDefault();safeGo('close-chat:'+activeChatId);}
    }
    const dialog=$('#dialog');
    if(e.key==='Tab'&&dialog.open){
      const items=[...dialog.querySelectorAll('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),summary,[tabindex="0"]')].filter(x=>x.getClientRects().length);
      const first=items[0],last=items[items.length-1];
      if(items.length&&((e.shiftKey&&document.activeElement===first)||(!e.shiftKey&&document.activeElement===last)||!dialog.contains(document.activeElement))){e.preventDefault();(e.shiftKey?last:first).focus();}
    }
    if(e.target.id==='chat-input'&&e.key==='Enter'&&!e.shiftKey&&!e.isComposing){e.preventDefault();send();}
  });
  $('#dialog').addEventListener('cancel',e=>{e.preventDefault();s.settingsOpen?closeSettings():closeDialog();});
  window.AppearanceEditor.connect({refresh:updateSettings,notify:toast,sync(config){s.theme=config.mode;s.density=config.density;s.contentSize=String(config.typography.readingSize);}});
  window.__dgosProto={get state(){return s;},get chats(){return chats;},get activeChatId(){return activeChatId;},get skills(){return skills;},go,render,openSettings,closeDialog,toast,icon,tile,badge,esc,btn};
  render();
})();
