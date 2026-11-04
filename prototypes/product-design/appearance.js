/* Design asset: declarative themes and a reversible, memory-only editor. */
(() => {
  'use strict';
  const clone = value => JSON.parse(JSON.stringify(value));
  const esc = value => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const fonts = {
    system: '-apple-system, BlinkMacSystemFont, "PingFang SC", "Microsoft YaHei", sans-serif',
    humanist: '"Avenir Next", "Segoe UI", "PingFang SC", sans-serif',
    serif: '"Songti SC", "Noto Serif CJK SC", Georgia, serif',
    mono: 'ui-monospace, "SFMono-Regular", Consolas, monospace',
  };
  const presets = {
    studio: {name:'暖灰工作室', desc:'暖灰表面、琥珀操作与轻量分栏', light:{accent:'#A46A24',background:'#F8F6EF',foreground:'#29323D'}, dark:{accent:'#DDB576',background:'#242521',foreground:'#EAE9E2'}},
    precision: {name:'精密蓝', desc:'清晰、中性，适合日常工作', light:{accent:'#1555D9',background:'#FAF8F1',foreground:'#242831'}, dark:{accent:'#94B7FF',background:'#202321',foreground:'#EEEEE7'}},
    paper: {name:'暖纸琥珀', desc:'柔和暖底，适合长时间阅读', light:{accent:'#985B05',background:'#FAF6ED',foreground:'#303129'}, dark:{accent:'#E9BE76',background:'#26231E',foreground:'#F0E8DA'}},
    forest: {name:'静谧松绿', desc:'低饱和表面与沉静绿色', light:{accent:'#176748',background:'#F3F7F3',foreground:'#26382E'}, dark:{accent:'#89D8AF',background:'#19251F',foreground:'#E0EEE5'}},
    slate: {name:'石墨紫', desc:'冷灰层次，专注内容', light:{accent:'#6942B5',background:'#F5F5FA',foreground:'#30313E'}, dark:{accent:'#C5ABFF',background:'#22212B',foreground:'#EEEBF6'}},
  };
  const scopes = {interface:'操作界面', reading:'阅读与输入', code:'代码与技术值'};
  const defaults = () => ({mode:'light', variants:{light:{preset:'studio',overrides:{}},dark:{preset:'studio',overrides:{}}}, typography:{uiFont:'system',readingFont:'system',codeFont:'mono',uiSize:12,readingSize:13,codeSize:12,lineHeight:1.75}, density:'1', transparency:false, motion:'system'});
  let saved = defaults(), draft = null, editMode = 'light', scope = 'interface', error = '', hooks;
  const mq = matchMedia('(prefers-color-scheme: dark)');
  const resolveMode = config => config.mode === 'auto' ? (mq.matches ? 'dark' : 'light') : config.mode;
  const rgb = hex => hex.slice(1).match(/../g).map(v => parseInt(v,16));
  const mix = (a,b,t) => '#' + rgb(a).map((v,i)=>Math.round(v*(1-t)+rgb(b)[i]*t).toString(16).padStart(2,'0')).join('');
  function contrast(a,b) {
    const luminance = hex => rgb(hex).map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4).reduce((sum,v,i)=>sum+v*[.2126,.7152,.0722][i],0);
    const x=luminance(a),y=luminance(b); return (Math.max(x,y)+.05)/(Math.min(x,y)+.05);
  }
  function colors(config,mode,area='interface') {
    const v=config.variants[mode], base=presets[v.preset][mode];
    const ui={...base,...v.overrides.interface};
    return area==='interface'?ui:{...ui,...v.overrides[area]};
  }
  function issues(config) {
    const result=[];
    for(const mode of ['light','dark']) for(const area of Object.keys(scopes)) {
      const c=colors(config,mode,area);
      if(contrast(c.foreground,c.background)<4.5) result.push(`${mode==='light'?'浅色':'深色'} · ${scopes[area]}：文字对比度不足 4.5:1`);
      if(area==='interface' && contrast(c.accent,c.background)<3) result.push(`${mode==='light'?'浅色':'深色'} · 强调色与背景不足 3:1`);
    }
    return result;
  }
  function variables(target,config,mode) {
    const c=colors(config,mode), t=config.typography, dark=mode==='dark';
    const set=(key,value)=>target.style.setProperty(key,value);
    const panelBase=mix(c.background,'#FFFFFF',dark?.055:.60);
    const railBase=mix(c.background,c.foreground,.035);
    const insetBase=mix(c.background,c.foreground,.025);
    const panel=mix(panelBase,c.accent,dark?.055:.05);
    const rail=mix(railBase,c.accent,dark?.06:.045);
    const inset=mix(insetBase,c.accent,dark?.035:.02);
    const muted=mix(c.foreground,c.background,.25);
    const tokens={canvas:c.background,rail,panel,inset,text:c.foreground,muted,border:mix(c.background,c.foreground,.15),control:mix(c.background,c.foreground,.32),blue:c.accent,'blue-soft':mix(c.background,c.accent,.12),'on-accent':contrast(c.accent,'#FFFFFF')>=contrast(c.accent,'#151923')?'#FFFFFF':'#151923','ui-font':fonts[t.uiFont],'reading-font':fonts[t.readingFont],'code-font':fonts[t.codeFont],'ui-font-size':t.uiSize+'px','content-size':t.readingSize+'px','code-size':t.codeSize+'px','reading-line-height':t.lineHeight,'ui-scale':config.density};
    for(const area of ['reading','code']) {const value=colors(config,mode,area);tokens[area+'-background']=value.background;tokens[area+'-foreground']=value.foreground;}
    for(const [key,value] of Object.entries(tokens)) set('--'+key,value);
    target.dataset.theme=mode;
    target.dataset.transparency=String(config.transparency);
    target.dataset.motion=config.motion;
  }
  function apply(config) {variables(document.documentElement,config,resolveMode(config));hooks?.sync(config);}
  function show() {
    if(!draft) return;
    for(const swatch of document.querySelectorAll('.preset-swatches')) {
      const palette=presets[swatch.dataset.preset][swatch.dataset.variant];
      for(const [key,color] of Object.entries(palette)) swatch.style.setProperty('--swatch-'+key,color);
    }
    if(!issues(draft).length&&!error) apply(draft); else apply(saved);
    const contrastValue=document.querySelector('#theme-contrast');
    const c=colors(draft,editMode,scope);
    if(contrastValue) contrastValue.textContent=`${contrast(c.foreground,c.background).toFixed(2)} : 1 · ${scopes[scope]}文字对比度`;
    const warning=document.querySelector('#theme-validation');
    if(warning) {const problems=issues(draft);warning.textContent=error || (problems.length?problems.join('；'):'文字与强调色基础配对检查通过；完整组件验收仍待生产实现。');warning.dataset.invalid=String(!!error||!!problems.length);}
    const applyButton=document.querySelector('[data-appearance-action="apply"]');
    if(applyButton) applyButton.disabled=!!issues(draft).length||!!error;
  }
  function refresh() {
    if(!draft) return;
    const main=document.querySelector('.settings-main'); if(!main) return;
    const scroll=main.scrollTop, focused=document.activeElement?.id;
    hooks.refresh(); show(); document.querySelector('.settings-main').scrollTop=scroll;
    if(focused) document.getElementById(focused)?.focus({preventScroll:true});
  }
  const option=(value,label,current)=>`<option value="${value}" ${String(value)===String(current)?'selected':''}>${label}</option>`;
  function select(id,label,value,options,desc='') {return `<label class="theme-field" for="${id}"><span>${label}</span><select id="${id}" data-appearance-field="${id}">${options.map(([v,l])=>option(v,l,value)).join('')}</select>${desc?`<small>${desc}</small>`:''}</label>`;}
  const action=(id,label,cls='secondary',attrs='')=>`<button type="button" class="${cls}" data-appearance-action="${id}" ${attrs}>${label}</button>`;
  function begin() {if(!draft){draft=clone(saved);editMode=resolveMode(saved);error='';}}
  function render() {
    begin();
    const variant=draft.variants[editMode], c=colors(draft,editMode,scope), t=draft.typography;
    const modified=Object.values(variant.overrides).reduce((n,x)=>n+Object.keys(x).length,0);
    return `<div class="appearance-editor">
      <section class="theme-mode-row" aria-label="主题模式"><div><h2>属于你的工作环境</h2><small>先选套装，再微调。浅色与深色分别保留。</small></div><div class="theme-segment">${[['light','浅色'],['dark','深色'],['auto','跟随系统']].map(([v,l])=>`<button type="button" data-appearance-action="mode:${v}" aria-pressed="${draft.mode===v}">${l}</button>`).join('')}</div></section>
      <div class="theme-controls">
      <section class="theme-section"><div class="row spread"><h2>主题套装</h2><div class="theme-segment">${[['light','浅色配置'],['dark','深色配置']].map(([v,l])=>`<button type="button" data-appearance-action="edit:${v}" aria-pressed="${editMode===v}">${l}</button>`).join('')}</div></div>
        <div class="theme-preset-grid">${Object.entries(presets).map(([id,p])=>`<button type="button" class="theme-preset ${variant.preset===id?'selected':''}" data-appearance-action="preset:${id}" aria-pressed="${variant.preset===id}" title="${p.desc}"><span class="preset-swatches" data-preset="${id}" data-variant="${editMode}"><i></i><i></i><i></i></span><span>${p.name}</span>${variant.preset===id?'<small>已选</small>':''}</button>`).join('')}</div><p class="theme-hint">${editMode==='light'?'浅色':'深色'}配置 · ${modified?modified+' 项自定义；更换套装时保留':'全部继承套装'}。套装不更改字体和尺寸。</p>
      </section>
      <section class="theme-section"><div class="row spread"><h2>颜色细节</h2>${action('reset-scope','恢复此区域','link')}</div>
        ${select('theme-scope','调整区域',scope,Object.entries(scopes),'操作界面覆盖全局；阅读与代码只覆盖各自内容。')}
        <div class="theme-color-list">${[['accent','强调色','主要操作、选中态与焦点'],['background','背景','当前区域的基础表面'],['foreground','前景','当前区域的主要文字']].filter(([key])=>scope==='interface'||key!=='accent').map(([key,label,desc])=>`<div class="theme-color-row"><div><label for="color-${key}">${label}</label><small>${desc}</small></div><div class="color-inputs"><input type="color" aria-label="${label}取色器" data-theme-color="${key}" value="${c[key]}"><input id="color-${key}" aria-label="${label}十六进制" data-theme-hex="${key}" value="${c[key].toUpperCase()}" maxlength="7" spellcheck="false"><button type="button" class="link inherit-button" data-appearance-action="inherit:${key}" ${!variant.overrides[scope]?.[key]?'disabled':''}>${variant.overrides[scope]?.[key]?'自定义 · 恢复':'继承'}</button></div></div>`).join('')}</div>
        <div id="theme-contrast" class="theme-contrast" aria-live="polite">${contrast(c.foreground,c.background).toFixed(2)} : 1 · ${scopes[scope]}文字对比度</div>
        <p id="theme-validation" role="status" class="theme-validation"></p>
      </section>
      <section class="theme-section"><h2>字体与排版</h2><p class="theme-hint">与明暗模式无关。仅使用本机字体，缺失时自动回退。</p><div class="theme-type-grid">
        ${select('uiFont','界面字体',t.uiFont,[['system','系统无衬线'],['humanist','人文无衬线']],'导航、按钮、表单与标题')}
        ${select('uiSize','界面字号',t.uiSize,[12,13,14,16].map(v=>[v,v+'px']))}
        ${select('readingFont','内容字体',t.readingFont,[['system','跟随系统无衬线'],['humanist','人文无衬线'],['serif','宋体 / 衬线']],'消息、输入与任务正文')}
        ${select('content-size','内容字号',t.readingSize,[13,14,16,18,20].map(v=>[v,v+'px']))}
        ${select('codeFont','代码字体',t.codeFont,[['mono','系统等宽'],['system','系统无衬线（含中文）']],'代码块、日志与技术标识')}
        ${select('codeSize','代码字号',t.codeSize,[12,13,14,16,18].map(v=>[v,v+'px']))}
        ${select('lineHeight','内容行高',t.lineHeight,[[1.5,'紧凑 · 1.5'],[1.75,'标准 · 1.75'],[2,'宽松 · 2.0']])}
        ${select('density','界面密度',draft.density,[['1','紧凑 · 默认'],['1.12','标准'],['1.25','宽松']],'控制几何尺寸；正文与代码字号独立')}
      </div></section>
      <details class="theme-section"><summary>更多外观选项</summary><label class="theme-check"><input type="checkbox" data-appearance-field="transparency" ${draft.transparency?'checked':''}>柔化侧栏背景 <small>仅模拟半透明叠色，无原生毛玻璃</small></label>${select('motion','动态效果',draft.motion,[['system','遵循系统'],['reduce','减少动态效果']])}<p class="theme-hint">状态色与权限提示保持系统语义，不随强调色改写。</p></details>
      <section class="theme-section"><h2>界面语言</h2><p class="theme-hint">语言是独立偏好，不随主题套装改变。本原型暂为中文稿。</p><select aria-label="界面语言" disabled><option>简体中文</option></select></section>
      </div>
    </div>`;
  }
  function footer() {return `<div class="theme-footer"><details class="theme-reset-menu"><summary>恢复默认</summary><div class="theme-reset-options">${action('reset-variant','恢复当前明暗配置','link')}${action('reset-all','恢复全部外观','link')}</div></details><small>预览不会自动保存</small><div class="row">${action('cancel','取消预览')}${action('apply','应用外观','primary')}</div></div>`;}
  function discard() {if(draft) apply(saved);draft=null;error='';}
  function updateColor(key,value) {
    if(!/^#[0-9a-f]{6}$/i.test(value)) {error='请输入 #RRGGBB 格式的颜色。';show();return;}
    error='';const v=draft.variants[editMode];v.overrides[scope]??={};v.overrides[scope][key]=value.toUpperCase();refresh();
  }
  document.addEventListener('click',event=>{
    const button=event.target.closest('[data-appearance-action]');if(!button)return;
    const [action,value]=button.dataset.appearanceAction.split(':');
    if(action==='mode') {draft.mode=value;editMode=value==='auto'?resolveMode(draft):value;}
    if(action==='edit') editMode=value;
    if(action==='preset') draft.variants[editMode].preset=value;
    if(action==='inherit') delete draft.variants[editMode].overrides[scope]?.[value];
    if(action==='reset-scope') delete draft.variants[editMode].overrides[scope];
    if(action==='reset-variant') draft.variants[editMode]={preset:'precision',overrides:{}};
    if(action==='reset-all') draft=defaults();
    if(['inherit','reset-scope','reset-variant','reset-all','preset'].includes(action)) error='';
    if(action==='apply') {if(issues(draft).length||error)return;saved=clone(draft);apply(saved);editMode=resolveMode(saved);hooks.notify('外观已应用到演示内存。');}
    if(action==='cancel') {discard();begin();hooks.notify('已恢复最近应用的外观。');}
    refresh();
  });
  document.addEventListener('change',event=>{
    const el=event.target;
    if(el.dataset.themeColor) {updateColor(el.dataset.themeColor,el.value);return;}
    if(el.dataset.themeHex) {updateColor(el.dataset.themeHex,el.value);return;}
    const key=el.dataset.appearanceField;if(!key)return;
    if(key==='theme-scope') scope=el.value;
    else if(key==='transparency') draft.transparency=el.checked;
    else if(key==='density'||key==='motion') draft[key]=el.value;
    else {const field=key==='content-size'?'readingSize':key;draft.typography[field]=['uiSize','readingSize','codeSize','lineHeight'].includes(field)?Number(el.value):el.value;}
    refresh();
  });
  mq.addEventListener('change',()=>{if(draft&&draft.mode==='auto')editMode=resolveMode(draft);if(draft&&document.querySelector('.settings-main'))refresh();else apply(saved);});
  window.AppearanceEditor={render,footer,show,discard,connect(value){hooks=value;apply(saved);},toggleMode(){discard();saved.mode=resolveMode(saved)==='light'?'dark':'light';apply(saved);},current(){return clone(saved);}};
})();
