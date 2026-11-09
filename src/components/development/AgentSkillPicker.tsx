import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Popover } from '@dgos/ui';
import { Sparkles } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession, type Engine } from '../../stores/agentRunnerStore';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { agentSkillsKey, type CapturedAgentSkill } from '../../stores/agentSkills';

type Skill = { name: string; path: string | null; enabled: boolean | null; classification: string };
type Observation = { session_id: string; generation: string; engine: Engine; domain: string; incomplete: boolean; skills: Skill[] };
export function AgentSkillPicker({ projectId, session, engine = session?.engine, disabled, active }: { projectId: string | null; session?: AgentSession; engine?: Engine; disabled: boolean; active: boolean }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const key = session ? agentSkillsKey(projectId, session) : '';
  const chosen = useDevelopmentDrafts(state => state.entries[key]?.value as CapturedAgentSkill[] | undefined) ?? [];
  const owner = JSON.stringify([epoch, generation, projectId, session?.id, session?.generation, engine, active, pending]);
  const latest = useRef(owner); latest.current = owner;
  const [panel, setPanel] = useState<string | null>(null);
  const [read, setRead] = useState<{ owner: string; skills: Skill[]; error: boolean } | null>(null);
  const [loading, setLoading] = useState<string | null>(null);
  const sequence = useRef(0);
  useEffect(() => () => { sequence.current += 1; }, []);
  useEffect(() => { sequence.current += 1; setPanel(null); setRead(null); setLoading(null); }, [owner]);
  const writable = active && !pending && !disabled && session?.status === 'idle' && !session.active_run && !session.pending.length;
  function current() {
    const runner = useAgentRunner.getState(), live = runner.session;
    return latest.current === owner && epoch === useDevelopment.getState().epoch && generation === contextGeneration() && active && !pending && session && runner.selected === session.id && live?.id === session.id && live.generation === session.generation && !runner.busy && !runner.acting && live.status === 'idle' && !live.active_run && !live.pending.length;
  }
  async function observe() {
    if (!current() || !writable || engine !== 'codex') return;
    const token = ++sequence.current;
    setRead(null); setLoading(owner);
    try {
      const value = await invoke<Observation>('observe_native_session', { input: { session_id: session!.id, generation: session!.generation, domain: 'skills' } });
      if (!current() || sequence.current !== token) return;
      if (value.session_id !== session!.id || value.generation !== session!.generation || value.engine !== 'codex' || value.domain !== 'skills' || value.incomplete) { setRead({ owner, skills: [], error: true }); return; }
      setRead({ owner, skills: value.skills.filter(skill => skill.classification === 'skill' && skill.enabled === true && skill.path && value.skills.filter(other => other.name === skill.name && other.path === skill.path).length === 1), error: false });
    } catch { if (current() && sequence.current === token) setRead({ owner, skills: [], error: true }); }
    finally { if (latest.current === owner && sequence.current === token) setLoading(null); }
  }
  function toggle(skill: Skill) {
    if (!current() || !writable || !skill.path || read?.owner !== owner || !read.skills.includes(skill)) return;
    useDevelopmentDrafts.setState(state => {
      const existing = (state.entries[key]?.value as CapturedAgentSkill[] | undefined) ?? [];
      const selected = existing.some(item => item.skill.name === skill.name && item.skill.path === skill.path);
      if (!selected && existing.length >= 8) return state;
      const value = selected ? existing.filter(item => item.skill.name !== skill.name || item.skill.path !== skill.path) : [...existing, { id: crypto.randomUUID(), skill: { name: skill.name, path: skill.path! } }];
      const entries = { ...state.entries };
      if (value.length) entries[key] = { kind: 'conversation', value, baseline: [] };
      else delete entries[key];
      return { entries };
    });
  }
  return <Popover label={t('agentSkills.title')} side="top" align="start" open={active && panel === owner} onOpenChange={open => { setPanel(open ? owner : null); if (open) void observe(); }} trigger={<Button size="xs" variant="ghost" disabled={!active || pending} aria-label={t('agentSkills.title')}><Sparkles size={14} aria-hidden="true" /><span>{t('agentSkills.title')}{chosen.length ? ` (${chosen.length})` : ''}</span></Button>} content={<div className="agent-control-popover"><h3>{t('agentSkills.title')}</h3><p>{t(engine === 'claude' ? 'agentSkills.unsupported' : !session ? 'agentSkills.beforeStart' : 'agentSkills.boundary')}</p>{session && engine === 'codex' ? <><Button size="xs" variant="secondary" disabled={!writable || loading === owner} onClick={() => void observe()}>{t('agentSkills.refresh')}</Button>{loading === owner ? <p role="status">{t('agentSkills.loading')}</p> : read?.owner === owner ? read.error ? <p role="alert">{t('agentSkills.unavailable')}</p> : !read.skills.length ? <p>{t('agentSkills.empty')}</p> : <div className="agent-control-options">{read.skills.map(skill => <label key={JSON.stringify([skill.name, skill.path])} className="dev-checkbox"><input type="checkbox" checked={chosen.some(item => item.skill.name === skill.name && item.skill.path === skill.path)} disabled={!writable || chosen.length >= 8 && !chosen.some(item => item.skill.name === skill.name && item.skill.path === skill.path)} onChange={() => toggle(skill)} /><span>{skill.name}<small className="block break-all text-ink-secondary">{skill.path}</small></span></label>)}</div> : null}{chosen.length ? <div><p>{t('agentSkills.selected')}</p>{chosen.map(item => <Button key={item.id} size="xs" variant="ghost" disabled={!writable} onClick={() => { if (current() && writable) useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', baseline: [], value: ((state.entries[key]?.value as CapturedAgentSkill[] | undefined) ?? []).filter(old => old.id !== item.id) } } })); }}>{item.skill.name} · {t('agentSkills.remove')}</Button>)}</div> : null}</> : null}</div>} />;
}
