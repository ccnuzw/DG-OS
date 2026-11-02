import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { Button, Select, cn } from '@dgos/ui';
import type { Model, Provider } from '../../stores/providerStore';
import type { Skill } from '../../stores/skillsStore';
import { useChatStore, type ChatSession } from '../../stores/chatStore';
import { FOCUS } from './ChatNotice';
import { fetchModels, fetchProviders, fetchSkills } from './catalog';

// Provider/model/skill of the active session (AC02, AC21). Radix Select items cannot carry an empty value, so
// "nothing chosen" travels as sentinels. A skill is either one chosen version or an explicit "no skill".
const UNSET = '__unset__';
const NO_SKILL = '__no_skill__';
const LINK = cn('text-primary underline', FOCUS);
export const SKILL_SELECT_ID = 'chat-skill-select';

const skillValue = (session: ChatSession): string =>
  session.skill_id ?? (session.no_skill_confirmed ? NO_SKILL : UNSET);

export default function TargetBar({ session }: { session: ChatSession }) {
  usePageLocale();
  const updateTarget = useChatStore(state => state.updateTarget);
  const generating = useChatStore(state => state.generation !== null);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [skills, setSkills] = useState<Skill[]>([]);
  const [models, setModels] = useState<Model[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [providerId, setProviderId] = useState(session.provider_id ?? UNSET);
  const [modelId, setModelId] = useState(session.model_id ?? UNSET);
  const [skill, setSkill] = useState(skillValue(session));
  const [status, setStatus] = useState<{ tone: 'ok' | 'error'; text: string } | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let active = true;
    Promise.all([fetchProviders(), fetchSkills()])
      .then(([p, s]) => { if (active) { setProviders(p); setSkills(s); } })
      .catch(() => { if (active) setStatus({ tone: 'error', text: t('chat.targetsFailed') }); })
      .finally(() => { if (active) setLoaded(true); });
    return () => { active = false; };
  }, []);

  useEffect(() => {
    setProviderId(session.provider_id ?? UNSET);
    setModelId(session.model_id ?? UNSET);
    setSkill(skillValue(session));
    setStatus(null);
  }, [session.id, session.provider_id, session.model_id, session.skill_id, session.no_skill_confirmed]);

  useEffect(() => {
    let active = true;
    setModels([]);
    if (providerId === UNSET) return;
    fetchModels(providerId)
      .then(list => { if (active) setModels(list); })
      .catch(() => { if (active) setStatus({ tone: 'error', text: t('chat.modelsFailed') }); });
    return () => { active = false; };
  }, [providerId]);

  const dirty = providerId !== (session.provider_id ?? UNSET) || modelId !== (session.model_id ?? UNSET) || skill !== skillValue(session);
  const ready = providerId !== UNSET && modelId !== UNSET;

  const apply = async () => {
    if (!ready || saving) return;
    setSaving(true);
    setStatus(null);
    const error = await updateTarget(session.id, {
      provider_id: providerId,
      model_id: modelId,
      skill_id: skill === UNSET || skill === NO_SKILL ? null : skill,
      no_skill: skill === NO_SKILL,
    });
    setSaving(false);
    setStatus(error ? { tone: 'error', text: error } : { tone: 'ok', text: t('chat.targetUpdated') });
  };

  const providerOptions = [{ value: UNSET, label: t('chat.selectProvider') }, ...providers.map(p => ({ value: p.id, label: p.name }))];
  const modelOptions = [{ value: UNSET, label: t('chat.selectModel') }, ...models.map(m => ({ value: m.model_id, label: m.model_name }))];
  const skillOptions = [
    { value: UNSET, label: t('chat.noSkillSelected') },
    { value: NO_SKILL, label: t('chat.explicitNoSkill') },
    ...skills.map(s => ({ value: s.skill_id, label: `${s.name} — v${s.version}` })),
  ];
  const locked = generating || saving;

  return (
    <section aria-label={t('chat.target')} className="flex flex-col gap-2 rounded-md border border-border bg-muted p-3">
      {loaded && !providers.length ? (
        <p className="m-0 text-sm text-muted-foreground">{t('chat.noProviderBefore')}<Link to="/providers" className={LINK}>{t('chat.providerPage')}</Link> {t('chat.noProviderAfter')}</p>
      ) : null}
      <div className="flex flex-wrap items-end gap-3">
        <div className="min-w-0 grow basis-40">
          <Select label={t('chat.provider')} options={providerOptions} value={providerId} disabled={locked} className="overflow-hidden"
            onChange={value => { setProviderId(value); setModelId(UNSET); setStatus(null); }} />
        </div>
        <div className="min-w-0 grow basis-40">
          <Select label={t('chat.model')} options={modelOptions} value={modelId} disabled={locked || providerId === UNSET} className="overflow-hidden"
            onChange={value => { setModelId(value); setStatus(null); }} />
        </div>
        <div className="min-w-0 grow basis-40">
          <Select id={SKILL_SELECT_ID} label={t('chat.skill')} options={skillOptions} value={skill} disabled={locked} className="overflow-hidden"
            onChange={value => { setSkill(value); setStatus(null); }} />
        </div>
        <Button variant="secondary" disabled={locked || !ready || !dirty} loading={saving} onClick={() => void apply()}>{t('chat.applyTarget')}</Button>
      </div>
      <p className="m-0 text-xs text-muted-foreground">{t('chat.targetHelp')}</p>
      {status ? <p role={status.tone === 'error' ? 'alert' : 'status'} className={cn('m-0 text-xs', status.tone === 'error' ? 'text-error-text' : 'text-success-text')}>{status.text}</p> : null}
    </section>
  );
}
