import { useEffect, useId, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession, type Engine } from '../../stores/agentRunnerStore';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { agentSkillsKey, type CapturedAgentSkill } from '../../stores/agentSkills';
import type { NativeObservedSkill } from '../../types/nativeExtensions';
import { commandReason } from './ClaudeCommandPicker';
import { codexSkillCandidates } from './AgentSkillPicker';
import './agent-slash-picker.css';

type Skill = NativeObservedSkill & { description?: string | null };
export type AgentSlashObservation = { session_id: string; generation: string; engine: Engine; domain: string; incomplete: boolean; skills: Skill[] };
type Candidate = { name: string; skill: Skill; kind: 'skill' | 'command' };
export type AgentSlashPickerProps = {
  session?: AgentSession; engine?: Engine; projectId: string | null;
  /** Read-only provisional selection before a native session is prepared. */
  skillDraftKey?: string;
  value: string; onChange: (value: string, expected: string) => void;
  inputRef: RefObject<HTMLTextAreaElement | null>; active: boolean; disabled?: boolean;
};

// Only a leading plain token or colon-qualified native namespace is an invocation
// intent. Ordinary paths, fenced code, prose and composition are never captured.
export function agentSlashQuery(value: string, caret: number): string | null {
  const prefix = value.slice(0, caret);
  const token = /^\/[\p{L}\p{N}_:-]*$/u.test(prefix)
    || /^\/(?:[\p{L}\p{N}_-]+\/)+[\p{L}\p{N}_-]+:[\p{L}\p{N}_:-]*$/u.test(prefix);
  return token && (!value.slice(caret) || /^\s/.test(value.slice(caret))) ? prefix.slice(1) : null;
}

export function AgentSlashPicker({ session, engine = session?.engine, projectId, skillDraftKey, value, onChange, inputRef, active, disabled = false }: AgentSlashPickerProps) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const key = session ? agentSkillsKey(projectId, session) : skillDraftKey ?? '';
  const chosen = useDevelopmentDrafts(state => state.entries[key]?.value as CapturedAgentSkill[] | undefined) ?? [];
  const owner = JSON.stringify([epoch, generation, projectId, key, session?.id, session?.generation, engine, active, pending, disabled, session?.status, session?.pending, session?.active_run]);
  const writable = active && !pending && !disabled && session?.status === 'idle' && !session.active_run && !session.pending.length;
  const [caret, setCaret] = useState<number | null>(null);
  const [composing, setComposing] = useState(false);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [read, setRead] = useState<{ owner: string; skills: Skill[]; error: boolean; changed?: boolean } | null>(null);
  const [loading, setLoading] = useState(false);
  const [index, setIndex] = useState(0);
  const sequence = useRef(0);
  const pendingCaret = useRef<{ owner: string; value: string; position: number } | null>(null);
  const latest = useRef({ owner, value, onChange }); latest.current = { owner, value, onChange };
  const id = useId();
  const query = !composing && active && !pending && caret !== null ? agentSlashQuery(value, caret) : null;
  const open = query !== null && dismissed !== `${owner}:${value}`;
  const openRef = useRef(open); openRef.current = open;
  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const selection = pendingCaret.current;
    pendingCaret.current = null;
    if (selection?.owner === owner && selection.value === value) input.setSelectionRange(selection.position, selection.position);
    setCaret(input.selectionStart === input.selectionEnd ? input.selectionStart : null);
  });
  function current() {
    const runner = useAgentRunner.getState(), live = runner.session;
    return latest.current.owner === owner && writable && session && epoch === useDevelopment.getState().epoch && generation === contextGeneration() && runner.selected === session.id && live?.generation === session.generation && live.id === session.id && !runner.busy && !runner.acting && live.status === 'idle' && !live.active_run && !live.pending.length;
  }
  function valid(result: AgentSlashObservation) { return result.session_id === session?.id && result.generation === session?.generation && result.engine === engine && result.domain === 'skills' && !result.incomplete; }
  async function observe() {
    if (!current()) return;
    const token = ++sequence.current;
    setLoading(true);
    try {
      const result = await invoke<AgentSlashObservation>('observe_native_session', { input: { session_id: session!.id, generation: session!.generation, domain: 'skills' } });
      if (current() && sequence.current === token && openRef.current) setRead({ owner, skills: valid(result) ? result.skills : [], error: !valid(result) });
    } catch { if (current() && sequence.current === token) setRead({ owner, skills: [], error: true }); }
    finally { if (latest.current.owner === owner && sequence.current === token) setLoading(false); }
  }
  useEffect(() => { sequence.current += 1; setRead(null); setLoading(false); setIndex(0); }, [owner]);
  useEffect(() => { if (open && writable && read?.owner !== owner && !loading) void observe(); }, [open, writable, owner, read?.owner, loading]);
  useEffect(() => { if (!open) { sequence.current += 1; setLoading(false); } }, [open]);
  useEffect(() => () => { sequence.current += 1; }, []);
  const skills = read?.owner === owner && !read.error ? read.skills : [];
  const candidates: Candidate[] = engine === 'codex' ? codexSkillCandidates(skills).map(skill => ({ name: skill.name, skill, kind: 'skill' })) : engine === 'claude' ? skills.flatMap(skill => [...new Set([skill.name, ...(skill.aliases ?? [])])].filter(name => !commandReason(skills, skill, name)).map(name => ({ name, skill, kind: 'command' as const }))) : [];
  const matches = candidates.filter(candidate => `${candidate.name} ${candidate.skill.description ?? ''} ${candidate.skill.path ?? ''} ${candidate.skill.source ?? ''}`.toLocaleLowerCase().includes((query ?? '').toLocaleLowerCase()));
  useEffect(() => setIndex(0), [query, owner]);
  async function choose(candidate: Candidate) {
    if (!current() || loading || !openRef.current || read?.owner !== owner) return;
    const text = latest.current.value;
    const input = inputRef.current;
    if (!input || input.selectionStart !== input.selectionEnd) return;
    const selection = input.selectionStart;
    const selectedQuery = agentSlashQuery(text, selection);
    if (selectedQuery === null) return;
    const token = ++sequence.current;
    setLoading(true);
    try {
      const result = await invoke<AgentSlashObservation>('observe_native_session', { input: { session_id: session!.id, generation: session!.generation, domain: 'skills' } });
      if (!current() || token !== sequence.current || !openRef.current) return;
      if (!valid(result)) { setRead({ owner, skills: [], error: true }); return; }
      const target = result.skills.find(skill => skill.name === candidate.skill.name && skill.path === candidate.skill.path && skill.builtin === candidate.skill.builtin);
      if (!target || JSON.stringify(result.skills) !== JSON.stringify(skills) || candidate.kind === 'command' && commandReason(result.skills, target, candidate.name)) { setRead({ owner, skills: result.skills, error: false, changed: true }); return; }
      // Rechecking never replaces edits made while the host was reading capabilities.
      if (latest.current.value !== text || inputRef.current !== input || input.selectionStart !== selection || input.selectionEnd !== selection) return;
      if (candidate.kind === 'skill') {
        let added = false;
        useDevelopmentDrafts.setState(state => {
          const existing = (state.entries[key]?.value as CapturedAgentSkill[] | undefined) ?? [];
          if (existing.some(item => item.skill.name === target.name && item.skill.path === target.path)) { added = true; return state; }
          if (existing.length >= 8 || !target.path || target.enabled !== true || target.classification !== 'skill') return state;
          added = true;
          return { entries: { ...state.entries, [key]: { kind: 'conversation', baseline: [], value: [...existing, { id: crypto.randomUUID(), skill: { name: target.name, path: target.path } }] } } };
        });
        if (!added) return;
      }
      const replacement = candidate.kind === 'command' ? `/${candidate.name} ` : '';
      const next = replacement + text.slice(selection).replace(/^ /, '');
      setDismissed(`${owner}:${next}`);
      pendingCaret.current = { owner, value: next, position: replacement.length };
      latest.current.onChange(next, text);
      inputRef.current?.focus({ preventScroll: true });
    } catch { if (current() && sequence.current === token) setRead({ owner, skills: [], error: true }); }
    finally { if (latest.current.owner === owner && sequence.current === token) setLoading(false); }
  }
  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const sync = () => setCaret(input.selectionStart === input.selectionEnd ? input.selectionStart : null);
    const start = () => setComposing(true);
    const end = () => { setComposing(false); sync(); };
    const keydown = (event: KeyboardEvent) => {
      if (!open || event.isComposing || event.keyCode === 229 || event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === 'Escape') { event.preventDefault(); setDismissed(`${owner}:${value}`); return; }
      if (!matches.length || !writable || loading) return;
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setIndex(previous => (previous + (event.key === 'ArrowDown' ? 1 : -1) + matches.length) % matches.length); }
      else if ((event.key === 'Enter' || event.key === 'Tab') && !event.shiftKey && !event.repeat) { event.preventDefault(); void choose(matches[Math.min(index, matches.length - 1)]); }
    };
    input.addEventListener('select', sync); input.addEventListener('input', sync); input.addEventListener('click', sync); input.addEventListener('keyup', sync);
    input.addEventListener('compositionstart', start); input.addEventListener('compositionend', end); input.addEventListener('keydown', keydown);
    if (open) { input.setAttribute('aria-controls', id); input.setAttribute('aria-expanded', 'true'); if (matches.length) input.setAttribute('aria-activedescendant', `${id}-${Math.min(index, matches.length - 1)}`); }
    return () => {
      input.removeEventListener('select', sync); input.removeEventListener('input', sync); input.removeEventListener('click', sync); input.removeEventListener('keyup', sync);
      input.removeEventListener('compositionstart', start); input.removeEventListener('compositionend', end); input.removeEventListener('keydown', keydown);
      input.removeAttribute('aria-controls'); input.removeAttribute('aria-expanded'); input.removeAttribute('aria-activedescendant');
    };
  });
  return <>
    {chosen.length ? <div className="agent-slash-selected" aria-label={t('agentSkills.selected')}>{chosen.map(item => <Button key={item.id} size="xs" variant="secondary" title={item.skill.path} disabled={!writable} onClick={() => { if (current()) useDevelopmentDrafts.setState(state => { const remaining = ((state.entries[key]?.value as CapturedAgentSkill[] | undefined) ?? []).filter(row => row.id !== item.id); const entries = { ...state.entries }; if (remaining.length) entries[key] = { kind: 'conversation', baseline: [], value: remaining }; else delete entries[key]; return { entries }; }); }}>{item.skill.name} · {t('agentSkills.remove')}</Button>)}</div> : null}
    {open ? <section className="agent-slash-picker" aria-label={t('agentSkills.slash.title')}>
      <header><strong>{t(engine === 'claude' ? 'agentSkills.commands.title' : 'agentSkills.title')}</strong><small>{t('agentSkills.slash.keyboard')}</small></header>
      <p>{t(engine === 'claude' ? 'agentSkills.commands.boundary' : 'agentSkills.boundary')}</p>
      {!session ? <p role="status">{t('agentSkills.slash.preparing')}</p> : !writable ? <p>{t('agentSkills.slash.unavailable')}</p> : loading ? <p role="status">{t('agentSkills.loading')}</p> : read?.error ? <p role="alert">{t('agentSkills.unavailable')}</p> : !matches.length ? <p>{t('agentSkills.slash.empty')}</p> : null}
      {read?.changed ? <p role="alert">{t('agentSkills.commands.changed')}</p> : null}
      <div id={id} role="listbox" aria-label={t('agentSkills.slash.title')}>
        {matches.map((candidate, position) => <button type="button" role="option" id={`${id}-${position}`} aria-selected={position === index} disabled={!writable || loading || candidate.kind === 'skill' && chosen.length >= 8 && !chosen.some(row => row.skill.path === candidate.skill.path && row.skill.name === candidate.name)} key={JSON.stringify([candidate.name, candidate.skill.path])} onMouseDown={event => event.preventDefault()} onClick={() => void choose(candidate)}>
          <strong>/{candidate.name}</strong><span>{candidate.skill.description || t(candidate.kind === 'skill' ? 'agentSkills.slash.skillDescription' : 'agentSkills.commands.addToDraft')}</span>
          <small>{candidate.skill.path || candidate.skill.source || candidate.skill.plugin_name || t('agentSkills.slash.unknownSource')}</small>
        </button>)}
      </div>
      {session ? <Button size="xs" variant="ghost" disabled={!writable || loading} onMouseDown={event => event.preventDefault()} onClick={() => void observe()}>{t('agentSkills.refresh')}</Button> : null}
    </section> : null}
  </>;
}
