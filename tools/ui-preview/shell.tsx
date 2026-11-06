import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { applyTheme } from '../../src/ui/theme/apply';
import { DEFAULTS } from '../../src/ui/appearance/model';
import './preview.css';
applyTheme(document.documentElement, DEFAULTS, 'light');
const scenes = ['controls','modal','patterns-ready','patterns-initial-error','patterns-refresh-error','patterns-empty','patterns-unconfirmed','settings','chat','chat-streaming','chat-approval','chat-error'];
const names = ['基础控件','长表单弹窗','列表与详情','首次失败','刷新失败保留旧数据','空列表','未确认结果','设置外壳与CLI','聊天长正文与代码','聊天运行中快照','聊天长参数审批','聊天错误与未知结果'];
const fields = [
  ['scenario','场景',scenes], ['locale','语言',['zh-CN','en-US']], ['theme','主题',['light','dark']],
  ['density','密度',['75','100','125','150','175']], ['uiSize','UI字号',['12','13','14','16']],
  ['readingSize','阅读字号',['13','14','16','18','20']], ['codeSize','代码字号',['12','13','14','16','18']],
  ['width','容器宽度',['380','520','800','1280']], ['fault','设置故障',['none','initial-error','probe-error','unknown','conflict']],
] as const;
const defaults = { scenario:'controls', locale:'zh-CN', theme:'light', density:'100', uiSize:'12', readingSize:'13', codeSize:'12', width:'1280', fault:'none' };
function PreviewShell() {
  const [params, setParams] = useState(() => {
    const query = new URLSearchParams(location.search);
    for (const [key,,options] of fields) if (!(options as readonly string[]).includes(query.get(key) ?? '')) query.set(key, defaults[key]);
    return query;
  });
  const [revision, reset] = useState(0);
  const update = (key: string, value: string) => {
    const next = new URLSearchParams(params); next.set(key,value); setParams(next); history.replaceState(null,'',`?${next}`);
  };
  return <><header className="preview-toolbar"><div><h1>DG-OS · 开发组件预览</h1><p>生产组件 · 隔离数据 · 无真实账户或外部执行</p></div>{fields.map(([key,label,options]) => <label key={key}>{label}<select value={params.get(key)!} onChange={event => update(key,event.target.value)}>{options.map((value,i) => <option key={value} value={value}>{key === 'scenario' ? names[i] : value}</option>)}</select></label>)}<button onClick={() => reset(value => value+1)}>重置当前场景</button></header><p className="preview-status">切换任一参数会重新载入场景，清除草稿、保存结果与浮层。流式场景为固定快照；MCP样板阻塞、未完成；xterm 6.0.0离屏reflow辅助树重复已复现、未修复；VoiceOver和独立物理键鼠未验收。本入口只服务开发回归。 <a href="/evidence/implementation" target="_blank" rel="noreferrer">MCP与xterm状态及验证证据（只读）</a> · <a href="/evidence/xterm-source" target="_blank" rel="noreferrer">xterm最小复现源码（只读，不执行）</a></p><div className="preview-stage"><iframe key={`${params}-${revision}`} id="preview-frame" title="隔离的生产组件场景" src={`/frame.html?${params}`} style={{width:Number(params.get('width'))}} /></div></>;
}
createRoot(document.getElementById('root')!).render(<PreviewShell/>);
