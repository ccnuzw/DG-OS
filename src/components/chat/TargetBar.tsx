import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { Button, Popover, Select, Stack, cn } from '@dgos/ui';
import type { Model, Provider } from '../../stores/providerStore';
import type { Skill } from '../../stores/skillsStore';
import { useChatStore, type ChatSession, type ChatTargetInput } from '../../stores/chatStore';
import { FOCUS } from './ChatNotice';
import { fetchModels, fetchProviders, fetchSkills } from './catalog';
import { GenerationControls, OrderedSkills } from './GenerationControls';
const UNSET = '__unset__';
const LINK = cn('text-primary underline', FOCUS);
export const SKILL_SELECT_ID = 'chat-skill-select';
const snapshot = (session: ChatSession): ChatTargetInput => ({ provider_id: session.provider_id, model_id: session.model_id, skill_ids: session.skills.map(s => s.skill_id), no_skill: session.no_skill_confirmed, generation: session.generation });
export default function TargetBar({ session, skillRequest = 0 }: { session: ChatSession; skillRequest?: number }) {
  usePageLocale();
  const updateTarget = useChatStore(state => state.updateTarget);
  const setSessionTargetDraft = useChatStore(state => state.setSessionTargetDraft);
  const targetDraft = useChatStore(state => state.sessionTargetDrafts[session.id] ?? null);
  const generating = useChatStore(state => state.generation !== null);
  const [target, setTarget] = useState<ChatTargetInput>(() => targetDraft ?? snapshot(session));
  const [providers, setProviders] = useState<Provider[]>([]);
  const [skills, setSkills] = useState<Skill[]>([]);
  const [models, setModels] = useState<Model[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [status, setStatus] = useState<{ tone: 'ok' | 'error'; text: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const [open, setOpen] = useState(false);
  useEffect(() => { if (skillRequest) setOpen(true); }, [skillRequest]);
  useEffect(() => {
    if (!open || !skillRequest) return;
    const frame = requestAnimationFrame(() => document.getElementById(SKILL_SELECT_ID)?.focus());
    return () => cancelAnimationFrame(frame);
  }, [open, skillRequest]);
  useEffect(() => {
    let active = true;
    Promise.all([fetchProviders(), fetchSkills()]).then(([p, s]) => { if (active) { setProviders(p); setSkills(s); } }).catch(() => { if (active) setStatus({ tone: 'error', text: t('chat.targetsFailed') }); }).finally(() => { if (active) setLoaded(true); });
    return () => { active = false; };
  }, [open]);
  useEffect(() => {
    setTarget(useChatStore.getState().sessionTargetDrafts[session.id] ?? snapshot(session));
    setStatus(null);
  }, [session]);
  useEffect(() => {
    let active = true;
    setModels([]);
    if (!target.provider_id) return;
    fetchModels(target.provider_id).then(list => { if (active) setModels(list); }).catch(() => { if (active) setStatus({ tone: 'error', text: t('chat.modelsFailed') }); });
    return () => { active = false; };
  }, [target.provider_id, open]);
  const patch = (values: Partial<ChatTargetInput>) => { const next = { ...target, ...values }; setTarget(next); setStatus(null); setSessionTargetDraft(session.id, next); };
  const dirty = JSON.stringify(target) !== JSON.stringify(snapshot(session));
  const ready = !!target.provider_id && !!target.model_id;
  const locked = generating || saving;
  const apply = async () => {
    if (!ready || locked) return;
    setSaving(true); setStatus(null);
    const error = await updateTarget(session.id, target);
    setSaving(false);
    if (!error) { setSessionTargetDraft(session.id, null); setOpen(false); }
    setStatus(error ? { tone: 'error', text: error } : { tone: 'ok', text: t('chat.targetUpdated') });
  };
  return <Popover label={t('chat.target')} side="top" align="start" open={open} onOpenChange={setOpen} trigger={<Button aria-label={t('chat.target')} variant="ghost" size="sm">{session.model_id ?? t('chat.selectModel')} · {session.skills.length} {t('chat.skill')}</Button>} content={<section aria-label={t('chat.target')} className="chat-target">
    {loaded && !providers.length && <p className="m-0 text-sm text-muted-foreground">{t('chat.noProviderBefore')}<Link to="/settings/providers" className={LINK}>{t('chat.providerPage')}</Link> {t('chat.noProviderAfter')}</p>}
    <Stack gap={3}>
      <Select label={t('chat.provider')} options={[{ value: UNSET, label: t('chat.selectProvider') }, ...providers.map(p => ({ value: p.id, label: p.name }))]} value={target.provider_id ?? UNSET} disabled={locked} onChange={v => patch({ provider_id: v === UNSET ? null : v, model_id: null })} />
      <Select label={t('chat.model')} options={[{ value: UNSET, label: t('chat.selectModel') }, ...models.map(m => ({ value: m.model_id, label: m.model_name }))]} value={target.model_id ?? UNSET} disabled={locked || !target.provider_id} onChange={v => patch({ model_id: v === UNSET ? null : v })} />
      <OrderedSkills id={SKILL_SELECT_ID} skills={skills} selected={target.skill_ids} onChange={skill_ids => patch({ skill_ids, no_skill: false })} without={target.no_skill} onWithoutChange={no_skill => patch({ no_skill })} disabled={locked} />
      <GenerationControls value={target.generation} onChange={generation => patch({ generation })} model={models.find(m => m.model_id === target.model_id)} disabled={locked} />
      <Button variant="secondary" disabled={locked || !ready || !dirty} loading={saving} onClick={() => void apply()}>{t('chat.applyTarget')}</Button>
    </Stack>
    {status && <p role={status.tone === 'error' ? 'alert' : 'status'} className={cn('m-0 text-xs', status.tone === 'error' ? 'text-error-text' : 'text-success-text')}>{status.text}</p>}
  </section>} />;
}
