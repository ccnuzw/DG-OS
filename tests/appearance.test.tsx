import { afterEach, describe, expect, it, vi } from 'vitest';
import { createAppearanceController, type AppearanceController } from '../src/ui/appearance/controller';
import { DEFAULTS, parseConfirmed, parseAppearance, parseRead, type Appearance, type AppearanceSnapshot } from '../src/ui/appearance/model';
const ready=(values:Appearance=structuredClone(DEFAULTS),revision=0)=>({kind:'ready',snapshot:{...values,revision},repairWarnings:[]});
const draft=()=>({...structuredClone(DEFAULTS),mode:'dark' as const,density:120 as const});
let controller:AppearanceController;
afterEach(()=>{controller?.dispose();vi.useRealTimers();});
function setup(read:()=>Promise<unknown>=async()=>ready(),write:(values:Appearance,revision:number)=>Promise<unknown>=async values=>({kind:'saved',snapshot:{...values,revision:1}})){
 const root=document.createElement('html');root.setAttribute('data-appearance-pending','');
 controller=createAppearanceController({root,read,write,writeTimeoutMs:20});return root;
}
describe('full appearance transaction',()=>{
 it('previews without writing and cancels to confirmed values',async()=>{
  const write=vi.fn();const root=setup(async()=>ready(),write);await controller.init();
  controller.preview(draft());expect(root.dataset.theme).toBe('dark');expect(root.dataset.density).toBe('120');expect(write).not.toHaveBeenCalled();
  controller.cancel();expect(root.dataset.theme).toBe('light');expect(controller.getSnapshot().draft).toEqual(DEFAULTS);
 });
 it('saves one complete draft and adopts the host revision',async()=>{
  let stored:AppearanceSnapshot={...structuredClone(DEFAULTS),revision:4};
  const root=setup(async()=>({kind:'ready',snapshot:stored,repairWarnings:[]}),async(values,revision)=>{expect(revision).toBe(4);stored={...values,revision:5};return {kind:'saved',snapshot:stored};});
  await controller.init();controller.preview(draft());await controller.apply();
  expect(controller.getSnapshot().confirmed?.revision).toBe(5);expect(stored.mode).toBe('dark');expect(root.dataset.theme).toBe('dark');
 });
 it('retains a conflicting draft while restoring the other editor confirmed theme',async()=>{
  const other={...structuredClone(DEFAULTS),density:90 as const,revision:2};
  const root=setup(async()=>ready(),async()=>({kind:'conflict',code:1005007,snapshot:other}));await controller.init();controller.preview(draft());await controller.apply();
  expect(root.dataset.theme).toBe('light');expect(root.dataset.density).toBe('90');expect(controller.getSnapshot().draft.mode).toBe('dark');expect(controller.getSnapshot().error).toBe('revision_conflict');
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
  const root=setup(async()=>({kind,safeDefaults:DEFAULTS,recoveryToken:'token',schemaVersion:6}));await controller.init();controller.preview(draft());await controller.apply();expect(controller.getSnapshot().confirmed).toBeNull();expect(root.dataset.theme).toBe('light');
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
 it('preserves genuine schema2 custom typography and revision without enabling reading inheritance',()=>{
  const old={...structuredClone(DEFAULTS),schemaVersion:2,revision:17,typography:{uiFontId:'humanist',readingFontId:'serif',codeFontId:'source-code-pro',uiSize:16,readingSize:18,codeSize:16,lineHeight:1.75}};
  const migrated=parseConfirmed(old);
  expect(migrated.schemaVersion).toBe(6);expect(migrated.revision).toBe(17);
  expect(migrated.typography).toEqual({uiFont:{kind:'local',family:'Segoe UI'},readingFont:{kind:'local',family:'Georgia'},codeFont:{kind:'local',family:'Source Code Pro'},uiSize:16,readingSize:18,codeSize:16,lineHeight:1.75,readingFollowUi:false});
  expect(()=>parseAppearance(old)).toThrow();
  const legacyDefault=parseConfirmed({...old,typography:{...old.typography,uiFontId:'system',readingFontId:'system',codeFontId:'mono',readingSize:13}});
  expect(legacyDefault.typography).toMatchObject({uiFont:{kind:'system-ui'},readingFont:{kind:'system-ui'},codeFont:{kind:'system-mono'},readingSize:13,readingFollowUi:false});
 });
 it('migrates genuine schema3 font IDs only on confirmed reads and preserves reading inheritance',()=>{
  const old={...structuredClone(DEFAULTS),schemaVersion:3,revision:9,typography:{uiFontId:'inter',readingFontId:'noto-sans-cjk-sc',codeFontId:'jetbrains-mono',uiSize:15,readingSize:18,codeSize:14,lineHeight:2,readingFollowUi:true}};
  const migrated=parseConfirmed(old);
  expect(migrated).toMatchObject({schemaVersion:6,revision:9,typography:{uiFont:{kind:'local',family:'Inter'},readingFont:{kind:'local',family:'Noto Sans CJK SC'},codeFont:{kind:'local',family:'JetBrains Mono'},uiSize:15,readingSize:18,codeSize:14,lineHeight:2,readingFollowUi:true}});
  expect(()=>parseAppearance(old)).toThrow();
 });
 it.each([2,3,4,5])('normalizes schema%s legacy density only on confirmed reads without mutating the source',schemaVersion=>{
  for(const [density,expected] of [[60,60],[65,70],[75,80],[100,100],[115,120],[125,130],[145,150],[150,150],[175,150]]) {
   const current=structuredClone(DEFAULTS);current.typography={...current.typography,uiFont:{kind:'local',family:'Saved Family'},uiSize:17,readingSize:21,codeSize:19,readingFollowUi:false};
   current.variants.light.overrides.reading={background:'#111111',foreground:'#eeeeee'};
   const typography=schemaVersion===4||schemaVersion===5?current.typography:{uiFontId:'humanist',readingFontId:'serif',codeFontId:'mono',uiSize:17,readingSize:21,codeSize:19,lineHeight:1.75,...(schemaVersion===3?{readingFollowUi:false}:{})};
   const old={...current,schemaVersion,density,typography,revision:23};const bytes=JSON.stringify(old);
   const read=parseRead({kind:'ready',snapshot:old,repairWarnings:['existing']});
   expect(read.kind).toBe('ready');if(read.kind!=='ready')throw new Error('Expected ready');
   expect(read.snapshot).toMatchObject({schemaVersion:6,density:expected,revision:23,variants:current.variants,typography:{uiSize:17,readingSize:21,codeSize:19,readingFollowUi:false}});
   expect(read.repairWarnings).toEqual(density===expected?['existing']:['existing','density_migrated']);
   if(schemaVersion===4||schemaVersion===5)expect(read.snapshot.typography).toEqual(current.typography);
   expect(JSON.stringify(old)).toBe(bytes);expect(()=>parseAppearance(old)).toThrow();
  }
 });
 it('accepts 150 without a density migration warning and rejects invalid current values without rounding',()=>{
  const current={...structuredClone(DEFAULTS),density:150 as const,revision:0};
  const read=parseRead({kind:'ready',snapshot:current,repairWarnings:[]});
  expect(read.kind).toBe('ready');if(read.kind!=='ready')throw new Error('Expected ready');
  expect(read.snapshot.density).toBe(150);expect(read.repairWarnings).toEqual([]);
  for(const density of [65,75,115,125,151,0,999])expect(()=>parseAppearance({...DEFAULTS,density})).toThrow();
  expect(()=>parseConfirmed({...DEFAULTS,density:115,revision:0})).toThrow();
 });
 it('treats schema7 as unsupported instead of migrating it',()=>{
  expect(parseRead({kind:'unsupported_schema',schemaVersion:7})).toMatchObject({kind:'unsupported_schema',schemaVersion:7});
 });
 it('retains migration warnings through cancel, verification, failure and conflict until an explicit successful save',async()=>{
  const legacy={...structuredClone(DEFAULTS),schemaVersion:4,density:145,revision:4};let outcome='failure';
  const write=vi.fn(async(values:Appearance)=>{
   if(outcome==='failure')throw {kind:'appearance_error'};
   return {kind:outcome,snapshot:{...values,revision:5}};
  });
  setup(async()=>({kind:'ready',snapshot:legacy,repairWarnings:[]}),write);await controller.init();
  controller.cancel();await controller.verify();expect(controller.getSnapshot().repairWarnings).toEqual(['density_migrated']);
  await controller.apply();expect(controller.getSnapshot().repairWarnings).toEqual(['density_migrated']);
  outcome='conflict';await controller.apply();expect(controller.getSnapshot().repairWarnings).toEqual(['density_migrated']);
  outcome='saved';await controller.apply();expect(controller.getSnapshot().repairWarnings).toEqual([]);expect(write).toHaveBeenCalledTimes(3);
 });
 it('clears a migration warning when verification confirms the explicit timed-out save',async()=>{
  vi.useFakeTimers();let stored:unknown={kind:'ready',snapshot:{...DEFAULTS,schemaVersion:4,density:75,revision:0},repairWarnings:[]};
  setup(async()=>stored,async values=>{stored=ready(values,1);return new Promise<never>(()=>{});});
  await controller.init();const pending=controller.apply();await vi.advanceTimersByTimeAsync(21);await pending;
  expect(controller.getSnapshot().repairWarnings).toEqual([]);expect(controller.getSnapshot().confirmed?.density).toBe(80);
 });
 it('preserves an unavailable saved local family through an unrelated complete-draft save',async()=>{
  const saved=structuredClone(DEFAULTS);saved.typography.uiFont={kind:'local',family:'Unavailable saved family'};
  const write=vi.fn(async(values:Appearance)=>({kind:'saved',snapshot:{...values,revision:5}}));
  const root=setup(async()=>ready(saved,4),write);await controller.init();
  expect(write).not.toHaveBeenCalled();expect(root.style.getPropertyValue('--font-family-ui')).not.toContain('Unavailable saved family');
  controller.preview({...saved,mode:'dark'});await controller.apply();
  expect(write).toHaveBeenCalledWith({...saved,mode:'dark'},4);
  expect(controller.getSnapshot().confirmed?.typography.uiFont).toEqual({kind:'local',family:'Unavailable saved family'});
 });
});
