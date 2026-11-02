import { useEffect, useState, type FormEvent } from 'react';
import { Button } from '@dgos/ui/components/Button';
import type { ChatSession, LanApi, LanTargets } from './api';
import { SELECT } from './classes';
import { chatErrorText } from './messages';
import { useRemoteLocale } from './i18n';

export interface NewSessionFormProps {
  api: LanApi;
  onCreated: (session: ChatSession) => void;
}

/** Picks provider, model and skill. The target is fixed once the session exists: there is no way to change it. */
export function NewSessionForm({ api, onCreated }: NewSessionFormProps) {
  const { t } = useRemoteLocale();
  const [targets, setTargets] = useState<LanTargets | null>(null);
  const [providerId, setProviderId] = useState('');
  const [modelId, setModelId] = useState('');
  const [skillId, setSkillId] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api.getTargets().then(
      (loaded) => {
        if (cancelled) return;
        setTargets(loaded);
        const first = loaded.providers.find((p) => p.models.length > 0);
        if (first) {
          setProviderId(first.id);
          setModelId(first.models[0].id);
        }
      },
      (failure: unknown) => {
        if (!cancelled) setError(chatErrorText(failure));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [api]);

  const providers = targets?.providers.filter((p) => p.models.length > 0) ?? [];
  const provider = providers.find((p) => p.id === providerId);

  function onProviderChange(id: string) {
    setProviderId(id);
    const next = providers.find((p) => p.id === id);
    setModelId(next ? next.models[0].id : '');
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !provider || !modelId) return;
    setBusy(true);
    setError('');
    try {
      const session = await api.createChatSession({
        provider_id: provider.id,
        model_id: modelId,
        skill_id: skillId || null,
        no_skill: !skillId,
      });
      onCreated(session);
    } catch (failure) {
      setError(chatErrorText(failure));
      setBusy(false);
    }
  }

  return (
    <form className="flex flex-col gap-4 p-4" onSubmit={(event) => void submit(event)}>
      {targets && providers.length === 0 ? (
        <p role="status" className="text-muted-foreground">
          {t('remote.noModel')}
        </p>
      ) : null}
      <label className="flex flex-col gap-1 text-sm font-medium">
        {t('remote.provider')}
        <select
          className={SELECT}
          value={providerId}
          onChange={(event) => onProviderChange(event.target.value)}
          disabled={!targets || busy}
        >
          {providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1 text-sm font-medium">
        {t('remote.model')}
        <select
          className={SELECT}
          value={modelId}
          onChange={(event) => setModelId(event.target.value)}
          disabled={!provider || busy}
        >
          {(provider?.models ?? []).map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1 text-sm font-medium">
        {t('remote.skill')}
        <select className={SELECT} value={skillId} onChange={(event) => setSkillId(event.target.value)} disabled={!targets || busy}>
          <option value="">{t('remote.noSkill')}</option>
          {(targets?.skills ?? []).map((s) => (
            <option key={s.id} value={s.id}>
              {s.name} ({s.version})
            </option>
          ))}
        </select>
      </label>
      <p className="text-sm text-muted-foreground">{t('remote.fixedTarget')}</p>
      {error ? (
        <p role="alert" className="text-sm text-error-text">
          {error}
        </p>
      ) : null}
      <Button type="submit" size="lg" loading={busy} disabled={!provider || !modelId}>
        {t('remote.startChat')}
      </Button>
    </form>
  );
}
