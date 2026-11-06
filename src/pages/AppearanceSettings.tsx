import { useEffect, useState } from 'react';
import { Button, DENSITIES, Input, RadioGroup, Select, Stack, THEMES, useAppearance } from '@dgos/ui';
import { DEFAULTS, type Appearance, type ResolvedTheme } from '../ui/appearance/model';
import { FONT_IDS, FONT_SIZES } from '../ui/theme/typography';
import { PRESETS, PRESET_IDS, type ThemeScope } from '../ui/theme/kernel';
import { LANGUAGE_PREFERENCES, updateLanguage, useLocale, type LanguagePreference } from '../i18n';
import { usePageError } from '../pageError';
import { useSurfaceCloseGuard } from '../workspace/SurfaceContext';
import { confirmDiscard } from '../workspace/closeCoordinator';

export default function AppearanceSettings() {
  const {state,controller}=useAppearance();
  const {preference,t}=useLocale();
  const reportError=usePageError('AppearanceSettings');
  const [mode,setMode]=useState<ResolvedTheme>('light');
  const [scope,setScope]=useState<ThemeScope>('interface');
  const draft=state.draft;
  const busy=state.phase==='applying';
  const blocked=busy||['loading','corrupt','unsupported_schema','unconfirmed'].includes(state.phase)||!state.confirmed;
  useSurfaceCloseGuard(async () => {
    const phase = controller.getSnapshot().phase;
    if (phase === 'applying' || phase === 'unconfirmed') return false;
    if (['drafting', 'invalid', 'failed'].includes(phase) && !await confirmDiscard()) return false;
    controller.cancel();
    return true;
  });
  useEffect(()=>{
    void controller.init();
    const leave=(event:KeyboardEvent)=>{if(event.key==='Escape'&&!event.defaultPrevented&&!document.querySelector('[data-radix-popper-content-wrapper]'))controller.cancel();};
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
    return()=>{unsubscribe();document.removeEventListener('click',deferNavigation,true);document.removeEventListener('keydown',deferNavigation,true);window.removeEventListener('keydown',leave);controller.cancel();};
  },[controller]);
  const change=(update:(next:Appearance)=>void)=>{const next=structuredClone(draft);update(next);controller.preview(next);};
  const variant=draft.variants[mode];
  const colors=variant.overrides[scope]??{};
  const fields=scope==='interface'?['accent','background','foreground'] as const:['background','foreground'] as const;
  return <div className="settings-content settings-sections appearance-editor">
    <h1 className="m-0 text-lg font-semibold">{t('appearance.title')}</h1>
    <section className="appearance-section"><RadioGroup className="appearance-options" label={t('appearance.language')} options={LANGUAGE_PREFERENCES.map(value=>({value,label:t(value==='zh-CN'?'languages.zhCN':value==='en-US'?'languages.enUS':'languages.system')}))} value={preference} onChange={value=>void updateLanguage(value as LanguagePreference).catch(reportError)} /></section>
    <section className="appearance-section"><Stack gap={4}>
      <RadioGroup className="appearance-options" label={t('appearance.theme')} disabled={blocked} options={THEMES.map(value=>({value,label:t(`appearance.theme${value==='auto'?'Auto':value==='light'?'Light':'Dark'}`)}))} value={draft.mode} onChange={value=>change(next=>{next.mode=value as Appearance['mode'];})}/>
      <RadioGroup className="appearance-options" label={t('appearance.editVariant')} options={(['light','dark'] as const).map(value=>({value,label:t(value==='light'?'appearance.themeLight':'appearance.themeDark')}))} value={mode} onChange={value=>setMode(value as ResolvedTheme)}/>
      <div role="group" aria-label={t('appearance.preset')} className="appearance-presets">{PRESET_IDS.map(value => <Button key={value} variant="secondary" className="appearance-preset" disabled={blocked} aria-pressed={variant.presetId === value} onClick={() => change(next => { next.variants[mode].presetId = value; })}><svg aria-hidden="true" viewBox="0 0 240 20" className="appearance-preset-sample"><rect width="120" height="20" fill={PRESETS[value][mode].background} /><rect x="120" width="80" height="20" fill={PRESETS[value][mode].foreground} /><rect x="200" width="40" height="20" fill={PRESETS[value][mode].accent} /></svg><span>{t(`appearance.preset.${value}`)}</span></Button>)}</div>
    </Stack></section>
    <section className="appearance-section"><Stack gap={4}>
      <RadioGroup className="appearance-options" label={t('appearance.region')} options={(['interface','reading','code'] as const).map(value=>({value,label:t(`appearance.scope.${value}`)}))} value={scope} onChange={value=>setScope(value as ThemeScope)}/>
      {fields.map(field=><div key={field} className="flex items-end gap-2"><Input label={t(`appearance.color.${field}`)} disabled={blocked} value={colors[field as keyof typeof colors]??''} placeholder={t('appearance.inherited')} onChange={event=>change(next=>{const region={...next.variants[mode].overrides[scope]};const value=event.target.value;if(value)(region as Record<string,string>)[field]=value;else delete (region as Record<string,string>)[field];next.variants[mode].overrides={...next.variants[mode].overrides,[scope]:region};})}/><Button variant="secondary" disabled={blocked} onClick={()=>change(next=>{const region={...next.variants[mode].overrides[scope]};delete(region as Record<string,string>)[field];next.variants[mode].overrides={...next.variants[mode].overrides,[scope]:region};})}>{t('appearance.restore')}</Button></div>)}
      <Button variant="secondary" disabled={blocked} onClick={()=>change(next=>{next.variants[mode].overrides={...next.variants[mode].overrides,[scope]:{}};})}>{t('appearance.restoreRegion')}</Button>
    </Stack></section>
    <section className="appearance-section"><Stack gap={4}>
      {(['ui','reading','code'] as const).map(role=><div key={role} className="grid grid-cols-2 gap-3">
        <Select label={t(`appearance.font.${role}`)} disabled={blocked} value={draft.typography[`${role}FontId`]} options={FONT_IDS[role].map(value=>({value,label:t(`appearance.fontId.${value}`)}))} onChange={value=>change(next=>{(next.typography as unknown as Record<string,unknown>)[`${role}FontId`]=value;})}/>
        <Select label={t(`appearance.size.${role}`)} disabled={blocked} value={String(draft.typography[`${role}Size`])} options={FONT_SIZES[role].map(value=>({value:String(value),label:`${value}px`}))} onChange={value=>change(next=>{next.typography[`${role}Size`]=Number(value);})}/>
      </div>)}
      <Select label={t('appearance.lineHeight')} disabled={blocked} value={String(draft.typography.lineHeight)} options={[1.5,1.75,2].map(value=>({value:String(value),label:String(value)}))} onChange={value=>change(next=>{next.typography.lineHeight=Number(value) as 1.5|1.75|2;})}/>
      <Select label={t('appearance.density')} disabled={blocked} value={String(draft.density)} options={DENSITIES.map(value=>({value:String(value),label:`${value}%`}))} onChange={value=>change(next=>{next.density=Number(value) as Appearance['density'];})}/>
      <Select label={t('appearance.motion')} disabled={blocked} value={draft.effects.reducedMotion} options={['system','reduce'].map(value=>({value,label:t(`appearance.motion.${value}`)}))} onChange={value=>change(next=>{next.effects.reducedMotion=value as 'system'|'reduce';})}/>
      <Select label={t('appearance.sidebar')} disabled={blocked} value={draft.effects.sidebarTreatment} options={['solid','soft'].map(value=>({value,label:t(`appearance.sidebar.${value}`)}))} onChange={value=>change(next=>{next.effects.sidebarTreatment=value as 'solid'|'soft';})}/>
    </Stack></section>
    <section className="appearance-section" aria-label={t('appearance.preview')}><h2 className="settings-section-title">{t('appearance.preview')}</h2><div className="flex flex-wrap gap-3"><Button>{t('appearance.primaryButton')}</Button><Button variant="secondary">{t('appearance.secondaryButton')}</Button></div><p className="type-reading">{t('appearance.previewReading')}</p><pre className="type-code overflow-x-auto">{t('appearance.previewCode')}</pre></section>
    {state.error?<p role="alert" className="text-sm text-status-error-text">{t(state.error==='revision_conflict'?'appearance.error.conflict':`appearance.error.${['invalid','failed','unconfirmed','corrupt','unsupported_schema'].includes(state.phase)?state.phase:'failed'}`)}</p>:null}
    <div className="settings-actions">
      <Button variant="secondary" disabled={busy} onClick={()=>controller.cancel()}>{t('appearance.cancelPreview')}</Button>
      <Button loading={busy} disabled={blocked||!['drafting','failed'].includes(state.phase)} onClick={()=>void controller.apply()}>{t('appearance.apply')}</Button>
      <Button variant="secondary" disabled={blocked} onClick={()=>controller.preview(structuredClone(DEFAULTS))}>{t('appearance.restoreAll')}</Button>
      <Button variant="secondary" disabled={blocked} onClick={()=>change(next=>{next.variants[mode]=structuredClone(DEFAULTS.variants[mode]);})}>{t('appearance.restoreVariant')}</Button>
      {state.phase==='unconfirmed'||(!state.confirmed&&state.phase==='failed')?<Button onClick={()=>void controller.verify()}>{t('appearance.verify')}</Button>:null}
      {state.phase==='corrupt'?<Button variant="danger" onClick={()=>void controller.recover()}>{t('appearance.recover')}</Button>:null}
    </div>
  </div>;
}
