import { afterEach, describe, expect, it, vi } from 'vitest';
import { createAppearanceController, type AppearanceController } from '../src/ui/appearance/controller';
import { DEFAULTS, type Appearance, type AppearanceSnapshot } from '../src/ui/appearance/model';
const ready=(values:Appearance=structuredClone(DEFAULTS),revision=0)=>({kind:'ready',snapshot:{...values,revision},repairWarnings:[]});
const draft=()=>({...structuredClone(DEFAULTS),mode:'dark' as const,density:150 as const});
let controller:AppearanceController;
afterEach(()=>{controller?.dispose();vi.useRealTimers();});
function setup(read:()=>Promise<unknown>=async()=>ready(),write:(values:Appearance,revision:number)=>Promise<unknown>=async values=>({kind:'saved',snapshot:{...values,revision:1}})){
 const root=document.createElement('html');root.setAttribute('data-appearance-pending','');
 controller=createAppearanceController({root,read,write,writeTimeoutMs:20});return root;
}
describe('full appearance transaction',()=>{
 it('previews without writing and cancels to confirmed values',async()=>{
  const write=vi.fn();const root=setup(async()=>ready(),write);await controller.init();
  controller.preview(draft());expect(root.dataset.theme).toBe('dark');expect(root.dataset.density).toBe('150');expect(write).not.toHaveBeenCalled();
  controller.cancel();expect(root.dataset.theme).toBe('light');expect(controller.getSnapshot().draft).toEqual(DEFAULTS);
 });
 it('saves one complete draft and adopts the host revision',async()=>{
  let stored:AppearanceSnapshot={...structuredClone(DEFAULTS),revision:4};
  const root=setup(async()=>({kind:'ready',snapshot:stored,repairWarnings:[]}),async(values,revision)=>{expect(revision).toBe(4);stored={...values,revision:5};return {kind:'saved',snapshot:stored};});
  await controller.init();controller.preview(draft());await controller.apply();
  expect(controller.getSnapshot().confirmed?.revision).toBe(5);expect(stored.mode).toBe('dark');expect(root.dataset.theme).toBe('dark');
 });
 it('retains a conflicting draft while restoring the other editor confirmed theme',async()=>{
  const other={...structuredClone(DEFAULTS),density:125 as const,revision:2};
  const root=setup(async()=>ready(),async()=>({kind:'conflict',code:1005007,snapshot:other}));await controller.init();controller.preview(draft());await controller.apply();
  expect(root.dataset.theme).toBe('light');expect(root.dataset.density).toBe('125');expect(controller.getSnapshot().draft.mode).toBe('dark');expect(controller.getSnapshot().error).toBe('revision_conflict');
 });
 it('restores confirmed on explicit storage failure and keeps the draft retryable',async()=>{
  const root=setup(async()=>ready(),async()=>{throw Object.assign(new Error('failed'),{kind:'appearance_error',code:1000000});});await controller.init();controller.preview(draft());await controller.apply();
  expect(root.dataset.theme).toBe('light');expect(controller.getSnapshot().phase).toBe('failed');expect(controller.getSnapshot().draft.mode).toBe('dark');
 });
 it('verifies a timed out successful commit without issuing a second write',async()=>{
  vi.useFakeTimers();let stored=ready();const write=vi.fn(async(values:Appearance)=>{stored=ready(values,1);return new Promise<never>(()=>{});});
  setup(async()=>stored,write);await controller.init();controller.preview(draft());const pending=controller.apply();await vi.advanceTimersByTimeAsync(21);await pending;
  expect(controller.getSnapshot().phase).toBe('applied');expect(controller.getSnapshot().confirmed?.revision).toBe(1);expect(write).toHaveBeenCalledTimes(1);
 });
 it('blocks writes when commit and verification are both unconfirmed',async()=>{
  vi.useFakeTimers();let reads=0;const write=vi.fn(()=>new Promise<never>(()=>{}));setup(async()=>{if(reads++)throw new Error('offline');return ready();},write);await controller.init();controller.preview(draft());const pending=controller.apply();await vi.advanceTimersByTimeAsync(21);await pending;
  expect(controller.getSnapshot().phase).toBe('unconfirmed');await controller.apply();expect(write).toHaveBeenCalledTimes(1);
 });
 it.each(['corrupt','unsupported_schema'])('does not invent a confirmed revision for %s',async kind=>{
  const root=setup(async()=>({kind,safeDefaults:DEFAULTS,recoveryToken:'token',schemaVersion:3}));await controller.init();controller.preview(draft());await controller.apply();expect(controller.getSnapshot().confirmed).toBeNull();expect(root.dataset.theme).toBe('light');
 });
 it('invalid drafts never reach global styles',async()=>{
  const root=setup();await controller.init();const before=root.getAttribute('style');const invalid=draft();invalid.variants.light.overrides.interface.accent='url(secret)';controller.preview(invalid);expect(controller.getSnapshot().phase).toBe('invalid');expect(root.getAttribute('style')).toBe(before);
 });
 it('keeps the last valid preview when later color input is invalid',async()=>{
  const root=setup();await controller.init();controller.preview(draft());
  const before=root.getAttribute('style');const invalid=draft();invalid.variants.dark.overrides.interface.accent='partial';controller.preview(invalid);
  expect(root.dataset.theme).toBe('dark');expect(root.getAttribute('style')).toBe(before);
  controller.cancel();expect(root.dataset.theme).toBe('light');
 });
 it('ignores a saved response after the account controller is disposed',async()=>{
  let finish!:(value:unknown)=>void;const root=setup(async()=>ready(),()=>new Promise(resolve=>{finish=resolve;}));await controller.init();controller.preview(draft());const pending=controller.apply();controller.dispose();finish({kind:'saved',snapshot:{...draft(),revision:1}});await pending;expect(controller.getSnapshot().confirmed?.revision).toBe(0);
 });
 it('follows system mode during preview and stops listening after disposal',async()=>{
  const root=document.createElement('html');let change!:()=>void;let dark=false;
  const remove=vi.fn();
  const media={get matches(){return dark;},addEventListener:(_name:string,listener:()=>void)=>{change=listener;},removeEventListener:remove} as unknown as MediaQueryList;
  controller=createAppearanceController({root,read:async()=>ready({...DEFAULTS,mode:'auto'}),write:vi.fn(),matchMedia:()=>media});
  await controller.init();expect(root.dataset.theme).toBe('light');dark=true;change();expect(root.dataset.theme).toBe('dark');
  controller.preview({...structuredClone(DEFAULTS),mode:'light'});dark=false;change();expect(root.dataset.theme).toBe('light');
  controller.dispose();expect(remove).toHaveBeenCalled();
 });
});
