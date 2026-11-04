import { applyTheme } from '../theme/apply';
import { DEFAULTS, appearanceValues, parseAppearance, parseConfirmed, parseRead, type Appearance, type AppearanceRead, type AppearanceSnapshot, type ResolvedTheme } from './model';

export type AppearancePhase = 'loading'|'applied'|'drafting'|'invalid'|'applying'|'failed'|'unconfirmed'|'corrupt'|'unsupported_schema';
export interface AppearanceState { phase:AppearancePhase; confirmed:AppearanceSnapshot|null; draft:Appearance; error:string|null; recoveryToken?:string }
export interface AppearanceDeps {
  root:HTMLElement;
  read:()=>Promise<unknown>;
  write:(snapshot:Appearance,expectedRevision:number)=>Promise<unknown>;
  recover?:(token:string)=>Promise<unknown>;
  matchMedia?:(query:string)=>MediaQueryList;
  readTimeoutMs?:number;
  writeTimeoutMs?:number;
  onWriteError?:(error:unknown)=>void;
}
export interface AppearanceController {
  init():Promise<void>;
  getSnapshot():AppearanceState;
  subscribe(listener:()=>void):()=>void;
  preview(values:Appearance):void;
  cancel():void;
  apply():Promise<void>;
  verify():Promise<void>;
  recover():Promise<void>;
  dispose():void;
}
function timeout<T>(promise:Promise<T>,ms:number):Promise<T> {
  return new Promise((resolve,reject)=>{ const timer=setTimeout(()=>reject(new Error('unconfirmed')),ms); promise.then(v=>{clearTimeout(timer);resolve(v);},e=>{clearTimeout(timer);reject(e);}); });
}
function canonical(value:unknown):unknown {
  if(Array.isArray(value))return value.map(canonical);
  if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).sort(([a],[b])=>a.localeCompare(b)).map(([key,item])=>[key,canonical(item)]));
  return typeof value==='string'&&/^#[0-9a-f]{6}$/i.test(value)?value.toLowerCase():value;
}
export function createAppearanceController(deps:AppearanceDeps):AppearanceController {
  let state:AppearanceState={phase:'loading',confirmed:null,draft:structuredClone(DEFAULTS),error:null};
  let initPromise:Promise<void>|null=null;
  let disposed=false;
  let sequence=0;
  let pending: {values:Appearance;revision:number}|null=null;
  const listeners=new Set<()=>void>();
  const query=deps.matchMedia?.('(prefers-color-scheme: dark)');
  let rendered:Appearance=structuredClone(DEFAULTS);
  const render=(values:Appearance)=>{
    const mode:ResolvedTheme=values.mode==='auto' ? query?.matches?'dark':'light' : values.mode;
    applyTheme(deps.root,values,mode); rendered=values;
  };
  const emit=()=>{for(const listener of listeners) listener();};
  const restore=()=>render(state.confirmed?appearanceValues(state.confirmed):DEFAULTS);
  const systemChange=()=>render(rendered);
  query?.addEventListener('change',systemChange);
  function accept(read:AppearanceRead,keepDraft=false):void {
    if(read.kind==='ready') {
      state={phase:'applied',confirmed:read.snapshot,draft:keepDraft?state.draft:appearanceValues(read.snapshot),error:null};
    } else {
      state={phase:read.kind,confirmed:null,draft:structuredClone(DEFAULTS),error:read.kind,...(read.kind==='corrupt'?{recoveryToken:read.recoveryToken}:{})};
    }
    restore(); emit();
  }
  async function init():Promise<void> {
    initPromise??=(async()=>{
      try {const raw=await timeout(deps.read(),deps.readTimeoutMs??1000);if(!disposed)accept(parseRead(raw));}
      catch {if(!disposed){state={...state,phase:'failed',error:'read_failed'};restore();emit();}}
      finally {deps.root.removeAttribute('data-appearance-pending');}
    })();return initPromise;
  }
  async function verify():Promise<void> {
    if(disposed)return;
    const seq=++sequence;
    try {
      const read=parseRead(await timeout(deps.read(),deps.readTimeoutMs??1000));
      if(disposed||seq!==sequence)return;
      if(pending&&read.kind==='ready') {
        const matched=read.snapshot.revision===pending.revision+1&&JSON.stringify(canonical(appearanceValues(read.snapshot)))===JSON.stringify(canonical(pending.values));
        const draft=state.draft;
        accept(read,!matched);
        if(!matched){state={...state,phase:'failed',draft,error:'revision_conflict'};emit();}
        pending=null;
      } else { pending=null; accept(read,true); }
    } catch {if(seq===sequence&&!disposed){state={...state,phase:'unconfirmed',error:'unconfirmed'};restore();emit();}}
  }
  return {
    init,getSnapshot:()=>state,subscribe(listener){listeners.add(listener);return()=>listeners.delete(listener);},
    preview(draft){
      if(['applying','unconfirmed','corrupt','unsupported_schema'].includes(state.phase)||!state.confirmed)return;
      try {const valid=parseAppearance(draft);render(valid);state={...state,draft:valid,phase:'drafting',error:null};}
      catch(error){state={...state,draft,phase:'invalid',error:error instanceof Error?error.message:'invalid'};}
      emit();
    },
    cancel(){if(state.phase==='applying')return;state={...state,draft:state.confirmed?appearanceValues(state.confirmed):structuredClone(DEFAULTS),phase:state.phase==='unconfirmed'?'unconfirmed':state.confirmed?'applied':state.phase,error:null};restore();emit();},
    async apply(){
      if(!state.confirmed||!['drafting','failed'].includes(state.phase))return;
      const values=parseAppearance(state.draft); const revision=state.confirmed.revision; const seq=++sequence;
      pending={values,revision};state={...state,phase:'applying',error:null};emit();
      try {
        const raw=await timeout(deps.write(values,revision),deps.writeTimeoutMs??10000);
        if(disposed||seq!==sequence)return;
        if(!raw||typeof raw!=='object'||!('kind'in raw)||!('snapshot'in raw))throw new Error('unconfirmed');
        const confirmed=parseConfirmed(raw.snapshot);
        if(raw.kind==='saved') {state={phase:'applied',confirmed,draft:appearanceValues(confirmed),error:null};}
        else if(raw.kind==='conflict'){state={...state,confirmed,phase:'failed',error:'revision_conflict'};}
        else throw new Error('unconfirmed');
        pending=null;
        restore();emit();
      }catch(error){
        if(disposed||seq!==sequence)return;
        const explicit=error&&typeof error==='object'&&'kind'in error&&error.kind==='appearance_error';
        state={...state,phase:explicit?'failed':'unconfirmed',error:explicit?'save_failed':'unconfirmed'};restore();emit();
        if(explicit){pending=null;deps.onWriteError?.(error);}else await verify();
      }
    },verify,
    async recover(){
      if(state.phase!=='corrupt'||!state.recoveryToken||!deps.recover)return;
      const seq=++sequence;const token=state.recoveryToken;state={...state,phase:'applying'};emit();
      try {const snapshot=parseConfirmed(await deps.recover(token));if(seq===sequence&&!disposed)accept({kind:'ready',snapshot,repairWarnings:[]});}
      catch {if(seq===sequence&&!disposed)await verify();}
    },
    dispose(){disposed=true;sequence++;query?.removeEventListener('change',systemChange);listeners.clear();},
  };
}
