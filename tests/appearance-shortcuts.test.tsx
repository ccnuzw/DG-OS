import { afterEach, describe, expect, it, vi } from 'vitest';
import { createAppearanceController, type AppearanceController } from '../src/ui/appearance/controller';
import { DEFAULTS, type Appearance } from '../src/ui/appearance/model';
import { installLayoutScaleShortcuts, layoutScaleDirection } from '../src/ui/appearance/shortcuts';

const ready=(values:Appearance=structuredClone(DEFAULTS),revision=0)=>({kind:'ready',snapshot:{...values,revision},repairWarnings:[]});
let controller:AppearanceController;
afterEach(()=>{controller?.dispose();vi.useRealTimers();});
function setup(write:(values:Appearance,revision:number)=>Promise<unknown>=async(values,revision)=>({kind:'saved',snapshot:{...values,revision:revision+1}}),initial:Appearance=structuredClone(DEFAULTS)){
 controller?.dispose();const root=document.createElement('html');controller=createAppearanceController({root,read:async()=>ready(initial),write,writeTimeoutMs:1000});return root;
}
const deferred=<T,>()=>Promise.withResolvers<T>();

describe('appearance layout scale controller',()=>{
 it('changes an active editor draft, can cancel, preserves invalid colors, and never autosaves',async()=>{
  const write=vi.fn();const root=setup(write);await controller.init();
  const endEditing=controller.beginEditing();
  expect(controller.adjustLayoutScale(1)).toBe(true);
  expect(controller.getSnapshot().draft.density).toBe(110);expect(root.dataset.density).toBe('110');expect(write).not.toHaveBeenCalled();
  controller.cancel();expect(controller.getSnapshot().draft.density).toBe(DEFAULTS.density);expect(root.dataset.density).toBe(String(DEFAULTS.density));
  const invalid={...structuredClone(DEFAULTS),variants:{...structuredClone(DEFAULTS.variants)}};
  invalid.variants.light.overrides.interface.accent='partial';controller.preview(invalid);
  expect(controller.getSnapshot().phase).toBe('invalid');expect(controller.adjustLayoutScale(1)).toBe(true);
  expect(controller.getSnapshot().draft.variants.light.overrides.interface.accent).toBe('partial');expect(controller.getSnapshot().draft.density).toBe(110);expect(write).not.toHaveBeenCalled();
  endEditing();
 });
 it('autosaves only confirmed appearance density and does not submit a hidden draft',async()=>{
  vi.useFakeTimers();const write=vi.fn(async(values:Appearance,revision:number)=>({kind:'saved',snapshot:{...values,revision:revision+1}}));setup(write);await controller.init();
  controller.preview({...structuredClone(DEFAULTS),mode:'dark'});
  expect(controller.getSnapshot().draft.mode).toBe('dark');
  expect(controller.adjustLayoutScale(1)).toBe(true);await vi.advanceTimersByTimeAsync(120);
  expect(write).toHaveBeenCalledTimes(1);expect(write).toHaveBeenCalledWith({...structuredClone(DEFAULTS),density:110},0);
  expect(controller.getSnapshot().confirmed?.mode).toBe(DEFAULTS.mode);expect(controller.getSnapshot().draft.mode).toBe('dark');
 });
 it('merges changes during the debounce and serializes a later target after the in-flight revision settles',async()=>{
  vi.useFakeTimers();const first=deferred<unknown>();const second=deferred<unknown>();
  const write=vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);setup(write);await controller.init();
  expect(controller.adjustLayoutScale(1)).toBe(true);expect(controller.adjustLayoutScale(1)).toBe(true);
  await vi.advanceTimersByTimeAsync(120);expect(write).toHaveBeenCalledTimes(1);expect(write.mock.calls[0][0].density).toBe(120);
  expect(controller.adjustLayoutScale(1)).toBe(true);await vi.advanceTimersByTimeAsync(500);
  expect(write).toHaveBeenCalledTimes(1);
  first.resolve({kind:'saved',snapshot:{...write.mock.calls[0][0],revision:1}});await vi.advanceTimersByTimeAsync(120);
  expect(write).toHaveBeenCalledTimes(2);expect(write.mock.calls[1][1]).toBe(1);expect(write.mock.calls[1][0].density).toBe(130);
  second.resolve({kind:'saved',snapshot:{...write.mock.calls[1][0],revision:2}});await vi.advanceTimersByTimeAsync(0);
  expect(controller.getSnapshot().confirmed?.density).toBe(130);expect(controller.getSnapshot().confirmed?.revision).toBe(2);
 });
 it('does not persist a boundary, and stops queued writes after failure, conflict, or unconfirmed response',async()=>{
  vi.useFakeTimers();const write=vi.fn(async()=>({kind:'saved',snapshot:{...DEFAULTS,revision:1}}));setup(write,{...structuredClone(DEFAULTS),density:60});await controller.init();
  expect(controller.adjustLayoutScale(-1)).toBe(false);await vi.advanceTimersByTimeAsync(500);expect(write).not.toHaveBeenCalled();controller.dispose();
  for(const outcome of ['failure','conflict','unconfirmed'] as const){
   const pending=Promise.withResolvers<unknown>();const writePending=vi.fn(()=>pending.promise);setup(writePending);await controller.init();
   controller.adjustLayoutScale(1);await vi.advanceTimersByTimeAsync(120);controller.adjustLayoutScale(1);
   if(outcome==='failure')pending.reject(Object.assign(new Error('failed'),{kind:'appearance_error'}));
   if(outcome==='conflict')pending.resolve({kind:'conflict',snapshot:{...DEFAULTS,density:100,revision:1}});
   if(outcome==='unconfirmed')pending.resolve({kind:'unknown',snapshot:{...DEFAULTS,density:120,revision:1}});
   await vi.advanceTimersByTimeAsync(500);expect(writePending).toHaveBeenCalledTimes(1);controller.dispose();
  }
 });
 it('suspends queued and future scale changes, while an in-flight completion cannot enqueue another write',async()=>{
  vi.useFakeTimers();const first=Promise.withResolvers<unknown>();const write=vi.fn().mockReturnValueOnce(first.promise);setup(write);await controller.init();
  expect(controller.adjustLayoutScale(1)).toBe(true);const release=controller.suspendLayoutScale();
  expect(controller.adjustLayoutScale(1)).toBe(false);expect(controller.getSnapshot().confirmed?.density).toBe(100);
  await vi.advanceTimersByTimeAsync(500);expect(write).not.toHaveBeenCalled();release();expect(controller.adjustLayoutScale(1)).toBe(true);
  await vi.advanceTimersByTimeAsync(120);expect(write).toHaveBeenCalledTimes(1);
  expect(controller.adjustLayoutScale(1)).toBe(true);const suspendDuringWrite=controller.suspendLayoutScale();
  first.resolve({kind:'saved',snapshot:{...write.mock.calls[0][0],revision:1}});await vi.advanceTimersByTimeAsync(500);
  expect(write).toHaveBeenCalledTimes(1);expect(controller.getSnapshot().confirmed?.density).toBe(110);
  expect(controller.adjustLayoutScale(1)).toBe(false);suspendDuringWrite();
 });
 it('ignores an in-flight response after disposal and keeps editor draft changes separate while a write settles',async()=>{
  vi.useFakeTimers();const request=deferred<unknown>();const write=vi.fn(()=>request.promise);setup(write);await controller.init();
  controller.adjustLayoutScale(1);await vi.advanceTimersByTimeAsync(120);expect(write).toHaveBeenCalledTimes(1);
  expect(controller.adjustLayoutScale(1)).toBe(true);const endEditing=controller.beginEditing();expect(controller.adjustLayoutScale(1)).toBe(false);
  request.resolve({kind:'saved',snapshot:{...write.mock.calls[0][0],revision:1}});await vi.advanceTimersByTimeAsync(0);
  expect(controller.getSnapshot().confirmed?.revision).toBe(1);expect(controller.getSnapshot().draft.density).toBe(110);
  endEditing();
  let finish!:(value:unknown)=>void;const late=setup(()=>new Promise(resolve=>{finish=resolve;}));await controller.init();controller.adjustLayoutScale(1);await vi.advanceTimersByTimeAsync(120);controller.dispose();finish({kind:'saved',snapshot:{...DEFAULTS,density:110,revision:1}});await vi.advanceTimersByTimeAsync(0);expect(controller.getSnapshot().confirmed?.revision).toBe(0);expect(late.dataset.density).toBe('110');
 });
});

describe('layout scale shortcuts',()=>{
 const event=(key:string,init:KeyboardEventInit={})=>new KeyboardEvent('keydown',{key,bubbles:true,cancelable:true,...init});
 it('recognizes mac command minus, equals, and shifted equals only',()=>{
  expect(layoutScaleDirection(event('-',{metaKey:true}))).toBe(-1);
  expect(layoutScaleDirection(event('=',{metaKey:true}))).toBe(1);
  expect(layoutScaleDirection(event('+',{metaKey:true,shiftKey:true}))).toBe(1);
  expect(layoutScaleDirection(event('=',{metaKey:true,shiftKey:true}))).toBe(1);
  for(const invalid of [event('-'),event('-',{metaKey:true,ctrlKey:true}),event('-',{metaKey:true,altKey:true}),event('-',{metaKey:true,isComposing:true}),event('-',{metaKey:true,keyCode:229})])expect(layoutScaleDirection(invalid)).toBeNull();
 });
 it('installs only on macOS, respects consumed and stopped events, and claims repeats and scale boundaries',async()=>{
  const write=vi.fn();setup(write,{...structuredClone(DEFAULTS),density:60});await controller.init();const target=window;const adjust=vi.spyOn(controller,'adjustLayoutScale');
  const removeLinux=installLayoutScaleShortcuts(target,controller,'Linux');const linux=event('-',{metaKey:true});target.dispatchEvent(linux);expect(linux.defaultPrevented).toBe(false);expect(adjust).not.toHaveBeenCalled();removeLinux();const linuxAfterRemove=event('-',{metaKey:true});target.dispatchEvent(linuxAfterRemove);expect(linuxAfterRemove.defaultPrevented).toBe(false);expect(adjust).not.toHaveBeenCalled();
  const remove=installLayoutScaleShortcuts(target,controller,'MacIntel');
  const handled=event('-',{metaKey:true});handled.preventDefault();target.dispatchEvent(handled);expect(adjust).not.toHaveBeenCalled();
  const stopped=event('-',{metaKey:true});document.addEventListener('keydown',ev=>ev.stopPropagation(),{once:true});document.dispatchEvent(stopped);expect(adjust).not.toHaveBeenCalled();
  const repeat=event('-',{metaKey:true,repeat:true});target.dispatchEvent(repeat);expect(repeat.defaultPrevented).toBe(true);expect(adjust).not.toHaveBeenCalled();
  for(const ignored of [event('-',{metaKey:true,ctrlKey:true}),event('-',{metaKey:true,altKey:true}),event('-',{metaKey:true,isComposing:true}),event('-',{metaKey:true,keyCode:229})]){
   target.dispatchEvent(ignored);expect(ignored.defaultPrevented).toBe(false);
  }
  const boundary=event('-',{metaKey:true});target.dispatchEvent(boundary);expect(boundary.defaultPrevented).toBe(true);expect(adjust).toHaveBeenCalledTimes(1);expect(adjust).toHaveBeenLastCalledWith(-1);
  remove();const afterRemove=event('-',{metaKey:true});target.dispatchEvent(afterRemove);expect(afterRemove.defaultPrevented).toBe(false);expect(adjust).toHaveBeenCalledTimes(1);
 });
});
