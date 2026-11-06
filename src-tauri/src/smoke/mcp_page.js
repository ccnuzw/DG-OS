// Evaluated only by the opt-in Rust acceptance driver against embedded production assets.
// No application/store state or responses are substituted here.
const all=(s,r=document)=>Array.from(r?.querySelectorAll(s)||[]);
const text=e=>(e?.textContent||'').trim();
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,ms=5000)=>{const t=Date.now();let last;while(Date.now()-t<ms){try{const v=fn();if(v)return v;}catch(error){last=String(error);}await wait(50);}if(last)throw Error('condition deadline: '+last);return false;};
const frame=async()=>{await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));await wait(150);};
const page=()=>document.querySelector('[data-workspace-surface="/mcp"] .mcp-page');
const rows=()=>all('.mcp-server-select',page());
const row=name=>{const found=rows().find(e=>all('span',e).some(n=>text(n)===name));if(!found)throw Error('server row missing: '+name);return found;};
const detail=()=>page()?.querySelector('.mcp-server-detail');
const form=()=>detail()?.querySelector('form[aria-label="Server configuration"]');
const dialog=()=>all('[role=dialog],[role=alertdialog]').find(e=>e.getBoundingClientRect().width>0&&!e.closest('[hidden]'));
const feedback=()=>detail()?.querySelector('[aria-label="Tool refresh result"]');
const visible=e=>!!e&&e.getBoundingClientRect().width>0&&e.getBoundingClientRect().height>0&&!e.closest('[hidden],[inert]')&&getComputedStyle(e).visibility!=='hidden';
const button=(label,root=document)=>all('button',root).find(e=>text(e)===label&&visible(e));
const field=label=>{const root=form();if(!root)return null;const named=all('input,textarea',root).find(e=>e.getAttribute('aria-label')===label);if(named)return named;const l=all('label',root).find(e=>text(e)===label);return l?(l.htmlFor?document.getElementById(l.htmlFor):l.querySelector('input,textarea')):null;};
const set=(el,value)=>{if(!el)throw Error('input absent');const proto=el instanceof HTMLTextAreaElement?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(el,value);el.dispatchEvent(new Event('input',{bubbles:true}));};
const go=path=>{const state={...history.state,idx:(history.state?.idx??0)+1,key:crypto.randomUUID()};history.pushState(state,'',path);dispatchEvent(new PopStateEvent('popstate',{state}));};
const rect=e=>{if(!e)return null;const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom};};
const audit=(surface,width)=>{
  const scope=surface==='modal'?dialog():surface==='form'?form():surface==='list'?page()?.querySelector('.mcp-server-list'):detail();
  if(!scope||!visible(scope))return {ok:false,error:'audit surface invisible',surface};
  const p=page(),r=scope.getBoundingClientRect(),pr=p.getBoundingClientRect();
  const horizontal=all('button,input,textarea,[role=combobox],h1,h2,strong,code,pre,.mcp-log-list',scope).filter(visible).filter(e=>{
    const b=e.getBoundingClientRect();return b.left<r.left-2||b.right>r.right+2;
  }).map(e=>({tag:e.tagName,label:e.type==='password'?'[password redacted]':text(e).slice(0,180),rect:rect(e)}));
  const pageOverflow=p.scrollWidth>p.clientWidth+2||document.documentElement.scrollWidth>innerWidth+2;
  const modalFits=surface!=='modal'||r.left>=-1&&r.right<=innerWidth+1&&r.top>=-1&&r.bottom<=innerHeight+1;
  const compact=!!document.querySelector('.mcp-columns.is-compact');
  const expected=width!==520||compact;
  return {ok:!pageOverflow&&horizontal.length===0&&modalFits&&expected,surface,viewport:{width:innerWidth,height:innerHeight},scope:rect(scope),page:rect(p),pageWidth:pr.width,pageScrollWidth:p.scrollWidth,pageClientWidth:p.clientWidth,documentScrollWidth:document.documentElement.scrollWidth,compact,horizontal,modalFits,rootFont:getComputedStyle(document.documentElement).fontSize,uiFont:getComputedStyle(document.documentElement).getPropertyValue('--font-size-ui'),theme:document.documentElement.dataset.theme,density:document.documentElement.dataset.density};
};
const snapshot=()=>({ok:true,url:location.href,at:new Date().toISOString(),visibility:document.visibilityState,hasFocus:document.hasFocus(),active:{tag:document.activeElement?.tagName,label:document.activeElement?.getAttribute('aria-label'),class:document.activeElement?.className,rect:rect(document.activeElement)},theme:document.documentElement.dataset.theme,density:document.documentElement.dataset.density,viewport:{width:innerWidth,height:innerHeight},body:document.body.innerText,rows:rows().map(e=>({text:text(e),pressed:e.getAttribute('aria-pressed'),rect:rect(e)})),inputs:all('input,textarea',page()).map(e=>({label:e.getAttribute('aria-label')||e.labels?.[0]?.textContent,type:e.type,value:e.type==='password'?'[redacted]':e.value,disabled:e.disabled,rect:rect(e)})),dialogs:all('[role=dialog],[role=alertdialog]').map(e=>({text:text(e),rect:rect(e)}))});
const assets=async()=>{
  const hash=async b=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',b))).map(x=>x.toString(16).padStart(2,'0')).join('');
  const urls=[...new Set([location.origin+'/',...all('script[src],link[href]').map(e=>e.src||e.href),...performance.getEntriesByType('resource').map(e=>e.name)].filter(u=>u.startsWith(location.origin)&&(/\/assets\/.*\.(js|css)(?:[?#]|$)/.test(u)||u===location.origin+'/')))];
  const resources=await Promise.all(urls.map(async url=>{const c=new AbortController(),timer=setTimeout(()=>c.abort(),4000);try{const response=await fetch(url,{cache:'no-store',signal:c.signal});if(!response.ok)throw Error(String(response.status));const bytes=await response.arrayBuffer();return {url,bytes:bytes.byteLength,sha256:await hash(bytes)};}catch(error){return {url,error:String(error)};}finally{clearTimeout(timer);}}));
  const embedded=!/^https?:$/.test(location.protocol)||location.hostname==='tauri.localhost';
  return {ok:embedded&&resources.some(r=>/McpManagement-[^/]+\.js/.test(r.url))&&resources.every(r=>r.sha256),source:'embedded production dist only; freshly fetched same-origin resource hashes; no preview or dev override',url:location.href,ua:navigator.userAgent,resources};
};
