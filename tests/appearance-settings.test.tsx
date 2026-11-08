import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Link, MemoryRouter } from 'react-router-dom';
import { disposeAppearance, initAppearance } from '@dgos/ui';
import AppearanceSettings from '../src/pages/AppearanceSettings';
import { DEFAULTS, type AppearanceSnapshot, type Appearance } from '../src/ui/appearance/model';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';
import { resetFontCatalogueForTesting } from '../src/ui/theme/fontCatalogue';

const installedFonts = [
  {family:'Aster Sans',monospace:false},
  {family:'Mono in name only',monospace:false},
  {family:'Quiet Code',monospace:true},
  {family:'青松正文',monospace:false},
];

const host = vi.hoisted(() => ({ stored: null as AppearanceSnapshot|null, sets: [] as unknown[], fail: false, gate:null as Promise<void>|null, fonts:[] as {family:string;monospace:boolean}[], fontsFail:false, fontsGate:null as Promise<void>|null }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: async (command:string,args?:{snapshot:Appearance;expectedRevision:number}) => {
  if(command==='list_local_fonts') {
    if(host.fontsGate)await host.fontsGate;
    return host.fontsFail ? {success:false,error:{code:1000000,message:'private font scanner details'}} : {success:true,data:structuredClone(host.fonts)};
  }
  if(command==='get_appearance_preferences')return {success:true,data:{kind:'ready',snapshot:host.stored,repairWarnings:[]}};
  if(command==='set_appearance_preferences') {
    host.sets.push(structuredClone(args));
    if(host.gate)await host.gate;
    if(host.fail)return {success:false,error:{code:1000000,message:'private storage details'}};
    host.stored={...args!.snapshot,revision:args!.expectedRevision+1};
    return {success:true,data:{kind:'saved',snapshot:host.stored}};
  }
  return null;
}}));
beforeEach(()=>{host.stored={...structuredClone(DEFAULTS),revision:4};host.sets=[];host.fail=false;host.gate=null;host.fonts=structuredClone(installedFonts);host.fontsFail=false;host.fontsGate=null;resetFontCatalogueForTesting();setLocaleForTesting('zh-CN');});
afterEach(()=>{cleanup();disposeAppearance();resetFontCatalogueForTesting();resetLocaleForTesting();});
async function open(){await initAppearance();render(<MemoryRouter  future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><AppearanceSettings/></MemoryRouter>);}
async function choosePaper(){fireEvent.click(within(screen.getByRole('region',{name:'浅色主题'})).getByRole('button',{name:'暖灰工作室 ▾'}));fireEvent.click(await screen.findByRole('button',{name:'暖纸琥珀'}));}
function dark(){return within(screen.getByRole('radiogroup',{name:'主题'})).getByRole('radio',{name:'深色'});}
async function fontPicker(role:'ui'|'reading'|'code') {
  fireEvent.click(screen.getByTestId(`appearance-${role}-font`));
  const dialog=await screen.findByRole('dialog',{name:role==='ui'?'界面字体':role==='reading'?'阅读字体':'代码字体'});
  return within(dialog).getByRole('searchbox',{name:'搜索本机字体'});
}
function localOption(family:string) {
  const option=screen.getAllByRole('option').find(item=>item.getAttribute('data-font-family')===family);
  if(!option)throw new Error(`Installed font option missing: ${family}`);
  return option;
}
function localOptions(){return screen.queryAllByRole('option').filter(item=>item.hasAttribute('data-font-family'));}
async function chooseFont(role:'ui'|'reading'|'code',family:string){
  const search=await fontPicker(role);fireEvent.change(search,{target:{value:family}});
  await waitFor(()=>expect(localOptions().some(item=>item.getAttribute('data-font-family')===family)).toBe(true));
  fireEvent.click(localOption(family));
  await waitFor(()=>expect(screen.queryByRole('searchbox',{name:'搜索本机字体'})).toBeNull());
}
describe('appearance draft transaction',()=>{
  it('previews without writing and cancel restores confirmed appearance',async()=>{
    await open();fireEvent.click(dark());
    expect(document.documentElement.dataset.theme).toBe('dark');expect(host.sets).toEqual([]);
    fireEvent.click(screen.getByRole('button',{name:'取消'}));
    expect(document.documentElement.dataset.theme).toBe('light');expect(host.sets).toEqual([]);
  });
  it('applies a complete draft once with its confirmed revision',async()=>{
    await open();fireEvent.click(dark());
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    await waitFor(()=>expect(host.stored?.revision).toBe(5));
    expect(host.sets).toEqual([{snapshot:{...DEFAULTS,mode:'dark'},expectedRevision:4}]);
    expect(document.documentElement.dataset.theme).toBe('dark');
  });
  it('offers all ten layout scales by keyboard and buttons, previews without writing, and cancels or saves',async()=>{
    await open();
    expect(screen.getByText('调整控件、图标与留白，不改变字号。')).toBeTruthy();
    const slider=screen.getByRole('slider',{name:'布局缩放'});
    const decrease=screen.getByRole('button',{name:'布局缩小 10%'});
    const increase=screen.getByRole('button',{name:'布局放大 10%'});
    const reset=screen.getByRole('button',{name:'恢复 100%'});
    const current=screen.getByLabelText('当前布局缩放');
    expect(slider.getAttribute('aria-valuemin')).toBe('60');expect(slider.getAttribute('aria-valuemax')).toBe('150');
    expect(current.textContent).toBe('100%');expect(reset).toHaveProperty('disabled',true);
    slider.focus();fireEvent.keyDown(slider,{key:'Home'});
    expect(decrease).toHaveProperty('disabled',true);
    for(const value of [60,70,80,90,100,110,120,130,140,150]){
      expect(slider.getAttribute('aria-valuenow')).toBe(String(value));
      expect(current.textContent).toBe(`${value}%`);expect(document.documentElement.dataset.density).toBe(String(value));
      if(value<150)fireEvent.keyDown(slider,{key:'ArrowRight'});
    }
    expect(increase).toHaveProperty('disabled',true);fireEvent.click(increase);expect(current.textContent).toBe('150%');
    fireEvent.keyDown(slider,{key:'ArrowLeft'});expect(current.textContent).toBe('140%');
    fireEvent.click(decrease);expect(current.textContent).toBe('130%');
    fireEvent.click(increase);expect(current.textContent).toBe('140%');
    fireEvent.click(reset);expect(current.textContent).toBe('100%');
    fireEvent.click(increase);expect(current.textContent).toBe('110%');
    expect(document.documentElement.style.getPropertyValue('--font-size-ui')).toBe('12px');expect(host.sets).toEqual([]);
    fireEvent.click(screen.getByRole('button',{name:'取消'}));expect(current.textContent).toBe('100%');expect(host.sets).toEqual([]);
    fireEvent.click(increase);fireEvent.click(increase);
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.sets).toEqual([{snapshot:{...DEFAULTS,density:120},expectedRevision:4}]);
    expect(host.stored?.density).toBe(120);expect(host.stored?.revision).toBe(5);
  });
  it('preserves migrated colors and typography until an explicit save',async()=>{
    const legacy={...structuredClone(DEFAULTS),schemaVersion:5,density:115,revision:4};
    legacy.typography={...legacy.typography,uiFont:{kind:'local',family:'Saved Local Family'},uiSize:17,readingSize:21,codeSize:19,readingFollowUi:false};
    legacy.variants.light.overrides.reading={background:'#111111',foreground:'#eeeeee'};
    host.stored=legacy as unknown as AppearanceSnapshot;
    await open();expect(host.sets).toEqual([]);expect(host.stored).toEqual(legacy);
    expect(screen.getByText(/旧版布局缩放已转换/)).toBeTruthy();
    expect(screen.getByLabelText('当前布局缩放').textContent).toBe('120%');
    fireEvent.click(screen.getByRole('button',{name:'布局缩小 10%'}));
    fireEvent.click(screen.getByRole('button',{name:'取消'}));expect(screen.getByLabelText('当前布局缩放').textContent).toBe('120%');
    expect(screen.getByText(/旧版布局缩放已转换/)).toBeTruthy();expect(host.sets).toEqual([]);
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    await waitFor(()=>expect(host.stored?.schemaVersion).toBe(6));
    expect(host.stored).toEqual({...legacy,schemaVersion:6,density:120,revision:5});expect(screen.queryByText(/旧版布局缩放已转换/)).toBeNull();
  });
  it('restores confirmed global appearance on failure while preserving a retryable draft',async()=>{
    await open();host.fail=true;fireEvent.click(dark());
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    await waitFor(()=>expect(document.documentElement.dataset.theme).toBe('light'));
    expect(dark().getAttribute('aria-checked')).toBe('true');
    expect(screen.getAllByRole('alert').some(alert=>alert.textContent?.includes('草稿保留'))).toBe(true);
    expect(document.body.textContent).not.toContain('private storage');
    host.fail=false;
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    await waitFor(()=>expect(host.stored?.mode).toBe('dark'));
  });
  it('restores confirmed appearance when the editor unmounts',async()=>{
    await initAppearance();const view=render(<MemoryRouter  future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><AppearanceSettings/></MemoryRouter>);
    fireEvent.click(dark());view.unmount();
    expect(document.documentElement.dataset.theme).toBe('light');expect(host.sets).toEqual([]);
  });
  it('defers a navigation click until the save has settled',async()=>{
    await initAppearance();let release!:()=>void;host.gate=new Promise(resolve=>{release=resolve;});
    render(<MemoryRouter  future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><AppearanceSettings/><Link to="/elsewhere">Leave</Link></MemoryRouter>);
    const navigate=vi.fn();screen.getByRole('link',{name:'Leave'}).addEventListener('click',navigate);
    fireEvent.click(dark());fireEvent.click(screen.getByRole('button',{name:'保存更改'}));
    fireEvent.click(screen.getByRole('link',{name:'Leave'}));expect(navigate).not.toHaveBeenCalled();
    await act(async()=>{release();});await waitFor(()=>expect(navigate).toHaveBeenCalledTimes(1));
  });
  it('keeps sliders and direct sizes in sync, isolates code, and rejects invalid input',async()=>{
    await open();
    const ui=screen.getByRole('spinbutton',{name:'界面字号 px'});
    const slider=screen.getByRole('slider',{name:'界面字号 滑杆'});
    fireEvent.change(ui,{target:{value:'20'}});
    expect(slider).toHaveProperty('value','20');
    expect(document.documentElement.style.getPropertyValue('--font-size-reading')).toBe('20px');
    expect(document.documentElement.style.getPropertyValue('--font-size-code')).toBe('12px');
    fireEvent.change(screen.getByRole('slider',{name:'代码字号 滑杆'}),{target:{value:'24'}});
    expect(screen.getByRole('spinbutton',{name:'代码字号 px'})).toHaveProperty('value','24');
    expect(document.documentElement.style.getPropertyValue('--font-size-ui')).toBe('20px');
    fireEvent.change(ui,{target:{value:'21'}});
    expect(ui.getAttribute('aria-invalid')).toBe('true');
    expect(document.documentElement.style.getPropertyValue('--font-size-ui')).toBe('20px');
    fireEvent.blur(ui);expect(ui).toHaveProperty('value','20');
    fireEvent.change(ui,{target:{value:''}});fireEvent.blur(ui);expect(ui).toHaveProperty('value','20');
    fireEvent.change(slider,{target:{value:'12'}});expect(ui).toHaveProperty('value','12');
  });
  it('preserves saved independent reading preferences when following UI and restores them on return',async()=>{
    host.stored!.typography.readingSize=22;
    host.stored!.typography.readingFont={kind:'local',family:'青松正文'};
    await open();
    fireEvent.change(screen.getByRole('spinbutton',{name:'界面字号 px'}),{target:{value:'17'}});
    fireEvent.click(screen.getByRole('radio',{name:'独立设置'}));
    expect(screen.getByRole('spinbutton',{name:'阅读字号 px'})).toHaveProperty('value','22');
    fireEvent.change(screen.getByRole('spinbutton',{name:'界面字号 px'}),{target:{value:'19'}});
    fireEvent.click(dark());
    expect(document.documentElement.style.getPropertyValue('--font-size-reading')).toBe('22px');
    fireEvent.click(screen.getByRole('radio',{name:'跟随界面'}));
    expect(document.documentElement.style.getPropertyValue('--font-size-reading')).toBe('19px');
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored?.typography).toMatchObject({readingFollowUi:true,uiSize:19,readingSize:22,readingFont:{kind:'local',family:'青松正文'}});
    fireEvent.click(screen.getByRole('radio',{name:'独立设置'}));
    expect(screen.getByRole('spinbutton',{name:'阅读字号 px'})).toHaveProperty('value','22');
    expect(document.documentElement.style.getPropertyValue('--font-size-reading')).toBe('22px');
    fireEvent.change(screen.getByRole('spinbutton',{name:'界面字号 px'}),{target:{value:'12'}});
    fireEvent.click(screen.getByRole('button',{name:'取消'}));
    expect(document.documentElement.style.getPropertyValue('--font-size-reading')).toBe('19px');
    fireEvent.click(screen.getByRole('button',{name:'恢复默认'}));
    expect(document.documentElement.style.getPropertyValue('--font-size-reading')).toBe('12px');
    expect(host.stored?.typography.uiSize).toBe(19);
  });
  it('searches the full installed catalogue beyond the first page and previews, cancels, then saves a family',async()=>{
    host.fonts=Array.from({length:75},(_,index)=>({family:`Installed Family ${String(index).padStart(2,'0')}`,monospace:false}));
    const family='青松正文';host.fonts.push({family,monospace:false});
    await open();const initial=document.documentElement.style.getPropertyValue('--font-family-ui');
    const search=await fontPicker('ui');
    await waitFor(()=>expect(localOptions().length).toBe(40));
    expect(screen.getByRole('button',{name:'上一页'})).toHaveProperty('disabled',true);
    fireEvent.click(screen.getByRole('button',{name:'下一页'}));
    expect(localOptions()).toHaveLength(36);
    expect(screen.getByRole('button',{name:'下一页'})).toHaveProperty('disabled',true);
    fireEvent.click(screen.getByRole('button',{name:'上一页'}));
    expect(localOptions()).toHaveLength(40);
    fireEvent.change(search,{target:{value:'青松'}});
    expect(localOptions().map(item=>item.getAttribute('data-font-family'))).toEqual([family]);
    fireEvent.click(localOption(family));
    expect(screen.getByTestId('appearance-ui-font').textContent).toContain(family);
    expect(document.documentElement.style.getPropertyValue('--font-family-ui')).toContain(`"${family}"`);
    expect(document.documentElement.style.getPropertyValue('--font-family-reading')).toContain(`"${family}"`);
    expect(host.sets).toEqual([]);
    fireEvent.click(screen.getByRole('button',{name:'取消'}));
    expect(document.documentElement.style.getPropertyValue('--font-family-ui')).toBe(initial);
    expect(screen.getByTestId('appearance-ui-font').textContent).not.toContain(family);
    await chooseFont('ui',family);
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored?.typography.uiFont).toEqual({kind:'local',family});
    expect(host.stored?.schemaVersion).toBe(6);expect(host.stored?.revision).toBe(5);expect(host.sets).toHaveLength(1);
  });
  it('uses native monospace metadata for code options rather than guessing from names',async()=>{
    await open();const search=await fontPicker('code');
    await waitFor(()=>expect(localOptions().map(item=>item.getAttribute('data-font-family'))).toEqual(['Quiet Code']));
    fireEvent.change(search,{target:{value:'Mono'}});
    expect(localOptions()).toHaveLength(0);
    fireEvent.change(search,{target:{value:'Quiet'}});fireEvent.click(localOption('Quiet Code'));
    expect(document.documentElement.style.getPropertyValue('--font-family-code')).toContain('"Quiet Code"');
    expect(document.documentElement.style.getPropertyValue('--font-family-ui')).not.toContain('Quiet Code');
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored?.typography.codeFont).toEqual({kind:'local',family:'Quiet Code'});
  });
  it('preserves a saved missing local selection when saving an unrelated dark-mode change',async()=>{
    const family='Removed Local Family';host.stored!.typography.uiFont={kind:'local',family};
    await open();await fontPicker('ui');
    await waitFor(()=>expect(localOptions().length).toBe(installedFonts.length));
    expect(screen.getByText(`${family} 不可用，当前使用系统默认。保存其他设置会保留原字体选择；选择另一字体才会替换。`)).toBeTruthy();
    expect(screen.queryByText(/无法读取本机字体目录/)).toBeNull();
    expect(screen.getByTestId('appearance-ui-font').textContent).toContain(family);
    expect(localOptions().some(item=>item.getAttribute('data-font-family')===family)).toBe(false);
    expect(document.documentElement.style.getPropertyValue('--font-family-ui')).not.toContain(family);
    expect(host.sets).toEqual([]);
    fireEvent.keyDown(screen.getByRole('searchbox',{name:'搜索本机字体'}),{key:'Escape'});
    fireEvent.click(dark());
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored?.mode).toBe('dark');expect(host.stored?.typography.uiFont).toEqual({kind:'local',family});
  });
  it('keeps selected font inheritance live across independent reading, dark mode, and save',async()=>{
    host.stored!.typography.readingFont={kind:'local',family:'青松正文'};
    await open();await chooseFont('ui','Aster Sans');
    fireEvent.click(screen.getByRole('radio',{name:'独立设置'}));
    expect(screen.getByTestId('appearance-reading-font').textContent).toContain('青松正文');
    await chooseFont('reading','青松正文');fireEvent.click(dark());
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(document.documentElement.style.getPropertyValue('--font-family-ui')).toContain('"Aster Sans"');
    expect(document.documentElement.style.getPropertyValue('--font-family-reading')).toContain('"青松正文"');
    fireEvent.click(screen.getByRole('radio',{name:'跟随界面'}));
    expect(document.documentElement.style.getPropertyValue('--font-family-reading')).toContain('"Aster Sans"');
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored?.typography).toMatchObject({uiFont:{kind:'local',family:'Aster Sans'},readingFont:{kind:'local',family:'青松正文'},readingFollowUi:true});
    expect(host.stored?.mode).toBe('dark');
  });
  it('distinguishes an unconfirmed catalogue failure from a missing family and recovers after retry',async()=>{
    const family='Aster Sans';host.stored!.typography.uiFont={kind:'local',family};host.fontsFail=true;
    await open();
    expect(screen.getAllByText(/无法读取本机字体目录/).length).toBeGreaterThan(0);
    expect(screen.queryByText(`${family} 不可用，当前使用系统默认。保存其他设置会保留原字体选择；选择另一字体才会替换。`)).toBeNull();
    expect(screen.getByTestId('appearance-ui-font').textContent).toContain(family);
    expect(document.documentElement.style.getPropertyValue('--font-family-ui')).not.toContain(family);
    const samples = ['light', 'dark'].map(mode => document.querySelector<HTMLElement>(`[data-appearance-sample="${mode}"]`)!);
    for (const sample of samples) expect(sample.style.getPropertyValue('--font-family-ui')).not.toContain(family);
    expect(document.body.textContent).not.toContain('private font scanner details');
    host.fontsFail=false;
    await act(async()=>{fireEvent.click(screen.getAllByRole('button',{name:'重试读取字体目录'})[0]);});
    await waitFor(()=>expect(document.documentElement.style.getPropertyValue('--font-family-ui')).toContain('"Aster Sans"'));
    for (const sample of samples) expect(sample.style.getPropertyValue('--font-family-ui')).toContain('"Aster Sans"');
    expect(screen.queryByText(/无法读取本机字体目录/)).toBeNull();
    expect(host.sets).toEqual([]);
  });
  it.each(['ui','code'] as const)('allows explicitly choosing the %s system default after catalogue failure',async role=>{
    host.stored!.typography[`${role}Font`]={kind:'local',family:'Saved Family'};host.fontsFail=true;
    await open();const search=await fontPicker(role);fireEvent.change(search,{target:{value:'Nothing matches'}});
    expect(localOptions()).toHaveLength(0);
    const system=within(screen.getByRole('listbox')).getByRole('option');
    expect(system.textContent).toContain(role==='code'?'系统等宽':'系统默认');
    expect(system).toHaveProperty('disabled',false);fireEvent.click(system);
    expect(screen.getByTestId(`appearance-${role}-font`).textContent).not.toContain('Saved Family');
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored!.typography[`${role}Font`]).toEqual({kind:role==='code'?'system-mono':'system-ui'});
  });
  it('keeps the system default selectable while catalogue availability is still pending',async()=>{
    const {promise,resolve:release}=Promise.withResolvers<void>();host.fontsGate=promise;
    const family='Pending Family';host.stored!.typography.uiFont={kind:'local',family};
    await open();
    expect(screen.getByText(`尚未确认 ${family} 是否可用，当前使用系统默认；原字体选择会保留。`)).toBeTruthy();
    await fontPicker('ui');expect(screen.getByText('正在读取本机字体目录…')).toBeTruthy();
    const system=within(screen.getByRole('listbox')).getByRole('option');
    expect(system).toHaveProperty('disabled',false);fireEvent.click(system);
    expect(screen.getByTestId('appearance-ui-font').textContent).not.toContain(family);
    await act(async()=>{release();});
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored?.typography.uiFont).toEqual({kind:'system-ui'});
  });
  it('exposes installed-font search in the active English locale',async()=>{
    setLocaleForTesting('en-US');await open();fireEvent.click(screen.getByTestId('appearance-ui-font'));
    const search=await screen.findByRole('searchbox',{name:'Search installed fonts'});
    fireEvent.change(search,{target:{value:'aster sans'}});fireEvent.click(localOption('Aster Sans'));
    expect(screen.getByTestId('appearance-ui-font').textContent).toContain('Aster Sans');
  });
  it('renders a quoted installed family as one CSS family and preserves its exact saved name',async()=>{
    const family='Studio "中文"';host.fonts.push({family,monospace:false});
    await open();await chooseFont('ui',family);
    expect(screen.getByTestId('appearance-ui-font').textContent).toContain(family);
    expect(document.documentElement.style.getPropertyValue('--font-family-ui').startsWith('"Studio \\22 中文\\22 ", ')).toBe(true);
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored?.typography.uiFont).toEqual({kind:'local',family});
  });
  it('persists a color edit with complete typography and disables save for an incomplete HEX value',async()=>{
    await open();
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'}),{target:{value:'#1555D9'}});
    expect(document.documentElement.style.getPropertyValue('--font-size-ui')).toBe('12px');
    expect(screen.getByRole('button',{name:'保存更改'})).toHaveProperty('disabled',false);
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 背景色 HEX'}),{target:{value:'#12'}});
    expect(screen.getByRole('button',{name:'保存更改'})).toHaveProperty('disabled',true);
    expect(host.sets).toEqual([]);
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 背景色 HEX'}),{target:{value:'#F8F6EF'}});
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    await waitFor(()=>expect(host.stored?.revision).toBe(5));
    expect(host.stored?.variants.light.overrides.interface?.accent).toBe('#1555D9');
    expect(host.stored?.typography).toEqual(DEFAULTS.typography);
  });

  it('requires an explicit choice when a customized mode changes preset',async()=>{
    await open();
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'}),{target:{value:'#1555D9'}});
    await choosePaper();
    const dialog=await screen.findByRole('dialog',{name:'切换主题套装'});
    expect(host.sets).toEqual([]);
    fireEvent.click(within(dialog).getByRole('button',{name:'取消'}));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'})).toHaveProperty('value','#1555D9');
    await choosePaper();
    fireEvent.click(within(await screen.findByRole('dialog',{name:'切换主题套装'})).getByRole('button',{name:'保留自定义'}));
    expect(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'})).toHaveProperty('value','#1555D9');
    expect(host.sets).toEqual([]);
  });
  it('uses the chosen preset base and clears only the edited mode color overrides',async()=>{
    await open();
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'}),{target:{value:'#1555D9'}});
    await choosePaper();
    const dialog=await screen.findByRole('dialog',{name:'切换主题套装'});
    fireEvent.click(within(dialog).getByRole('button',{name:'使用新套装原色'}));
    expect(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'})).toHaveProperty('value','#985B05');
    expect(host.sets).toEqual([]);
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    await waitFor(()=>expect(host.stored?.revision).toBe(5));
    expect(host.stored?.variants.light).toEqual({presetId:'paper',overrides:{interface:{},reading:{},code:{}}});
    expect(host.stored?.variants.dark).toEqual(DEFAULTS.variants.dark);
    expect(host.stored?.typography).toEqual(DEFAULTS.typography);
  });
  it('isolates invalid inactive colors while allowing independent light edits and only saves after correction',async()=>{
    await open();
    const before=document.documentElement.getAttribute('style');
    fireEvent.change(screen.getByRole('textbox',{name:'深色主题 · 文字色 HEX'}),{target:{value:'#242521'}});
    expect(document.documentElement.getAttribute('style')).toBe(before);
    expect(screen.getByRole('button',{name:'保存更改'})).toHaveProperty('disabled',true);
    expect(screen.getAllByRole('alert').some(x=>x.textContent?.includes('深色主题')&&x.textContent?.includes('4.5:1'))).toBe(true);
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'}),{target:{value:'#1555D9'}});
    expect(screen.getByRole('textbox',{name:'深色主题 · 文字色 HEX'})).toHaveProperty('value','#242521');
    fireEvent.click(screen.getByRole('switch',{name:'深色主题 · 文字色 自定义'}));
    expect(screen.getByRole('button',{name:'保存更改'})).toHaveProperty('disabled',false);
    expect(document.documentElement.dataset.theme).toBe('light');
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'保存更改'}));});
    expect(host.stored?.variants.light.overrides.interface?.accent).toBe('#1555D9');
    expect(host.stored?.variants.dark).toEqual(DEFAULTS.variants.dark);
  });

  it('clears incomplete text on cancel and current-mode restore even when the effective color never changed',async()=>{
    await open();
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'}),{target:{value:'#12'}});
    fireEvent.click(screen.getByRole('button',{name:'取消'}));
    expect(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'})).toHaveProperty('value','#A46A24');
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'}),{target:{value:'#12'}});
    fireEvent.click(screen.getByRole('button',{name:'恢复浅色主题配色'}));
    expect(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'})).toHaveProperty('value','#A46A24');
    expect(screen.getByRole('button',{name:'保存更改'})).toHaveProperty('disabled',true);
  });

  it('keeps unrelated typography draft when Escape cancels only a picker then whole-page cancel restores confirmed values',async()=>{
    await open();
    fireEvent.change(screen.getByRole('spinbutton',{name:'界面字号 px'}),{target:{value:'18'}});
    fireEvent.click(screen.getByRole('button',{name:'浅色主题 · 强调色 · 选择颜色'}));
    const picker=await screen.findByRole('dialog',{name:'浅色主题 · 强调色 · 选择颜色'});
    fireEvent.change(within(picker).getByRole('textbox',{name:'浅色主题 · 强调色 HEX'}),{target:{value:'#1555D9'}});
    fireEvent.keyDown(picker,{key:'Escape'});
    await waitFor(()=>expect(screen.queryByRole('dialog')).toBeNull());
    expect(screen.getByRole('spinbutton',{name:'界面字号 px'})).toHaveProperty('value','18');
    expect(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'})).toHaveProperty('value','#A46A24');
    fireEvent.click(screen.getByRole('button',{name:'取消'}));
    expect(screen.getByRole('spinbutton',{name:'界面字号 px'})).toHaveProperty('value','12');
    expect(host.sets).toEqual([]);
  });
  it('disables repeat saves when draft values return to the confirmed snapshot',async()=>{
    await open();
    expect(screen.getByRole('button',{name:'保存更改'})).toHaveProperty('disabled',true);
    fireEvent.click(dark());
    fireEvent.click(within(screen.getByRole('radiogroup',{name:'主题'})).getByRole('radio',{name:'浅色'}));
    expect(screen.getByRole('button',{name:'保存更改'})).toHaveProperty('disabled',true);
    expect(host.sets).toEqual([]);
  });
  it('retains both mode edits and typography through a locale rerender and saves the complete snapshot',async()=>{
    await open();
    fireEvent.change(screen.getByRole('textbox',{name:'浅色主题 · 强调色 HEX'}),{target:{value:'#1555D9'}});
    fireEvent.change(screen.getByRole('textbox',{name:'深色主题 · 背景色 HEX'}),{target:{value:'#19251F'}});
    fireEvent.change(screen.getByRole('spinbutton',{name:'界面字号 px'}),{target:{value:'18'}});
    act(()=>setLocaleForTesting('en-US'));
    expect(screen.getByRole('textbox',{name:'Light theme · Accent color HEX'})).toHaveProperty('value','#1555D9');
    expect(screen.getByRole('textbox',{name:'Dark theme · Background color HEX'})).toHaveProperty('value','#19251F');
    expect(screen.getByRole('spinbutton',{name:'Interface size px'})).toHaveProperty('value','18');
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'Save changes'}));});
    expect(host.stored?.variants.light.overrides.interface?.accent).toBe('#1555D9');
    expect(host.stored?.variants.dark.overrides.interface?.background).toBe('#19251F');
    expect(host.stored?.typography.uiSize).toBe(18);
  });
});
