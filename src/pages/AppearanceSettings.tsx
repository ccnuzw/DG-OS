import { useEffect, useId, useState } from 'react';
import { RadioGroup as RadioPrimitive } from 'radix-ui';
import { Button, Modal, Popover, Select, Slider, Stack, THEMES, Tooltip, useAppearance } from '@dgos/ui';
import { DEFAULTS, appearanceValues, type Appearance, type ResolvedTheme } from '../ui/appearance/model';
import { AppearanceTypography } from './AppearanceTypography';
import { AppearanceComparison } from './AppearanceComparison';
import { PRESETS, PRESET_IDS, type PresetId, type ThemeScope } from '../ui/theme/kernel';
import { colorIssues, effectiveColors, overrideCount, resetRegion, setColor, withPreset, type ColorFieldName } from '../ui/appearance/colors';
import { ColorField } from '../ui/appearance/ColorField';
import { LANGUAGE_PREFERENCES, updateLanguage, useLocale, type LanguagePreference } from '../i18n';
import { usePageError } from '../pageError';
import { useSurfaceCloseGuard } from '../workspace/SurfaceContext';
import { confirmDiscard } from '../workspace/closeCoordinator';
import './settings-content.css';

function Segments({ label, value, options, disabled, onChange }: { label: string; value: string; options: { value: string; label: string }[]; disabled?: boolean; onChange: (value: string) => void }) {
  const id = useId();
  return <div className="appearance-mode-control"><span id={id}>{label}</span><RadioPrimitive.Root aria-labelledby={id} className="appearance-segments" value={value} disabled={disabled} onValueChange={onChange}>{options.map(option=><RadioPrimitive.Item key={option.value} value={option.value} className="ui-focus">{option.label}</RadioPrimitive.Item>)}</RadioPrimitive.Root></div>;
}

function Palette({ preset, mode }: { preset: PresetId; mode: ResolvedTheme }) {
  const colors = PRESETS[preset][mode];
  return <svg aria-hidden="true" viewBox="0 0 240 20" className="appearance-preset-sample"><rect width="120" height="20" fill={colors.background}/><rect x="120" width="80" height="20" fill={colors.foreground}/><rect x="200" width="40" height="20" fill={colors.accent}/></svg>;
}

// Canonical comparison follows the snapshot boundary, including case-insensitive HEX values.
function comparable(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) => {
    if (typeof item === 'string' && /^#[\da-f]{6}$/i.test(item)) return item.toLowerCase();
    if (item && typeof item === 'object' && !Array.isArray(item)) return Object.fromEntries(Object.entries(item).sort(([a],[b])=>a.localeCompare(b)));
    return item;
  });
}

export default function AppearanceSettings() {
  const {state,controller}=useAppearance();
  const {preference,t}=useLocale();
  const reportError=usePageError('AppearanceSettings');
  const [pendingPreset,setPendingPreset]=useState<{mode:ResolvedTheme;preset:PresetId}|null>(null);
  const [presetMenu,setPresetMenu]=useState<ResolvedTheme|null>(null);
  const [invalidColorFields,setInvalidColorFields]=useState<ReadonlySet<string>>(()=>new Set());
  const [colorResets,setColorResets]=useState<Record<string,number>>({});
  const [fontFailed,setFontFailed]=useState(false);
  const [languageBusy,setLanguageBusy]=useState(false);
  const [runtimeMode,setRuntimeMode]=useState(document.documentElement.dataset.theme);
  const resetColorInputs=(targetMode?:ResolvedTheme,targetScope?:ThemeScope)=>{
    setColorResets(current=>{
      const next={...current};
      for(const m of (targetMode?[targetMode]:['light','dark']))for(const s of (targetScope?[targetScope]:['interface','reading','code']))next[`${m}:${s}`]=(next[`${m}:${s}`]??0)+1;
      return next;
    });
    setInvalidColorFields(current=>new Set([...current].filter(key=>targetMode&&(!key.startsWith(`${targetMode}:`)||(targetScope&&!key.startsWith(`${targetMode}:${targetScope}:`))))));
  };
  useEffect(()=>{
    const failed=()=>setFontFailed(true);
    const observer=new MutationObserver(()=>setRuntimeMode(document.documentElement.dataset.theme));
    observer.observe(document.documentElement,{attributes:true,attributeFilter:['data-theme']});
    document.documentElement.addEventListener('appearance-font-error',failed);
    return()=>{observer.disconnect();document.documentElement.removeEventListener('appearance-font-error',failed);};
  },[]);
  const draft=state.draft;
  const busy=state.phase==='applying';
  const blocked=busy||['loading','corrupt','unsupported_schema','unconfirmed'].includes(state.phase)||!state.confirmed;
  const changed=!!state.confirmed&&comparable(draft)!==comparable(appearanceValues(state.confirmed));
  useSurfaceCloseGuard(async () => {
    const current=controller.getSnapshot();
    if (current.phase === 'applying' || current.phase === 'unconfirmed') return false;
    if ((invalidColorFields.size>0 || (current.confirmed&&comparable(current.draft)!==comparable(appearanceValues(current.confirmed)))) && !await confirmDiscard()) return false;
    controller.cancel();
    return true;
  });
  useEffect(()=>{
    const endEditing=controller.beginEditing();
    void controller.init();
    const leave=(event:KeyboardEvent)=>{if(event.key==='Escape'&&!event.defaultPrevented&&!document.querySelector('[data-radix-popper-content-wrapper]')){resetColorInputs();controller.cancel();}};
    let deferred: HTMLElement | null = null;
    const leaving = () => ['applying','unconfirmed'].includes(controller.getSnapshot().phase);
    const deferNavigation = (event: MouseEvent | KeyboardEvent) => {
      const target = event.target instanceof Element ? event.target.closest<HTMLElement>('a[href], [role="tab"]') : null;
      if (!target || !leaving() || (event instanceof KeyboardEvent && !['Enter',' '].includes(event.key))) return;
      event.preventDefault(); event.stopImmediatePropagation(); deferred = target;
    };
    const unsubscribe = controller.subscribe(() => {
      if (!leaving() && deferred) { const target = deferred; deferred = null; target.click(); }
    });
    document.addEventListener('click', deferNavigation, true);
    document.addEventListener('keydown', deferNavigation, true);
    window.addEventListener('keydown',leave);
    return()=>{endEditing();unsubscribe();document.removeEventListener('click',deferNavigation,true);document.removeEventListener('keydown',deferNavigation,true);window.removeEventListener('keydown',leave);controller.cancel();};
  },[controller]);
  const change=(update:(next:Appearance)=>void)=>{const next=structuredClone(draft);update(next);controller.preview(next);};
  const allIssues=colorIssues(draft);
  const migrationPending=state.repairWarnings.includes('density_migrated');
  const saveDisabled=blocked||(!changed&&!migrationPending)||invalidColorFields.size>0||allIssues.length>0||!(['drafting','failed'].includes(state.phase)||(state.phase==='applied'&&migrationPending));
  const clearInvalid=(key:string,invalid:boolean)=>setInvalidColorFields(current=>{
    if(current.has(key)===invalid)return current;
    const next=new Set(current);if(invalid)next.add(key);else next.delete(key);return next;
  });
  const modeTitle=(mode:ResolvedTheme)=>t(`appearance.scheme.${mode}`);
  const choosePreset=(mode:ResolvedTheme,preset:PresetId)=>{
    setPresetMenu(null);
    if(draft.variants[mode].presetId===preset)return;
    if(!overrideCount(draft.variants[mode])){change(next=>{next.variants[mode]=withPreset(next.variants[mode],preset,false);});return;}
    setPendingPreset({mode,preset});
  };
  const renderField=(mode:ResolvedTheme,scope:ThemeScope,field:ColorFieldName)=>{
    const effective=effectiveColors(draft.variants[mode],mode,scope);
    const invalidKey=`${mode}:${scope}:${field}`;
    const issues=allIssues.filter(issue=>issue.mode===mode&&issue.scope===scope&&issue.fields.includes(field));
    return <ColorField key={`${invalidKey}:${colorResets[`${mode}:${scope}`]??0}`} label={`${modeTitle(mode)} · ${scope==='interface'?'':`${t(`appearance.scope.${scope}`)} · `}${t(`appearance.color.${field}`)}`} value={effective[field]} override={draft.variants[mode].overrides[scope]?.[field]} recommendations={PRESET_IDS.map(id=>PRESETS[id][mode][field])} disabled={blocked} error={issues.map(issue=>t(`appearance.colorValidation.${issue.kind}`)).join('；')||undefined}
      onValidityChange={invalid=>clearInvalid(invalidKey,invalid)}
      onChange={value=>change(next=>{next.variants[mode]=scope==='interface'?setColor(next.variants[mode],'interface',field,value):setColor(next.variants[mode],scope,field as Exclude<ColorFieldName,'accent'>,value);})}/>;
  };
  const explainIssue=(issue:typeof allIssues[number])=>t('appearance.colorValidation',{mode:modeTitle(issue.mode),scope:t(`appearance.scope.${issue.scope}`),field:issue.fields.map(field=>t(`appearance.color.${field}`)).join(' / '),reason:t(`appearance.colorValidation.${issue.kind}`)});
  const applyPreset=(keep:boolean)=>{const selection=pendingPreset;setPendingPreset(null);if(selection)change(next=>{next.variants[selection.mode]=withPreset(next.variants[selection.mode],selection.preset,keep);});};
  return <div className="appearance-editor">
    <div className="appearance-body"><div className="appearance-content settings-sections">
      <h1 className="m-0 text-lg font-semibold">{t('appearance.title')}</h1>
      {state.repairWarnings.includes('density_migrated')?<p role="status" className="appearance-hint">{t('appearance.densityMigrated')}</p>:null}
      <section className="appearance-section" aria-labelledby="appearance-global-heading">
        <h2 id="appearance-global-heading" className="settings-section-title">{t('appearance.global')}</h2>
        <div className="appearance-top">
        <div><Segments label={t('appearance.theme')} disabled={blocked} options={THEMES.map(value=>({value,label:t(`appearance.theme${value==='auto'?'Auto':value==='light'?'Light':'Dark'}`)}))} value={draft.mode} onChange={value=>change(next=>{next.mode=value as Appearance['mode'];})}/>{draft.mode==='auto'?<p className="appearance-hint">{t('appearance.resolvedMode',{mode:t(runtimeMode==='dark'?'appearance.themeDark':'appearance.themeLight')})}</p>:null}</div>
        <div><Select label={t('appearance.language')} disabled={languageBusy} options={LANGUAGE_PREFERENCES.map(value=>({value,label:t(value==='zh-CN'?'languages.zhCN':value==='en-US'?'languages.enUS':'languages.system')}))} value={preference} onChange={value=>{setLanguageBusy(true);void updateLanguage(value as LanguagePreference).catch(reportError).finally(()=>setLanguageBusy(false));}}/><p className="appearance-hint">{t('appearance.languageImmediate')}</p></div>
        </div>
        <AppearanceComparison draft={draft}/>
      </section>
      <section className="appearance-section" aria-labelledby="appearance-colors-heading"><h2 id="appearance-colors-heading" className="settings-section-title">{t('appearance.colors')}</h2><div className="appearance-pair">
        {(['light','dark'] as const).map(mode=>{const variant=draft.variants[mode];return <section key={mode} className="appearance-scheme" aria-label={modeTitle(mode)} data-appearance-mode={mode}>
          <div className="appearance-scheme-heading">
            <h3 className="settings-section-title">{modeTitle(mode)}</h3>
            <Button variant="ghost" disabled={blocked} onClick={()=>{resetColorInputs(mode);change(next=>{next.variants[mode]=structuredClone(DEFAULTS.variants[mode]);});}}>{t('appearance.restoreScheme',{mode:modeTitle(mode)})}</Button>
          </div>
          <Popover label={`${modeTitle(mode)} · ${t('appearance.preset')}`} open={presetMenu===mode} onOpenChange={open=>setPresetMenu(open?mode:null)} align="start" trigger={<Button variant="secondary" disabled={blocked} className="appearance-preset-trigger"><Palette preset={variant.presetId} mode={mode}/><span>{t(`appearance.preset.${variant.presetId}`)} ▾</span></Button>} content={<div className="appearance-preset-menu" role="group" aria-label={`${modeTitle(mode)} · ${t('appearance.preset')}`}>{PRESET_IDS.map(preset=><Button key={preset} variant="secondary" className="appearance-preset" aria-pressed={variant.presetId===preset} onClick={()=>choosePreset(mode,preset)}><Palette preset={preset} mode={mode}/><span>{t(`appearance.preset.${preset}`)}</span></Button>)}</div>} />
          {(['accent','background','foreground'] as const).map(field=>renderField(mode,'interface',field))}
          <details className="appearance-content-colors"><summary>{t('appearance.contentColors')}</summary><Stack gap={4}>{(['reading','code'] as const).map(region=><section className="appearance-color-region" key={region} aria-label={`${modeTitle(mode)} · ${t(`appearance.scope.${region}`)}`}><div className="appearance-color-region-heading"><h4>{t(`appearance.scope.${region}`)}</h4><Button variant="ghost" disabled={blocked} onClick={()=>{resetColorInputs(mode,region);change(next=>{next.variants[mode]=resetRegion(next.variants[mode],region);});}}>{t('appearance.restoreRegion')}</Button></div>{(['background','foreground'] as const).map(field=>renderField(mode,region,field))}</section>)}</Stack></details>
          {allIssues.filter(issue=>issue.mode===mode).map((issue,index)=><p key={index} role="alert" className="appearance-error">{explainIssue(issue)}</p>)}
        </section>;})}
      </div></section>
      <AppearanceTypography value={draft.typography} disabled={blocked} onChange={typography=>change(next=>{next.typography=typography;})}/>
      {fontFailed?<p role="alert" className="appearance-error">{t('appearance.fontFailed')}</p>:null}
      <section className="appearance-section" aria-labelledby="appearance-effects-heading"><h2 id="appearance-effects-heading" className="settings-section-title">{t('appearance.more')}</h2><div className="appearance-more-fields">
        <div className="appearance-layout-scale">
          <div className="appearance-scale-heading"><span>{t('appearance.density')}</span><output aria-label={t('appearance.densityCurrent')}>{draft.density}%</output><Button variant="ghost" disabled={blocked||draft.density===100} onClick={()=>change(next=>{next.density=100;})}>{t('appearance.densityReset')}</Button></div>
          <div className="appearance-scale-inputs">
            <Tooltip content={t('appearance.densityDecreaseHint')}><Button variant="secondary" iconOnly aria-label={t('appearance.densityDecrease')} disabled={blocked||draft.density<=60} onClick={()=>change(next=>{next.density=(next.density-10) as Appearance['density'];})}>−</Button></Tooltip>
            <div className="appearance-scale-track"><Slider label={t('appearance.density')} hideLabel min={60} max={150} step={10} value={draft.density} disabled={blocked} onValueChange={value=>change(next=>{next.density=value as Appearance['density'];})}/><div className="appearance-scale-ticks" aria-hidden="true"><span>60%</span><span>100%</span><span>150%</span></div></div>
            <Tooltip content={t('appearance.densityIncreaseHint')}><Button variant="secondary" iconOnly aria-label={t('appearance.densityIncrease')} disabled={blocked||draft.density>=150} onClick={()=>change(next=>{next.density=(next.density+10) as Appearance['density'];})}>+</Button></Tooltip>
          </div>
          <p className="appearance-hint">{t('appearance.densityHint')}</p>
        </div>
        <Select label={t('appearance.motion')} disabled={blocked} value={draft.effects.reducedMotion} options={['system','reduce'].map(value=>({value,label:t(`appearance.motion.${value}`)}))} onChange={value=>change(next=>{next.effects.reducedMotion=value as 'system'|'reduce';})}/>
        <Select label={t('appearance.sidebar')} disabled={blocked} value={draft.effects.sidebarTreatment} options={['solid','soft'].map(value=>({value,label:t(`appearance.sidebar.${value}`)}))} onChange={value=>change(next=>{next.effects.sidebarTreatment=value as 'solid'|'soft';})}/>
      </div></section>
    </div></div>
    <footer className="appearance-footer">
      {state.error?<p role="alert" className="appearance-error">{t(state.error==='revision_conflict'?'appearance.error.conflict':`appearance.error.${['invalid','failed','unconfirmed','corrupt','unsupported_schema'].includes(state.phase)?state.phase:'failed'}`)}</p>:null}
      <div className="appearance-footer-actions"><Button variant="secondary" disabled={blocked} onClick={()=>{resetColorInputs();controller.preview(structuredClone(DEFAULTS));}}>{t('appearance.restoreAll')}</Button>
        <p role="status" className="appearance-save-state">{t(busy?'appearance.saving':invalidColorFields.size||allIssues.length?'appearance.fixErrors':changed?'appearance.unsaved':'appearance.saved')}</p>
        <div className="appearance-save-actions"><Button variant="secondary" disabled={busy||state.phase==='unconfirmed'} onClick={()=>{resetColorInputs();controller.cancel();}}>{t('appearance.cancelPreview')}</Button>
          {state.phase==='unconfirmed'||(!state.confirmed&&state.phase==='failed')?<Button onClick={()=>void controller.verify()}>{t('appearance.verify')}</Button>:null}
          {state.phase==='corrupt'?<Button variant="danger" onClick={()=>void controller.recover()}>{t('appearance.recover')}</Button>:<Button disabled={saveDisabled} onClick={()=>void controller.apply()}>{t('appearance.apply')}</Button>}
        </div>
      </div>
    </footer>
    <Modal open={pendingPreset!==null} onOpenChange={open=>{if(!open)setPendingPreset(null);}} kind="form" title={t('appearance.colorResetPresetTitle')} description={t('appearance.colorResetPresetContent',{count:pendingPreset?overrideCount(draft.variants[pendingPreset.mode]):0})} footer={<div className="appearance-preset-confirm-actions"><Button variant="secondary" onClick={()=>setPendingPreset(null)}>{t('common.cancel')}</Button><Button variant="secondary" onClick={()=>applyPreset(true)}>{t('appearance.colorKeepCustom')}</Button><Button onClick={()=>applyPreset(false)}>{t('appearance.colorUsePreset')}</Button></div>}/>
  </div>;
}
