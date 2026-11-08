import { applyTheme } from '../theme/apply';
import { DEFAULTS, appearanceValues, parseAppearance, parseConfirmed, parseRead, type Appearance, type AppearanceRead, type AppearanceSnapshot, type ResolvedTheme } from './model';
import { loadFontCatalogue, subscribeFontCatalogue } from '../theme/fontCatalogue';

export type AppearancePhase = 'loading'|'applied'|'drafting'|'invalid'|'applying'|'failed'|'unconfirmed'|'corrupt'|'unsupported_schema';
export interface AppearanceState { phase:AppearancePhase; confirmed:AppearanceSnapshot|null; draft:Appearance; error:string|null; repairWarnings:string[]; recoveryToken?:string; layoutFeedback?:Appearance['density'] }
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
  beginEditing():()=>void;
  adjustLayoutScale(direction:-1|1):boolean;
  suspendLayoutScale():()=>void;
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
  let state:AppearanceState={phase:'loading',confirmed:null,draft:structuredClone(DEFAULTS),error:null,repairWarnings:[]};
  let initPromise:Promise<void>|null=null;
  let disposed=false;
  let sequence=0;
  let editors=0;
  let layoutSuspensions=0;
  let layoutTimer:ReturnType<typeof setTimeout>|undefined;
  let feedbackTimer:ReturnType<typeof setTimeout>|undefined;
  let layoutTarget:Appearance['density']|null=null;
  let layoutWriting=false;
  const clearLayoutQueue=()=>{clearTimeout(layoutTimer);layoutTimer=undefined;layoutTarget=null;};
  let pending: {values:Appearance;revision:number;hiddenDraft?:Appearance}|null=null;
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
  const stopFonts = subscribeFontCatalogue(() => { if (!disposed) render(rendered); });
  function accept(read:AppearanceRead,keepDraft=false,saved=false):void {
    if(read.kind==='ready') {
      state={phase:'applied',confirmed:read.snapshot,draft:keepDraft?state.draft:appearanceValues(read.snapshot),error:null,repairWarnings:saved?[]:[...new Set([...state.repairWarnings,...read.repairWarnings])]};
    } else {
      state={phase:read.kind,confirmed:null,draft:structuredClone(DEFAULTS),error:read.kind,repairWarnings:state.repairWarnings,...(read.kind==='corrupt'?{recoveryToken:read.recoveryToken}:{})};
    }
    restore(); emit();
  }
  async function init():Promise<void> {
    void loadFontCatalogue();
    initPromise??=(async()=>{
      try {const raw=await timeout(deps.read(),deps.readTimeoutMs??1000);if(!disposed)accept(parseRead(raw));}
      catch {if(!disposed){state={...state,phase:'failed',error:'read_failed'};restore();emit();}}
      finally {deps.root.removeAttribute('data-appearance-pending');}
    })();return initPromise;
  }
  async function save(values:Appearance,automatic=false):Promise<void> {
    if(!state.confirmed||disposed)return;
    const revision=state.confirmed.revision;const seq=++sequence;
    const hiddenDraft=automatic&&JSON.stringify(canonical(state.draft))!==JSON.stringify(canonical(appearanceValues(state.confirmed)))?state.draft:null;
    pending={values,revision,...(hiddenDraft?{hiddenDraft}:{})};layoutWriting=automatic;state={...state,phase:'applying',error:null};emit();
    try {
      const raw=await timeout(deps.write(values,revision),deps.writeTimeoutMs??10000);
      if(disposed||seq!==sequence)return;
      if(!raw||typeof raw!=='object'||!('kind'in raw)||!('snapshot'in raw))throw new Error('unconfirmed');
      const confirmed=parseConfirmed(raw.snapshot);
      if(raw.kind==='saved')state={phase:'applied',confirmed,draft:hiddenDraft?{...hiddenDraft,density:confirmed.density}:appearanceValues(confirmed),error:null,repairWarnings:[],layoutFeedback:state.layoutFeedback};
      else if(raw.kind==='conflict'){clearLayoutQueue();state={...state,confirmed,phase:'failed',error:'revision_conflict'};}
      else throw new Error('unconfirmed');
      pending=null;layoutWriting=false;
      restore();
      if(automatic&&!editors&&!layoutSuspensions&&state.phase==='applied'&&layoutTarget!==null&&layoutTarget!==confirmed.density){render({...appearanceValues(confirmed),density:layoutTarget});scheduleLayoutSave();}
      else clearLayoutQueue();
      emit();
    }catch(error){
      if(disposed||seq!==sequence)return;
      clearLayoutQueue();layoutWriting=false;
      const explicit=error&&typeof error==='object'&&'kind'in error&&error.kind==='appearance_error';
      state={...state,phase:explicit?'failed':'unconfirmed',error:explicit?'save_failed':'unconfirmed'};restore();emit();
      if(explicit){pending=null;deps.onWriteError?.(error);}else await verify();
    }
  }
  function scheduleLayoutSave():void {
    clearTimeout(layoutTimer);
    layoutTimer=setTimeout(()=>{
      layoutTimer=undefined;
      if(disposed||editors||layoutSuspensions||layoutWriting||layoutTarget===null||!state.confirmed)return;
      if(layoutTarget===state.confirmed.density){clearLayoutQueue();return;}
      void save({...appearanceValues(state.confirmed),density:layoutTarget},true);
    },120);
  }
  async function verify():Promise<void> {
    if(disposed)return;
    clearLayoutQueue();
    const seq=++sequence;
    try {
      const read=parseRead(await timeout(deps.read(),deps.readTimeoutMs??1000));
      if(disposed||seq!==sequence)return;
      if(pending&&read.kind==='ready') {
        const matched=read.snapshot.revision===pending.revision+1&&JSON.stringify(canonical(appearanceValues(read.snapshot)))===JSON.stringify(canonical(pending.values));
        const draft=state.draft;
        accept(read,!matched,matched);
        if(matched&&pending.hiddenDraft){state={...state,draft:{...pending.hiddenDraft,density:read.snapshot.density}};emit();}
        if(!matched){state={...state,phase:'failed',draft,error:'revision_conflict'};emit();}
        pending=null;
      } else { pending=null; accept(read,true); }
    } catch {if(seq===sequence&&!disposed){state={...state,phase:'unconfirmed',error:'unconfirmed'};restore();emit();}}
  }
  return {
    init,getSnapshot:()=>state,subscribe(listener){listeners.add(listener);return()=>listeners.delete(listener);},
    suspendLayoutScale(){
      layoutSuspensions++;clearLayoutQueue();
      if(!layoutWriting)restore();
      let active=true;return()=>{if(active){active=false;layoutSuspensions--;}};
    },
    beginEditing(){
      editors++;
      // A queued preview becomes this editor's draft; an already submitted write must settle first.
      if(!layoutWriting&&layoutTarget!==null){state={...state,draft:{...state.draft,density:layoutTarget},phase:'drafting'};clearLayoutQueue();emit();}
      if(layoutWriting)clearLayoutQueue();
      let active=true;return()=>{if(active){active=false;editors--;}};
    },
    adjustLayoutScale(direction){
      if(disposed||layoutSuspensions||!state.confirmed||['loading','unconfirmed','corrupt','unsupported_schema'].includes(state.phase)||(state.phase==='applying'&&!layoutWriting)||(!editors&&state.error!==null))return false;
      if(editors&&layoutWriting)return false;
      const current=editors?state.draft.density:layoutTarget??state.confirmed.density;
      const density=Math.max(60,Math.min(150,current+direction*10)) as Appearance['density'];
      if(density===current)return false;
      if(editors){
        // Invalid color drafts remain intact; only geometry uses the last valid preview.
        const draft={...state.draft,density};
        render({...rendered,density});
        state={...state,draft,phase:state.phase==='invalid'?'invalid':'drafting'};emit();
      }else{
        layoutTarget=density;render({...appearanceValues(state.confirmed),density});emit();
        if(!layoutWriting)scheduleLayoutSave();
      }
      clearTimeout(feedbackTimer);
      state={...state,layoutFeedback:density};emit();
      feedbackTimer=setTimeout(()=>{if(!disposed){state={...state,layoutFeedback:undefined};emit();}},1400);
      return true;
    },
    preview(draft){
      if(['applying','unconfirmed','corrupt','unsupported_schema'].includes(state.phase)||!state.confirmed)return;
      try {const valid=parseAppearance(draft);render(valid);state={...state,draft:valid,phase:'drafting',error:null};}
      catch(error){state={...state,draft,phase:'invalid',error:error instanceof Error?error.message:'invalid'};}
      emit();
    },
    cancel(){if(state.phase==='applying')return;clearLayoutQueue();state={...state,draft:state.confirmed?appearanceValues(state.confirmed):structuredClone(DEFAULTS),phase:state.phase==='unconfirmed'?'unconfirmed':state.confirmed?'applied':state.phase,error:null};restore();emit();},
    async apply(){
      if(!state.confirmed||!(['drafting','failed'].includes(state.phase)||(state.phase==='applied'&&state.repairWarnings.includes('density_migrated'))))return;
      clearLayoutQueue();
      await save(parseAppearance(state.draft));
    },verify,
    async recover(){
      if(state.phase!=='corrupt'||!state.recoveryToken||!deps.recover)return;
      const seq=++sequence;const token=state.recoveryToken;state={...state,phase:'applying'};emit();
      try {const snapshot=parseConfirmed(await deps.recover(token));if(seq===sequence&&!disposed)accept({kind:'ready',snapshot,repairWarnings:[]},false,true);}
      catch {if(seq===sequence&&!disposed)await verify();}
    },
    dispose(){disposed=true;sequence++;clearLayoutQueue();clearTimeout(feedbackTimer);stopFonts();query?.removeEventListener('change',systemChange);listeners.clear();},
  };
}
