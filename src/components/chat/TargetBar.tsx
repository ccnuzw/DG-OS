import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { ArrowDown, ArrowUp, Brain, Puzzle } from 'lucide-react';
import { Button, Checkbox, Icon, Input, Popover, cn } from '@dgos/ui';
import type { Model, Provider } from '../../stores/providerStore';
import type { Skill } from '../../stores/skillsStore';
import { useChatStore, type ChatSession, type ChatTargetInput } from '../../stores/chatStore';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { fetchModels, fetchProviders, fetchSkills } from './catalog';
import { GenerationControls, ReasoningControls } from './GenerationControls';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { useDevelopment } from '../../stores/developmentStore';

const LINK = cn('chat-picker-manage text-accent-default', FOCUS_RING);
export const SKILL_SELECT_ID = 'chat-skill-select';
type Picker = 'skills' | 'settings' | 'model' | 'effort';
interface TargetSlots { skills: ReactNode; settings: ReactNode; model: ReactNode; effort: ReactNode; context: ReactNode; feedback: ReactNode }
const snapshot = (session: ChatSession): ChatTargetInput => ({ provider_id: session.provider_id, model_id: session.model_id, skill_ids: session.skills.map(s => s.skill_id), no_skill: session.no_skill_confirmed, generation: session.generation });

export default function TargetBar({ session, skillRequest = 0, children }: { session: ChatSession; skillRequest?: number; children: (slots: TargetSlots) => ReactNode }) {
  usePageLocale();
  const surfaceActive = useSurfaceActive();
  const account = useDevelopment(state => state.epoch);
  const updateTarget = useChatStore(state => state.updateTarget);
  const setSessionTargetDraft = useChatStore(state => state.setSessionTargetDraft);
  const targetDraft = useChatStore(state => state.sessionTargetDrafts[session.id] ?? null);
  const generating = useChatStore(state => state.generation !== null);
  const stale = useChatStore(state => state.sessionViews[session.id]?.stale ?? (state.activeSessionId === session.id && state.activeStale));
  const [target, setTarget] = useState<ChatTargetInput>(() => targetDraft ?? snapshot(session));
  const [menu, setMenu] = useState<Picker | null>(null);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [skills, setSkills] = useState<Skill[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [providerError, setProviderError] = useState(false);
  const [skillError, setSkillError] = useState(false);
  const [catalogRevision, setCatalogRevision] = useState(0);
  const [browseProvider, setBrowseProvider] = useState<string | null>(target.provider_id);
  const [modelQuery, setModelQuery] = useState('');
  const [skillQuery, setSkillQuery] = useState('');
  const [skillCategory, setSkillCategory] = useState<string | null>(null);
  const [modelCatalog, setModelCatalog] = useState<Record<string, Model[]>>({});
  const [modelState, setModelState] = useState<{ provider: string | null; loading: boolean; failed: boolean }>({ provider: null, loading: false, failed: false });
  const [status, setStatus] = useState<{ tone: 'ok' | 'error'; text: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const sessionId = useRef(session.id);
  sessionId.current = session.id;
  const modelProvider = menu === 'model' ? browseProvider : target.provider_id;

  useEffect(() => {
    if (!surfaceActive) return;
    let active = true;
    setCatalogLoading(true);
    setProviderError(false);
    setSkillError(false);
    Promise.allSettled([fetchProviders(), fetchSkills()]).then(([providerResult, skillResult]) => {
      if (!active) return;
      if (providerResult.status === 'fulfilled') setProviders(providerResult.value);
      else setProviderError(true);
      if (skillResult.status === 'fulfilled') setSkills(skillResult.value);
      else setSkillError(true);
      setCatalogLoading(false);
    });
    return () => { active = false; };
  }, [catalogRevision, surfaceActive, account, session.id]);
  useEffect(() => {
    const invalidate = () => setCatalogRevision(value => value + 1);
    window.addEventListener('dgos:catalog-invalidated', invalidate);
    return () => window.removeEventListener('dgos:catalog-invalidated', invalidate);
  }, []);
  useEffect(() => { if (menu) setCatalogRevision(value => value + 1); }, [menu]);
  useEffect(() => {
    setTarget(useChatStore.getState().sessionTargetDrafts[session.id] ?? snapshot(session));
  }, [session, targetDraft]);
  useEffect(() => { setStatus(null); setMenu(null); setBrowseProvider(session.provider_id); }, [session.id]);
  useEffect(() => { if (skillRequest) setMenu('skills'); }, [skillRequest]);
  useEffect(() => {
    if (menu !== 'skills' || !skillRequest) return;
    const frame = requestAnimationFrame(() => document.getElementById(SKILL_SELECT_ID)?.focus());
    return () => cancelAnimationFrame(frame);
  }, [menu, skillRequest]);
  useEffect(() => {
    if (!surfaceActive || !modelProvider) return;
    let active = true;
    setModelState({ provider: modelProvider, loading: true, failed: false });
    fetchModels(modelProvider).then(models => {
      if (!active) return;
      setModelCatalog(current => ({ ...current, [modelProvider]: models }));
      setModelState({ provider: modelProvider, loading: false, failed: false });
    }).catch(() => { if (active) setModelState({ provider: modelProvider, loading: false, failed: true }); });
    return () => { active = false; };
  }, [modelProvider, catalogRevision, surfaceActive, account, session.id]);

  const locked = generating || saving || stale;
  const dirty = JSON.stringify(target) !== JSON.stringify(snapshot(session));
  const ready = !!target.provider_id && !!target.model_id;
  const patch = (values: Partial<ChatTargetInput>) => {
    if (locked || savingRef.current) return;
    const next = { ...target, ...values };
    setTarget(next);
    setStatus(null);
    setSessionTargetDraft(session.id, next);
  };
  const openPicker = (picker: Picker, open: boolean) => {
    if (!open) { setMenu(current => current === picker ? null : current); return; }
    if (picker === 'model') { setBrowseProvider(target.provider_id ?? providers[0]?.id ?? null); setModelQuery(''); }
    setMenu(picker);
  };
  const apply = async () => {
    if (!ready || !dirty || locked || savingRef.current) return;
    const applyingSession = session.id;
    const applyingTarget = target;
    savingRef.current = true;
    setSaving(true);
    setStatus(null);
    const error = await updateTarget(applyingSession, applyingTarget);
    savingRef.current = false;
    setSaving(false);
    if (!error) setSessionTargetDraft(applyingSession, null);
    if (sessionId.current !== applyingSession) return;
    if (!error) setMenu(null);
    setStatus(error ? { tone: 'error', text: error } : { tone: 'ok', text: t('chat.targetUpdated') });
  };
  const moveSkill = (index: number, offset: number) => {
    const selected = [...target.skill_ids];
    [selected[index], selected[index + offset]] = [selected[index + offset], selected[index]];
    patch({ skill_ids: selected, no_skill: false });
  };
  const removeSkill = (id: string) => { if (locked) return; patch({ skill_ids: target.skill_ids.filter(item => item !== id), no_skill: false }); setMenu('skills'); };
  const selectedProvider = providers.find(provider => provider.id === target.provider_id);
  const selectedModel = target.provider_id ? modelCatalog[target.provider_id]?.find(model => model.model_id === target.model_id) : undefined;
  const browseModels = browseProvider ? modelCatalog[browseProvider] ?? [] : [];
  const modelsLoading = !!browseProvider && (modelState.provider !== browseProvider || modelState.loading);
  const modelsFailed = modelState.provider === browseProvider && modelState.failed;
  const filteredModels = browseModels.filter(model => `${model.model_name} ${model.model_id}`.toLocaleLowerCase().includes(modelQuery.trim().toLocaleLowerCase()));
  const skillCategories = [...new Set(skills.map(skill => skill.manifest?.category?.trim()).filter((category): category is string => !!category))];
  const activeSkillCategory = skillCategory && skillCategories.includes(skillCategory) ? skillCategory : null;
  const filteredSkills = skills.filter(skill => (!activeSkillCategory || skill.manifest?.category?.trim() === activeSkillCategory) && `${skill.name} ${skill.skill_id} ${skill.description ?? ''}`.toLocaleLowerCase().includes(skillQuery.trim().toLocaleLowerCase()));
  const skillName = (id: string) => skills.find(skill => skill.skill_id === id)?.name ?? id;
  const effort = target.generation.reasoning?.effort;
  const effortLabel = t(`generation.${effort === 'low' ? 'low' : effort === 'medium' ? 'effortMedium' : effort === 'high' ? 'high' : 'default'}`);
  const footer = <footer className="chat-picker-footer">
    <span className={cn('chat-picker-description', dirty && 'chat-target-pending')}>{t(dirty ? 'chat.targetDraftHelp' : 'chat.targetHelp')}</span>
    <Button variant="secondary" disabled={locked || !ready || !dirty} loading={saving} onClick={() => void apply()}>{t('chat.applyTarget')}</Button>
    {status?.tone === 'error' && <p role="alert" className="chat-target-feedback text-status-error-text">{status.text}</p>}
  </footer>;
  const header = (picker: Picker, label: string) => <header className="chat-composer-popover-header"><strong>{label}</strong><Button variant="ghost" size="xxs" iconOnly aria-label={t('chat.closePicker')} onClick={() => openPicker(picker, false)}><Icon name="close" size="sm" /></Button></header>;
  const retry = <Button variant="link" size="sm" onClick={() => setCatalogRevision(value => value + 1)}>{t('chat.retry')}</Button>;
  const skillsContent = <section className="chat-target-popover chat-skills-popover">
    {header('skills', t('chat.selectSkills'))}
    <Input id={SKILL_SELECT_ID} type="search" label={t('chat.searchSkills')} placeholder={t('chat.searchSkillsPlaceholder')} value={skillQuery} onChange={event => setSkillQuery(event.target.value)} />
    <nav className="chat-skill-categories" aria-label={t('chat.skillCategories')}><Button variant="ghost" size="xxs" aria-pressed={!activeSkillCategory} onClick={() => setSkillCategory(null)}>{t('chat.allSkillCategories')}</Button>{skillCategories.map(category => <Button key={category} variant="ghost" size="xxs" aria-pressed={activeSkillCategory === category} onClick={() => setSkillCategory(category)}>{category}</Button>)}</nav>
    <div className="chat-picker-results" aria-label={t('chat.selectSkills')} aria-busy={catalogLoading}>
      {catalogLoading ? <p role="status" className="chat-picker-empty">{t('chat.loadingSkills')}</p> : skillError ? <div role="alert" className="chat-picker-empty">{t('chat.skillsFailed')} {retry}</div> : !skills.length ? <p className="chat-picker-empty">{t('chat.noAvailableSkills')}</p> : !filteredSkills.length ? <p className="chat-picker-empty">{t('chat.noMatchingSkills')}</p> : filteredSkills.map(skill => <Button key={skill.skill_id} variant="ghost" className="chat-skill-option" aria-label={skill.name} aria-pressed={target.skill_ids.includes(skill.skill_id)} disabled={locked} onClick={() => patch({ skill_ids: target.skill_ids.includes(skill.skill_id) ? target.skill_ids.filter(id => id !== skill.skill_id) : [...target.skill_ids, skill.skill_id], no_skill: false })}>
        <span className="chat-skill-icon"><Puzzle size={16} aria-hidden="true" /></span><span className="chat-picker-option-copy"><strong>{skill.name}</strong><small>{skill.description || t('chat.noDescription')}</small></span><span className="chat-skill-version">{skill.version}</span>{target.skill_ids.includes(skill.skill_id) && <Icon name="check" size="sm" />}
      </Button>)}
    </div>
    {target.skill_ids.length > 0 && <section className="chat-selected-skills" aria-label={t('chat.selectedSkillOrder')}><strong className="chat-picker-caption">{t('chat.selectedSkillOrder')}</strong><ol>{target.skill_ids.map((id, index) => <li key={id} className="chat-selected-skill-row"><span>{skillName(id)}</span><div className="chat-selected-skill-actions"><Button size="xxs" variant="ghost" iconOnly disabled={locked || index === 0} aria-label={`${t('generation.moveUp')} ${skillName(id)}`} onClick={() => moveSkill(index, -1)}><ArrowUp size={14} aria-hidden="true" /></Button><Button size="xxs" variant="ghost" iconOnly disabled={locked || index === target.skill_ids.length - 1} aria-label={`${t('generation.moveDown')} ${skillName(id)}`} onClick={() => moveSkill(index, 1)}><ArrowDown size={14} aria-hidden="true" /></Button><Button size="xxs" variant="ghost" iconOnly disabled={locked} aria-label={t('chat.removeSkill', { name: skillName(id) })} onClick={() => removeSkill(id)}><Icon name="close" size="sm" /></Button></div></li>)}</ol></section>}
    {!target.skill_ids.length && <Checkbox label={t('chat.explicitNoSkill')} checked={target.no_skill} disabled={locked} onChange={no_skill => patch({ no_skill })} />}
    {target.skill_ids.length > 1 && <p className="chat-picker-description">{t('generation.combinationNotice')}</p>}
    <Link to="/skills" state={{ capabilityEntry: { runtime: 'dgos' } }} className={LINK}>{t('chat.manageSkills')}</Link>
    {footer}
  </section>;
  const modelContent = <section className="chat-target-popover chat-model-popover">
    {header('model', t('chat.selectProviderModel'))}
    <div className="chat-model-picker-body"><nav className="chat-provider-list" aria-label={t('chat.provider')} aria-busy={catalogLoading}>
      {catalogLoading ? <p role="status" className="chat-picker-empty">{t('chat.loadingProviders')}</p> : providerError ? <div role="alert" className="chat-picker-empty">{t('chat.providersFailed')} {retry}</div> : !providers.length ? <p className="chat-picker-empty">{t('chat.noProviderBefore')}<Link to="/settings/providers" className={LINK}>{t('chat.providerPage')}</Link>{t('chat.noProviderAfter')}</p> : providers.map(provider => <Button key={provider.id} variant="ghost" className="chat-provider-option" aria-label={provider.name} aria-pressed={browseProvider === provider.id} disabled={locked} onClick={() => { setBrowseProvider(provider.id); setModelQuery(''); }}><span className="chat-provider-mark" aria-hidden="true">{provider.name.slice(0, 1)}</span><span>{provider.name}</span>{browseProvider === provider.id && <Icon name="chevron-right" size="sm" />}</Button>)}
      <Link to="/settings/providers" className={LINK}><Icon name="settings" size="sm" />{t('chat.manageProviders')}</Link>
    </nav><section className="chat-model-list" aria-label={t('chat.model')}><Input type="search" label={t('chat.searchModels')} placeholder={t('chat.searchModelsPlaceholder')} value={modelQuery} onChange={event => setModelQuery(event.target.value)} /><p className="chat-picker-caption">{providers.find(provider => provider.id === browseProvider)?.name ?? t('chat.selectProvider')}</p><div className="chat-picker-results" aria-busy={modelsLoading}>
      {!browseProvider ? <p className="chat-picker-empty">{t('chat.selectProvider')}</p> : modelsLoading ? <p role="status" className="chat-picker-empty">{t('chat.loadingModels')}</p> : modelsFailed ? <div role="alert" className="chat-picker-empty">{t('chat.modelsFailed')} {retry}</div> : !browseModels.length ? <p className="chat-picker-empty">{t('chat.noAvailableModels')}</p> : !filteredModels.length ? <p className="chat-picker-empty">{t('chat.noMatchingModels')}</p> : filteredModels.map(model => <Button key={model.id} variant="ghost" className="chat-model-option" aria-label={model.model_name} aria-pressed={target.provider_id === browseProvider && target.model_id === model.model_id} disabled={locked} onClick={() => patch({ provider_id: browseProvider, model_id: model.model_id })}><span className="chat-picker-option-copy"><strong>{model.model_name}</strong><small>{model.model_id}</small></span>{model.context_window ? <span className="chat-model-context">{t('chat.modelContext', { count: model.context_window })}</span> : null}{target.provider_id === browseProvider && target.model_id === model.model_id && <Icon name="check" size="sm" />}</Button>)}
    </div></section></div>
    {footer}
  </section>;
  const settingsContent = <section className="chat-target-popover chat-settings-popover">{header('settings', t('chat.sessionSettings'))}<GenerationControls value={target.generation} onChange={generation => patch({ generation })} model={selectedModel} disabled={locked} />{footer}</section>;
  const effortContent = <section className="chat-target-popover chat-effort-popover">{header('effort', t('chat.reasoningSettings'))}<ReasoningControls value={target.generation} onChange={generation => patch({ generation })} disabled={locked} /><p className="chat-picker-description">{t('generation.reasoningCapability', { state: t(`generation.${selectedModel?.capabilities?.reasoning ?? 'unknown'}`) })}</p>{footer}</section>;

  return children({
    skills: <Popover label={t('chat.selectSkills')} side="top" align="start" open={menu === 'skills'} onOpenChange={open => openPicker('skills', open)} trigger={<Button aria-label={t('chat.selectSkills')} variant="ghost" size="sm" className="chat-composer-skill-button"><Puzzle size={16} aria-hidden="true" /><span>{t('chat.skills')}</span>{target.skill_ids.length > 0 && <span className="chat-skill-count">{target.skill_ids.length}</span>}</Button>} content={skillsContent} />,
    settings: <Popover label={t('chat.sessionSettings')} side="top" align="start" open={menu === 'settings'} onOpenChange={open => openPicker('settings', open)} trigger={<Button aria-label={t('chat.sessionSettings')} title={t('chat.sessionSettings')} variant="ghost" size="sm" iconOnly className="chat-composer-settings-button"><Icon name="settings" /></Button>} content={settingsContent} />,
    model: <Popover label={t('chat.selectProviderModel')} side="top" align="end" open={menu === 'model'} onOpenChange={open => openPicker('model', open)} trigger={<Button aria-label={t('chat.target')} title={t('chat.selectProviderModel')} variant="ghost" size="sm" className="chat-composer-model-button"><span className="chat-provider-mini" aria-hidden="true">{selectedProvider?.name.slice(0, 1) ?? <Icon name="globe" size="sm" />}</span><span className="chat-model-button-label"><small>{selectedProvider?.name ?? target.provider_id ?? t('chat.provider')}</small><span>{selectedModel?.model_name ?? target.model_id ?? t('chat.selectModel')}</span></span><Icon name="chevron-down" size="sm" /></Button>} content={modelContent} />,
    effort: <Popover label={t('chat.reasoningSettings')} side="top" align="end" open={menu === 'effort'} onOpenChange={open => openPicker('effort', open)} trigger={<Button aria-label={t('chat.reasoningEffort', { effort: effortLabel })} title={t('chat.reasoningSettings')} variant="ghost" size="sm" className="chat-composer-effort-button"><Brain size={14} aria-hidden="true" /><span>{effortLabel}</span><Icon name="chevron-down" size="sm" /></Button>} content={effortContent} />,
    context: <>{target.skill_ids.map(id => <Button key={id} variant="ghost" size="xxs" className="chat-context-chip chat-skill-chip" disabled={locked} aria-label={t('chat.removeSkill', { name: skillName(id) })} onClick={() => removeSkill(id)}><Puzzle size={13} aria-hidden="true" /><span>{skillName(id)}</span><Icon name="close" size="sm" /></Button>)}</>,
    feedback: <>{dirty && <span role="status" className="chat-target-pending">{t('chat.targetDraftPending')}</span>}{status?.tone === 'error' && !menu && <span role="alert" className="chat-target-feedback text-status-error-text">{status.text}</span>}<span role="status" className={cn('text-status-success-text', status?.tone !== 'ok' && 'sr-only')}>{status?.tone === 'ok' ? status.text : ''}</span></>,
  });
}
