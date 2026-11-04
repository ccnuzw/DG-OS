import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Link, MemoryRouter } from 'react-router-dom';
import { disposeAppearance, initAppearance } from '@dgos/ui';
import AppearanceSettings from '../src/pages/AppearanceSettings';
import { DEFAULTS, type AppearanceSnapshot, type Appearance } from '../src/ui/appearance/model';
import { resetLocaleForTesting, setLocaleForTesting } from '../src/i18n/runtime';

const host = vi.hoisted(() => ({ stored: null as AppearanceSnapshot|null, sets: [] as unknown[], fail: false, gate:null as Promise<void>|null }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: async (command:string,args?:{snapshot:Appearance;expectedRevision:number}) => {
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
beforeEach(()=>{host.stored={...structuredClone(DEFAULTS),revision:4};host.sets=[];host.fail=false;host.gate=null;setLocaleForTesting('zh-CN');});
afterEach(()=>{cleanup();disposeAppearance();resetLocaleForTesting();});
async function open(){await initAppearance();render(<MemoryRouter><AppearanceSettings/></MemoryRouter>);}
function dark(){return within(screen.getByRole('radiogroup',{name:'主题'})).getByRole('radio',{name:'暗色'});}
describe('appearance draft transaction',()=>{
  it('previews without writing and cancel restores confirmed appearance',async()=>{
    await open();fireEvent.click(dark());
    expect(document.documentElement.dataset.theme).toBe('dark');expect(host.sets).toEqual([]);
    fireEvent.click(screen.getByRole('button',{name:'取消预览'}));
    expect(document.documentElement.dataset.theme).toBe('light');expect(host.sets).toEqual([]);
  });
  it('applies a complete draft once with its confirmed revision',async()=>{
    await open();fireEvent.click(dark());
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'应用外观'}));});
    await waitFor(()=>expect(host.stored?.revision).toBe(5));
    expect(host.sets).toEqual([{snapshot:{...DEFAULTS,mode:'dark'},expectedRevision:4}]);
    expect(document.documentElement.dataset.theme).toBe('dark');
  });
  it('restores confirmed global appearance on failure while preserving a retryable draft',async()=>{
    await open();host.fail=true;fireEvent.click(dark());
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'应用外观'}));});
    await waitFor(()=>expect(document.documentElement.dataset.theme).toBe('light'));
    expect(dark().getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('alert').textContent).toContain('草稿保留');
    expect(document.body.textContent).not.toContain('private storage');
    host.fail=false;
    await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'应用外观'}));});
    await waitFor(()=>expect(host.stored?.mode).toBe('dark'));
  });
  it('restores confirmed appearance when the editor unmounts',async()=>{
    await initAppearance();const view=render(<MemoryRouter><AppearanceSettings/></MemoryRouter>);
    fireEvent.click(dark());view.unmount();
    expect(document.documentElement.dataset.theme).toBe('light');expect(host.sets).toEqual([]);
  });
  it('defers a navigation click until the save has settled',async()=>{
    await initAppearance();let release!:()=>void;host.gate=new Promise(resolve=>{release=resolve;});
    render(<MemoryRouter><AppearanceSettings/><Link to="/elsewhere">Leave</Link></MemoryRouter>);
    const navigate=vi.fn();screen.getByRole('link',{name:'Leave'}).addEventListener('click',navigate);
    fireEvent.click(dark());fireEvent.click(screen.getByRole('button',{name:'应用外观'}));
    fireEvent.click(screen.getByRole('link',{name:'Leave'}));expect(navigate).not.toHaveBeenCalled();
    await act(async()=>{release();});await waitFor(()=>expect(navigate).toHaveBeenCalledTimes(1));
  });
});
